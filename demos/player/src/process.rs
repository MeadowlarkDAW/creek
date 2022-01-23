use creek::read::ReadError;
use creek::{Decoder, ReadDiskStream, SeekMode, SymphoniaDecoder};
use rtrb::{Consumer, Producer};

use crate::{GuiToProcessMsg, ProcessToGuiMsg};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaybackState {
    Paused,
    Playing,
}

pub struct Process {
    read_disk_stream: Option<ReadDiskStream<SymphoniaDecoder>>,

    to_gui_tx: Producer<ProcessToGuiMsg>,
    from_gui_rx: Consumer<GuiToProcessMsg>,

    playback_state: PlaybackState,
    had_cache_miss_last_cycle: bool,

    loop_start: usize,
    loop_end: usize,

    fatal_error: bool,
}

impl Process {
    pub fn new(
        to_gui_tx: Producer<ProcessToGuiMsg>,
        from_gui_rx: Consumer<GuiToProcessMsg>,
    ) -> Self {
        Self {
            read_disk_stream: None,
            to_gui_tx,
            from_gui_rx,

            playback_state: PlaybackState::Paused,
            had_cache_miss_last_cycle: false,

            loop_start: 0,
            loop_end: 0,

            fatal_error: false,
        }
    }

    pub fn process(&mut self, data: &mut [f32]) {
        if self.fatal_error {
            silence(data);
            return;
        }

        match self.try_process(data) {
            Ok(_) => {}
            Err(e) => {
                match e {
                    ReadError::FatalError(_) => {
                        self.fatal_error = true;
                    }
                    _ => {}
                }

                println!("{:?}", e);
                silence(data);
            }
        }
    }

    fn try_process(
        &mut self,
        mut data: &mut [f32],
    ) -> Result<(), ReadError<<SymphoniaDecoder as Decoder>::FatalError>> {
        // Process messages from GUI.
        while let Ok(msg) = self.from_gui_rx.pop() {
            match msg {
                GuiToProcessMsg::UseStream(read_disk_stream) => {
                    self.playback_state = PlaybackState::Paused;
                    self.loop_start = 0;
                    self.loop_end = 0;
                    self.read_disk_stream = Some(read_disk_stream);
                }
                GuiToProcessMsg::SetLoop { start, end } => {
                    self.loop_start = start;
                    self.loop_end = end;

                    if start != 0 {
                        if let Some(read_disk_stream) = &mut self.read_disk_stream {
                            // cache loop starting position (cache_index 1 = loop start cache)
                            read_disk_stream.cache(1, start)?;
                        }
                    }
                }
                GuiToProcessMsg::PlayResume => {
                    self.playback_state = PlaybackState::Playing;
                }
                GuiToProcessMsg::Pause => {
                    self.playback_state = PlaybackState::Paused;
                }
                GuiToProcessMsg::Stop => {
                    self.playback_state = PlaybackState::Paused;

                    if let Some(read_disk_stream) = &mut self.read_disk_stream {
                        read_disk_stream.seek(self.loop_start, SeekMode::Auto)?;

                        let _ = self
                            .to_gui_tx
                            .push(ProcessToGuiMsg::PlaybackPos(read_disk_stream.playhead()));
                    }
                }
                GuiToProcessMsg::Restart => {
                    self.playback_state = PlaybackState::Playing;

                    if let Some(read_disk_stream) = &mut self.read_disk_stream {
                        read_disk_stream.seek(self.loop_start, SeekMode::Auto)?;
                    }
                }
                GuiToProcessMsg::SeekTo(pos) => {
                    if let Some(read_disk_stream) = &mut self.read_disk_stream {
                        read_disk_stream.seek(pos, SeekMode::Auto)?;
                    }
                }
            }
        }

        let mut cache_missed_this_cycle = false;
        if let Some(read_disk_stream) = &mut self.read_disk_stream {
            // Update client and check if it is ready.
            if !read_disk_stream.is_ready()? {
                cache_missed_this_cycle = true;
                // Warn UI of buffering.
                let _ = self.to_gui_tx.push(ProcessToGuiMsg::Buffering);

                // We can choose to either continue reading (which will return silence),
                // or pause playback until the buffer is filled. This demo uses the former.
            }

            if let PlaybackState::Paused = self.playback_state {
                // Paused, do nothing.
                silence(data);
                return Ok(());
            }

            let num_frames = read_disk_stream.info().num_frames;
            let num_channels = usize::from(read_disk_stream.info().num_channels);

            // Keep reading data until output buffer is filled.
            while data.len() >= num_channels {
                let read_frames = data.len() / 2;

                let mut playhead = read_disk_stream.playhead();

                // If user seeks ahead of the loop end, continue playing until the end
                // of the file.
                let loop_end = if playhead < self.loop_end {
                    self.loop_end
                } else {
                    num_frames
                };

                let read_data = read_disk_stream.read(read_frames)?;

                playhead += read_data.num_frames();
//                 if playhead >= loop_end {
//                     // Copy up to the end of the loop.
//                     let to_end_of_loop = read_data.num_frames() - (playhead - loop_end);
//
//                     if read_data.num_channels() == 1 {
//                         let ch = read_data.read_channel(0);
//
//                         for i in 0..to_end_of_loop {
//                             data[i * 2] = ch[i];
//                             data[(i * 2) + 1] = ch[i];
//                         }
//                     } else if read_data.num_channels() == 2 {
//                         let ch1 = read_data.read_channel(0);
//                         let ch2 = read_data.read_channel(1);
//
//                         for i in 0..to_end_of_loop {
//                             data[i * 2] = ch1[i];
//                             data[(i * 2) + 1] = ch2[i];
//                         }
//                     }
//
//                     read_disk_stream.seek(self.loop_start, SeekMode::Auto)?;
//
//                     data = &mut data[to_end_of_loop * 2..];
//                 } else {
                    // Else copy all the read data.
                    if read_data.num_channels() == 1 {
                        let ch = read_data.read_channel(0);

                        for i in 0..read_data.num_frames() {
                            data[i * 2] = ch[i];
                            data[(i * 2) + 1] = ch[i];
                        }
                    } else if read_data.num_channels() == 2 {
                        let ch1 = read_data.read_channel(0);
                        let ch2 = read_data.read_channel(1);

                        for i in 0..read_data.num_frames() {
                            data[i * 2] = ch1[i];
                            data[(i * 2) + 1] = ch2[i];
                        }
                    }

                    data = &mut data[read_data.num_frames() * 2..];
//                 }
            }

            let _ = self
                .to_gui_tx
                .push(ProcessToGuiMsg::PlaybackPos(read_disk_stream.playhead()));
        } else {
            // Output silence until file is recieved.
            silence(data);
        }

        // When the cache misses, the buffer is filled with silence. So the next
        // buffer after the cache miss is starting from silence. To avoid an audible
        // pop, apply a ramping gain from 0 up to unity.
        if self.had_cache_miss_last_cycle {
            let buffer_size = data.len() as f32;
            for (i, sample) in data.iter_mut().enumerate() {
                *sample *= i as f32 / buffer_size;
            }
        }

        self.had_cache_miss_last_cycle = cache_missed_this_cycle;
        Ok(())
    }
}

fn silence(data: &mut [f32]) {
    for sample in data.iter_mut() {
        *sample = 0.0;
    }
}

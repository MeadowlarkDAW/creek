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

        if let Err(e) = self.try_process(data) {
            if matches!(e, ReadError::FatalError(_)) {
                self.fatal_error = true;
            }

            println!("{:?}", e);
            silence(data);
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

        if let Some(read_disk_stream) = &mut self.read_disk_stream {
            // Update client and check if it is ready.
            if !read_disk_stream.is_ready()? {
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

                let read_res = read_disk_stream.read(read_frames)?;

                playhead += read_res.frames;
                if playhead >= loop_end {
                    // Copy up to the end of the loop.
                    let to_end_of_loop = read_res.frames - (playhead - loop_end);

                    if read_res.channels.len() == 1 {
                        let src_part = &read_res.channels[0][0..to_end_of_loop];

                        copy_stereo_into_interleaved_buffer(src_part, src_part, data);
                    } else if read_res.channels.len() == 2 {
                        let src_ch_1 = &read_res.channels[0][0..to_end_of_loop];
                        let src_ch_2 = &read_res.channels[1][0..to_end_of_loop];

                        copy_stereo_into_interleaved_buffer(src_ch_1, src_ch_2, data);
                    }

                    read_disk_stream.seek(self.loop_start, SeekMode::Auto)?;

                    data = &mut data[to_end_of_loop * 2..];
                } else {
                    // Else copy all the read data.
                    if read_res.channels.len() == 1 {
                        let src_part = &read_res.channels[0][0..read_res.frames];

                        copy_stereo_into_interleaved_buffer(src_part, src_part, data);
                    } else if read_res.channels.len() == 2 {
                        let src_ch_1 = &read_res.channels[0][0..read_res.frames];
                        let src_ch_2 = &read_res.channels[1][0..read_res.frames];

                        copy_stereo_into_interleaved_buffer(src_ch_1, src_ch_2, data);
                    }

                    data = &mut data[read_res.frames * 2..];
                }
            }

            let _ = self
                .to_gui_tx
                .push(ProcessToGuiMsg::PlaybackPos(read_disk_stream.playhead()));
        } else {
            // Output silence until file is received.
            silence(data);
        }

        Ok(())
    }
}

fn silence(data: &mut [f32]) {
    data.fill(0.0);
}

fn copy_stereo_into_interleaved_buffer(src_ch_1: &[f32], src_ch_2: &[f32], dst: &mut [f32]) {
    let src_ch_2 = &src_ch_2[0..src_ch_1.len()];
    let dst = &mut dst[0..src_ch_1.len() * 2];

    for (i, dst_frame) in dst.chunks_exact_mut(2).enumerate() {
        dst_frame[0] = src_ch_1[i];
        dst_frame[1] = src_ch_2[i];
    }
}

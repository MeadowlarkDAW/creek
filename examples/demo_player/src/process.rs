use eframe::egui::paint::stats;
use rt_audio_disk_stream::ReadClient;
use rtrb::{Consumer, Producer};

use crate::{GuiToProcessMsg, ProcessToGuiMsg};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransportState {
    Paused,
    Playing,
}

pub struct Process {
    read_client: Option<ReadClient>,

    to_gui_tx: Producer<ProcessToGuiMsg>,
    from_gui_rx: Consumer<GuiToProcessMsg>,

    transport_pos: usize,
    transport_len: usize,
    transport_state: TransportState,

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
            read_client: None,
            to_gui_tx,
            from_gui_rx,

            transport_pos: 0,
            transport_len: 0,
            transport_state: TransportState::Paused,

            loop_start: 0,
            loop_end: 0,

            fatal_error: false,
        }
    }

    pub fn process(&mut self, mut data: &mut [f32]) {
        if self.fatal_error {
            silence(data);
            return;
        }

        // Process messages from GUI.
        while let Ok(msg) = self.from_gui_rx.pop() {
            match msg {
                GuiToProcessMsg::UseStream(read_client) => {
                    self.transport_pos = 0;
                    self.transport_state = TransportState::Paused;
                    self.transport_len = read_client.info().num_frames;

                    self.loop_start = 0;
                    self.loop_end = 0;

                    self.read_client = Some(read_client);
                }
                GuiToProcessMsg::SetLoop { start, end } => {
                    self.loop_start = start;
                    self.loop_end = end;

                    if start != 0 {
                        if let Some(read_client) = &mut self.read_client {
                            // cache loop starting position (cache_index 1 == loop start cache)
                            if let Err(e) = read_client.cache(1, start) {
                                println!("{:?}", e);
                                self.fatal_error = true;
                                return;
                            }
                        }
                    }
                }
                GuiToProcessMsg::PlayResume => {
                    self.transport_state = TransportState::Playing;
                }
                GuiToProcessMsg::Pause => {
                    self.transport_state = TransportState::Paused;
                }
                GuiToProcessMsg::Stop => {
                    self.transport_state = TransportState::Paused;

                    if let Err(e) = self.go_to_loop_start() {
                        println!("{:?}", e);
                        self.fatal_error = true;
                        return;
                    }
                }
                GuiToProcessMsg::Restart => {
                    self.transport_state = TransportState::Playing;

                    if let Err(e) = self.go_to_loop_start() {
                        println!("{:?}", e);
                        self.fatal_error = true;
                        return;
                    }
                }
                GuiToProcessMsg::SeekTo(pos) => {
                    if pos != self.transport_pos {
                        if pos == self.loop_start {
                            if let Err(e) = self.go_to_loop_start() {
                                println!("{:?}", e);
                                self.fatal_error = true;
                                return;
                            }
                        } else {
                            self.transport_pos = pos;

                            if let Some(read_client) = &mut self.read_client {
                                // cache-index 2 == temporary cache
                                if let Err(e) = read_client.seek_to(2, pos) {
                                    println!("{:?}", e);
                                    self.fatal_error = true;
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Update client and check if it is ready.
        let is_ready = if let Some(read_client) = &mut self.read_client {
            match read_client.is_ready() {
                Ok(ready) => ready,
                Err(e) => {
                    println!("{:?}", e);
                    self.fatal_error = true;
                    return;
                }
            }
        } else {
            false
        };

        if let Some(read_client) = &mut self.read_client {
            if let TransportState::Paused = self.transport_state {
                // Do nothing
                silence(data);
                return;
            }

            if !is_ready {
                println!("buffering");
                // TODO: Warn UI of buffering.
                silence(data);
                return;
            }

            while data.len() > 1 {
                let read_frames = (data.len() / 2).min(rt_audio_disk_stream::BLOCK_SIZE);

                match read_client.read(read_frames) {
                    Ok(read_data) => {
                        // Advance the playing position.
                        self.transport_pos += read_data.num_frames();
                        if self.transport_pos >= self.loop_end {
                            // Copy up to the end of the loop.

                            let to_end_of_loop =
                                read_data.num_frames() - (self.transport_pos - self.loop_end);

                            if read_data.num_channels() == 1 {
                                let ch = read_data.read_channel(0);

                                for i in 0..to_end_of_loop {
                                    data[i * 2] = ch[i];
                                    data[(i * 2) + 1] = ch[i];
                                }
                            } else if read_data.num_channels() == 2 {
                                let ch1 = read_data.read_channel(0);
                                let ch2 = read_data.read_channel(1);

                                for i in 0..to_end_of_loop {
                                    data[i * 2] = ch1[i];
                                    data[(i * 2) + 1] = ch2[i];
                                }
                            }

                            // Rust's borrow checker is tricky, so just inline `go_to_loop_start()` here.
                            if let Err(e) = {
                                self.transport_pos = self.loop_start;

                                let cache_index = if self.loop_start == 0 {
                                    // Use the start-of-file cache that the UI cached before-hand.
                                    0
                                } else {
                                    // Use the start-of-loop cache.
                                    1
                                };

                                read_client.seek_to(cache_index, self.loop_start)
                            } {
                                println!("{:?}", e);
                                self.fatal_error = true;
                                return;
                            }

                            data = &mut data[to_end_of_loop * 2..];
                        } else {
                            // Copy all the frames.

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
                        }
                    }
                    Err(e) => {
                        // Stop playback on error.
                        println!("{:?}", e);
                        break;
                    }
                }
            }

            let _ = self
                .to_gui_tx
                .push(ProcessToGuiMsg::TransportPos(self.transport_pos));
        } else {
            // Output silence until file is recieved.
            silence(data)
        }

        // Noise test
        /*
        for sample in data.iter_mut() {
            *sample = (self.rng.rand_float() * 2.0) - 1.0;
        }
        */
    }

    fn go_to_loop_start(&mut self) -> Result<bool, rt_audio_disk_stream::ReadError> {
        self.transport_pos = self.loop_start;

        let cache_index = if self.loop_start == 0 {
            // Use the start-of-file cache that the UI cached before-hand.
            0
        } else {
            // Use the start-of-loop cache.
            1
        };

        if let Some(read_client) = &mut self.read_client {
            return read_client.seek_to(cache_index, self.loop_start);
        }

        Ok(false)
    }
}

fn silence(data: &mut [f32]) {
    for sample in data.iter_mut() {
        *sample = 0.0;
    }
}

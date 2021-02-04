use rt_audio_disk_stream::ReadClient;
use rtrb::{Consumer, Producer};

use crate::{GuiToProcessMsg, ProcessToGuiMsg};

pub struct Process {
    read_client: Option<ReadClient>,

    to_gui_tx: Producer<ProcessToGuiMsg>,
    from_gui_rx: Consumer<GuiToProcessMsg>,
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
        }
    }

    pub fn process(&mut self, mut data: &mut [f32]) {
        // Process messages from GUI.
        while let Ok(msg) = self.from_gui_rx.pop() {
            match msg {
                GuiToProcessMsg::PlayStream(read_client) => {
                    self.read_client = Some(read_client);
                }
            }
        }

        if let Some(read_client) = &mut self.read_client {
            while data.len() > 1 {
                let read_frames = (data.len() / 2).min(rt_audio_disk_stream::BLOCK_SIZE);

                match read_client.read(read_frames) {
                    Ok(read_data) => {
                        if read_data.num_channels() == 1 {
                            let ch = read_data.read_channel(0);

                            for i in 0..read_frames {
                                data[i * 2] = ch[i];
                                data[(i * 2) + 1] = ch[i];
                            }
                        } else if read_data.num_channels() == 2 {
                            let ch1 = read_data.read_channel(0);
                            let ch2 = read_data.read_channel(1);

                            for i in 0..read_frames {
                                data[i * 2] = ch1[i];
                                data[(i * 2) + 1] = ch2[i];
                            }
                        }
                    }
                    Err(e) => {
                        // Stop playback on error.
                        println!("{:?}", e);
                        break;
                    }
                }

                data = &mut data[read_frames * 2..];
            }
        } else {
            // Output silence until file is recieved.
            for sample in data.iter_mut() {
                *sample = 0.0;
            }
        }

        // Noise test
        /*
        for sample in data.iter_mut() {
            *sample = (self.rng.rand_float() * 2.0) - 1.0;
        }
        */
    }
}

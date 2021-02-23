use rt_audio_disk_stream::error::WavFatalError;
use rt_audio_disk_stream::write::WriteError;
use rt_audio_disk_stream::{wav_bit_depth, WavEncoder, WriteDiskStream};
use rtrb::{Consumer, Producer};

use crate::{GuiToProcessMsg, ProcessToGuiMsg};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaybackState {
    Paused,
    Playing,
}

pub struct Process {
    write_stream_u8: Option<WriteDiskStream<WavEncoder<wav_bit_depth::Uint8>>>,
    write_stream_i16: Option<WriteDiskStream<WavEncoder<wav_bit_depth::Int16>>>,
    write_stream_i24: Option<WriteDiskStream<WavEncoder<wav_bit_depth::Int24>>>,
    write_stream_f32: Option<WriteDiskStream<WavEncoder<wav_bit_depth::Float32>>>,

    to_gui_tx: Producer<ProcessToGuiMsg>,
    from_gui_rx: Consumer<GuiToProcessMsg>,

    playback_state: PlaybackState,

    fatal_error: bool,
    finished: bool,
}

impl Process {
    pub fn new(
        to_gui_tx: Producer<ProcessToGuiMsg>,
        from_gui_rx: Consumer<GuiToProcessMsg>,
    ) -> Self {
        Self {
            write_stream_u8: None,
            write_stream_i16: None,
            write_stream_i24: None,
            write_stream_f32: None,
            to_gui_tx,
            from_gui_rx,

            playback_state: PlaybackState::Paused,

            fatal_error: false,
            finished: false,
        }
    }

    pub fn process(&mut self, data: &mut [f32]) {
        match self.try_process(data) {
            Ok(_) => {}
            Err(e) => {
                match e {
                    WriteError::FatalError(_) => {
                        self.fatal_error = true;
                        let _ = self.to_gui_tx.push(ProcessToGuiMsg::FatalError);
                    }
                    _ => {}
                }

                println!("{:?}", e);
                silence(data);
            }
        }
    }

    fn try_process(&mut self, mut data: &mut [f32]) -> Result<(), WriteError<WavFatalError>> {
        // Process messages from GUI.
        while let Ok(msg) = self.from_gui_rx.pop() {
            match msg {
                GuiToProcessMsg::UseStreamU8(write_stream) => {
                    if let Some(_) = self.write_stream_u8.take() {
                        self.write_stream_u8 = Some(write_stream);
                    }
                    let _ = self.write_stream_i16.take();
                    let _ = self.write_stream_i24.take();
                    let _ = self.write_stream_f32.take();

                    self.fatal_error = false;
                    self.finished = false;
                    self.playback_state = PlaybackState::Paused;
                }
                GuiToProcessMsg::UseStreamInt16(write_stream) => {
                    let _ = self.write_stream_u8.take();
                    if let Some(_) = self.write_stream_i16.take() {
                        self.write_stream_i16 = Some(write_stream);
                    }
                    let _ = self.write_stream_i24.take();
                    let _ = self.write_stream_f32.take();

                    self.fatal_error = false;
                    self.finished = false;
                    self.playback_state = PlaybackState::Paused;
                }
                GuiToProcessMsg::UseStreamInt24(write_stream) => {
                    let _ = self.write_stream_u8.take();
                    let _ = self.write_stream_i16.take();
                    if let Some(_) = self.write_stream_i24.take() {
                        self.write_stream_i24 = Some(write_stream);
                    }
                    let _ = self.write_stream_f32.take();

                    self.fatal_error = false;
                    self.finished = false;
                    self.playback_state = PlaybackState::Paused;
                }
                GuiToProcessMsg::UseStreamFloat32(write_stream) => {
                    let _ = self.write_stream_u8.take();
                    let _ = self.write_stream_i16.take();
                    let _ = self.write_stream_i24.take();
                    if let Some(_) = self.write_stream_f32.take() {
                        self.write_stream_f32 = Some(write_stream);
                    }

                    self.fatal_error = false;
                    self.finished = false;
                    self.playback_state = PlaybackState::Paused;
                }
                GuiToProcessMsg::PlayResume => {
                    if !self.fatal_error && !self.finished {
                        self.playback_state = PlaybackState::Playing;
                    }
                }
                GuiToProcessMsg::Pause => {
                    self.playback_state = PlaybackState::Paused;
                }
                GuiToProcessMsg::Finish => {
                    if !self.fatal_error && !self.finished {
                        if let Some(mut write_stream) = self.write_stream_u8.take() {
                            write_stream.finish_and_close()?;
                        }
                        if let Some(mut write_stream) = self.write_stream_i16.take() {
                            write_stream.finish_and_close()?;
                        }
                        if let Some(mut write_stream) = self.write_stream_i24.take() {
                            write_stream.finish_and_close()?;
                        }
                        if let Some(mut write_stream) = self.write_stream_f32.take() {
                            write_stream.finish_and_close()?;
                        }
                    }
                    self.finished = true;
                }
                GuiToProcessMsg::Discard => {
                    if !self.fatal_error && !self.finished {
                        if let Some(mut write_stream) = self.write_stream_u8.take() {
                            write_stream.discard_and_close()?;
                        }
                        if let Some(mut write_stream) = self.write_stream_i16.take() {
                            write_stream.discard_and_close()?;
                        }
                        if let Some(mut write_stream) = self.write_stream_i24.take() {
                            write_stream.discard_and_close()?;
                        }
                        if let Some(mut write_stream) = self.write_stream_f32.take() {
                            write_stream.discard_and_close()?;
                        }
                    }
                    self.finished = true;
                }
                GuiToProcessMsg::DiscardAndRestart => {
                    if !self.fatal_error && !self.finished {
                        if let Some(mut write_stream) = self.write_stream_u8.take() {
                            write_stream.discard_and_restart()?;
                        }
                        if let Some(mut write_stream) = self.write_stream_i16.take() {
                            write_stream.discard_and_restart()?;
                        }
                        if let Some(mut write_stream) = self.write_stream_i24.take() {
                            write_stream.discard_and_restart()?;
                        }
                        if let Some(mut write_stream) = self.write_stream_f32.take() {
                            write_stream.discard_and_restart()?;
                        }
                    }
                }
            }
        }

        silence(data);

        Ok(())
    }
}

fn silence(data: &mut [f32]) {
    for sample in data.iter_mut() {
        *sample = 0.0;
    }
}

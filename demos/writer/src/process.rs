use creek::error::WavFatalError;
use creek::write::WriteError;
use creek::{wav_bit_depth, WavEncoder, WriteDiskStream};
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
    freq: f32,
    one_over_sample_rate: f32,
    phase: f32,

    fatal_error: bool,
    finished: bool,

    f32_buffer: Vec<f32>,
    u8_buffer: Vec<u8>,
    i16_buffer: Vec<i16>,
    i24_buffer: Vec<i32>,
}

impl Process {
    pub fn new(
        to_gui_tx: Producer<ProcessToGuiMsg>,
        from_gui_rx: Consumer<GuiToProcessMsg>,
        sample_rate: u32,
    ) -> Self {
        Self {
            write_stream_u8: None,
            write_stream_i16: None,
            write_stream_i24: None,
            write_stream_f32: None,
            to_gui_tx,
            from_gui_rx,

            playback_state: PlaybackState::Paused,
            freq: 261.626,
            one_over_sample_rate: 1.0 / sample_rate as f32,

            fatal_error: false,
            finished: false,
            phase: 0.0,

            // Probably overkill with the capacity, but just make sure
            // this never allocates new data for realtime safety.
            f32_buffer: Vec::with_capacity(44100),
            u8_buffer: Vec::with_capacity(44100),
            i16_buffer: Vec::with_capacity(44100),
            i24_buffer: Vec::with_capacity(44100),
        }
    }

    pub fn process(&mut self, data: &mut [f32]) {
        if let Err(e) = self.try_process(data) {
            if matches!(e, WriteError::FatalError(_)) {
                self.fatal_error = true;
                let _ = self.to_gui_tx.push(ProcessToGuiMsg::FatalError);
            }

            println!("{:?}", e);
            silence(data);
        }
    }

    fn try_process(&mut self, data: &mut [f32]) -> Result<(), WriteError<WavFatalError>> {
        // Process messages from GUI.
        while let Ok(msg) = self.from_gui_rx.pop() {
            match msg {
                GuiToProcessMsg::SetFreq(freq) => {
                    self.freq = freq;
                }
                GuiToProcessMsg::UseStreamUint8(write_stream) => {
                    self.write_stream_u8 = Some(write_stream);
                    let _ = self.write_stream_i16.take();
                    let _ = self.write_stream_i24.take();
                    let _ = self.write_stream_f32.take();

                    self.fatal_error = false;
                    self.finished = false;
                    self.phase = 0.0;
                    self.playback_state = PlaybackState::Paused;
                }
                GuiToProcessMsg::UseStreamInt16(write_stream) => {
                    let _ = self.write_stream_u8.take();
                    self.write_stream_i16 = Some(write_stream);
                    let _ = self.write_stream_i24.take();
                    let _ = self.write_stream_f32.take();

                    self.fatal_error = false;
                    self.finished = false;
                    self.phase = 0.0;
                    self.playback_state = PlaybackState::Paused;
                }
                GuiToProcessMsg::UseStreamInt24(write_stream) => {
                    let _ = self.write_stream_u8.take();
                    let _ = self.write_stream_i16.take();
                    self.write_stream_i24 = Some(write_stream);
                    let _ = self.write_stream_f32.take();

                    self.fatal_error = false;
                    self.finished = false;
                    self.phase = 0.0;
                    self.playback_state = PlaybackState::Paused;
                }
                GuiToProcessMsg::UseStreamFloat32(write_stream) => {
                    let _ = self.write_stream_u8.take();
                    let _ = self.write_stream_i16.take();
                    let _ = self.write_stream_i24.take();
                    self.write_stream_f32 = Some(write_stream);

                    self.fatal_error = false;
                    self.finished = false;
                    self.phase = 0.0;
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
                        if let Some(write_stream) = &mut self.write_stream_u8 {
                            write_stream.discard_and_restart()?;
                        }
                        if let Some(write_stream) = &mut self.write_stream_i16 {
                            write_stream.discard_and_restart()?;
                        }
                        if let Some(write_stream) = &mut self.write_stream_i24 {
                            write_stream.discard_and_restart()?;
                        }
                        if let Some(write_stream) = &mut self.write_stream_f32 {
                            write_stream.discard_and_restart()?;
                        }
                    }
                }
            }
        }

        if self.playback_state == PlaybackState::Playing && !self.finished && !self.fatal_error {
            self.f32_buffer.clear();

            // Only supporting stereo output for this demo.
            let num_frames = data.len() / 2;

            assert!(num_frames <= self.f32_buffer.capacity());

            // Generate sine wav into data.
            let amp: f32 = 0.85;
            let phase_inc = std::f32::consts::TAU * self.freq * self.one_over_sample_rate;
            for _ in 0..num_frames {
                let sine = self.phase.sin();
                self.phase += phase_inc;
                self.f32_buffer.push(amp * sine);
            }
            while self.phase >= std::f32::consts::TAU {
                self.phase -= std::f32::consts::TAU;
            }

            // Copy data into output.
            copy_mono_into_interleaved_stereo_buffer(&self.f32_buffer[0..num_frames], data);

            // Send data to be written to file.
            if let Some(write_stream) = &mut self.write_stream_u8 {
                self.u8_buffer.clear();
                assert!(num_frames <= self.u8_buffer.capacity());

                // Convert to u8
                for i in 0..num_frames {
                    self.u8_buffer
                        .push((((self.f32_buffer[i] + 1.0) / 2.0) * std::u8::MAX as f32) as u8);
                }

                write_stream.write(&[&self.u8_buffer, &self.u8_buffer])?;
            }
            if let Some(write_stream) = &mut self.write_stream_i16 {
                self.i16_buffer.clear();
                assert!(num_frames <= self.i16_buffer.capacity());

                // Convert to i16
                for i in 0..num_frames {
                    self.i16_buffer
                        .push((self.f32_buffer[i] * std::i16::MAX as f32) as i16);
                }

                write_stream.write(&[&self.i16_buffer, &self.i16_buffer])?;
            }
            if let Some(write_stream) = &mut self.write_stream_i24 {
                self.i24_buffer.clear();
                assert!(num_frames <= self.i24_buffer.capacity());

                // Convert to i24
                for i in 0..num_frames {
                    self.i24_buffer
                        .push((self.f32_buffer[i] * 0x7FFFFF as f32) as i32);
                }

                write_stream.write(&[&self.i24_buffer, &self.i24_buffer])?;
            }
            if let Some(write_stream) = &mut self.write_stream_f32 {
                write_stream.write(&[&self.f32_buffer, &self.f32_buffer])?;
            }
        } else {
            silence(data);
        }

        Ok(())
    }
}

fn silence(data: &mut [f32]) {
    data.fill(0.0);
}

fn copy_mono_into_interleaved_stereo_buffer(src: &[f32], dst: &mut [f32]) {
    let dst = &mut dst[0..src.len() * 2];

    for (i, dst_frame) in dst.chunks_exact_mut(2).enumerate() {
        dst_frame[0] = src[i];
        dst_frame[1] = src[i];
    }
}

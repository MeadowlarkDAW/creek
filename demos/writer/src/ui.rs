use creek::{wav_bit_depth, WavEncoder, WriteDiskStream};
use eframe::{egui, epi};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::{GuiToProcessMsg, ProcessToGuiMsg};

#[derive(Debug, Clone, Copy, PartialEq)]
enum BitRate {
    Uint8,
    Int16,
    Int24,
    Float32,
}

pub struct DemoWriterApp {
    file_active: bool,
    bit_rate: BitRate,

    playing: bool,
    written_frames: usize,

    to_player_tx: Producer<GuiToProcessMsg>,
    from_player_rx: Consumer<ProcessToGuiMsg>,

    frame_close_tx: Producer<()>,
    frame_close_rx: Option<Consumer<()>>,

    freq: f32,
    sample_rate: u32,

    fatal_error: bool,
}

impl DemoWriterApp {
    pub fn new(
        to_player_tx: Producer<GuiToProcessMsg>,
        from_player_rx: Consumer<ProcessToGuiMsg>,
        sample_rate: u32,
    ) -> Self {
        let (frame_close_tx, frame_close_rx) = RingBuffer::new(1);

        Self {
            file_active: false,
            bit_rate: BitRate::Int24,
            playing: false,
            written_frames: 0,
            to_player_tx,
            from_player_rx,
            frame_close_tx,
            frame_close_rx: Some(frame_close_rx),
            freq: 261.626,
            sample_rate,
            fatal_error: false,
        }
    }
}

impl epi::App for DemoWriterApp {
    fn name(&self) -> &str {
        "rt-audio-disk-stream demo writer"
    }

    fn update(&mut self, ctx: &egui::CtxRef, frame: &mut epi::Frame<'_>) {
        if let Some(mut frame_close_rx) = self.frame_close_rx.take() {
            // Spawn thread that calls a repaint 60 times a second.

            let repaint_signal = frame.repaint_signal().clone();

            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs_f64(1.0 / 60.0));

                    // Check if app has closed.
                    if let Ok(_) = frame_close_rx.pop() {
                        break;
                    }

                    repaint_signal.request_repaint();
                }
            });
        }

        while let Ok(msg) = self.from_player_rx.pop() {
            match msg {
                ProcessToGuiMsg::FramesWritten(frames_written) => {
                    self.written_frames = frames_written;
                }
                ProcessToGuiMsg::FatalError => {
                    self.fatal_error = true;
                    self.playing = false;
                    self.file_active = false;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::warn_if_debug_build(ui);

            if self.file_active {
                ui.horizontal(|ui| {
                    let play_label = if self.playing { "Pause" } else { "Play" };
                    if ui.button(play_label).clicked() && !self.fatal_error {
                        if self.playing {
                            self.playing = false;

                            let _ = self.to_player_tx.push(GuiToProcessMsg::Pause);
                        } else {
                            self.playing = true;

                            let _ = self.to_player_tx.push(GuiToProcessMsg::PlayResume);
                        }
                    }

                    if ui.button("Finish").clicked() && !self.fatal_error {
                        self.playing = false;
                        self.fatal_error = false;
                        let _ = self.to_player_tx.push(GuiToProcessMsg::Finish);
                        self.file_active = false;
                    }

                    if ui.button("Discard").clicked() && !self.fatal_error {
                        self.playing = false;
                        self.fatal_error = false;
                        let _ = self.to_player_tx.push(GuiToProcessMsg::Discard);
                        self.file_active = false;
                    }

                    if ui.button("Discard And Restart").clicked() && !self.fatal_error {
                        self.playing = false;
                        let _ = self.to_player_tx.push(GuiToProcessMsg::DiscardAndRestart);
                    }
                });

                if ui
                    .add(
                        egui::Slider::f32(&mut self.freq, 30.0..=20_000.0)
                            .logarithmic(true)
                            .text("pitch"),
                    )
                    .dragged()
                {
                    let _ = self.to_player_tx.push(GuiToProcessMsg::SetFreq(self.freq));
                }
            } else {
                if ui
                    .add(egui::RadioButton::new(
                        self.bit_rate == BitRate::Uint8,
                        "Uint8",
                    ))
                    .clicked()
                {
                    self.bit_rate = BitRate::Uint8;
                }
                if ui
                    .add(egui::RadioButton::new(
                        self.bit_rate == BitRate::Int16,
                        "Int16",
                    ))
                    .clicked()
                {
                    self.bit_rate = BitRate::Int16;
                }
                if ui
                    .add(egui::RadioButton::new(
                        self.bit_rate == BitRate::Int24,
                        "Int24",
                    ))
                    .clicked()
                {
                    self.bit_rate = BitRate::Int24;
                }
                if ui
                    .add(egui::RadioButton::new(
                        self.bit_rate == BitRate::Float32,
                        "Float32",
                    ))
                    .clicked()
                {
                    self.bit_rate = BitRate::Float32;
                }

                if ui.button("Create File").clicked() {
                    self.playing = false;

                    match self.bit_rate {
                        BitRate::Uint8 => {
                            let write_stream =
                                WriteDiskStream::<WavEncoder<wav_bit_depth::Uint8>>::new(
                                    "./examples/demo_writer/out_files/output.wav",
                                    2,
                                    self.sample_rate,
                                    Default::default(),
                                )
                                .unwrap();

                            self.to_player_tx
                                .push(GuiToProcessMsg::UseStreamUint8(write_stream))
                                .unwrap();
                        }
                        BitRate::Int16 => {
                            let write_stream =
                                WriteDiskStream::<WavEncoder<wav_bit_depth::Int16>>::new(
                                    "./examples/demo_writer/out_files/output.wav",
                                    2,
                                    self.sample_rate,
                                    Default::default(),
                                )
                                .unwrap();

                            self.to_player_tx
                                .push(GuiToProcessMsg::UseStreamInt16(write_stream))
                                .unwrap();
                        }
                        BitRate::Int24 => {
                            let write_stream =
                                WriteDiskStream::<WavEncoder<wav_bit_depth::Int24>>::new(
                                    "./examples/demo_writer/out_files/output.wav",
                                    2,
                                    self.sample_rate,
                                    Default::default(),
                                )
                                .unwrap();

                            self.to_player_tx
                                .push(GuiToProcessMsg::UseStreamInt24(write_stream))
                                .unwrap();
                        }
                        BitRate::Float32 => {
                            let write_stream =
                                WriteDiskStream::<WavEncoder<wav_bit_depth::Float32>>::new(
                                    "./examples/demo_writer/out_files/output.wav",
                                    2,
                                    self.sample_rate,
                                    Default::default(),
                                )
                                .unwrap();

                            self.to_player_tx
                                .push(GuiToProcessMsg::UseStreamFloat32(write_stream))
                                .unwrap();
                        }
                    }

                    self.file_active = true;
                }
            }
        });
    }
}

impl Drop for DemoWriterApp {
    fn drop(&mut self) {
        self.frame_close_tx.push(()).unwrap();
    }
}

use eframe::{egui, epi};
use rt_audio_disk_stream::{wav_bit_depth, WavEncoder, WriteDiskStream};
use rtrb::{Consumer, Producer, RingBuffer};

use crate::{GuiToProcessMsg, ProcessToGuiMsg};

pub struct DemoWriterApp {
    playing: bool,
    written_frames: usize,

    to_player_tx: Producer<GuiToProcessMsg>,
    from_player_rx: Consumer<ProcessToGuiMsg>,

    frame_close_tx: Producer<()>,
    frame_close_rx: Option<Consumer<()>>,

    fatal_error: bool,
}

impl DemoWriterApp {
    pub fn new(
        mut to_player_tx: Producer<GuiToProcessMsg>,
        from_player_rx: Consumer<ProcessToGuiMsg>,
        sample_rate: u32,
    ) -> Self {
        // Setup write stream -------------------------------------------------------------

        // Open the write stream.
        let write_stream = WriteDiskStream::<WavEncoder<wav_bit_depth::Float32>>::new(
            "./examples/demo_writer/out_files/test.wav",
            2,
            sample_rate,
            Default::default(),
        )
        .unwrap();

        to_player_tx
            .push(GuiToProcessMsg::UseStreamFloat32(write_stream))
            .unwrap();

        let (frame_close_tx, frame_close_rx) = RingBuffer::new(1).split();

        Self {
            playing: false,
            written_frames: 0,
            to_player_tx,
            from_player_rx,
            frame_close_tx,
            frame_close_rx: Some(frame_close_rx),
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
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::warn_if_debug_build(ui);

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
                    let _ = self.to_player_tx.push(GuiToProcessMsg::Finish);
                }

                if ui.button("Discard").clicked() && !self.fatal_error {
                    self.playing = false;
                    let _ = self.to_player_tx.push(GuiToProcessMsg::Discard);
                }

                if ui.button("Discard And Restart").clicked() && !self.fatal_error {
                    self.playing = true;
                    let _ = self.to_player_tx.push(GuiToProcessMsg::DiscardAndRestart);
                }
            });
        });
    }
}

impl Drop for DemoWriterApp {
    fn drop(&mut self) {
        self.frame_close_tx.push(()).unwrap();
    }
}

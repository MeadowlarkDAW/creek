use eframe::{egui, epi};
use rt_audio_disk_stream::AudioDiskStream;
use rtrb::{Consumer, Producer, RingBuffer};

use crate::{GuiToProcessMsg, ProcessToGuiMsg};

static BUFFERING_FADEOUT_FRAMES: usize = 25;

pub struct DemoPlayerApp {
    playing: bool,
    current_frame: usize,
    num_frames: usize,
    transport_control: TransportControl,

    to_player_tx: Producer<GuiToProcessMsg>,
    from_player_rx: Consumer<ProcessToGuiMsg>,

    frame_close_tx: Producer<()>,
    frame_close_rx: Option<Consumer<()>>,

    loop_start: usize,
    loop_end: usize,

    buffering_anim: usize,

    cache_size: usize,
}

impl DemoPlayerApp {
    pub fn new(
        mut to_player_tx: Producer<GuiToProcessMsg>,
        from_player_rx: Consumer<ProcessToGuiMsg>,
    ) -> Self {
        let opts = rt_audio_disk_stream::StreamOptions {
            num_caches: 2,
            ..Default::default()
        };

        let cache_size = opts.num_cache_blocks * rt_audio_disk_stream::BLOCK_SIZE;

        let mut test_client =
            AudioDiskStream::open_read("./test_files/wav_i24_stereo.wav", 0, opts).unwrap();

        // Cache the start of the file and store it in cache number 0.
        test_client.seek_to(0, Some(0)).unwrap();
        test_client.block_until_ready().unwrap();

        let num_frames = test_client.info().num_frames;

        to_player_tx
            .push(GuiToProcessMsg::UseStream(test_client))
            .unwrap();

        let loop_start = 0;
        let loop_end = num_frames;

        to_player_tx
            .push(GuiToProcessMsg::SetLoop {
                start: loop_start,
                end: loop_end,
            })
            .unwrap();

        let (frame_close_tx, frame_close_rx) = RingBuffer::new(1).split();

        Self {
            playing: false,
            current_frame: 0,
            num_frames,
            transport_control: Default::default(),

            frame_close_tx,
            frame_close_rx: Some(frame_close_rx),

            to_player_tx,
            from_player_rx,

            loop_start,
            loop_end,

            buffering_anim: 0,

            cache_size,
        }
    }
}

impl epi::App for DemoPlayerApp {
    fn name(&self) -> &str {
        "rt-audio-disk-stream demo player"
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
                ProcessToGuiMsg::TransportPos(pos) => {
                    self.current_frame = pos;
                }
                ProcessToGuiMsg::Buffering => {
                    self.buffering_anim = BUFFERING_FADEOUT_FRAMES;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::warn_if_debug_build(ui);

            ui.horizontal(|ui| {
                let play_label = if self.playing { "Pause" } else { "Play" };
                if ui.button(play_label).clicked {
                    if self.playing {
                        self.playing = false;

                        let _ = self.to_player_tx.push(GuiToProcessMsg::Pause);
                    } else {
                        self.playing = true;

                        let _ = self.to_player_tx.push(GuiToProcessMsg::PlayResume);
                    }
                }

                if ui.button("Stop").clicked {
                    self.playing = false;
                    let _ = self.to_player_tx.push(GuiToProcessMsg::Stop);
                }

                if ui.button("Restart").clicked {
                    self.playing = true;
                    let _ = self.to_player_tx.push(GuiToProcessMsg::Restart);
                }
            });

            let mut loop_start = self.loop_start;
            let mut loop_end = self.loop_end;
            ui.add(egui::Slider::usize(&mut loop_start, 0..=self.num_frames).text("loop start"));
            ui.add(egui::Slider::usize(&mut loop_end, 0..=self.num_frames).text("loop end"));
            if loop_start != self.loop_start || loop_end != self.loop_end {
                if ui.input().mouse.released || ui.input().key_pressed(egui::Key::Enter) {
                    if loop_end <= loop_start {
                        if loop_start == self.num_frames {
                            loop_start = self.num_frames - 1;
                            loop_end = self.num_frames;
                        } else {
                            loop_end = loop_start + 1;
                        }
                    };

                    self.loop_start = loop_start;
                    self.loop_end = loop_end;

                    self.to_player_tx
                        .push(GuiToProcessMsg::SetLoop {
                            start: loop_start,
                            end: loop_end,
                        })
                        .unwrap();
                }
            }

            if self.buffering_anim > 0 {
                ui.label(
                    egui::Label::new("Status: Buffered")
                        .text_color(egui::Color32::from_rgb(255, 0, 0)),
                );
                self.buffering_anim -= 1;
            } else {
                ui.label(
                    egui::Label::new("Status: Ready")
                        .text_color(egui::Color32::from_rgb(0, 255, 0)),
                );
            }

            let (_, user_seeked) = self.transport_control.ui(
                ui,
                &mut self.current_frame,
                self.num_frames,
                self.loop_start,
                self.loop_end,
                self.cache_size,
            );

            if user_seeked {
                let _ = self
                    .to_player_tx
                    .push(GuiToProcessMsg::SeekTo(self.current_frame));
            }
        });
    }
}

impl Drop for DemoPlayerApp {
    fn drop(&mut self) {
        self.frame_close_tx.push(()).unwrap();
    }
}

struct TransportControl {
    rail_stroke: egui::Stroke,
    handle_stroke: egui::Stroke,
    loop_stroke: egui::Stroke,
    cache_stroke: egui::Stroke,

    seeking: bool,
}

impl Default for TransportControl {
    fn default() -> Self {
        Self {
            rail_stroke: egui::Stroke::new(1.0, egui::Color32::GRAY),
            handle_stroke: egui::Stroke::new(1.0, egui::Color32::WHITE),
            loop_stroke: egui::Stroke::new(1.0, egui::Color32::from_rgb(0, 255, 0)),
            cache_stroke: egui::Stroke::new(1.0, egui::Color32::from_rgb(0, 0, 255)),
            seeking: false,
        }
    }
}

impl TransportControl {
    const PADDING: f32 = 20.0;

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        value: &mut usize,
        max_value: usize,
        loop_start: usize,
        loop_end: usize,
        cache_size: usize,
    ) -> (egui::Response, bool) {
        let (response, painter) =
            ui.allocate_painter(ui.available_size_before_wrap_finite(), egui::Sense::drag());
        let rect = response.rect;

        let mut shapes = vec![];

        let rail_y = rect.top() + 20.0;
        let start_x = rect.left() + Self::PADDING;
        let end_x = rect.right() - Self::PADDING;
        let rail_width = end_x - start_x;

        // Draw rail.
        shapes.push(egui::Shape::line_segment(
            [
                egui::Pos2::new(start_x, rail_y),
                egui::Pos2::new(end_x, rail_y),
            ],
            self.rail_stroke,
        ));

        // Drop loop range.
        let loop_start_x = start_x + ((loop_start as f32 / max_value as f32) * rail_width);
        let loop_end_x = start_x + ((loop_end as f32 / max_value as f32) * rail_width);

        shapes.push(egui::Shape::line_segment(
            [
                egui::Pos2::new(loop_start_x, rail_y),
                egui::Pos2::new(loop_end_x, rail_y),
            ],
            self.loop_stroke,
        ));

        if let Some(press_origin) = ui.input().mouse.press_origin {
            if press_origin.x >= start_x
                && press_origin.x <= end_x
                && press_origin.y >= rail_y - 10.0
                && press_origin.y <= rail_y + 10.0
            {
                if let Some(mouse_pos) = ui.input().mouse.pos {
                    let handle_x = mouse_pos.x - start_x;
                    *value = (((handle_x / rail_width) * max_value as f32).round() as isize)
                        .max(0)
                        .min(max_value as isize) as usize;

                    self.seeking = true;
                }
            }
        }

        let mut changed: bool = false;
        if ui.input().mouse.released && self.seeking {
            self.seeking = false;

            changed = true;
        }

        let handle_x = start_x + ((*value as f32 / max_value as f32) * rail_width);

        // Draw handle.
        shapes.push(egui::Shape::line_segment(
            [
                egui::Pos2::new(handle_x, rail_y - 10.0),
                egui::Pos2::new(handle_x, rail_y + 10.0),
            ],
            self.handle_stroke,
        ));

        // Draw cached ranges.
        let caches: [usize; 2] = [0, loop_start];
        let cache_width = (cache_size as f32 / max_value as f32) * rail_width;
        for cache_pos in caches.iter() {
            let x = start_x + ((*cache_pos as f32 / max_value as f32) * rail_width);
            shapes.push(egui::Shape::line_segment(
                [
                    egui::Pos2::new(x, rail_y + 30.0),
                    egui::Pos2::new(x + cache_width, rail_y + 30.0),
                ],
                self.cache_stroke,
            ));
        }

        painter.extend(shapes);

        (response, changed)
    }
}

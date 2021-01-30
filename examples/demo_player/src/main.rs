use eframe::{egui, epi};
use rt_audio_disk_stream::AudioDiskStream;

fn main() {
    let app = DemoPlayerApp::default();
    eframe::run_native(Box::new(app));
}

struct DemoPlayerApp {
    playing: bool,
    current_frame: usize,
    max_frame: usize,
    transport_control: TransportControl,
}

impl Default for DemoPlayerApp {
    fn default() -> Self {
        Self {
            playing: false,
            current_frame: 0,
            max_frame: 10000,
            transport_control: Default::default(),
        }
    }
}

impl epi::App for DemoPlayerApp {
    fn name(&self) -> &str {
        "rt-audio-disk-stream demo player"
    }

    fn update(&mut self, ctx: &egui::CtxRef, frame: &mut epi::Frame<'_>) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::warn_if_debug_build(ui);

            let play_label = if self.playing { "||" } else { ">" };

            if ui.button(play_label).clicked {
                self.playing = !self.playing;
            }

            self.transport_control
                .ui(ui, &mut self.current_frame, self.max_frame);
        });
    }
}

struct TransportControl {
    rail_stroke: egui::Stroke,
    handle_stroke: egui::Stroke,
}

impl Default for TransportControl {
    fn default() -> Self {
        Self {
            rail_stroke: egui::Stroke::new(1.0, egui::Color32::GRAY),
            handle_stroke: egui::Stroke::new(1.0, egui::Color32::WHITE),
        }
    }
}

impl TransportControl {
    const PADDING: f32 = 20.0;

    pub fn ui(&mut self, ui: &mut egui::Ui, value: &mut usize, max_value: usize) -> egui::Response {
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
                }
            }
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

        painter.extend(shapes);

        response
    }
}

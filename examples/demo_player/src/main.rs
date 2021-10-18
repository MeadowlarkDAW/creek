use creek::{ReadDiskStream, SymphoniaDecoder};
use rtrb::RingBuffer;

mod output;
mod process;
mod ui;

pub enum GuiToProcessMsg {
    UseStream(ReadDiskStream<SymphoniaDecoder>),
    SetLoop { start: usize, end: usize },
    PlayResume,
    Pause,
    Stop,
    Restart,
    SeekTo(usize),
}

pub enum ProcessToGuiMsg {
    PlaybackPos(usize),
    Buffering,
}

fn main() {
    let (to_gui_tx, from_process_rx) = RingBuffer::<ProcessToGuiMsg>::new(256);
    let (to_process_tx, from_gui_rx) = RingBuffer::<GuiToProcessMsg>::new(64);

    let app = ui::DemoPlayerApp::new(to_process_tx, from_process_rx);
    let _cpal_stream = output::Output::new(to_gui_tx, from_gui_rx);

    eframe::run_native(Box::new(app));
}

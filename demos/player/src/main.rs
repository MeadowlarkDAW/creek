use creek::{ReadDiskStream, SymphoniaDecoder};
use rtrb::RingBuffer;

mod output;
mod process;
mod ui;

#[allow(clippy::large_enum_variant)]
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
    let cli_arg = std::env::args()
        .nth(1)
        .unwrap_or("./test_files/wav_i24_stereo.wav".to_string());
    let file_path = std::path::PathBuf::from(cli_arg);

    let (to_gui_tx, from_process_rx) = RingBuffer::<ProcessToGuiMsg>::new(256);
    let (to_process_tx, from_gui_rx) = RingBuffer::<GuiToProcessMsg>::new(64);

    let _cpal_stream = output::Output::new(to_gui_tx, from_gui_rx);

    eframe::run_native(
        "creek demo player",
        eframe::NativeOptions::default(),
        Box::new(|cc| {
            Box::new(ui::DemoPlayerApp::new(
                to_process_tx,
                from_process_rx,
                file_path,
                cc,
            ))
        }),
    )
    .unwrap();
}

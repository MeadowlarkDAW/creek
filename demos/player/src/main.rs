use creek::{ReadDiskStream, SymphoniaDecoder};
use rtrb::RingBuffer;

mod output;
mod process;
mod ui;

pub enum GuiToProcessMsg {
    // Note, you could opt to not wrap the stream struct inside of a `Box` and
    // not have to worry about sending it back to be deallocated. Doing so may
    // also sometimes perform a bit better by avoiding a pointer dereference.
    // But doing so will increase the size of the enum by quite a bit.
    UseStream(Box<ReadDiskStream<SymphoniaDecoder>>),
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
    DropOldStream(Box<ReadDiskStream<SymphoniaDecoder>>),
}

fn main() {
    let cli_arg = std::env::args()
        .nth(1)
        .unwrap_or("./test_files/wav_i24_stereo.wav".to_string());
    let file_path = std::path::PathBuf::from(cli_arg);

    let (to_gui_tx, from_process_rx) = RingBuffer::<ProcessToGuiMsg>::new(256);
    let (to_process_tx, from_gui_rx) = RingBuffer::<GuiToProcessMsg>::new(64);

    let _cpal_stream = output::spawn_cpal_stream(to_gui_tx, from_gui_rx);

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

use rt_audio_disk_stream::{wav_bit_depth, WavEncoder, WriteDiskStream};
use rtrb::RingBuffer;

mod output;
mod process;
mod ui;

pub enum GuiToProcessMsg {
    SetFreq(f32),
    UseStreamUint8(WriteDiskStream<WavEncoder<wav_bit_depth::Uint8>>),
    UseStreamInt16(WriteDiskStream<WavEncoder<wav_bit_depth::Int16>>),
    UseStreamInt24(WriteDiskStream<WavEncoder<wav_bit_depth::Int24>>),
    UseStreamFloat32(WriteDiskStream<WavEncoder<wav_bit_depth::Float32>>),
    PlayResume,
    Pause,
    Finish,
    Discard,
    DiscardAndRestart,
}

pub enum ProcessToGuiMsg {
    FramesWritten(usize),
    FatalError,
}

fn main() {
    let (to_gui_tx, from_process_rx) = RingBuffer::<ProcessToGuiMsg>::new(256).split();
    let (to_process_tx, from_gui_rx) = RingBuffer::<GuiToProcessMsg>::new(64).split();

    let (_cpal_stream, sample_rate) = output::Output::new(to_gui_tx, from_gui_rx);
    let app = ui::DemoWriterApp::new(to_process_tx, from_process_rx, sample_rate);

    eframe::run_native(Box::new(app));
}

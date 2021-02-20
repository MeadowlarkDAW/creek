use std::time;

mod read;
mod write;

pub use read::{
    open_read, DataBlock, Decoder, ReadDiskStream, ReadError, ReadStreamOptions, SeekMode,
};
pub use write::{open_write, Encoder, WriteBlock, WriteDiskStream, WriteError, WriteStreamOptions};

const SERVER_WAIT_TIME: time::Duration = time::Duration::from_millis(1);

/// Info about the file.
#[derive(Clone)]
pub struct FileInfo<FileParams> {
    /// The total number of frames in the file.
    pub num_frames: usize,
    /// The number of channels in the file.
    pub num_channels: u16,
    /// The sample rate of the file (if it exists).
    pub sample_rate: Option<f64>,

    /// Additional info provided by the decoder.
    pub params: FileParams,
}

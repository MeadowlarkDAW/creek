use std::error::Error;
use std::fmt::Debug;
use std::path::PathBuf;

use super::WriteBlock;
use crate::FileInfo;

/// A type that encodes a file in a write stream.
pub trait Encoder: Sized + 'static {
    /// The data type of a single sample. (i.e. `f32`)
    type T: Copy + Clone + Default + Send;

    /// Any additional options for creating a file with this encoder.
    type AdditionalOpts: Send + Default + Debug;

    /// Any additional information on the file.
    type FileParams: Clone + Send;

    /// The error type while opening the file.
    type OpenError: Error + Send;

    /// The error type when a fatal error occurs.
    type FatalError: Error + Send;

    /// The default number of frames in a write block.
    const DEFAULT_BLOCK_SIZE: usize;

    /// The default number of write blocks. This must be sufficiently large to
    /// ensure there are enough write blocks for the client in the worst case
    /// write latency scenerio.
    const DEFAULT_NUM_WRITE_BLOCKS: usize;

    /// Open the file for writing.
    ///
    /// * `file` - The path of the file to open.
    /// * `num_channels` - The number of audio channels in the file.
    /// * `sample_rate` - The sample rate of the audio data.
    /// * `block_size` - The block size to use.
    /// * `max_num_write_blocks` - The number of write blocks this stream is using.
    /// * `additional_opts` - Any additional encoder-specific options.
    fn new(
        file: PathBuf,
        num_channels: u16,
        sample_rate: f64,
        block_size: usize,
        num_write_blocks: usize,
        additional_opts: Self::AdditionalOpts,
    ) -> Result<(Self, FileInfo<Self::FileParams>), Self::OpenError>;

    unsafe fn encode(&mut self, write_block: &WriteBlock<Self::T>) -> Result<(), Self::FatalError>;

    fn finish_file(&mut self) -> Result<(), Self::FatalError>;

    fn discard_file(&mut self) -> Result<(), Self::FatalError>;

    fn discard_and_restart(&mut self) -> Result<(), Self::FatalError>;
}

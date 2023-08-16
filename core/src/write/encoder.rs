use std::error::Error;
use std::fmt::Debug;
use std::path::PathBuf;
use std::time::Duration;

use crate::{AudioBlock, FileInfo};

/// The return status of writing to a file.
#[derive(Debug, Clone, Copy)]
pub enum WriteStatus {
    /// Written ok.
    Ok,
    /// Written ok, but the file has (or is about to) reach
    /// the maximum file size for this codec. A new file
    /// will be created to hold more data.
    ///
    /// This returns the total number of files.
    /// (Including the one created with this stream and the new one that
    /// is being created right now).
    ReachedMaxSize { num_files: u32 },
}

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
    const DEFAULT_BLOCK_FRAMES: usize;

    /// The default number of write blocks. This must be sufficiently large to
    /// ensure there are enough write blocks for the client in the worst case
    /// write latency scenerio.
    const DEFAULT_NUM_WRITE_BLOCKS: usize;

    /// The default interval for how often the encoder polls for data.
    const DEFAULT_POLL_INTERVAL: Duration;

    /// Open the file for writing.
    ///
    /// * `file` - The path of the file to open.
    /// * `num_channels` - The number of audio channels in the file.
    /// * `sample_rate` - The sample rate of the audio data.
    /// * `block_frames` - The block size to use.
    /// * `max_num_write_blocks` - The number of write blocks this stream is using.
    /// * `additional_opts` - Any additional encoder-specific options.
    fn new(
        file: PathBuf,
        num_channels: u16,
        sample_rate: u32,
        block_frames: usize,
        num_write_blocks: usize,
        poll_interval: Duration,
        additional_opts: Self::AdditionalOpts,
    ) -> Result<(Self, FileInfo<Self::FileParams>), Self::OpenError>;

    /// Write a block of data to the file.
    ///
    /// If the write was successful, return `WriteStatus::Ok`.
    ///
    /// If the codec has a maximum file size (i.e. 4GB for WAV), then keep track of
    /// how many bytes were written. Once the file is full (or about full), finish the
    /// file, close it, and create a new file with the characters "_XXX" appended to
    /// the file name (i.e. "_001" for the first file, "_002" for the second, etc.)
    /// This helper function `num_files_to_file_name_extension()` can be used to find
    /// this extension.
    fn encode(&mut self, block: &AudioBlock<Self::T>) -> Result<WriteStatus, Self::FatalError>;

    /// Finish up the file and then close it.
    fn finish_file(&mut self) -> Result<(), Self::FatalError>;

    /// Delete all created files. Do not start over.
    fn discard_file(&mut self) -> Result<(), Self::FatalError>;

    /// Delete all created files and start over from the beginning.
    fn discard_and_restart(&mut self) -> Result<(), Self::FatalError>;
}

/// Converts the current total number of files created (including the one created
/// with this stream and the new one that is being created right now) to the extension
/// to append to the end of the file name.
///
/// This extension is in the format "_XXX". (i.e. "_001", "_002", etc.)
pub fn num_files_to_file_name_extension(num_files: u32) -> String {
    if num_files <= 1 {
        return String::from("");
    }

    let extension_num = num_files - 1;
    if extension_num < 10 {
        format!("_00{}", extension_num)
    } else if extension_num < 100 {
        format!("_0{}", extension_num)
    } else {
        format!("_{}", extension_num)
    }
}

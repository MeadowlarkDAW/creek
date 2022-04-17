use std::path::PathBuf;
use std::{error::Error, fmt::Debug};

use super::DataBlock;
use crate::FileInfo;

pub enum DecoderReadDirection {
    Forward,
    Backward
}

/// A type that decodes a file in a read stream.
pub trait Decoder: Sized + 'static {
    /// The data type of a single sample. (i.e. `f32`)
    type T: Copy + Clone + Default + Send;

    /// Any additional options for opening a file with this decoder.
    type AdditionalOpts: Send + Default + Debug;

    /// Any additional information on the file.
    type FileParams: Clone + Send;

    /// The error type while opening the file.
    type OpenError: Error + Send;

    /// The error type when a fatal error occurs.
    type FatalError: Error + Send;

    /// The default number of frames in a prefetch block.
    const DEFAULT_BLOCK_SIZE: usize;

    /// The default number of prefetch blocks in a cache block. This will cause a cache to be
    /// used whenever the stream is seeked to a frame in the range:
    ///
    /// `[cache_start, cache_start + (num_cache_blocks * block_size))`
    ///
    /// If this is 0, then the cache is only used when seeked to exactly `cache_start`.
    const DEFAULT_NUM_CACHE_BLOCKS: usize;

    /// The number of prefetch blocks to store ahead of the cache block. This must be
    /// sufficiently large to ensure enough to time to fill the buffer in the worst
    /// case latency scenerio.
    const DEFAULT_NUM_LOOK_AHEAD_BLOCKS: usize;

    /// Open the file and start reading from `start_frame`.
    ///
    /// Please note this algorithm depends on knowing the exact number of frames in a file.
    /// Do **not** return an approximate length in the returned `FileInfo`.
    fn new(
        file: PathBuf,
        start_frame: usize,
        block_size: usize,
        additional_opts: Self::AdditionalOpts,
    ) -> Result<(Self, FileInfo<Self::FileParams>), Self::OpenError>;

    /// Change the read direction. This allows reverse playback.
    fn direction(&mut self, direction: DecoderReadDirection);

    /// Seek to a frame in the file. If a frame lies outside of the end of the file,
    /// set the read position the end of the file instead of returning an error.
    fn seek(&mut self, frame: usize) -> Result<(), Self::FatalError>;

    /// Decode data into the `data_block` starting from the read position. This is streaming,
    /// meaning the next call to `decode()` should pick up where the previous left off.
    ///
    /// If the end of the file is reached, fill data up to the end of the file, then set the
    /// read position to the last frame in the file and do nothing.
    ///
    /// ## Unsafe
    /// This is marked as "unsafe" because a `data_block` may be uninitialized, causing
    /// undefined behavior if data is not filled into the block. It is your responsibility to
    /// always fill the block (unless the end of the file is reached, in which case the server
    /// will tell the client to not read data past that frame).
    unsafe fn decode(
        &mut self,
        data_block: &mut DataBlock<Self::T>,
    ) -> Result<(), Self::FatalError>;

    /// Return the current read position.
    fn current_frame(&self) -> usize;
}

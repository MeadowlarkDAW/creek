use std::path::PathBuf;
use std::time::Duration;
use std::{error::Error, fmt::Debug};

use super::AudioBlock;
use crate::FileInfo;

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
    const DEFAULT_BLOCK_FRAMES: usize;

    /// The default number of prefetch blocks in a cache block. This will cause a cache to be
    /// used whenever the stream is seeked to a frame in the range:
    ///
    /// `[cache_start, cache_start + (num_cache_blocks * block_frames))`
    ///
    /// If this is 0, then the cache is only used when seeked to exactly `cache_start`.
    const DEFAULT_NUM_CACHE_BLOCKS: usize;

    /// The number of prefetch blocks to store ahead of the cache block. This must be
    /// sufficiently large to ensure enough to time to fill the buffer in the worst
    /// case latency scenerio.
    const DEFAULT_NUM_LOOK_AHEAD_BLOCKS: usize;

    /// The default interval for how often the decoder polls for data.
    const DEFAULT_POLL_INTERVAL: Duration;

    /// Open the file and start reading from `start_frame`.
    ///
    /// Please note this algorithm depends on knowing the exact number of frames in a file.
    /// Do **not** return an approximate length in the returned `FileInfo`.
    fn new(
        file: PathBuf,
        start_frame: usize,
        block_frames: usize,
        poll_interval: Duration,
        additional_opts: Self::AdditionalOpts,
    ) -> Result<(Self, FileInfo<Self::FileParams>), Self::OpenError>;

    /// Seek to a frame in the file. If a frame lies outside of the end of the file,
    /// set the read position the end of the file instead of returning an error.
    fn seek(&mut self, frame: usize) -> Result<(), Self::FatalError>;

    /// Decode data into the `block` starting from the read position. This is streaming,
    /// meaning the next call to `decode()` should pick up where the previous left off.
    ///
    /// If the end of the file is reached, fill data up to the end of the file, then set the
    /// read position to the last frame in the file and do nothing.
    ///
    /// The block must be filled entirely. If there is not enough data to fill it, fill the
    /// rest with zeros.
    ///
    /// Do not resize any Vecs contained in the `block`.
    fn decode(&mut self, block: &mut AudioBlock<Self::T>) -> Result<(), Self::FatalError>;

    /// Return the current read position.
    fn current_frame(&self) -> usize;
}

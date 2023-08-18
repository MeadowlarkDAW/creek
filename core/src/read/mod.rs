mod data;
mod decoder;
mod read_stream;
mod server;

pub mod error;

use std::time::Duration;

use crate::AudioBlock;

pub use decoder::Decoder;
pub use error::{FatalReadError, ReadError};
pub use read_stream::{ReadDiskStream, ReadResult, SeekMode};

use data::AudioBlockCache;
use server::ReadServer;

enum ServerToClientMsg<D: Decoder> {
    ReadIntoBlockRes {
        block_index: usize,
        block: AudioBlock<D::T>,
        wanted_start_frame: usize,
    },
    CacheRes {
        cache_index: usize,
        cache: AudioBlockCache<D::T>,
        wanted_start_frame: usize,
    },
    FatalError(D::FatalError),
}

enum ClientToServerMsg<D: Decoder> {
    ReadIntoBlock {
        block_index: usize,
        block: Option<AudioBlock<D::T>>,
        start_frame: usize,
    },
    DisposeBlock {
        block: AudioBlock<D::T>,
    },
    SeekTo {
        frame: usize,
    },
    Cache {
        cache_index: usize,
        cache: Option<AudioBlockCache<D::T>>,
        start_frame: usize,
    },
    DisposeCache {
        cache: AudioBlockCache<D::T>,
    },
}

/// Options for a read stream.
#[derive(Debug, Clone, Copy)]
pub struct ReadStreamOptions<D: Decoder> {
    /// The number of prefetch blocks in a cache block. This will cause a cache to be
    /// used whenever the stream is seeked to a frame in the range:
    ///
    /// `[cache_start, cache_start + (num_cache_blocks * block_frames))`
    ///
    /// If this is 0, then the cache is only used when seeked to exactly `cache_start`.
    ///
    /// This will cause a panic if `num_cache_blocks + num_look_ahead_blocks < 3`.
    pub num_cache_blocks: usize,

    /// The maximum number of caches that can be active in this stream. Keep in mind each
    /// cache uses some memory (but memory is only allocated when the cache is created).
    ///
    /// The default is `1`.
    pub num_caches: usize,

    /// Any additional decoder-specific options.
    pub additional_opts: D::AdditionalOpts,

    /// The number of prefetch blocks to store ahead of the cache block. This must be
    /// sufficiently large to ensure enough to time to fill the buffer in the worst
    /// case latency scenario.
    ///
    /// This should be left alone unless you know what you are doing.
    ///
    /// This will cause a panic if `num_cache_blocks + num_look_ahead_blocks < 3`.
    pub num_look_ahead_blocks: usize,

    /// The number of frames in a prefetch block.
    ///
    /// This should be left alone unless you know what you are doing.
    pub block_frames: usize,

    /// The size of the realtime ring buffer that sends data to and from the stream the the
    /// internal IO server. This must be sufficiently large enough to avoid stalling the channels.
    ///
    /// Set this to `None` to automatically find a generous size based on the other read options.
    /// This should be left as `None` unless you know what you are doing.
    ///
    /// The default is `None`.
    pub server_msg_channel_size: Option<usize>,

    /// How often the decoder should poll for data.
    ///
    /// If this is `None`, then the Decoder's default will be used.
    ///
    /// The default is `None`.
    pub decoder_poll_interval: Option<Duration>,
}

impl<D: Decoder> Default for ReadStreamOptions<D> {
    fn default() -> Self {
        ReadStreamOptions {
            block_frames: D::DEFAULT_BLOCK_FRAMES,
            num_cache_blocks: D::DEFAULT_NUM_CACHE_BLOCKS,
            additional_opts: Default::default(),
            num_look_ahead_blocks: D::DEFAULT_NUM_LOOK_AHEAD_BLOCKS,
            num_caches: 1,
            server_msg_channel_size: None,
            decoder_poll_interval: None,
        }
    }
}

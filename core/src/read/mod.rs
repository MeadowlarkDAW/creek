mod data;
mod decoder;
mod read_stream;
mod server;

pub mod error;

pub use data::{DataBlock, ReadData};
pub use decoder::Decoder;
pub use error::{FatalReadError, ReadError};
pub use read_stream::{ReadDiskStream, SeekMode};

use data::{DataBlockCache, HeapData};
use server::ReadServer;

pub(crate) enum ServerToClientMsg<D: Decoder> {
    ReadIntoBlockRes {
        block_index: usize,
        block: DataBlock<D::T>,
        wanted_start_frame: usize,
    },
    CacheRes {
        cache_index: usize,
        cache: DataBlockCache<D::T>,
        wanted_start_frame: usize,
    },
    FatalError(D::FatalError),
}

pub(crate) enum ClientToServerMsg<D: Decoder> {
    ReadIntoBlock {
        block_index: usize,
        block: Option<DataBlock<D::T>>,
        start_frame: usize,
    },
    DisposeBlock {
        block: DataBlock<D::T>,
    },
    SeekTo {
        frame: usize,
    },
    Cache {
        cache_index: usize,
        cache: Option<DataBlockCache<D::T>>,
        start_frame: usize,
    },
    DisposeCache {
        cache: DataBlockCache<D::T>,
    },
}

/// Options for a read stream.
#[derive(Debug, Clone, Copy)]
pub struct ReadStreamOptions<D: Decoder> {
    /// The number of prefetch blocks in a cache block. This will cause a cache to be
    /// used whenever the stream is seeked to a frame in the range:
    ///
    /// `[cache_start, cache_start + (num_cache_blocks * block_size))`
    ///
    /// If this is 0, then the cache is only used when seeked to exactly `cache_start`.
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
    pub num_look_ahead_blocks: usize,

    /// The number of frames in a prefetch block.
    ///
    /// This should be left alone unless you know what you are doing.
    pub block_size: usize,

    /// The size of the realtime ring buffer that sends data to and from the stream the the
    /// internal IO server. This must be sufficiently large enough to avoid stalling the channels.
    ///
    /// Set this to `None` to automatically find a generous size based on the other read options.
    /// This should be left as `None` unless you know what you are doing.
    ///
    /// The default is `None`.
    pub server_msg_channel_size: Option<usize>,
}

impl<D: Decoder> Default for ReadStreamOptions<D> {
    fn default() -> Self {
        ReadStreamOptions {
            block_size: D::DEFAULT_BLOCK_SIZE,
            num_cache_blocks: D::DEFAULT_NUM_CACHE_BLOCKS,
            additional_opts: Default::default(),
            num_look_ahead_blocks: D::DEFAULT_NUM_LOOK_AHEAD_BLOCKS,
            num_caches: 1,
            server_msg_channel_size: None,
        }
    }
}

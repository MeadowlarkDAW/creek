use std::path::PathBuf;

use rtrb::RingBuffer;

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
    /// case latency scenerio.
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

/// Open a new realtime-safe disk-streaming reader.
///
/// * `file` - The path to the file to open.
/// * `start_frame` - The frame in the file to start reading from.
/// * `stream_opts` - Additional stream options.
pub fn open_read<D: Decoder, P: Into<PathBuf>>(
    file: P,
    start_frame: usize,
    stream_opts: ReadStreamOptions<D>,
) -> Result<ReadDiskStream<D>, D::OpenError> {
    assert_ne!(stream_opts.block_size, 0);
    assert_ne!(stream_opts.num_look_ahead_blocks, 0);
    assert_ne!(stream_opts.server_msg_channel_size, Some(0));

    // Reserve ample space for the message channels.
    let msg_channel_size = stream_opts.server_msg_channel_size.unwrap_or(
        ((stream_opts.num_cache_blocks + stream_opts.num_look_ahead_blocks) * 4)
            + (stream_opts.num_caches * 4)
            + 8,
    );

    let (to_server_tx, from_client_rx) =
        RingBuffer::<ClientToServerMsg<D>>::new(msg_channel_size).split();
    let (to_client_tx, from_server_rx) =
        RingBuffer::<ServerToClientMsg<D>>::new(msg_channel_size).split();

    // Create dedicated close signal.
    let (close_signal_tx, close_signal_rx) = RingBuffer::<Option<HeapData<D::T>>>::new(1).split();

    let file: PathBuf = file.into();

    match ReadServer::new(
        file,
        start_frame,
        stream_opts.num_cache_blocks + stream_opts.num_look_ahead_blocks,
        stream_opts.block_size,
        to_client_tx,
        from_client_rx,
        close_signal_rx,
        stream_opts.additional_opts,
    ) {
        Ok(file_info) => {
            let client = ReadDiskStream::new(
                to_server_tx,
                from_server_rx,
                close_signal_tx,
                start_frame,
                stream_opts.num_cache_blocks,
                stream_opts.num_look_ahead_blocks,
                stream_opts.num_caches,
                stream_opts.block_size,
                file_info,
            );

            Ok(client)
        }
        Err(e) => Err(e),
    }
}

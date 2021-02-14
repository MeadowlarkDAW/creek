use std::path::PathBuf;
use std::time;

use rtrb::RingBuffer;

mod read;

pub use read::{DataBlock, Decoder, FileInfo, ReadDiskStream, ReadError, SeekMode};

use read::{ClientToServerMsg, HeapData, ReadServer, ServerToClientMsg};

const SERVER_WAIT_TIME: time::Duration = time::Duration::from_millis(1);

/// Options for a read stream.
#[derive(Debug, Clone, Copy)]
pub struct ReadOptions<D: Decoder> {
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

    /// Any addition decoder-specific options.
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

impl<D: Decoder> Default for ReadOptions<D> {
    fn default() -> Self {
        ReadOptions {
            block_size: D::DEFAULT_BLOCK_SIZE,
            num_cache_blocks: D::DEFAULT_NUM_CACHE_BLOCKS,
            num_look_ahead_blocks: D::DEFAULT_NUM_LOOK_AHEAD_BLOCKS,
            num_caches: 1,
            additional_opts: Default::default(),
            server_msg_channel_size: None,
        }
    }
}

/// Open a new realtime-safe disk-streaming reader.
///
/// * `file` - The path to the file to open.
/// * `start_frame` - The frame in the file to start reading from.
/// * `options` - Any additional options.
pub fn open_read<D: Decoder, P: Into<PathBuf>>(
    file: P,
    start_frame: usize,
    options: ReadOptions<D>,
) -> Result<ReadDiskStream<D>, D::OpenError> {
    assert_ne!(options.block_size, 0);
    assert_ne!(options.num_look_ahead_blocks, 0);

    // Reserve ample space for the message channels.
    let msg_channel_size = options.server_msg_channel_size.unwrap_or(
        ((options.num_cache_blocks + options.num_look_ahead_blocks) * 4)
            + (options.num_caches * 4)
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
        options.num_cache_blocks + options.num_look_ahead_blocks,
        options.block_size,
        to_client_tx,
        from_client_rx,
        close_signal_rx,
        options.additional_opts,
    ) {
        Ok(file_info) => {
            let client = ReadDiskStream::new(
                to_server_tx,
                from_server_rx,
                close_signal_tx,
                start_frame,
                options.num_cache_blocks,
                options.num_look_ahead_blocks,
                options.num_caches,
                options.block_size,
                file_info,
            );

            Ok(client)
        }
        Err(e) => Err(e),
    }
}

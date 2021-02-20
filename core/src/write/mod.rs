use std::path::PathBuf;

use rtrb::RingBuffer;

mod data;
mod encoder;
mod server;
mod write_stream;

pub mod error;

pub use data::WriteBlock;
pub use encoder::Encoder;
pub use error::WriteError;
pub use write_stream::WriteDiskStream;

use data::HeapData;
use server::WriteServer;

pub(crate) enum ServerToClientMsg<E: Encoder> {
    NewWriteBlock { block: WriteBlock<E::T> },
    Finished,
    FatalError(E::FatalError),
}

pub(crate) enum ClientToServerMsg<E: Encoder> {
    WriteBlock { block: WriteBlock<E::T> },
    FinishFile,
    DiscardFile,
    DiscardAndRestart,
}

/// Options for a write stream.
#[derive(Debug, Clone, Copy)]
pub struct WriteStreamOptions<E: Encoder> {
    /// Any additional encoder-specific options.
    pub additional_opts: E::AdditionalOpts,

    /// The number of write blocks to reserve. This must be sufficiently large to
    /// ensure there are enough write blocks for the client in the worst case
    /// write latency scenerio.
    ///
    /// This should be left alone unless you know what you are doing.
    pub num_write_blocks: usize,

    /// The number of frames in a write block.
    ///
    /// This should be left alone unless you know what you are doing.
    pub block_size: usize,

    /// The size of the realtime ring buffer that sends data to and from the stream the the
    /// internal IO server. This must be sufficiently large enough to avoid stalling the channels.
    ///
    /// Set this to `None` to automatically find a generous size based on the other write options.
    /// This should be left as `None` unless you know what you are doing.
    ///
    /// The default is `None`.
    pub server_msg_channel_size: Option<usize>,
}

impl<E: Encoder> Default for WriteStreamOptions<E> {
    fn default() -> Self {
        WriteStreamOptions {
            additional_opts: Default::default(),
            num_write_blocks: E::DEFAULT_NUM_WRITE_BLOCKS,
            block_size: E::DEFAULT_BLOCK_SIZE,
            server_msg_channel_size: None,
        }
    }
}

/// Open a new realtime-safe disk-streaming writer.
///
/// * `file` - The path to the file to open.
/// * `num_channels` - The number of channels in the file.
/// * `sample_rate` - The sample rate of the file.
/// * `stream_opts` - Additional stream options.
pub fn open_write<E: Encoder, P: Into<PathBuf>>(
    file: P,
    num_channels: u16,
    sample_rate: f64,
    stream_opts: WriteStreamOptions<E>,
) -> Result<WriteDiskStream<E>, E::OpenError> {
    assert_ne!(num_channels, 0);
    assert_ne!(sample_rate, 0.0);
    assert_ne!(stream_opts.block_size, 0);
    assert_ne!(stream_opts.num_write_blocks, 0);
    assert_ne!(stream_opts.server_msg_channel_size, Some(0));

    // Reserve ample space for the message channels.
    let msg_channel_size = stream_opts
        .server_msg_channel_size
        .unwrap_or((stream_opts.num_write_blocks * 4) + 8);

    let (to_server_tx, from_client_rx) =
        RingBuffer::<ClientToServerMsg<E>>::new(msg_channel_size).split();
    let (to_client_tx, from_server_rx) =
        RingBuffer::<ServerToClientMsg<E>>::new(msg_channel_size).split();

    // Create dedicated close signal.
    let (close_signal_tx, close_signal_rx) = RingBuffer::<Option<HeapData<E::T>>>::new(1).split();

    let file: PathBuf = file.into();

    match WriteServer::new(
        file,
        stream_opts.num_write_blocks,
        stream_opts.block_size,
        num_channels,
        sample_rate,
        to_client_tx,
        from_client_rx,
        close_signal_rx,
        stream_opts.additional_opts,
    ) {
        Ok(file_info) => {
            let client = WriteDiskStream::new(
                to_server_tx,
                from_server_rx,
                close_signal_tx,
                stream_opts.num_write_blocks,
                stream_opts.block_size,
                file_info,
            );

            Ok(client)
        }
        Err(e) => Err(e),
    }
}

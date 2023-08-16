mod data;
mod encoder;
mod server;
mod write_stream;

pub mod error;

use std::time::Duration;

pub use encoder::{num_files_to_file_name_extension, Encoder, WriteStatus};
pub use error::{FatalWriteError, WriteError};
pub use write_stream::WriteDiskStream;

use data::WriteBlock;
use server::WriteServer;

enum ServerToClientMsg<E: Encoder> {
    NewWriteBlock { block: WriteBlock<E::T> },
    Finished,
    ReachedMaxSize { num_files: u32 },
    FatalError(E::FatalError),
}

enum ClientToServerMsg<E: Encoder> {
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
    ///
    /// This will cause a panic if set to less than 3.
    pub num_write_blocks: usize,

    /// The number of frames in a write block.
    ///
    /// This should be left alone unless you know what you are doing.
    pub block_frames: usize,

    /// The size of the realtime ring buffer that sends data to and from the stream the the
    /// internal IO server. This must be sufficiently large enough to avoid stalling the channels.
    ///
    /// Set this to `None` to automatically find a generous size based on the other write options.
    /// This should be left as `None` unless you know what you are doing.
    ///
    /// The default is `None`.
    pub server_msg_channel_size: Option<usize>,

    /// How often the encoder should poll for data.
    ///
    /// If this is `None`, then the Encoder's default will be used.
    ///
    /// The default is `None`.
    pub encoder_poll_interval: Option<Duration>,
}

impl<E: Encoder> Default for WriteStreamOptions<E> {
    fn default() -> Self {
        WriteStreamOptions {
            additional_opts: Default::default(),
            num_write_blocks: E::DEFAULT_NUM_WRITE_BLOCKS,
            block_frames: E::DEFAULT_BLOCK_FRAMES,
            server_msg_channel_size: None,
            encoder_poll_interval: None,
        }
    }
}

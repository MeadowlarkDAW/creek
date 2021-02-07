use std::path::PathBuf;
use std::time;

use rtrb::RingBuffer;

mod read;

pub use read::{Decoder, FileInfo, OpenError, ReadClient, ReadError};

use read::{ClientToServerMsg, HeapData, ReadServer, ServerToClientMsg};

pub static DEFAULT_NUM_PREFETCH_BLOCKS: usize = 20;
pub static DEFAULT_NUM_CACHES: usize = 1;

pub const BLOCK_SIZE: usize = 8192;
const SERVER_WAIT_TIME: time::Duration = time::Duration::from_millis(1);

static SILENCE_BUFFER: [f32; BLOCK_SIZE] = [0.0; BLOCK_SIZE];

#[derive(Debug, Clone, Copy)]
pub struct StreamOptions {
    pub num_prefetch_blocks: usize,
    pub num_caches: usize,
    pub decode_verify: bool,
}

impl Default for StreamOptions {
    fn default() -> Self {
        StreamOptions {
            num_prefetch_blocks: DEFAULT_NUM_PREFETCH_BLOCKS,
            num_caches: DEFAULT_NUM_CACHES,
            decode_verify: false,
        }
    }
}

pub struct AudioDiskStream {}

impl AudioDiskStream {
    pub fn open_read<P: Into<PathBuf>>(
        file: P,
        start_frame: usize,
        options: StreamOptions,
    ) -> Result<ReadClient, OpenError> {
        // Reserve ample space for the message channels.
        let msg_channel_size = (options.num_prefetch_blocks * 4) + (options.num_caches * 4) + 8;

        let (to_server_tx, from_client_rx) =
            RingBuffer::<ClientToServerMsg>::new(msg_channel_size).split();
        let (to_client_tx, from_server_rx) =
            RingBuffer::<ServerToClientMsg>::new(msg_channel_size).split();

        // Create dedicated close signal.
        let (close_tx, close_rx) = RingBuffer::<Option<HeapData>>::new(1).split();

        let file: PathBuf = file.into();

        match ReadServer::new(
            file,
            options.decode_verify,
            start_frame,
            options.num_prefetch_blocks,
            to_client_tx,
            from_client_rx,
            close_rx,
        ) {
            Ok(file_info) => {
                let client = ReadClient::new(
                    to_server_tx,
                    from_server_rx,
                    close_tx,
                    start_frame,
                    options.num_prefetch_blocks,
                    options.num_caches,
                    file_info,
                );

                Ok(client)
            }
            Err(e) => Err(e),
        }
    }
}

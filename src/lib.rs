use std::path::PathBuf;
use std::time;

use rtrb::RingBuffer;

mod read;

pub use read::{Decoder, FileInfo, OpenError, ReadClient, ReadError};

use read::{ClientToServerMsg, HeapData, ReadServer, ServerToClientMsg};

pub const BLOCK_SIZE: usize = 8192;
pub const NUM_PREFETCH_BLOCKS: usize = 20;
pub const MSG_CHANNEL_SIZE: usize = 128;
pub const SERVER_WAIT_TIME: time::Duration = time::Duration::from_millis(1);

static SILENCE_BUFFER: [f32; BLOCK_SIZE] = [0.0; BLOCK_SIZE];

pub struct AudioDiskStream {}

impl AudioDiskStream {
    pub fn open_read<P: Into<PathBuf>>(
        file: P,
        start_frame: usize,
        max_num_caches: usize,
        decode_verify: bool,
    ) -> Result<ReadClient, OpenError> {
        let (to_server_tx, from_client_rx) =
            RingBuffer::<ClientToServerMsg>::new(MSG_CHANNEL_SIZE).split();
        let (to_client_tx, from_server_rx) =
            RingBuffer::<ServerToClientMsg>::new(MSG_CHANNEL_SIZE).split();

        // Create dedicated close signal.
        let (close_tx, close_rx) = RingBuffer::<Option<HeapData>>::new(1).split();

        let file: PathBuf = file.into();

        match ReadServer::new(
            file,
            decode_verify,
            start_frame,
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
                    max_num_caches,
                    file_info,
                );

                Ok(client)
            }
            Err(e) => Err(e),
        }
    }
}

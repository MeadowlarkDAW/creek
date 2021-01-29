use std::marker::PhantomData;
use std::path::PathBuf;
use std::time;

use rtrb::RingBuffer;

mod read;

pub use read::{Decoder, FileInfo, ReadClient, ReadError};

use read::{ClientToServerMsg, HeapData, ReadServer, ServerToClientMsg};

pub const BLOCK_SIZE: usize = 4096;
pub const NUM_PREFETCH_BLOCKS: usize = 4;
pub const MSG_CHANNEL_SIZE: usize = 64;
pub const SERVER_WAIT_TIME: time::Duration = time::Duration::from_millis(1);

static SILENCE_BUFFER: [f32; BLOCK_SIZE] = [0.0; BLOCK_SIZE];

pub struct AudioDiskStream<D: Decoder + 'static> {
    phantom: PhantomData<D>,
}

impl<D: Decoder + 'static> AudioDiskStream<D> {
    pub fn open_read<P: Into<PathBuf>>(
        file: P,
        start_frame_in_file: usize,
        max_num_caches: usize,
    ) -> Result<ReadClient<D>, D::OpenError> {
        let (to_server_tx, from_client_rx) =
            RingBuffer::<ClientToServerMsg>::new(MSG_CHANNEL_SIZE).split();
        let (to_client_tx, from_server_rx) =
            RingBuffer::<ServerToClientMsg<D>>::new(MSG_CHANNEL_SIZE).split();

        // Create dedicated close signal.
        let (close_tx, close_rx) = RingBuffer::<Option<HeapData>>::new(1).split();

        let file: PathBuf = file.into();

        match ReadServer::new(
            file,
            start_frame_in_file,
            to_client_tx,
            from_client_rx,
            close_rx,
        ) {
            Ok(file_info) => {
                let client = ReadClient::new(
                    to_server_tx,
                    from_server_rx,
                    close_tx,
                    start_frame_in_file,
                    max_num_caches,
                    file_info,
                );

                Ok(client)
            }
            Err(e) => Err(e),
        }
    }
}

use std::marker::PhantomData;
use std::path::PathBuf;
use std::time;

use rtrb::RingBuffer;

mod read;

pub use read::{DataBlock, Decoder, FileInfo, ReadClient, ReadError};

use read::{ClientToServerMsg, HeapData, ReadServer, ServerToClientMsg};

const SERVER_WAIT_TIME: time::Duration = time::Duration::from_millis(1);

#[derive(Debug, Clone, Copy)]
pub struct StreamOptions<D: Decoder> {
    pub block_size: usize,
    pub num_cache_blocks: usize,
    pub num_look_ahead_blocks: usize,
    pub num_caches: usize,
    pub additional_opts: D::AdditionalOpts,
    pub phantom: PhantomData<D>,
}

impl<D: Decoder> Default for StreamOptions<D> {
    fn default() -> Self {
        StreamOptions {
            block_size: D::DEFAULT_BLOCK_SIZE,
            num_cache_blocks: D::DEFAULT_NUM_CACHE_BLOCKS,
            num_look_ahead_blocks: D::DEFAULT_NUM_LOOK_AHEAD_BLOCKS,
            num_caches: D::DEFAULT_NUM_CACHES,
            additional_opts: Default::default(),
            phantom: PhantomData::default(),
        }
    }
}

pub struct AudioDiskStream<D: Decoder> {
    phantom: PhantomData<D>,
}

impl<D: Decoder> AudioDiskStream<D> {
    pub fn open_read<P: Into<PathBuf>>(
        file: P,
        start_frame: usize,
        options: StreamOptions<D>,
    ) -> Result<ReadClient<D>, D::OpenError> {
        assert_ne!(options.block_size, 0);
        assert_ne!(options.num_look_ahead_blocks, 0);

        // Reserve ample space for the message channels.
        let msg_channel_size = ((options.num_cache_blocks + options.num_look_ahead_blocks) * 4)
            + (options.num_caches * 4)
            + 8;

        let (to_server_tx, from_client_rx) =
            RingBuffer::<ClientToServerMsg<D>>::new(msg_channel_size).split();
        let (to_client_tx, from_server_rx) =
            RingBuffer::<ServerToClientMsg<D>>::new(msg_channel_size).split();

        // Create dedicated close signal.
        let (close_tx, close_rx) = RingBuffer::<Option<HeapData<D::T>>>::new(1).split();

        let file: PathBuf = file.into();

        match ReadServer::new(
            file,
            start_frame,
            options.num_cache_blocks + options.num_look_ahead_blocks,
            options.block_size,
            to_client_tx,
            from_client_rx,
            close_rx,
            options.additional_opts,
        ) {
            Ok(file_info) => {
                let client = ReadClient::new(
                    to_server_tx,
                    from_server_rx,
                    close_tx,
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
}

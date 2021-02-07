use std::path::PathBuf;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::SERVER_WAIT_TIME;

use super::error::OpenError;
use super::{
    ClientToServerMsg, DataBlock, DataBlockCache, Decoder, FileInfo, HeapData, ServerToClientMsg,
};

pub(crate) struct ReadServer {
    to_client_tx: Producer<ServerToClientMsg>,
    from_client_rx: Consumer<ClientToServerMsg>,
    close_signal_rx: Consumer<Option<HeapData>>,

    decoder: Decoder,

    block_pool: Vec<DataBlock>,
    cache_pool: Vec<DataBlockCache>,

    num_channels: usize,

    run: bool,
}

impl ReadServer {
    pub fn new(
        file: PathBuf,
        verify: bool,
        start_frame: usize,
        to_client_tx: Producer<ServerToClientMsg>,
        from_client_rx: Consumer<ClientToServerMsg>,
        close_signal_rx: Consumer<Option<HeapData>>,
    ) -> Result<FileInfo, OpenError> {
        let (mut open_tx, mut open_rx) = RingBuffer::<Result<FileInfo, OpenError>>::new(1).split();

        std::thread::spawn(move || {
            match Decoder::new(file, start_frame, verify) {
                Ok((decoder, file_info)) => {
                    let num_channels = file_info.num_channels;

                    // Push cannot fail because only one message is ever sent.
                    let _ = open_tx.push(Ok(file_info));

                    ReadServer::run(Self {
                        to_client_tx,
                        from_client_rx,
                        close_signal_rx,
                        decoder,
                        block_pool: Vec::new(),
                        cache_pool: Vec::new(),
                        num_channels,
                        run: true,
                    });
                }
                Err(e) => {
                    // Push cannot fail because only one message is ever sent.
                    let _ = open_tx.push(Err(e));
                }
            }
        });

        loop {
            if let Ok(res) = open_rx.pop() {
                return res;
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }
    }

    fn run(mut self) {
        while self.run {
            // Check for close signal.
            if let Ok(heap_data) = self.close_signal_rx.pop() {
                // Drop heap data here.
                let _ = heap_data;
                self.run = false;
                break;
            }

            while let Ok(msg) = self.from_client_rx.pop() {
                match msg {
                    ClientToServerMsg::ReadIntoBlock {
                        block_index,
                        block,
                        start_frame,
                    } => {
                        let mut block = block.unwrap_or(
                            // Try using one in the pool if it exists.
                            self.block_pool.pop().unwrap_or(
                                // No blocks in pool. Create a new one.
                                DataBlock::new(self.num_channels),
                            ),
                        );

                        match self.decoder.decode_into(&mut block) {
                            Ok(()) => {
                                self.send_msg(ServerToClientMsg::ReadIntoBlockRes {
                                    block_index,
                                    block,
                                    wanted_start_frame: start_frame,
                                });
                            }
                            Err(e) => {
                                self.send_msg(ServerToClientMsg::FatalError(e));
                                self.run = false;
                                break;
                            }
                        }
                    }
                    ClientToServerMsg::DisposeBlock { block } => {
                        // Store the block to be reused.
                        self.block_pool.push(block);
                    }
                    ClientToServerMsg::SeekTo { frame } => {
                        if let Err(e) = self.decoder.seek_to(frame) {
                            self.send_msg(ServerToClientMsg::FatalError(e));
                            self.run = false;
                            break;
                        }
                    }
                    ClientToServerMsg::Cache {
                        cache_index,
                        cache,
                        start_frame,
                    } => {
                        let mut cache = cache.unwrap_or(
                            // Try using one in the pool if it exists.
                            self.cache_pool.pop().unwrap_or(
                                // No caches in pool. Create a new one.
                                DataBlockCache::new(self.num_channels),
                            ),
                        );

                        let current_frame = self.decoder.current_frame();

                        // Seek to the position the client wants to cache.
                        if let Err(e) = self.decoder.seek_to(start_frame) {
                            self.send_msg(ServerToClientMsg::FatalError(e));
                            self.run = false;
                            break;
                        }

                        // Fill the cache
                        for block in cache.blocks.iter_mut() {
                            if let Err(e) = self.decoder.decode_into(block) {
                                self.send_msg(ServerToClientMsg::FatalError(e));
                                self.run = false;
                                break;
                            }
                        }

                        // Seek back to the previous position.
                        if let Err(e) = self.decoder.seek_to(current_frame) {
                            self.send_msg(ServerToClientMsg::FatalError(e));
                            self.run = false;
                            break;
                        }

                        self.send_msg(ServerToClientMsg::CacheRes {
                            cache_index,
                            cache,
                            wanted_start_frame: start_frame,
                        });
                    }
                    ClientToServerMsg::DisposeCache { cache } => {
                        // Store the cache to be reused.
                        self.cache_pool.push(cache);
                    }
                }
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }
    }

    fn send_msg(&mut self, msg: ServerToClientMsg) {
        // Block until message can be sent.
        loop {
            if !self.to_client_tx.is_full() {
                break;
            }

            // Check for close signal to avoid waiting forever.
            if let Ok(heap_data) = self.close_signal_rx.pop() {
                // Drop heap data here.
                let _ = heap_data;
                self.run = false;
                break;
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }

        // Push will never fail because we made sure a slot is available in the
        // previous step (or the stream has closed, in which case an error doesn't
        // matter).
        let _ = self.to_client_tx.push(msg);
    }
}

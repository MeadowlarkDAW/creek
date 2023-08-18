use std::{path::PathBuf, time::Duration};

use rtrb::{Consumer, Producer, RingBuffer};

use super::data::HeapData;
use super::{AudioBlock, AudioBlockCache, ClientToServerMsg, Decoder, ServerToClientMsg};
use crate::{FileInfo, BLOCKING_POLL_INTERVAL};

pub(super) struct ReadServer<D: Decoder> {
    to_client_tx: Producer<ServerToClientMsg<D>>,
    from_client_rx: Consumer<ClientToServerMsg<D>>,
    close_signal_rx: Consumer<Option<HeapData<D::T>>>,

    decoder: D,

    block_pool: Vec<AudioBlock<D::T>>,
    cache_pool: Vec<AudioBlockCache<D::T>>,

    num_channels: usize,
    num_prefetch_blocks: usize,
    block_frames: usize,

    run: bool,
    client_closed: bool,
    poll_interval: Duration,
}

impl<D: Decoder> ReadServer<D> {
    #[allow(clippy::new_ret_no_self)] // TODO: Rename to `spawn` (breaking API change)
    #[allow(clippy::too_many_arguments)] // TODO: Reduce number of arguments
    pub(super) fn new(
        file: PathBuf,
        start_frame: usize,
        num_prefetch_blocks: usize,
        block_frames: usize,
        poll_interval: Duration,
        to_client_tx: Producer<ServerToClientMsg<D>>,
        from_client_rx: Consumer<ClientToServerMsg<D>>,
        close_signal_rx: Consumer<Option<HeapData<D::T>>>,
        additional_opts: D::AdditionalOpts,
    ) -> Result<FileInfo<D::FileParams>, D::OpenError> {
        let (mut open_tx, mut open_rx) =
            RingBuffer::<Result<FileInfo<D::FileParams>, D::OpenError>>::new(1);

        std::thread::spawn(move || {
            match D::new(
                file,
                start_frame,
                block_frames,
                poll_interval,
                additional_opts,
            ) {
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
                        num_channels: usize::from(num_channels),
                        num_prefetch_blocks,
                        block_frames,
                        run: true,
                        client_closed: false,
                        poll_interval,
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

            std::thread::sleep(BLOCKING_POLL_INTERVAL);
        }
    }

    fn run(mut self) {
        #[allow(clippy::type_complexity)] // TODO: Use a dedicated type for the elements
        let mut cache_requests: Vec<(usize, Option<AudioBlockCache<D::T>>, usize)> = Vec::new();

        while self.run {
            let mut do_sleep = true;

            // Check for close signal.
            if let Ok(heap_data) = self.close_signal_rx.pop() {
                // Drop heap data here.
                let _ = heap_data;
                self.run = false;
                self.client_closed = true;
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
                                AudioBlock::new(self.num_channels, self.block_frames),
                            ),
                        );

                        let decode_res = self.decoder.decode(&mut block);

                        match decode_res {
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
                                do_sleep = false;
                                break;
                            }
                        }
                    }
                    ClientToServerMsg::DisposeBlock { block } => {
                        // Store the block to be reused.
                        self.block_pool.push(block);
                    }
                    ClientToServerMsg::SeekTo { frame } => {
                        if let Err(e) = self.decoder.seek(frame) {
                            self.send_msg(ServerToClientMsg::FatalError(e));
                            self.run = false;
                            do_sleep = false;
                            break;
                        }
                    }
                    ClientToServerMsg::Cache {
                        cache_index,
                        cache,
                        start_frame,
                    } => {
                        // Prioritize read blocks over caching.
                        cache_requests.push((cache_index, cache, start_frame));
                    }
                    ClientToServerMsg::DisposeCache { cache } => {
                        // Store the cache to be reused.
                        self.cache_pool.push(cache);
                    }
                }
            }

            while let Some((cache_index, cache, start_frame)) = cache_requests.pop() {
                let mut cache = cache.unwrap_or(
                    // Try using one in the pool if it exists.
                    self.cache_pool.pop().unwrap_or(
                        // No caches in pool. Create a new one.
                        AudioBlockCache::new(
                            self.num_channels,
                            self.num_prefetch_blocks,
                            self.block_frames,
                        ),
                    ),
                );

                let playhead_frame = self.decoder.playhead_frame();

                // Seek to the position the client wants to cache.
                if let Err(e) = self.decoder.seek(start_frame) {
                    self.send_msg(ServerToClientMsg::FatalError(e));
                    self.run = false;
                    do_sleep = false;
                    break;
                }

                // Fill the cache
                for block in cache.blocks.iter_mut() {
                    let decode_res = self.decoder.decode(block);

                    if let Err(e) = decode_res {
                        self.send_msg(ServerToClientMsg::FatalError(e));
                        self.run = false;
                        do_sleep = false;
                        break;
                    }
                }

                // Seek back to the previous position.
                if let Err(e) = self.decoder.seek(playhead_frame) {
                    self.send_msg(ServerToClientMsg::FatalError(e));
                    self.run = false;
                    do_sleep = false;
                    break;
                }

                self.send_msg(ServerToClientMsg::CacheRes {
                    cache_index,
                    cache,
                    wanted_start_frame: start_frame,
                });

                // If any new messages have been received while caching, prioritize those
                // over filling any additional caches.
                if !self.from_client_rx.is_empty() {
                    do_sleep = false;
                    break;
                }
            }

            if do_sleep {
                std::thread::sleep(self.poll_interval);
            }
        }

        // If client has not closed yet, wait until it does before closing.
        if !self.client_closed {
            loop {
                if let Ok(heap_data) = self.close_signal_rx.pop() {
                    // Drop heap data here.
                    let _ = heap_data;
                    break;
                }

                std::thread::sleep(BLOCKING_POLL_INTERVAL);
            }
        }
    }

    fn send_msg(&mut self, msg: ServerToClientMsg<D>) {
        // Do nothing if stream has been closed.
        if !self.run {
            return;
        }

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
                self.client_closed = true;
                break;
            }

            std::thread::sleep(BLOCKING_POLL_INTERVAL);
        }

        // Push will never fail because we made sure a slot is available in the
        // previous step (or the stream has closed, in which case an error doesn't
        // matter).
        let _ = self.to_client_tx.push(msg);
    }
}

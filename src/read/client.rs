use rtrb::{Consumer, Producer};

use crate::{BLOCK_SIZE, SERVER_WAIT_TIME, SILENCE_BUFFER};

use super::{
    ClientToServerMsg, DataBlock, DataBlockCacheEntry, DataBlockEntry, FileInfo, HeapData,
    ReadData, ReadError, ServerToClientMsg,
};

pub struct ReadClient {
    to_server_tx: Producer<ClientToServerMsg>,
    from_server_rx: Consumer<ServerToClientMsg>,
    close_signal_tx: Producer<Option<HeapData>>,

    heap_data: Option<HeapData>,

    current_block_index: usize,
    next_block_index: usize,
    current_block_start_frame: usize,
    current_frame_in_block: usize,

    temp_cache_index: usize,
    temp_seek_cache_index: usize,
    is_temp_cache: bool,

    num_prefetch_blocks: usize,

    file_info: FileInfo,
    error: bool,
}

impl ReadClient {
    pub(crate) fn new(
        to_server_tx: Producer<ClientToServerMsg>,
        from_server_rx: Consumer<ServerToClientMsg>,
        close_signal_tx: Producer<Option<HeapData>>,
        start_frame: usize,
        num_prefetch_blocks: usize,
        max_num_caches: usize,
        file_info: FileInfo,
    ) -> Self {
        let read_buffer = DataBlock::new(file_info.num_channels);

        // Reserve the last two caches as temporary caches.
        let max_num_caches = max_num_caches + 2;

        let mut caches: Vec<DataBlockCacheEntry> = Vec::with_capacity(max_num_caches);
        for _ in 0..max_num_caches {
            caches.push(DataBlockCacheEntry {
                cache: None,
                wanted_start_frame: 0,
            });
        }

        let temp_cache_index = max_num_caches - 2;
        let temp_seek_cache_index = max_num_caches - 1;

        let mut prefetch_buffer: Vec<DataBlockEntry> = Vec::with_capacity(num_prefetch_blocks);
        let mut wanted_start_frame = start_frame;
        for _ in 0..num_prefetch_blocks {
            prefetch_buffer.push(DataBlockEntry {
                use_cache: None,
                block: None,
                wanted_start_frame,
            });

            wanted_start_frame += BLOCK_SIZE;
        }

        let heap_data = Some(HeapData {
            read_buffer,
            prefetch_buffer,
            caches,
        });

        Self {
            to_server_tx,
            from_server_rx,
            close_signal_tx,

            heap_data,

            current_block_index: 0,
            next_block_index: 1,
            current_block_start_frame: start_frame,
            current_frame_in_block: 0,

            temp_cache_index,
            temp_seek_cache_index,
            is_temp_cache: false,

            num_prefetch_blocks,

            file_info,
            error: false,
        }
    }

    pub fn num_caches(&self) -> usize {
        // This check should never fail because it can only be `None` in the destructor.
        if let Some(heap) = &self.heap_data {
            heap.caches.len() - 2
        } else {
            0
        }
    }

    pub fn cache(&mut self, cache_index: usize, start_frame: usize) -> Result<bool, ReadError> {
        if self.error {
            return Err(ReadError::ServerClosed);
        }

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self
            .heap_data
            .as_mut()
            .ok_or_else(|| ReadError::UnknownFatalError)?;

        if self.is_temp_cache {
            self.is_temp_cache = false;
        } else if cache_index >= heap.caches.len() - 2 {
            return Err(ReadError::CacheIndexOutOfRange {
                index: cache_index,
                caches_len: heap.caches.len() - 2,
            });
        }

        let mut do_cache = true;
        if heap.caches[cache_index].wanted_start_frame == start_frame {
            if heap.caches[cache_index].cache.is_some() {
                do_cache = false;
            }
        }

        if do_cache {
            // Check that at-least two message slots are open.
            if self.to_server_tx.slots() < 2 {
                return Err(ReadError::MsgChannelFull);
            }

            heap.caches[cache_index].wanted_start_frame = start_frame;
            let mut cache = heap.caches[cache_index].cache.take();

            // If any blocks are currently using this cache, then set this cache as the
            // temporary cache and tell each block to use that instead.
            let mut using_cache = false;
            for block in heap.prefetch_buffer.iter_mut() {
                if let Some(index) = block.use_cache {
                    if index == cache_index {
                        block.use_cache = Some(self.temp_cache_index);
                        using_cache = true;
                    }
                }
            }
            if using_cache {
                if let Some(cache) = heap.caches[self.temp_cache_index].cache.take() {
                    // Tell the server to deallocate the old temporary cache.
                    // This cannot fail because we made sure that a slot is available in
                    // the previous step.
                    let _ = self
                        .to_server_tx
                        .push(ClientToServerMsg::DisposeCache { cache });
                }

                heap.caches[self.temp_cache_index].cache = cache.take();
            }

            // This cannot fail because we made sure that a slot is available in
            // the previous step.
            let _ = self.to_server_tx.push(ClientToServerMsg::Cache {
                cache_index,
                cache,
                start_frame,
            });

            return Ok(false);
        }

        Ok(true)
    }

    pub fn seek_to(
        &mut self,
        start_frame: usize,
        cache_index: Option<usize>,
    ) -> Result<bool, ReadError> {
        if self.error {
            return Err(ReadError::ServerClosed);
        }

        // Check that at-least two message slots are open.
        if self.to_server_tx.slots() < 2 {
            return Err(ReadError::MsgChannelFull);
        }

        let cache_index = if let Some(cache_index) = cache_index {
            cache_index
        } else {
            self.is_temp_cache = true;
            self.temp_seek_cache_index
        };

        let cache_exists = self.cache(cache_index, start_frame)?;

        self.current_block_start_frame = start_frame;
        self.current_frame_in_block = 0;
        self.current_block_index = 0;
        self.next_block_index = 1;

        // Request the server to start fetching blocks ahead of the cache.
        // This cannot fail because we made sure that a slot is available in
        // the previous step.
        let _ = self.to_server_tx.push(ClientToServerMsg::SeekTo {
            frame: self.current_block_start_frame + (self.num_prefetch_blocks * BLOCK_SIZE),
        });

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self
            .heap_data
            .as_mut()
            .ok_or_else(|| ReadError::UnknownFatalError)?;

        // Tell each prefetch block to use the cache.
        for block in heap.prefetch_buffer.iter_mut() {
            block.use_cache = Some(cache_index);
        }

        Ok(cache_exists)
    }

    /// Returns true if there is data to be read, false otherwise.
    ///
    /// Note the `read()` method can still be called if this returns false,
    /// it will just output silence instead.
    pub fn is_ready(&mut self) -> Result<bool, ReadError> {
        self.poll()?;

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self
            .heap_data
            .as_ref()
            .ok_or_else(|| ReadError::UnknownFatalError)?;

        // Check if the next two blocks are ready.

        if let Some(cache_index) = heap.prefetch_buffer[self.current_block_index].use_cache {
            // This check should never fail because it can only be `None` in the destructor.
            if heap.caches[cache_index].cache.is_none() {
                // Cache has not been recieved yet.
                return Ok(false);
            }
        } else if heap.prefetch_buffer[self.current_block_index]
            .block
            .is_none()
        {
            // Block has not been recieved yet.
            return Ok(false);
        }

        if let Some(cache_index) = heap.prefetch_buffer[self.next_block_index].use_cache {
            // This check should never fail because it can only be `None` in the destructor.
            if heap.caches[cache_index].cache.is_none() {
                // Cache has not been recieved yet.
                return Ok(false);
            }
        } else if heap.prefetch_buffer[self.next_block_index].block.is_none() {
            // Block has not been recieved yet.
            return Ok(false);
        }

        Ok(true)
    }

    // This should not be used in a real-time situation.
    pub fn block_until_ready(&mut self) -> Result<(), ReadError> {
        loop {
            if self.is_ready()? {
                break;
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }

        Ok(())
    }

    fn poll(&mut self) -> Result<(), ReadError> {
        // Retrieve any data sent from the server.

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self
            .heap_data
            .as_mut()
            .ok_or_else(|| ReadError::UnknownFatalError)?;

        loop {
            // Check that there is at-least one slot open before popping the next message.
            if self.to_server_tx.is_full() {
                return Err(ReadError::MsgChannelFull);
            }

            if let Ok(msg) = self.from_server_rx.pop() {
                match msg {
                    ServerToClientMsg::ReadIntoBlockRes {
                        block_index,
                        block,
                        wanted_start_frame,
                    } => {
                        let prefetch_block = &mut heap.prefetch_buffer[block_index];

                        // Only use results from the latest request.
                        if wanted_start_frame == prefetch_block.wanted_start_frame {
                            if let Some(prefetch_block) = prefetch_block.block.take() {
                                // Tell the IO server to deallocate the old block.
                                // This cannot fail because we made sure that a slot is available in
                                // a previous step.
                                let _ = self.to_server_tx.push(ClientToServerMsg::DisposeBlock {
                                    block: prefetch_block,
                                });
                            }

                            // Store the new block into the prefetch buffer.
                            prefetch_block.block = Some(block);
                        } else {
                            // Tell the server to deallocate the block.
                            // This cannot fail because we made sure that a slot is available in
                            // a previous step.
                            let _ = self
                                .to_server_tx
                                .push(ClientToServerMsg::DisposeBlock { block });
                        }
                    }
                    ServerToClientMsg::CacheRes {
                        cache_index,
                        cache,
                        wanted_start_frame,
                    } => {
                        let cache_entry = &mut heap.caches[cache_index];

                        // Only use results from the latest request.
                        if wanted_start_frame == cache_entry.wanted_start_frame {
                            if let Some(cache_entry) = cache_entry.cache.take() {
                                // Tell the IO server to deallocate the old cache.
                                // This cannot fail because we made sure that a slot is available in
                                // a previous step.
                                let _ = self
                                    .to_server_tx
                                    .push(ClientToServerMsg::DisposeCache { cache: cache_entry });
                            }

                            // Store the new cache.
                            cache_entry.cache = Some(cache);
                        } else {
                            // Tell the server to deallocate the cache.
                            // This cannot fail because we made sure that a slot is available in
                            // a previous step.
                            let _ = self
                                .to_server_tx
                                .push(ClientToServerMsg::DisposeCache { cache });
                        }
                    }
                    ServerToClientMsg::FatalError(e) => {
                        self.error = true;
                        return Err(e.into());
                    }
                }
            } else {
                break;
            }
        }

        Ok(())
    }

    /// Read the next slice of data with length `length`.
    pub fn read(&mut self, mut frames: usize) -> Result<ReadData, ReadError> {
        if self.error {
            return Err(ReadError::ServerClosed);
        }

        if frames > BLOCK_SIZE {
            return Err(ReadError::ReadLengthOutOfRange(frames));
        }

        self.poll()?;

        // Check that there is at-least one slot open for when `advance_to_next_block()` is called.
        if self.to_server_tx.is_full() {
            return Err(ReadError::MsgChannelFull);
        }

        // Check if the end of the file was reached.
        if self.current_frame() >= self.file_info.num_frames {
            self.current_block_start_frame = 0;
            self.current_frame_in_block = 0;
            return Err(ReadError::EndOfFile);
        }
        let mut reached_end_of_file = false;
        if self.current_frame() + frames >= self.file_info.num_frames {
            frames = self.file_info.num_frames - self.current_frame();
            reached_end_of_file = true;
        }

        let end_frame_in_block = self.current_frame_in_block + frames;
        if end_frame_in_block > BLOCK_SIZE {
            // Data spans between two blocks, so two copies need to be performed.

            // Copy from first block.
            let first_len = BLOCK_SIZE - self.current_frame_in_block;
            let second_len = frames - first_len;
            {
                // This check should never fail because it can only be `None` in the destructor.
                let heap = self
                    .heap_data
                    .as_mut()
                    .ok_or_else(|| ReadError::UnknownFatalError)?;

                // Get the first block of data.
                let current_block_data = {
                    let current_block = &heap.prefetch_buffer[self.current_block_index];

                    if let Some(cache_index) = current_block.use_cache {
                        if let Some(cache) = &heap.caches[cache_index].cache {
                            Some(&cache.blocks[self.current_block_index])
                        } else {
                            // If cache is empty, output silence instead.
                            None
                        }
                    } else {
                        if let Some(block) = &current_block.block {
                            Some(block)
                        } else {
                            // TODO: warn of buffer underflow.
                            None
                        }
                    }
                };

                for i in 0..heap.read_buffer.block.len() {
                    let read_buffer_part = &mut heap.read_buffer.block[i][0..first_len];

                    let from_buffer_part = if let Some(block) = current_block_data {
                        &block.block[i]
                            [self.current_frame_in_block..self.current_frame_in_block + first_len]
                    } else {
                        // Output silence.
                        &SILENCE_BUFFER[0..first_len]
                    };

                    read_buffer_part.copy_from_slice(from_buffer_part);
                }

                // Keep this from growing indefinitely.
                //self.current_block_start_frame = current_block_start_frame;
            }

            self.advance_to_next_block()?;

            // Copy from second block
            {
                // This check should never fail because it can only be `None` in the destructor.
                let heap = self
                    .heap_data
                    .as_mut()
                    .ok_or_else(|| ReadError::UnknownFatalError)?;

                // Get the next block of data.
                let next_block_data = {
                    let next_block = &heap.prefetch_buffer[self.current_block_index];

                    if let Some(cache_index) = next_block.use_cache {
                        if let Some(cache) = &heap.caches[cache_index].cache {
                            Some(&cache.blocks[self.current_block_index])
                        } else {
                            // If cache is empty, output silence instead.
                            None
                        }
                    } else {
                        if let Some(block) = &next_block.block {
                            Some(block)
                        } else {
                            // TODO: warn of buffer underflow.
                            None
                        }
                    }
                };

                for i in 0..heap.read_buffer.block.len() {
                    let read_buffer_part =
                        &mut heap.read_buffer.block[i][first_len..first_len + second_len];

                    let from_buffer_part = if let Some(block) = next_block_data {
                        &block.block[i][0..second_len]
                    } else {
                        // Output silence.
                        &SILENCE_BUFFER[0..second_len]
                    };

                    read_buffer_part.copy_from_slice(from_buffer_part);
                }

                self.current_frame_in_block = second_len;
            }
        } else {
            // Only need to copy from current block.
            {
                // This check should never fail because it can only be `None` in the destructor.
                let heap = self
                    .heap_data
                    .as_mut()
                    .ok_or_else(|| ReadError::UnknownFatalError)?;

                // Get the first block of data.
                let current_block_data = {
                    let current_block = &heap.prefetch_buffer[self.current_block_index];

                    if let Some(cache_index) = current_block.use_cache {
                        if let Some(cache) = &heap.caches[cache_index].cache {
                            Some(&cache.blocks[self.current_block_index])
                        } else {
                            // If cache is empty, output silence instead.
                            None
                        }
                    } else {
                        if let Some(block) = &current_block.block {
                            Some(block)
                        } else {
                            // TODO: warn of buffer underflow.
                            None
                        }
                    }
                };

                for i in 0..heap.read_buffer.block.len() {
                    let read_buffer_part = &mut heap.read_buffer.block[i][0..frames];

                    let from_buffer_part = if let Some(block) = current_block_data {
                        &block.block[i]
                            [self.current_frame_in_block..self.current_frame_in_block + frames]
                    } else {
                        // Output silence.
                        &SILENCE_BUFFER[0..frames]
                    };

                    read_buffer_part.copy_from_slice(from_buffer_part);
                }
            }

            self.current_frame_in_block = end_frame_in_block;
            if self.current_frame_in_block == BLOCK_SIZE {
                self.advance_to_next_block()?;
                self.current_frame_in_block = 0;
            }
        }

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self
            .heap_data
            .as_mut()
            .ok_or_else(|| ReadError::UnknownFatalError)?;

        // This check should never fail because it can only be `None` in the destructor.
        Ok(ReadData::new(
            &heap.read_buffer,
            frames,
            reached_end_of_file,
        ))
    }

    fn advance_to_next_block(&mut self) -> Result<(), ReadError> {
        // This check should never fail because it can only be `None` in the destructor.
        let heap = self
            .heap_data
            .as_mut()
            .ok_or_else(|| ReadError::UnknownFatalError)?;

        let entry = &mut heap.prefetch_buffer[self.current_block_index];

        // Request a new block of data that is one block ahead of the
        // latest block in the prefetch buffer.
        let wanted_start_frame =
            self.current_block_start_frame + (self.num_prefetch_blocks * BLOCK_SIZE);

        entry.use_cache = None;
        entry.wanted_start_frame = wanted_start_frame;

        // This cannot fail because the caller function `read` makes sure there
        // is at-least one slot open before calling this function.
        let _ = self.to_server_tx.push(ClientToServerMsg::ReadIntoBlock {
            block_index: self.current_block_index,
            // Send block to be re-used by the IO server.
            block: entry.block.take(),
            start_frame: wanted_start_frame,
        });

        self.current_block_index += 1;
        if self.current_block_index >= self.num_prefetch_blocks {
            self.current_block_index = 0;
        }

        self.next_block_index += 1;
        if self.next_block_index >= self.num_prefetch_blocks {
            self.next_block_index = 0;
        }

        self.current_block_start_frame += BLOCK_SIZE;

        Ok(())
    }

    pub fn current_frame(&self) -> usize {
        self.current_block_start_frame + self.current_frame_in_block
    }

    pub fn info(&self) -> &FileInfo {
        &self.file_info
    }
}

impl Drop for ReadClient {
    fn drop(&mut self) {
        // Tell the server to deallocate any heap data.
        // This cannot fail because this is the only place the signal is ever sent.
        let _ = self.close_signal_tx.push(self.heap_data.take());
    }
}

use rtrb::{Consumer, Producer};

use super::{ClientToServerMsg, Encoder, HeapData, ServerToClientMsg, WriteBlock, WriteError};
use crate::{FileInfo, SERVER_WAIT_TIME};

/// A realtime-safe disk-streaming writer of audio files.
pub struct WriteDiskStream<E: Encoder> {
    to_server_tx: Producer<ClientToServerMsg<E>>,
    from_server_rx: Consumer<ServerToClientMsg<E>>,
    close_signal_tx: Producer<Option<HeapData<E::T>>>,

    heap_data: Option<HeapData<E::T>>,

    block_size: usize,

    file_info: FileInfo<E::FileParams>,
    restart_count: usize,
    finished: bool,
    finish_complete: bool,
    error: bool,
}

impl<E: Encoder> WriteDiskStream<E> {
    pub(crate) fn new(
        to_server_tx: Producer<ClientToServerMsg<E>>,
        from_server_rx: Consumer<ServerToClientMsg<E>>,
        close_signal_tx: Producer<Option<HeapData<E::T>>>,
        num_write_blocks: usize,
        block_size: usize,
        file_info: FileInfo<E::FileParams>,
    ) -> Self {
        let mut block_pool: Vec<WriteBlock<E::T>> = Vec::with_capacity(num_write_blocks);
        for _ in 0..num_write_blocks - 2 {
            block_pool.push(WriteBlock::new(
                usize::from(file_info.num_channels),
                block_size,
            ));
        }

        Self {
            to_server_tx,
            from_server_rx,
            close_signal_tx,

            heap_data: Some(HeapData {
                block_pool,
                current_block: Some(WriteBlock::new(
                    usize::from(file_info.num_channels),
                    block_size,
                )),
                next_block: Some(WriteBlock::new(
                    usize::from(file_info.num_channels),
                    block_size,
                )),
            }),

            block_size,

            file_info,
            restart_count: 0,
            finished: false,
            finish_complete: false,
            error: false,
        }
    }

    pub fn is_ready(&mut self) -> Result<bool, WriteError<E::FatalError>> {
        if self.error {
            return Err(WriteError::IOServerClosed);
        }

        if self.finished {
            return Err(WriteError::FileFinished);
        }

        self.poll()?;

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self.heap_data.as_ref().unwrap();

        Ok(heap.current_block.is_some()
            && heap.next_block.is_some()
            && !self.to_server_tx.is_full())
    }

    /// Blocks the current thread until the stream is ready to be written to.
    ///
    /// NOTE: This should ***never*** be used in a real-time thread..
    pub fn block_until_ready(&mut self) -> Result<(), WriteError<E::FatalError>> {
        loop {
            if self.is_ready()? {
                break;
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }

        Ok(())
    }

    pub fn write(&mut self, buffer: &mut [Vec<E::T>]) -> Result<(), WriteError<E::FatalError>> {
        if self.error {
            return Err(WriteError::IOServerClosed);
        }

        if self.finished {
            return Err(WriteError::FileFinished);
        }

        // Check that the buffer is valid.
        if buffer.len() != usize::from(self.file_info.num_channels) {
            return Err(WriteError::InvalidBuffer);
        }
        // Check buffer sizes.
        let buffer_len = buffer[0].len();
        if buffer_len > self.block_size {
            return Err(WriteError::BufferTooLong {
                buffer_len,
                block_size: self.block_size,
            });
        }
        for ch in buffer.iter().skip(1) {
            if ch.len() != buffer_len {
                return Err(WriteError::InvalidBuffer);
            }
        }

        self.poll()?;

        // Check that there is at-least one slot open.
        if self.to_server_tx.is_full() {
            return Err(WriteError::IOServerChannelFull);
        }

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self.heap_data.as_mut().unwrap();

        // Check that there are available blocks to write to.
        if let Some(mut current_block) = heap.current_block.take() {
            if let Some(mut next_block) = heap.next_block.take() {
                if current_block.written_frames + buffer_len > self.block_size {
                    // Need to copy to two blocks.

                    let first_len = self.block_size - current_block.written_frames;
                    let second_len = buffer_len - first_len;

                    // Copy into first block.
                    for (buffer_ch, write_ch) in buffer.iter().zip(current_block.block.iter_mut()) {
                        &mut write_ch[current_block.written_frames..]
                            .copy_from_slice(&buffer_ch[0..first_len]);
                    }
                    current_block.written_frames = self.block_size;

                    // Send the now filled block to the IO server for writing.
                    // This cannot fail because we made sure there was a slot open in
                    // a previous step.
                    current_block.restart_count = self.restart_count;
                    let _ = self.to_server_tx.push(ClientToServerMsg::WriteBlock {
                        block: current_block,
                    });

                    // Copy the remaining data into the second block.
                    for (buffer_ch, write_ch) in buffer.iter().zip(next_block.block.iter_mut()) {
                        &mut write_ch[0..second_len].copy_from_slice(&buffer_ch[first_len..]);
                    }
                    next_block.written_frames = second_len;

                    // Move the next-up block into the current block.
                    heap.current_block = Some(next_block);

                    // Try to use one of the blocks from the pool for the next-up block.
                    heap.next_block = heap.block_pool.pop();
                } else {
                    // Only need to copy to first block.

                    let end = current_block.written_frames + buffer_len;

                    for (buffer_ch, write_ch) in buffer.iter().zip(current_block.block.iter_mut()) {
                        &mut write_ch[current_block.written_frames..end].copy_from_slice(buffer_ch);
                    }
                    current_block.written_frames = end;

                    if current_block.written_frames == self.block_size {
                        // Block is filled. Sent it to the IO server for writing.
                        // This cannot fail because we made sure there was a slot open in
                        // a previous step.
                        current_block.restart_count = self.restart_count;
                        let _ = self.to_server_tx.push(ClientToServerMsg::WriteBlock {
                            block: current_block,
                        });

                        // Move the next-up block into the current block.
                        heap.current_block = Some(next_block);

                        // Try to use one of the blocks from the pool for the next block.
                        heap.next_block = heap.block_pool.pop();
                    } else {
                        heap.current_block = Some(current_block);
                        heap.next_block = Some(next_block);
                    }
                }
            } else {
                heap.current_block = Some(current_block);
                return Err(WriteError::Underflow);
            }
        } else {
            return Err(WriteError::Underflow);
        }

        Ok(())
    }

    pub fn finish_and_close(&mut self) -> Result<(), WriteError<E::FatalError>> {
        if self.error {
            return Err(WriteError::IOServerClosed);
        }

        if self.finished {
            return Err(WriteError::FileFinished);
        }

        // Check that there is at-least one slot open.
        if self.to_server_tx.is_full() {
            return Err(WriteError::IOServerChannelFull);
        }

        // This cannot fail because we made sure there was a slot open in
        // a previous step.
        let _ = self.to_server_tx.push(ClientToServerMsg::FinishFile);

        self.finished = true;

        Ok(())
    }

    pub fn discard_and_close(&mut self) -> Result<(), WriteError<E::FatalError>> {
        if self.error {
            return Err(WriteError::IOServerClosed);
        }

        if self.finished {
            return Err(WriteError::FileFinished);
        }

        // Check that there is at-least one slot open.
        if self.to_server_tx.is_full() {
            return Err(WriteError::IOServerChannelFull);
        }

        // This cannot fail because we made sure there was a slot open in
        // a previous step.
        let _ = self.to_server_tx.push(ClientToServerMsg::DiscardFile);

        self.finished = true;

        Ok(())
    }

    pub fn discard_and_restart(&mut self) -> Result<(), WriteError<E::FatalError>> {
        if self.error {
            return Err(WriteError::IOServerClosed);
        }

        if self.finished {
            return Err(WriteError::FileFinished);
        }

        // Check that there is at-least one slot open.
        if self.to_server_tx.is_full() {
            return Err(WriteError::IOServerChannelFull);
        }

        // This cannot fail because we made sure there was a slot open in
        // a previous step.
        let _ = self.to_server_tx.push(ClientToServerMsg::DiscardAndRestart);

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self.heap_data.as_mut().unwrap();

        if let Some(block) = &mut heap.current_block {
            block.written_frames = 0;
        }

        self.restart_count += 1;

        Ok(())
    }

    fn poll(&mut self) -> Result<(), WriteError<E::FatalError>> {
        if self.error {
            return Err(WriteError::IOServerClosed);
        }

        // Retrieve any data sent from the server.

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self.heap_data.as_mut().unwrap();

        while let Ok(msg) = self.from_server_rx.pop() {
            match msg {
                ServerToClientMsg::NewWriteBlock { block } => {
                    if heap.current_block.is_none() {
                        heap.current_block = Some(block);
                    } else if heap.next_block.is_none() {
                        heap.next_block = Some(block);
                    } else {
                        // Store the block in the pool.
                        // This will never allocate new data because the server can
                        // only send blocks that have been sent to it by this client.
                        heap.block_pool.push(block);
                    }
                }
                ServerToClientMsg::Finished => {
                    self.finish_complete = true;
                }
                ServerToClientMsg::FatalError(e) => {
                    self.error = true;
                    return Err(WriteError::FatalError(e));
                }
            }
        }

        Ok(())
    }

    pub fn finish_complete(&self) -> bool {
        self.finish_complete
    }
}

impl<E: Encoder> Drop for WriteDiskStream<E> {
    fn drop(&mut self) {
        // Tell the server to deallocate any heap data.
        // This cannot fail because this is the only place the signal is ever sent.
        let _ = self.close_signal_tx.push(self.heap_data.take());
    }
}

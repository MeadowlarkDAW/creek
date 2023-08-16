use rtrb::{Consumer, Producer, RingBuffer};
use std::path::PathBuf;

use super::data::HeapData;
use super::error::{FatalWriteError, WriteError};
use super::{
    ClientToServerMsg, Encoder, ServerToClientMsg, WriteBlock, WriteServer, WriteStreamOptions,
};
use crate::{FileInfo, BLOCKING_POLL_INTERVAL};

/// A realtime-safe disk-streaming writer of audio files.
pub struct WriteDiskStream<E: Encoder> {
    to_server_tx: Producer<ClientToServerMsg<E>>,
    from_server_rx: Consumer<ServerToClientMsg<E>>,
    close_signal_tx: Producer<Option<HeapData<E::T>>>,

    heap_data: Option<HeapData<E::T>>,

    block_frames: usize,

    file_info: FileInfo<E::FileParams>,
    restart_count: usize,
    closed: bool,
    file_finished: bool,
    fatal_error: bool,

    num_files: u32,
}

impl<E: Encoder> WriteDiskStream<E> {
    /// Open a new realtime-safe disk-streaming writer.
    ///
    /// * `file` - The path to the file to open.
    /// * `num_channels` - The number of channels in the file.
    /// * `sample_rate` - The sample rate of the file.
    /// * `stream_opts` - Additional stream options.
    pub fn new<P: Into<PathBuf>>(
        file: P,
        num_channels: u16,
        sample_rate: u32,
        stream_opts: WriteStreamOptions<E>,
    ) -> Result<WriteDiskStream<E>, E::OpenError> {
        assert_ne!(num_channels, 0);
        assert_ne!(sample_rate, 0);
        assert_ne!(stream_opts.block_frames, 0);
        assert!(stream_opts.num_write_blocks > 2);
        assert_ne!(stream_opts.server_msg_channel_size, Some(0));

        // Reserve ample space for the message channels.
        let msg_channel_size = stream_opts
            .server_msg_channel_size
            .unwrap_or((stream_opts.num_write_blocks * 4) + 8);

        let (to_server_tx, from_client_rx) =
            RingBuffer::<ClientToServerMsg<E>>::new(msg_channel_size);
        let (to_client_tx, from_server_rx) =
            RingBuffer::<ServerToClientMsg<E>>::new(msg_channel_size);

        // Create dedicated close signal.
        let (close_signal_tx, close_signal_rx) = RingBuffer::<Option<HeapData<E::T>>>::new(1);

        let poll_interval = stream_opts
            .encoder_poll_interval
            .unwrap_or(E::DEFAULT_POLL_INTERVAL);

        let file: PathBuf = file.into();

        match WriteServer::spawn(
            file,
            stream_opts.num_write_blocks,
            stream_opts.block_frames,
            num_channels,
            sample_rate,
            poll_interval,
            to_client_tx,
            from_client_rx,
            close_signal_rx,
            stream_opts.additional_opts,
        ) {
            Ok(file_info) => {
                let client = WriteDiskStream::create(
                    to_server_tx,
                    from_server_rx,
                    close_signal_tx,
                    stream_opts.num_write_blocks,
                    stream_opts.block_frames,
                    file_info,
                );

                Ok(client)
            }
            Err(e) => Err(e),
        }
    }

    fn create(
        to_server_tx: Producer<ClientToServerMsg<E>>,
        from_server_rx: Consumer<ServerToClientMsg<E>>,
        close_signal_tx: Producer<Option<HeapData<E::T>>>,
        num_write_blocks: usize,
        block_frames: usize,
        file_info: FileInfo<E::FileParams>,
    ) -> Self {
        let mut block_pool: Vec<WriteBlock<E::T>> = (0..num_write_blocks)
            .map(|_| WriteBlock::new(usize::from(file_info.num_channels), block_frames))
            .collect();

        let current_block = block_pool.pop();
        let next_block = block_pool.pop();

        Self {
            to_server_tx,
            from_server_rx,
            close_signal_tx,

            heap_data: Some(HeapData {
                block_pool,
                current_block,
                next_block,
            }),

            block_frames,

            file_info,
            restart_count: 0,
            closed: false,
            file_finished: false,
            fatal_error: false,

            num_files: 1,
        }
    }

    /// Returns true if the stream is ready for writing, false otherwise.
    ///
    /// This is realtime-safe.
    ///
    /// In theory this should never return false, but this function is here
    /// as a sanity-check.
    pub fn is_ready(&mut self) -> Result<bool, WriteError<E::FatalError>> {
        if self.fatal_error || self.closed {
            return Err(WriteError::FatalError(FatalWriteError::StreamClosed));
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
    /// This is ***not*** realtime-safe.
    ///
    /// In theory you shouldn't need this, but this function is here
    /// as a sanity-check.
    pub fn block_until_ready(&mut self) -> Result<(), WriteError<E::FatalError>> {
        loop {
            if self.is_ready()? {
                break;
            }

            std::thread::sleep(BLOCKING_POLL_INTERVAL);
        }

        Ok(())
    }

    /// Write the buffer of frames into the file.
    ///
    /// This is realtime-safe.
    ///
    /// Some codecs (like WAV) have a maximum size of 4GB. If more than 4GB of data is
    /// pushed to this stream, then a new file will automatically be created to hold
    /// more data. The name of this file will be the same name as the main file with
    /// "_XXX" appended to the end (i.e. "_001", "_002", etc.).
    /// `WriteDiskStream::num_files()` can be used to get the total numbers of files that
    /// have been created.
    pub fn write(&mut self, buffer: &[&[E::T]]) -> Result<(), WriteError<E::FatalError>> {
        if self.fatal_error || self.closed {
            return Err(WriteError::FatalError(FatalWriteError::StreamClosed));
        }

        // Check that the buffer is valid.
        if buffer.len() != usize::from(self.file_info.num_channels) {
            return Err(WriteError::InvalidBuffer);
        }
        // Check buffer sizes.
        let buffer_len = buffer[0].len();
        if buffer_len > self.block_frames {
            return Err(WriteError::BufferTooLong {
                buffer_len,
                block_frames: self.block_frames,
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
                if current_block.block.frames_written + buffer_len > self.block_frames {
                    // Need to copy to two blocks.

                    let first_len = self.block_frames - current_block.block.frames_written;
                    let second_len = buffer_len - first_len;

                    // Copy into first block.
                    for (buffer_ch, write_ch) in
                        buffer.iter().zip(current_block.block.channels.iter_mut())
                    {
                        write_ch[current_block.block.frames_written..]
                            .copy_from_slice(&buffer_ch[0..first_len]);
                    }
                    current_block.block.frames_written = self.block_frames;

                    // Send the now filled block to the IO server for writing.
                    // This cannot fail because we made sure there was a slot open in
                    // a previous step.
                    current_block.restart_count = self.restart_count;
                    let _ = self.to_server_tx.push(ClientToServerMsg::WriteBlock {
                        block: current_block,
                    });

                    // Copy the remaining data into the second block.
                    for (buffer_ch, write_ch) in
                        buffer.iter().zip(next_block.block.channels.iter_mut())
                    {
                        write_ch[0..second_len].copy_from_slice(&buffer_ch[first_len..]);
                    }
                    next_block.block.frames_written = second_len;

                    // Move the next-up block into the current block.
                    heap.current_block = Some(next_block);

                    // Try to use one of the blocks from the pool for the next-up block.
                    heap.next_block = heap.block_pool.pop();
                } else {
                    // Only need to copy to first block.

                    let end = current_block.block.frames_written + buffer_len;

                    for (buffer_ch, write_ch) in
                        buffer.iter().zip(current_block.block.channels.iter_mut())
                    {
                        write_ch[current_block.block.frames_written..end]
                            .copy_from_slice(buffer_ch);
                    }
                    current_block.block.frames_written = end;

                    if current_block.block.frames_written == self.block_frames {
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

                self.file_info.num_frames += buffer_len;
            } else {
                heap.current_block = Some(current_block);
                return Err(WriteError::Underflow);
            }
        } else {
            return Err(WriteError::Underflow);
        }

        Ok(())
    }

    /// Finish the file and close the stream. `WriteDiskStream::write()` cannot be used
    /// after calling this.
    ///
    /// This is realtime-safe.
    ///
    /// Because this method is realtime safe and doesn't block, the file may still be in
    /// the process of finishing when this method returns. If you wish to make sure that
    /// the file has successfully finished, periodically call
    /// `WriteDiskStream::poll_file_finished()` after this.
    pub fn finish_and_close(&mut self) -> Result<(), WriteError<E::FatalError>> {
        if self.fatal_error || self.closed {
            return Err(WriteError::FatalError(FatalWriteError::StreamClosed));
        }

        {
            // This check should never fail because it can only be `None` in the destructor.
            let heap = self.heap_data.as_mut().unwrap();

            if let Some(mut current_block) = heap.current_block.take() {
                if current_block.block.frames_written > 0 {
                    // Send the last bit of remaining samples to be encoded.

                    // Check that there is at-least one slot open.
                    if self.to_server_tx.is_full() {
                        return Err(WriteError::IOServerChannelFull);
                    }

                    current_block.restart_count = self.restart_count;
                    let _ = self.to_server_tx.push(ClientToServerMsg::WriteBlock {
                        block: current_block,
                    });
                } else {
                    heap.current_block = Some(current_block);
                }
            }
        }

        // Check that there is at-least one slot open.
        if self.to_server_tx.is_full() {
            return Err(WriteError::IOServerChannelFull);
        }

        // This cannot fail because we made sure there was a slot open in
        // a previous step.
        let _ = self.to_server_tx.push(ClientToServerMsg::FinishFile);

        Ok(())
    }

    /// Delete all files created by this stream and close the stream.
    /// `WriteDiskStream::write()` cannot be used after calling this.
    ///
    /// This is realtime-safe.
    ///
    /// Because this method is realtime safe and doesn't block, the file may still be in
    /// the process of finishing when this method returns. If you wish to make sure that
    /// the file has successfully finished, periodically call
    /// `WriteDiskStream::poll_file_finished()` after this.
    pub fn discard_and_close(&mut self) -> Result<(), WriteError<E::FatalError>> {
        if self.fatal_error || self.closed {
            return Err(WriteError::FatalError(FatalWriteError::StreamClosed));
        }

        // Check that there is at-least one slot open.
        if self.to_server_tx.is_full() {
            return Err(WriteError::IOServerChannelFull);
        }

        // This cannot fail because we made sure there was a slot open in
        // a previous step.
        let _ = self.to_server_tx.push(ClientToServerMsg::DiscardAndClose);

        self.closed = true;
        self.num_files = 0;

        Ok(())
    }

    /// Delete all files created by this stream and start over. `WriteDiskStream::write()`
    /// can continue to be called after calling this.
    ///
    /// This is realtime-safe.
    pub fn discard_and_restart(&mut self) -> Result<(), WriteError<E::FatalError>> {
        if self.fatal_error || self.closed {
            return Err(WriteError::FatalError(FatalWriteError::StreamClosed));
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
            block.block.frames_written = 0;
        }

        self.restart_count += 1;
        self.file_info.num_frames = 0;
        self.num_files = 1;

        Ok(())
    }

    /// Finish the file and consume the stream. This will block the current thread until
    /// either the file has successfully finished or an error is returned.
    ///
    /// This is ***not*** realtime-safe.
    pub fn finish_blocking(mut self) -> Result<(), WriteError<E::FatalError>> {
        self.finish_and_close()?;

        loop {
            if self.poll_file_finished()? {
                break;
            }

            std::thread::sleep(BLOCKING_POLL_INTERVAL);
        }

        Ok(())
    }

    /// Delete all files created by this stream and consume the stream. This will block
    /// the current thread until either the file has successfully finished or an error
    /// is returned.
    ///
    /// This is ***not*** realtime-safe.
    pub fn discard_blocking(mut self) -> Result<(), WriteError<E::FatalError>> {
        self.discard_and_close()?;

        loop {
            if self.poll_file_finished()? {
                break;
            }

            std::thread::sleep(BLOCKING_POLL_INTERVAL);
        }

        Ok(())
    }

    fn poll(&mut self) -> Result<(), WriteError<E::FatalError>> {
        if self.fatal_error {
            return Err(WriteError::FatalError(FatalWriteError::StreamClosed));
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
                ServerToClientMsg::FileFinished => {
                    self.file_finished = true;
                }
                ServerToClientMsg::ReachedMaxSize { num_files } => {
                    self.num_files = num_files;
                }
                ServerToClientMsg::FatalError(e) => {
                    self.fatal_error = true;
                    return Err(WriteError::FatalError(FatalWriteError::EncoderError(e)));
                }
            }
        }

        Ok(())
    }

    /// Return info about the file.
    ///
    /// This is realtime-safe.
    pub fn info(&self) -> &FileInfo<E::FileParams> {
        &self.file_info
    }

    /// Returns whether or not this stream is closed.
    ///
    /// This is realtime-safe.
    pub fn closed(&self) -> bool {
        self.closed
    }

    /// Poll to see if the file has successfully finished being written to disk after
    /// a call to `WriteDiskStream::finish_and_close()` (or successfully discarded
    /// after a call to `WriteDiskStream::discard_and_close()`)
    ///
    /// This is realtime-safe.
    ///
    /// If an error is returned, then it may mean that there was an error writing
    /// the file.
    pub fn poll_file_finished(&mut self) -> Result<bool, WriteError<E::FatalError>> {
        self.poll()?;

        Ok(self.file_finished)
    }

    /// Returns the total number of files created by this stream. This can be more
    /// than one depending on the codec and the number of written frames.
    ///
    /// This is realtime-safe.
    pub fn num_files(&self) -> u32 {
        self.num_files
    }
}

impl<E: Encoder> Drop for WriteDiskStream<E> {
    fn drop(&mut self) {
        // Tell the server to deallocate any heap data.
        // This cannot fail because this is the only place the signal is ever sent.
        let _ = self.close_signal_tx.push(self.heap_data.take());
    }
}

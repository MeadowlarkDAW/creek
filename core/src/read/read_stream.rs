use rtrb::{Consumer, Producer, RingBuffer};
use std::path::PathBuf;

use super::data::{DataBlockCacheEntry, DataBlockEntry};
use super::error::{FatalReadError, ReadError};
use super::{
    ClientToServerMsg, DataBlock, Decoder, HeapData, ReadData, ReadServer, ReadStreamOptions,
    ServerToClientMsg,
};
use crate::{FileInfo, SERVER_WAIT_TIME};

/// Describes how to search for suitable caches when seeking in a [`ReadDiskStream`].
///
/// If a suitable cache is found, then reading can resume immediately. If not, then
/// the stream will need to buffer before it can read data. In this case, you may
/// decide to either continue reading (which will return silence) or to pause
/// playback temporarily.
///
/// [`ReadDiskStream`]: struct.ReadDiskStream.html
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SeekMode {
    /// Automatically search for a suitable cache to use. This is the default mode.
    #[default]
    Auto,
    /// Only try one cache with the given index. If you already know a suitable cache,
    /// this can be more performant than searching each cache individually.
    TryOne(usize),
    /// Try the given cache with the given index, and if it is not suitable, automatically
    /// search for a suitable one. If you already know a suitable cache, this can be
    /// more performant than searching each cache individually.
    TryOneThenAuto(usize),
    /// Seek without searching for a suitable cache. This **will** cause the stream
    /// to buffer.
    NoCache,
}

/// A realtime-safe disk-streaming reader of audio files.
pub struct ReadDiskStream<D: Decoder> {
    to_server_tx: Producer<ClientToServerMsg<D>>,
    from_server_rx: Consumer<ServerToClientMsg<D>>,
    close_signal_tx: Producer<Option<HeapData<D::T>>>,

    heap_data: Option<HeapData<D::T>>,

    current_block_index: usize,
    next_block_index: usize,
    current_block_start_frame: usize,
    current_frame_in_block: usize,

    temp_cache_index: usize,
    temp_seek_cache_index: usize,

    num_prefetch_blocks: usize,
    prefetch_size: usize,
    cache_size: usize,
    block_size: usize,

    file_info: FileInfo<D::FileParams>,
    fatal_error: bool,
}

impl<D: Decoder> ReadDiskStream<D> {
    /// Open a new realtime-safe disk-streaming reader.
    ///
    /// * `file` - The path to the file to open.
    /// * `start_frame` - The frame in the file to start reading from.
    /// * `stream_opts` - Additional stream options.
    pub fn new<P: Into<PathBuf>>(
        file: P,
        start_frame: usize,
        stream_opts: ReadStreamOptions<D>,
    ) -> Result<ReadDiskStream<D>, D::OpenError> {
        assert_ne!(stream_opts.block_size, 0);
        assert_ne!(stream_opts.num_look_ahead_blocks, 0);
        assert_ne!(stream_opts.server_msg_channel_size, Some(0));

        // Reserve ample space for the message channels.
        let msg_channel_size = stream_opts.server_msg_channel_size.unwrap_or(
            ((stream_opts.num_cache_blocks + stream_opts.num_look_ahead_blocks) * 4)
                + (stream_opts.num_caches * 4)
                + 8,
        );

        let (to_server_tx, from_client_rx) =
            RingBuffer::<ClientToServerMsg<D>>::new(msg_channel_size);
        let (to_client_tx, from_server_rx) =
            RingBuffer::<ServerToClientMsg<D>>::new(msg_channel_size);

        // Create dedicated close signal.
        let (close_signal_tx, close_signal_rx) = RingBuffer::<Option<HeapData<D::T>>>::new(1);

        let file: PathBuf = file.into();

        match ReadServer::new(
            file,
            start_frame,
            stream_opts.num_cache_blocks + stream_opts.num_look_ahead_blocks,
            stream_opts.block_size,
            to_client_tx,
            from_client_rx,
            close_signal_rx,
            stream_opts.additional_opts,
        ) {
            Ok(file_info) => {
                let client = ReadDiskStream::create(
                    to_server_tx,
                    from_server_rx,
                    close_signal_tx,
                    start_frame,
                    stream_opts.num_cache_blocks,
                    stream_opts.num_look_ahead_blocks,
                    stream_opts.num_caches,
                    stream_opts.block_size,
                    file_info,
                );

                Ok(client)
            }
            Err(e) => Err(e),
        }
    }

    #[allow(clippy::too_many_arguments)] // TODO: Reduce number of arguments
    pub(crate) fn create(
        to_server_tx: Producer<ClientToServerMsg<D>>,
        from_server_rx: Consumer<ServerToClientMsg<D>>,
        close_signal_tx: Producer<Option<HeapData<D::T>>>,
        start_frame: usize,
        num_cache_blocks: usize,
        num_look_ahead_blocks: usize,
        max_num_caches: usize,
        block_size: usize,
        file_info: FileInfo<D::FileParams>,
    ) -> Self {
        let num_prefetch_blocks = num_cache_blocks + num_look_ahead_blocks;

        let read_buffer = DataBlock::new(usize::from(file_info.num_channels), block_size);

        // Reserve the last two caches as temporary caches.
        let max_num_caches = max_num_caches + 2;

        let mut caches: Vec<DataBlockCacheEntry<D::T>> = Vec::with_capacity(max_num_caches);
        for _ in 0..max_num_caches {
            caches.push(DataBlockCacheEntry {
                cache: None,
                wanted_start_frame: 0,
            });
        }

        let temp_cache_index = max_num_caches - 1;
        let temp_seek_cache_index = max_num_caches - 2;

        let mut prefetch_buffer: Vec<DataBlockEntry<D::T>> =
            Vec::with_capacity(num_prefetch_blocks);
        let mut wanted_start_frame = start_frame;
        for _ in 0..num_prefetch_blocks {
            prefetch_buffer.push(DataBlockEntry {
                use_cache_index: None,
                block: None,
                wanted_start_frame,
            });

            wanted_start_frame += block_size;
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

            num_prefetch_blocks,
            prefetch_size: num_prefetch_blocks * block_size,
            cache_size: num_cache_blocks * block_size,
            block_size,

            file_info,
            fatal_error: false,
        }
    }

    /// Return the total number of caches available in this stream.
    ///
    /// This is realtime-safe.
    pub fn num_caches(&self) -> usize {
        // This check should never fail because it can only be `None` in the destructor.
        if let Some(heap) = &self.heap_data {
            heap.caches.len() - 2
        } else {
            0
        }
    }

    /// Returns whether a cache can be moved seamlessly without silencing current playback (true)
    /// or not (false).
    ///
    /// This is realtime-safe.
    ///
    /// If the position of a cache is changed while the playback stream is currently relying on it,
    /// then it will attempt to store the cache in a temporary buffer to allow playback to resume
    /// seamlessly.
    ///
    /// However, in the case where the cache is moved multiple times in quick succession while being
    /// relied on, then any blocks relying on the oldest cache will be silenced. In this case, (false)
    /// will be returned.
    pub fn can_move_cache(&mut self, cache_index: usize) -> bool {
        // This check should never fail because it can only be `None` in the destructor.
        let heap = self.heap_data.as_ref().unwrap();

        let mut using_cache = false;
        let mut using_temp_cache = false;
        for block in &heap.prefetch_buffer {
            if let Some(index) = block.use_cache_index {
                if index == cache_index {
                    using_cache = true;
                } else if index == self.temp_cache_index {
                    using_temp_cache = true;
                }
            }
        }

        !(using_cache && using_temp_cache)
    }

    /// Request to cache a new area in the file.
    ///
    /// This is realtime-safe.
    ///
    /// * `cache_index` - The index of the cache to use. Use `ReadDiskStream::num_caches()` to see
    /// how many caches have been assigned to this stream.
    /// * `start_frame` - The frame in the file to start filling in the cache from. If any portion lies
    /// outside the end of the file, then that portion will be ignored.
    ///
    /// If the cache already exists, then it will be overwritten. If the cache already starts from this
    /// position, then nothing will be done and (false) will be returned. Otherwise, (true) will be
    /// returned.
    ///
    /// In the case where the position of a cache is changed while the playback stream is currently
    /// relying on it, then it will attempt to store the cache in a temporary buffer to allow playback
    /// to resume seamlessly.
    ///
    /// However, in the case where the cache is moved multiple times in quick succession while being
    /// relied on, then any blocks relying on the oldest cache will be silenced. See
    /// `ReadDiskStream::can_move_cache()` to check if a cache can be seamlessly moved first.
    pub fn cache(
        &mut self,
        cache_index: usize,
        start_frame: usize,
    ) -> Result<bool, ReadError<D::FatalError>> {
        if self.fatal_error {
            return Err(ReadError::FatalError(FatalReadError::StreamClosed));
        }

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self.heap_data.as_mut().unwrap();

        if cache_index >= heap.caches.len() - 2 {
            return Err(ReadError::CacheIndexOutOfRange {
                index: cache_index,
                num_caches: heap.caches.len() - 2,
            });
        }

        if start_frame != heap.caches[cache_index].wanted_start_frame
            || heap.caches[cache_index].cache.is_none()
        {
            // Check that at-least two message slots are open.
            if self.to_server_tx.slots() < 2 + self.num_prefetch_blocks {
                return Err(ReadError::IOServerChannelFull);
            }

            heap.caches[cache_index].wanted_start_frame = start_frame;
            let mut cache = heap.caches[cache_index].cache.take();

            // If any blocks are currently using this cache, then set this cache as the
            // temporary cache and tell each block to use that instead.
            let mut using_cache = false;
            let mut using_temp_cache = false;
            for block in heap.prefetch_buffer.iter_mut() {
                if let Some(index) = block.use_cache_index {
                    if index == cache_index {
                        block.use_cache_index = Some(self.temp_cache_index);
                        using_cache = true;
                    } else if index == self.temp_cache_index {
                        using_temp_cache = true;
                    }
                }
            }
            if using_cache {
                if let Some(old_cache) = heap.caches[self.temp_cache_index].cache.take() {
                    // If any blocks are currently using the old temporary cache, dispose those blocks.
                    if using_temp_cache {
                        for block in heap.prefetch_buffer.iter_mut() {
                            if let Some(index) = block.use_cache_index {
                                if index == self.temp_cache_index {
                                    block.use_cache_index = None;
                                    if let Some(block) = block.block.take() {
                                        // Tell the server to deallocate the old block.
                                        // This cannot fail because we made sure that a slot is available in
                                        // the previous step.
                                        let _ = self
                                            .to_server_tx
                                            .push(ClientToServerMsg::DisposeBlock { block });
                                    }
                                }
                            }
                        }
                    }

                    // Tell the server to deallocate the old temporary cache.
                    // This cannot fail because we made sure that a slot is available in
                    // the previous step.
                    let _ = self
                        .to_server_tx
                        .push(ClientToServerMsg::DisposeCache { cache: old_cache });
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

            return Ok(true);
        }

        Ok(false)
    }

    /// Request to seek playback to a new position in the file.
    ///
    /// This is realtime-safe.
    ///
    /// * `frame` - The position in the file to seek to. If this lies outside of the end of
    /// the file, then playback will return silence.
    /// * `seek_mode` - Describes how to search for a suitable cache to use.
    ///
    /// If a suitable cache is found, then (true) is returned meaning that playback can resume immediately
    /// without any buffering. Otherwise (false) is returned meaning that playback will need to
    /// buffer first. In this case, you may choose to continue reading (which will return silence), or
    /// to pause playback temporarily.
    pub fn seek(
        &mut self,
        frame: usize,
        seek_mode: SeekMode,
    ) -> Result<bool, ReadError<D::FatalError>> {
        if self.fatal_error {
            return Err(ReadError::FatalError(FatalReadError::StreamClosed));
        }

        // Check that enough message slots are open.
        if self.to_server_tx.slots() < 3 + self.num_prefetch_blocks {
            return Err(ReadError::IOServerChannelFull);
        }

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self.heap_data.as_mut().unwrap();

        let mut found_cache = None;

        if let Some(cache_index) = match seek_mode {
            SeekMode::TryOne(cache_index) => Some(cache_index),
            SeekMode::TryOneThenAuto(cache_index) => Some(cache_index),
            _ => None,
        } {
            if heap.caches[cache_index].cache.is_some() {
                let cache_start_frame = heap.caches[cache_index].wanted_start_frame;
                if frame == cache_start_frame
                    || (frame > cache_start_frame && frame < cache_start_frame + self.cache_size)
                {
                    found_cache = Some(cache_index);
                }
            }
        }

        if found_cache.is_none() {
            let auto_search = match seek_mode {
                SeekMode::Auto | SeekMode::TryOneThenAuto(_) => true,
                SeekMode::NoCache | SeekMode::TryOne(_) => false,
            };

            if auto_search {
                // Check previous caches.
                for i in 0..heap.caches.len() - 2 {
                    if heap.caches[i].cache.is_some() {
                        let cache_start_frame = heap.caches[i].wanted_start_frame;
                        if frame == cache_start_frame
                            || (frame > cache_start_frame
                                && frame < cache_start_frame + self.cache_size)
                        {
                            found_cache = Some(i);
                            break;
                        }
                    }
                }
            }
        }

        if let Some(cache_index) = found_cache {
            // Find the position in the old cache.
            let cache_start_frame = heap.caches[cache_index].wanted_start_frame;
            let mut delta = frame - cache_start_frame;
            let mut block_i = 0;
            while delta >= self.block_size {
                block_i += 1;
                delta -= self.block_size
            }

            self.current_block_start_frame = cache_start_frame + (block_i * self.block_size);
            self.current_frame_in_block = delta;
            self.current_block_index = block_i;
            self.next_block_index = block_i + 1;
            if self.next_block_index >= self.num_prefetch_blocks {
                self.next_block_index = 0;
            }

            // Tell remaining blocks to use the cache.
            for i in block_i..heap.prefetch_buffer.len() {
                heap.prefetch_buffer[i].use_cache_index = Some(cache_index);
            }

            // Request the server to start fetching blocks ahead of the cache.
            // This cannot fail because we made sure that a slot is available in
            // the previous step.
            let mut wanted_start_frame = cache_start_frame + self.prefetch_size;
            let _ = self.to_server_tx.push(ClientToServerMsg::SeekTo {
                frame: wanted_start_frame,
            });

            // Fetch remaining blocks.
            for i in 0..block_i {
                // This cannot fail because we made sure there are enough slots available
                // in the previous step.
                let _ = self.to_server_tx.push(ClientToServerMsg::ReadIntoBlock {
                    block_index: i,
                    block: heap.prefetch_buffer[i].block.take(),
                    start_frame: wanted_start_frame,
                });
                heap.prefetch_buffer[i].use_cache_index = None;
                heap.prefetch_buffer[i].wanted_start_frame = wanted_start_frame;
                wanted_start_frame += self.block_size;
            }

            Ok(true)
        } else {
            // Create a new temporary seek cache.
            // This cannot fail because we made sure that a slot is available in
            // the previous step.
            heap.caches[self.temp_seek_cache_index].wanted_start_frame = frame;
            let _ = self.to_server_tx.push(ClientToServerMsg::Cache {
                cache_index: self.temp_seek_cache_index,
                cache: heap.caches[self.temp_seek_cache_index].cache.take(),
                start_frame: frame,
            });

            // Start from beginning of new cache.
            self.current_block_start_frame = frame;
            self.current_frame_in_block = 0;
            self.current_block_index = 0;
            self.next_block_index = 1;

            // Request the server to start fetching blocks ahead of the cache.
            // This cannot fail because we made sure that a slot is available in
            // the previous step.
            let _ = self.to_server_tx.push(ClientToServerMsg::SeekTo {
                frame: self.current_block_start_frame + self.prefetch_size,
            });

            // Tell each prefetch block to use the cache.
            for block in heap.prefetch_buffer.iter_mut() {
                block.use_cache_index = Some(self.temp_seek_cache_index);
            }

            Ok(false)
        }
    }

    /// Returns true if the stream is finished buffering and there is data can be read
    /// right now, false otherwise.
    ///
    /// This is realtime-safe.
    ///
    /// In the case where `false` is returned, then you may choose to continue reading
    /// (which will return silence), or to pause playback temporarily.
    pub fn is_ready(&mut self) -> Result<bool, ReadError<D::FatalError>> {
        self.poll()?;

        if self.to_server_tx.is_full() {
            return Ok(false);
        }

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self.heap_data.as_ref().unwrap();

        // Check if the next two blocks are ready.

        if let Some(cache_index) = heap.prefetch_buffer[self.current_block_index].use_cache_index {
            // This check should never fail because it can only be `None` in the destructor.
            if heap.caches[cache_index].cache.is_none() {
                // Cache has not been received yet.
                return Ok(false);
            }
        } else if heap.prefetch_buffer[self.current_block_index]
            .block
            .is_none()
        {
            // Block has not been received yet.
            return Ok(false);
        }

        if let Some(cache_index) = heap.prefetch_buffer[self.next_block_index].use_cache_index {
            // This check should never fail because it can only be `None` in the destructor.
            if heap.caches[cache_index].cache.is_none() {
                // Cache has not been received yet.
                return Ok(false);
            }
        } else if heap.prefetch_buffer[self.next_block_index].block.is_none() {
            // Block has not been received yet.
            return Ok(false);
        }

        Ok(true)
    }

    /// Blocks the current thread until the stream is done buffering.
    ///
    /// NOTE: This is ***not*** realtime-safe. This is only useful
    /// for making sure a stream is ready before sending it to a realtime thread.
    pub fn block_until_ready(&mut self) -> Result<(), ReadError<D::FatalError>> {
        loop {
            if self.is_ready()? {
                break;
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }

        Ok(())
    }

    /// Blocks the current thread until the given buffer is filled.
    ///
    /// NOTE: This is ***not*** realtime-safe.
    ///
    /// This will start reading from the stream's current playhead (this can be changed
    /// beforehand with `ReadDiskStream::seek()`). This is streaming, meaning the next call to
    /// `fill_buffer_blocking()` or `ReadDiskStream::read()` will pick up from where the previous
    /// call ended.
    ///
    /// ## Returns
    /// This will return the number of frames that were written to the buffer. This may be less
    /// than the length of the buffer if the end of the file was reached, so use this as a check
    /// if the entire buffer was filled or not.
    ///
    /// ## Error
    /// This will return an error if the number of channels in the buffer does not equal the number
    /// of channels in the stream, if the length of each channel is not the same, or if there was
    /// an internal error with reading the stream.
    pub fn fill_buffer_blocking(
        &mut self,
        buffer: &mut [Vec<D::T>],
    ) -> Result<usize, ReadError<D::FatalError>> {
        if buffer.len() != usize::from(self.file_info.num_channels) {
            return Err(ReadError::InvalidBuffer);
        }

        let buffer_len = buffer[0].len();

        // Sanity check that all channels are the same length.
        for ch in buffer.iter().skip(1) {
            if ch.len() != buffer_len {
                return Err(ReadError::InvalidBuffer);
            }
        }

        let mut frames_written = 0;
        while frames_written < buffer_len {
            let mut reached_end_of_file = false;

            while self.is_ready()? {
                let read_frames = (buffer_len - frames_written).min(self.block_size);

                let read_data = self.read(read_frames)?;
                for (i, ch) in buffer.iter_mut().enumerate() {
                    (*ch)[frames_written..frames_written + read_data.num_frames()]
                        .copy_from_slice(read_data.read_channel(i));
                }

                frames_written += read_data.num_frames();

                if read_data.reached_end_of_file() {
                    reached_end_of_file = true;
                    break;
                }
            }

            if reached_end_of_file {
                break;
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }

        Ok(frames_written)
    }

    fn poll(&mut self) -> Result<(), ReadError<D::FatalError>> {
        if self.fatal_error {
            return Err(ReadError::FatalError(FatalReadError::StreamClosed));
        }

        // Retrieve any data sent from the server.

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self.heap_data.as_mut().unwrap();

        loop {
            // Check that there is at-least one slot open before popping the next message.
            if self.to_server_tx.is_full() {
                return Err(ReadError::IOServerChannelFull);
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
                        self.fatal_error = true;
                        return Err(ReadError::FatalError(FatalReadError::DecoderError(e)));
                    }
                }
            } else {
                break;
            }
        }

        Ok(())
    }

    /// Read the next chunk of `frames` in the stream from the current playhead position.
    ///
    /// This is realtime-safe.
    ///
    /// This is *streaming*, meaning the next call to `read()` will pick up where the
    /// previous call left off.
    ///
    /// If the stream is currently buffering, (false) will be returned, and the playhead will still
    /// advance but will output silence. Otherwise, data can be read and (true) is returned. To check
    /// if the stream is ready beforehand, call `ReadDiskStream::is_ready()`.
    ///
    /// If the end of a file is reached, then only the amount of frames up to the end will be returned,
    /// and playback will return silence on each subsequent call to `read()`.
    ///
    /// NOTE: If the number of `frames` exceeds the block size of the decoder, then that block size
    /// will be used instead. This can be retrieved using `ReadDiskStream::block_size()`.
    pub fn read(
        &mut self,
        mut frames: usize,
    ) -> Result<ReadData<'_, D::T>, ReadError<D::FatalError>> {
        if self.fatal_error {
            return Err(ReadError::FatalError(FatalReadError::StreamClosed));
        }

        frames = frames.min(self.block_size);

        self.poll()?;

        // Check that there is at-least one slot open for when `advance_to_next_block()` is called.
        if self.to_server_tx.is_full() {
            return Err(ReadError::IOServerChannelFull);
        }

        // Check if the end of the file was reached.
        if self.playhead() >= self.file_info.num_frames {
            return Err(ReadError::EndOfFile);
        }
        let mut reached_end_of_file = false;
        if self.playhead() + frames >= self.file_info.num_frames {
            frames = self.file_info.num_frames - self.playhead();
            reached_end_of_file = true;
        }

        let end_frame_in_block = self.current_frame_in_block + frames;
        if end_frame_in_block > self.block_size {
            // Data spans between two blocks, so two copies need to be performed.

            // Copy from first block.
            let first_len = self.block_size - self.current_frame_in_block;
            let second_len = frames - first_len;
            {
                // This check should never fail because it can only be `None` in the destructor.
                let heap = self.heap_data.as_mut().unwrap();

                heap.read_buffer.clear();

                // Get the first block of data.
                let current_block_data = {
                    let current_block = &heap.prefetch_buffer[self.current_block_index];

                    match current_block.use_cache_index {
                        Some(cache_index) => {
                            if let Some(cache) = &heap.caches[cache_index].cache {
                                Some(&cache.blocks[self.current_block_index])
                            } else {
                                // If cache is empty, output silence instead.
                                None
                            }
                        }
                        None => {
                            if let Some(block) = &current_block.block {
                                Some(block)
                            } else {
                                // TODO: warn of buffer underflow.
                                None
                            }
                        }
                    }
                };

                if let Some(block) = current_block_data {
                    for (read_buffer_ch, block_ch) in
                        heap.read_buffer.block.iter_mut().zip(block.block.iter())
                    {
                        read_buffer_ch.extend_from_slice(
                            &block_ch[self.current_frame_in_block
                                ..self.current_frame_in_block + first_len],
                        );
                    }
                } else {
                    // Output silence.
                    for ch in heap.read_buffer.block.iter_mut() {
                        ch.resize(ch.len() + first_len, Default::default());
                    }
                }
            }

            self.advance_to_next_block()?;

            // Copy from second block
            {
                // This check should never fail because it can only be `None` in the destructor.
                let heap = self.heap_data.as_mut().unwrap();

                // Get the next block of data.
                let next_block_data = {
                    let next_block = &heap.prefetch_buffer[self.current_block_index];

                    match next_block.use_cache_index {
                        Some(cache_index) => {
                            if let Some(cache) = &heap.caches[cache_index].cache {
                                Some(&cache.blocks[self.current_block_index])
                            } else {
                                // If cache is empty, output silence instead.
                                None
                            }
                        }
                        None => {
                            if let Some(block) = &next_block.block {
                                Some(block)
                            } else {
                                // TODO: warn of buffer underflow.
                                None
                            }
                        }
                    }
                };

                if let Some(block) = next_block_data {
                    for (read_buffer_ch, block_ch) in
                        heap.read_buffer.block.iter_mut().zip(block.block.iter())
                    {
                        read_buffer_ch.extend_from_slice(&block_ch[0..second_len]);
                    }
                } else {
                    // Output silence.
                    for ch in heap.read_buffer.block.iter_mut() {
                        ch.resize(ch.len() + second_len, Default::default());
                    }
                }

                self.current_frame_in_block = second_len;
            }
        } else {
            // Only need to copy from current block.
            {
                // This check should never fail because it can only be `None` in the destructor.
                let heap = self.heap_data.as_mut().unwrap();

                heap.read_buffer.clear();

                // Get the first block of data.
                let current_block_data = {
                    let current_block = &heap.prefetch_buffer[self.current_block_index];

                    match current_block.use_cache_index {
                        Some(cache_index) => {
                            if let Some(cache) = &heap.caches[cache_index].cache {
                                Some(&cache.blocks[self.current_block_index])
                            } else {
                                // If cache is empty, output silence instead.
                                None
                            }
                        }
                        None => {
                            if let Some(block) = &current_block.block {
                                Some(block)
                            } else {
                                // TODO: warn of buffer underflow.
                                None
                            }
                        }
                    }
                };

                if let Some(block) = current_block_data {
                    for (read_buffer_ch, block_ch) in
                        heap.read_buffer.block.iter_mut().zip(block.block.iter())
                    {
                        read_buffer_ch.extend_from_slice(
                            &block_ch
                                [self.current_frame_in_block..self.current_frame_in_block + frames],
                        );
                    }
                } else {
                    // Output silence.
                    for ch in heap.read_buffer.block.iter_mut() {
                        ch.resize(ch.len() + frames, Default::default());
                    }
                }
            }

            self.current_frame_in_block = end_frame_in_block;
            if self.current_frame_in_block == self.block_size {
                self.advance_to_next_block()?;
                self.current_frame_in_block = 0;
            }
        }

        // This check should never fail because it can only be `None` in the destructor.
        let heap = self.heap_data.as_mut().unwrap();

        // This check should never fail because it can only be `None` in the destructor.
        Ok(ReadData::new(
            &heap.read_buffer,
            frames,
            reached_end_of_file,
        ))
    }

    fn advance_to_next_block(&mut self) -> Result<(), ReadError<D::FatalError>> {
        // This check should never fail because it can only be `None` in the destructor.
        let heap = self.heap_data.as_mut().unwrap();

        let entry = &mut heap.prefetch_buffer[self.current_block_index];

        // Request a new block of data that is one block ahead of the
        // latest block in the prefetch buffer.
        let wanted_start_frame = self.current_block_start_frame + (self.prefetch_size);

        entry.use_cache_index = None;
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

        self.current_block_start_frame += self.block_size;

        Ok(())
    }

    /// Return the current frame of the playhead.
    ///
    /// This is realtime-safe.
    pub fn playhead(&self) -> usize {
        self.current_block_start_frame + self.current_frame_in_block
    }

    /// Return info about the file.
    ///
    /// This is realtime-safe.
    pub fn info(&self) -> &FileInfo<D::FileParams> {
        &self.file_info
    }

    /// Return the block size used by this decoder.
    ///
    /// This is realtime-safe.
    pub fn block_size(&self) -> usize {
        self.block_size
    }
}

impl<D: Decoder> Drop for ReadDiskStream<D> {
    fn drop(&mut self) {
        // Tell the server to deallocate any heap data.
        // This cannot fail because this is the only place the signal is ever sent.
        let _ = self.close_signal_tx.push(self.heap_data.take());
    }
}

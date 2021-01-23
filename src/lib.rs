use core::num;

use rtrb::{Consumer, Producer, RingBuffer};

pub const BLOCK_SIZE: usize = 4096;
pub const NUM_PREFETCH_BLOCKS: usize = 4;

enum IOToStreamMsg {
    ReadIntoBlockRes {
        block_index: usize,
        block: DataBlock,
    },
}

enum StreamToIOMsg {
    ReadIntoBlock {
        block_index: usize,
        block: Option<DataBlock>,
        starting_smp_in_file: usize,
    },
    DisposeBlock {
        block: DataBlock,
    },
}

struct DataBlock {
    pub block: Vec<Vec<f32>>,
    pub starting_smp_in_file: usize,
}

struct DataBlockEntry {
    pub use_start_cache: bool,
    pub block: Option<DataBlock>,
    pub start_cache: DataBlock,
    pub wanted_start_smp: usize,
}

pub struct IOWorker {
    to_stream_tx: Producer<IOToStreamMsg>,
    from_stream_tx: Consumer<StreamToIOMsg>,
}

impl IOWorker {
    pub(crate) fn new(
        to_stream_tx: Producer<IOToStreamMsg>,
        from_stream_tx: Consumer<StreamToIOMsg>,
    ) -> Self {
        Self {
            to_stream_tx,
            from_stream_tx,
        }
    }
}

pub struct ReadData<'a> {
    data: &'a Vec<Vec<f32>>,
    len: usize,
}

impl<'a> ReadData<'a> {
    pub(crate) fn new(data: &'a Vec<Vec<f32>>, len: usize) -> Self {
        Self { data, len }
    }

    pub fn read_channel(&self, channel: usize) -> &[f32] {
        &self.data[channel][0..self.len]
    }

    pub fn num_channels(&self) -> usize {
        self.data.len()
    }

    pub fn buffer_len(&self) -> usize {
        self.len
    }
}

pub struct StreamReader {
    to_io_tx: Producer<StreamToIOMsg>,
    from_io_tx: Consumer<IOToStreamMsg>,

    read_buffer: Vec<Vec<f32>>,
    prefetch_buffer: Vec<DataBlockEntry>,

    num_channels: usize,
    current_block_index: usize,
    next_block_index: usize,
    current_block_file_smp: usize,
    current_smp_in_block: usize,
}

impl StreamReader {
    pub(crate) fn new(
        to_io_tx: Producer<StreamToIOMsg>,
        from_io_tx: Consumer<IOToStreamMsg>,
        mut start_cache: Vec<Option<DataBlock>>,
        starting_smp_in_file: usize,
        num_channels: usize,
    ) -> Self {
        let mut read_buffer: Vec<Vec<f32>> = Vec::with_capacity(num_channels);
        for _ in 0..num_channels {
            let mut data: Vec<f32> = Vec::with_capacity(BLOCK_SIZE);

            // Safe because algorithm will always ensure that data can only be
            // read once it's written to.
            unsafe {
                data.set_len(BLOCK_SIZE);
            }

            read_buffer.push(data);
        }

        let mut wanted_start_smp = starting_smp_in_file;
        let mut prefetch_buffer: Vec<DataBlockEntry> = Vec::with_capacity(NUM_PREFETCH_BLOCKS);
        for i in 0..NUM_PREFETCH_BLOCKS {
            let block = DataBlockEntry {
                use_start_cache: true,
                block: None,
                start_cache: start_cache[i].take().unwrap(),
                wanted_start_smp,
            };
            prefetch_buffer.push(block);

            wanted_start_smp += BLOCK_SIZE;
        }

        Self {
            to_io_tx,
            from_io_tx,

            read_buffer,
            prefetch_buffer,

            num_channels,
            current_block_index: 0,
            next_block_index: 1,
            current_block_file_smp: starting_smp_in_file,
            current_smp_in_block: 0,
        }
    }

    pub fn current_file_sample(&self) -> usize {
        self.current_block_file_smp + self.current_smp_in_block
    }

    /// Start again from the beginning of this stream.
    ///
    /// This is realtime-safe because the stream stores a cache of its starting data.
    pub fn go_to_start(&mut self) {
        let mut wanted_start_smp = self.prefetch_buffer[0].start_cache.starting_smp_in_file;

        for block in self.prefetch_buffer.iter_mut() {
            block.use_start_cache = true;
            block.wanted_start_smp = wanted_start_smp;

            wanted_start_smp += BLOCK_SIZE;
        }
    }

    /// Read the next slice of data with length `length`.
    ///
    /// PLEASE NOTE: You must call `is_ready()` beforehand on *all* streams that will be read. If *any* stream returns `false` there,
    /// then *all* playback must be stopped until all streams are ready again. If playback is not stopped,
    /// then `read()` will likely panic or return data from an incorrect playback position.
    ///
    /// ## Panics
    /// This will panic if `length` > `BLOCK_SIZE` (4096).
    pub fn read(&mut self, length: usize) -> ReadData {
        assert!(length <= BLOCK_SIZE);

        while let Ok(msg) = self.from_io_tx.pop() {
            match msg {
                IOToStreamMsg::ReadIntoBlockRes { block_index, block } => {
                    // Safe because `block_index` can only be sent from this struct, which makes sure it
                    // is always constrained to be in-bounds inside `advance_to_next_block()`.
                    let prefetch_block =
                        unsafe { self.prefetch_buffer.get_unchecked_mut(block_index) };

                    // Only use results from the latest request.
                    if block.starting_smp_in_file == prefetch_block.wanted_start_smp {
                        if let Some(prefetch_block) = prefetch_block.block.take() {
                            // Tell the IO server to deallocate the block.
                            self.to_io_tx
                                .push(StreamToIOMsg::DisposeBlock {
                                    block: prefetch_block,
                                })
                                .expect("Stream to IO serverful full");
                        }

                        prefetch_block.block = Some(block);
                    } else {
                        // Tell the IO server to deallocate the block.
                        self.to_io_tx
                            .push(StreamToIOMsg::DisposeBlock { block })
                            .expect("Stream to IO serverful full");
                    }
                }
            }
        }

        let current_block_data = {
            // Safe because indexes are always constrained to be in-bounds inside `advance_to_next_block()`.
            let current_block =
                unsafe { self.prefetch_buffer.get_unchecked(self.current_block_index) };

            if current_block.use_start_cache {
                &current_block.start_cache
            } else {
                if let Some(block) = &current_block.block {
                    block
                } else {
                    panic!("A buffer underflow occurred. Please check for underflows before calling read().")
                }
            }
        };

        let end_smp_in_block = self.current_smp_in_block + length;

        if end_smp_in_block > BLOCK_SIZE {
            let first_len = BLOCK_SIZE - self.current_smp_in_block;
            let second_len = length - first_len;

            // Copy from first block
            for i in 0..self.read_buffer.len() {
                // Safe because data blocks will always have the same number of channels.
                let read_buffer_part =
                    unsafe { &mut self.read_buffer.get_unchecked_mut(i)[0..first_len] };
                // Safe because data blocks will always have the same number of channels.
                let from_buffer_part = unsafe {
                    &current_block_data.block.get_unchecked(i)
                        [self.current_smp_in_block..self.current_smp_in_block + first_len]
                };

                read_buffer_part.copy_from_slice(from_buffer_part);
            }

            self.advance_to_next_block();

            let next_block_data = {
                // Safe because indexes are always constrained to be in-bounds inside `advance_to_next_block()`.
                let next_block =
                    unsafe { self.prefetch_buffer.get_unchecked(self.current_block_index) };

                if next_block.use_start_cache {
                    &next_block.start_cache
                } else {
                    if let Some(block) = &next_block.block {
                        block
                    } else {
                        panic!("A buffer underflow occurred. Please check for underflows before calling read().")
                    }
                }
            };

            // Copy from second block
            for i in 0..self.read_buffer.len() {
                // Safe because data blocks will always have the same number of channels.
                let read_buffer_part = unsafe {
                    &mut self.read_buffer.get_unchecked_mut(i)[first_len..first_len + second_len]
                };
                // Safe because data blocks will always have the same number of channels.
                let from_buffer_part =
                    unsafe { &next_block_data.block.get_unchecked(i)[0..second_len] };

                read_buffer_part.copy_from_slice(from_buffer_part);
            }

            // Advance.
            self.current_block_file_smp = next_block_data.starting_smp_in_file;
            self.current_smp_in_block = second_len;
        } else {
            // Only need to copy from current block.
            for i in 0..self.read_buffer.len() {
                // Safe because data blocks will always have the same number of channels.
                let read_buffer_part =
                    unsafe { &mut self.read_buffer.get_unchecked_mut(i)[0..length] };
                // Safe because data blocks will always have the same number of channels.
                let from_buffer_part = unsafe {
                    &current_block_data.block.get_unchecked(i)
                        [self.current_smp_in_block..self.current_smp_in_block + length]
                };

                read_buffer_part.copy_from_slice(from_buffer_part);
            }

            // Advance.
            self.current_smp_in_block = end_smp_in_block;
            if self.current_smp_in_block == BLOCK_SIZE {
                self.advance_to_next_block();

                // Safe because indexes are always constrained to be in-bounds inside `advance_to_next_block()`.
                self.current_block_file_smp = if let Some(next_block) = unsafe {
                    &self
                        .prefetch_buffer
                        .get_unchecked(self.current_block_index)
                        .block
                } {
                    next_block.starting_smp_in_file
                } else {
                    self.current_block_file_smp + BLOCK_SIZE
                };
                self.current_smp_in_block = 0;
            }
        }

        ReadData::new(&self.read_buffer, length)
    }

    /// Returns `true` when data can be read, `false`, otherwise.
    ///
    /// PLEASE NOTE: If *any* stream that will be read returns `false` here, then *all* playback must be stopped until all
    /// streams are ready again. If playback is not stopped, then `read()` will likely panic or return data
    /// from an incorrect playback position.
    #[inline]
    pub fn is_ready(&self) -> bool {
        // Safe because indexes are always constrained to be in-bounds inside `advance_to_next_block()`.
        unsafe {
            self.prefetch_buffer
                .get_unchecked(self.current_block_index)
                .block
                .is_some()
                && self
                    .prefetch_buffer
                    .get_unchecked(self.next_block_index)
                    .block
                    .is_some()
        }
    }

    fn advance_to_next_block(&mut self) {
        // Safe because indexes are always constrained to be in-bounds inside `advance_to_next_block()`.
        let block = unsafe {
            self.prefetch_buffer
                .get_unchecked_mut(self.current_block_index)
                .block
                .take()
        };

        self.to_io_tx
            .push(StreamToIOMsg::ReadIntoBlock {
                block_index: self.current_block_index,
                // Send block to be re-used by the IO server.
                block,
                starting_smp_in_file: self.current_block_file_smp
                    + (NUM_PREFETCH_BLOCKS * BLOCK_SIZE),
            })
            .expect("Stream to IO serverful full");

        self.current_block_index += 1;
        if self.current_block_index >= NUM_PREFETCH_BLOCKS {
            self.current_block_index = 0;
        }

        self.next_block_index += 1;
        if self.next_block_index >= NUM_PREFETCH_BLOCKS {
            self.next_block_index = 0;
        }
    }

    pub fn num_channels(&self) -> usize {
        self.num_channels
    }
}

pub struct StreamDispatcher {
    read_streams: Vec<StreamReader>,
}

impl StreamDispatcher {
    pub fn open_read_stream() {}
}

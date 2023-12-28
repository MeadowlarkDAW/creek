/// A prefetch data block.
pub struct DataBlock<T: Copy + Clone + Default + Send> {
    pub block: Vec<Vec<T>>,
}

impl<T: Copy + Clone + Default + Send> DataBlock<T> {
    pub fn new(num_channels: usize, block_size: usize) -> Self {
        DataBlock {
            block: (0..num_channels)
                .map(|_| Vec::with_capacity(block_size))
                .collect(),
        }
    }

    pub fn clear(&mut self) {
        for ch in self.block.iter_mut() {
            ch.clear();
        }
    }

    pub(crate) fn ensure_correct_size(&mut self, block_size: usize) {
        // If the decoder didn't fill enough frames, then fill the rest with zeros.
        for ch in self.block.iter_mut() {
            if ch.len() < block_size {
                ch.resize(block_size, Default::default());
            }
        }
    }
}

pub(crate) struct DataBlockCache<T: Copy + Clone + Default + Send> {
    pub blocks: Vec<DataBlock<T>>,
}

impl<T: Copy + Clone + Default + Send> DataBlockCache<T> {
    pub(crate) fn new(num_channels: usize, num_prefetch_blocks: usize, block_size: usize) -> Self {
        Self {
            blocks: (0..num_prefetch_blocks)
                .map(|_| DataBlock::new(num_channels, block_size))
                .collect(),
        }
    }
}

pub(crate) struct DataBlockEntry<T: Copy + Clone + Default + Send> {
    pub use_cache_index: Option<usize>,
    pub block: Option<DataBlock<T>>,
    pub wanted_start_frame: usize,
}

pub(crate) struct DataBlockCacheEntry<T: Copy + Clone + Default + Send> {
    pub cache: Option<DataBlockCache<T>>,
    pub wanted_start_frame: usize,
}

pub(crate) struct HeapData<T: Copy + Clone + Default + Send> {
    pub read_buffer: DataBlock<T>,
    pub prefetch_buffer: Vec<DataBlockEntry<T>>,
    pub caches: Vec<DataBlockCacheEntry<T>>,
}

/// The sample data returned by a `ReadClient`.
pub struct ReadData<'a, T: Copy + Clone + Default + Send> {
    data: &'a DataBlock<T>,
    len: usize,
    reached_end_of_file: bool,
}

impl<'a, T: Copy + Clone + Default + Send> ReadData<'a, T> {
    pub(crate) fn new(data: &'a DataBlock<T>, len: usize, reached_end_of_file: bool) -> Self {
        Self {
            data,
            len,
            reached_end_of_file,
        }
    }

    /// Read a single channel of samples.
    ///
    /// Use `ReadData::num_channels()` to get the number of available channels.
    ///
    /// The length of this data will be equal to `ReadData::num_frames()`.
    pub fn read_channel(&self, channel: usize) -> &[T] {
        &self.data.block[channel][0..self.len]
    }

    /// Return the number of channels in this data.
    pub fn num_channels(&self) -> usize {
        self.data.block.len()
    }

    /// Return the number of samples in a single channel of data.
    pub fn num_frames(&self) -> usize {
        self.len
    }

    /// This returns (true) if the last frame in this data is the end of the file,
    /// (false) otherwise.
    pub fn reached_end_of_file(&self) -> bool {
        self.reached_end_of_file
    }
}

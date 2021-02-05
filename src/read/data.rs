use crate::{BLOCK_SIZE, NUM_PREFETCH_BLOCKS};

pub struct DataBlock {
    pub block: Vec<[f32; BLOCK_SIZE]>,
    pub start_frame: usize,
}

impl DataBlock {
    pub(crate) fn new(num_channels: usize) -> Self {
        let mut block: Vec<[f32; BLOCK_SIZE]> = Vec::with_capacity(num_channels);
        for _ in 0..num_channels {
            // Safe because block will be always filled before it is sent to be read by the client.
            let data: [f32; BLOCK_SIZE] =
                unsafe { std::mem::MaybeUninit::<[f32; BLOCK_SIZE]>::uninit().assume_init() };
            block.push(data);
        }

        DataBlock {
            block,
            start_frame: 0,
        }
    }
}

pub(crate) struct DataBlockCache {
    pub blocks: Vec<DataBlock>,
}

impl DataBlockCache {
    pub(crate) fn new(num_channels: usize) -> Self {
        let mut blocks: Vec<DataBlock> = Vec::with_capacity(NUM_PREFETCH_BLOCKS);
        for _ in 0..NUM_PREFETCH_BLOCKS {
            blocks.push(DataBlock::new(num_channels));
        }

        Self { blocks }
    }
}

pub(crate) struct DataBlockEntry {
    pub use_cache: Option<usize>,
    pub block: Option<DataBlock>,
    pub wanted_start_frame: usize,
}

pub(crate) struct DataBlockCacheEntry {
    pub cache: Option<DataBlockCache>,
    pub wanted_start_frame: usize,
}

pub(crate) struct HeapData {
    pub read_buffer: DataBlock,
    pub prefetch_buffer: Vec<DataBlockEntry>,
    pub caches: Vec<DataBlockCacheEntry>,
}

/// The sample data returned by a `ReadClient`.
pub struct ReadData<'a> {
    data: &'a DataBlock,
    len: usize,
    reached_end_of_file: bool,
}

impl<'a> ReadData<'a> {
    pub(crate) fn new(data: &'a DataBlock, len: usize, reached_end_of_file: bool) -> Self {
        Self {
            data,
            len,
            reached_end_of_file,
        }
    }

    pub fn read_channel(&self, channel: usize) -> &[f32] {
        &self.data.block[channel][0..self.len]
    }

    pub fn num_channels(&self) -> usize {
        self.data.block.len()
    }

    pub fn num_frames(&self) -> usize {
        self.len
    }

    pub fn reached_end_of_file(&self) -> bool {
        self.reached_end_of_file
    }
}

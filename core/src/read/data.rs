use crate::AudioBlock;

pub(super) struct AudioBlockCache<T: Copy + Clone + Default + Send> {
    pub blocks: Vec<AudioBlock<T>>,
}

impl<T: Copy + Clone + Default + Send> AudioBlockCache<T> {
    pub(super) fn new(
        num_channels: usize,
        num_prefetch_blocks: usize,
        block_frames: usize,
    ) -> Self {
        let mut blocks: Vec<AudioBlock<T>> = Vec::with_capacity(num_prefetch_blocks);
        for _ in 0..num_prefetch_blocks {
            blocks.push(AudioBlock::new(num_channels, block_frames));
        }

        Self { blocks }
    }
}

pub(super) struct AudioBlockEntry<T: Copy + Clone + Default + Send> {
    pub use_cache_index: Option<usize>,
    pub block: Option<AudioBlock<T>>,
    pub wanted_start_frame: usize,
}

pub(super) struct AudioBlockCacheEntry<T: Copy + Clone + Default + Send> {
    pub cache: Option<AudioBlockCache<T>>,
    pub wanted_start_frame: usize,
}

pub(super) struct HeapData<T: Copy + Clone + Default + Send> {
    pub read_buffer: AudioBlock<T>,
    pub prefetch_buffer: Vec<AudioBlockEntry<T>>,
    pub caches: Vec<AudioBlockCacheEntry<T>>,
}

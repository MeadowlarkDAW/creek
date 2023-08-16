use crate::AudioBlock;

pub(super) struct WriteBlock<T: Copy + Clone + Default + Send> {
    pub block: AudioBlock<T>,
    pub restart_count: usize,
}

impl<T: Copy + Clone + Default + Send> WriteBlock<T> {
    pub fn new(num_channels: usize, block_frames: usize) -> Self {
        let mut block = AudioBlock::new(num_channels, block_frames);
        block.frames_written = 0;

        WriteBlock {
            block,
            restart_count: 0,
        }
    }
}

pub(super) struct HeapData<T: Copy + Clone + Default + Send> {
    pub block_pool: Vec<WriteBlock<T>>,
    pub current_block: Option<WriteBlock<T>>,
    pub next_block: Option<WriteBlock<T>>,
}

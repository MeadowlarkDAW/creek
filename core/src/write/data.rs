/// A block to write data to.
pub struct WriteBlock<T: Copy + Clone + Default + Send> {
    pub(crate) block: Vec<Vec<T>>,

    pub(crate) restart_count: usize,
}

impl<T: Copy + Clone + Default + Send> WriteBlock<T> {
    pub fn new(num_channels: usize, block_size: usize) -> Self {
        WriteBlock {
            block: (0..num_channels)
                .map(|_| Vec::with_capacity(block_size))
                .collect(),
            restart_count: 0,
        }
    }

    pub fn block(&self) -> &[Vec<T>] {
        self.block.as_slice()
    }

    pub fn written_frames(&self) -> usize {
        self.block[0].len()
    }

    pub fn clear(&mut self) {
        for ch in self.block.iter_mut() {
            ch.clear();
        }
    }
}

pub(crate) struct HeapData<T: Copy + Clone + Default + Send> {
    pub block_pool: Vec<WriteBlock<T>>,
    pub current_block: Option<WriteBlock<T>>,
    pub next_block: Option<WriteBlock<T>>,
}

/// A block to write data to.
pub struct WriteBlock<T: Copy + Clone + Default + Send> {
    pub(crate) block: Vec<Vec<T>>,

    pub(crate) written_frames: usize,
    pub(crate) restart_count: usize,
}

impl<T: Copy + Clone + Default + Send> WriteBlock<T> {
    pub fn new(num_channels: usize, block_size: usize) -> Self {
        let mut block: Vec<Vec<T>> = Vec::with_capacity(num_channels);
        for _ in 0..num_channels {
            let mut data: Vec<T> = Vec::with_capacity(block_size);
            // Safe because block will be always filled before it is sent to be
            // written by the IO server.
            unsafe { data.set_len(block_size) };
            block.push(data);
        }

        WriteBlock {
            block,
            written_frames: 0,
            restart_count: 0,
        }
    }

    pub fn block(&self) -> &[Vec<T>] {
        self.block.as_slice()
    }

    pub fn written_frames(&self) -> usize {
        self.written_frames
    }
}

pub(crate) struct HeapData<T: Copy + Clone + Default + Send> {
    pub block_pool: Vec<WriteBlock<T>>,
    pub current_block: Option<WriteBlock<T>>,
    pub next_block: Option<WriteBlock<T>>,
}

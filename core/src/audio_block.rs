/// A block of audio data.
pub struct AudioBlock<T: Copy + Clone + Default + Send> {
    /// The buffers of samples, one for each channel.
    ///
    /// Do not resize any of these `Vec`s.
    pub channels: Vec<Vec<T>>,
    pub(crate) frames_written: usize,
    block_frames: usize,
}

impl<T: Copy + Clone + Default + Send> AudioBlock<T> {
    pub fn new(num_channels: usize, block_frames: usize) -> Self {
        AudioBlock {
            channels: (0..num_channels)
                .map(|_| vec![Default::default(); block_frames])
                .collect(),
            frames_written: block_frames,
            block_frames,
        }
    }

    /// The number of frames written to this block (only relevant for
    /// write streams).
    pub fn frames_written(&self) -> usize {
        self.frames_written
    }

    /// The number of frames allocated to this block.
    pub fn capacity(&self) -> usize {
        self.block_frames
    }
}

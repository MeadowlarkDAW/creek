use std::path::PathBuf;
use std::{error::Error, fmt::Debug};

use super::DataBlock;

#[derive(Clone)]
pub struct FileInfo<FileParams> {
    pub params: FileParams,

    pub num_frames: usize,
    pub num_channels: usize,
    pub sample_rate: Option<u32>,
}

pub trait Decoder: Sized + 'static {
    type T: Copy + Clone + Default + Send;
    type AdditionalOpts: Send + Default + Debug;
    type FileParams: Clone + Send;
    type OpenError: Error + Send;
    type FatalError: Error + Send;

    const DEFAULT_BLOCK_SIZE: usize;
    const DEFAULT_NUM_CACHE_BLOCKS: usize;
    const DEFAULT_NUM_LOOK_AHEAD_BLOCKS: usize;
    const DEFAULT_NUM_CACHES: usize;

    fn new(
        file: PathBuf,
        start_frame: usize,
        block_size: usize,
        additional_opts: Self::AdditionalOpts,
    ) -> Result<(Self, FileInfo<Self::FileParams>), Self::OpenError>;

    fn seek(&mut self, frame: usize) -> Result<(), Self::FatalError>;

    unsafe fn decode(
        &mut self,
        data_block: &mut DataBlock<Self::T>,
    ) -> Result<(), Self::FatalError>;

    fn current_frame(&self) -> usize;
}

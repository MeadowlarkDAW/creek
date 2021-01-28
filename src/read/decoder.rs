use std::error::Error;
use std::path::PathBuf;

use super::DataBlock;

#[derive(Clone)]
pub struct FileInfo<Params: Send + Clone> {
    pub params: Params,

    pub num_frames: usize,
    pub num_channels: usize,
    pub sample_rate: u32,
}

pub trait Decoder: Sized {
    type OpenError: 'static + Error + Send;
    type DecodeWarning: Error + Send;
    type FatalError: Error + Send;
    type Params: Send + Clone;

    fn new(
        file: PathBuf,
        start_frame: usize,
    ) -> Result<(Self, FileInfo<Self::Params>), Self::OpenError>;

    fn seek_to(&mut self, frame: usize) -> Result<(), Self::FatalError>;

    fn decode_into(&mut self, block: &mut DataBlock) -> Result<(), Self::FatalError>;

    fn current_frame(&self) -> usize;
}

mod client;
mod data;
mod server;

pub mod error;

pub(crate) use data::{DataBlock, DataBlockCache, DataBlockCacheEntry, DataBlockEntry, HeapData};
pub(crate) use server::ReadServer;

pub use client::ReadClient;
pub use data::ReadData;

pub struct FileInfo {
    pub params: symphonia::core::codecs::CodecParameters,

    pub num_frames: usize,
    pub num_channels: usize,
    pub sample_rate: u32,
}

pub(crate) enum ServerToClientMsg {
    ReadIntoBlockRes {
        block_index: usize,
        block: DataBlock,
    },
    CacheRes {
        cache_index: usize,
        cache: DataBlockCache,
    },
    FatalError(symphonia::core::errors::Error),
}

pub(crate) enum ClientToServerMsg {
    ReadIntoBlock {
        block_index: usize,
        block: Option<DataBlock>,
        starting_frame_in_file: usize,
    },
    DisposeBlock {
        block: DataBlock,
    },
    SeekTo {
        frame: usize,
    },
    Cache {
        cache_index: usize,
        cache: Option<DataBlockCache>,
        starting_frame_in_file: usize,
    },
    DisposeCache {
        cache: DataBlockCache,
    },
}

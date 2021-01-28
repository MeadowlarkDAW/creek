mod client;
mod data;
mod decoder;
mod server;

pub(crate) use data::{DataBlockCache, DataBlockCacheEntry, DataBlockEntry, HeapData};
pub(crate) use server::ReadServer;

pub use client::{ReadClient, ReadError};
pub use data::DataBlock;
pub use data::ReadData;
pub use decoder::{Decoder, FileInfo};

pub(crate) enum ServerToClientMsg<D: Decoder> {
    ReadIntoBlockRes {
        block_index: usize,
        block: DataBlock,
    },
    CacheRes {
        cache_index: usize,
        cache: DataBlockCache,
    },
    FatalError(D::FatalError),
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

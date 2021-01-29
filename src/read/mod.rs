mod client;
mod data;
mod decoder;
mod server;

pub mod error;

pub(crate) use data::{DataBlockCache, DataBlockCacheEntry, DataBlockEntry, HeapData};
pub(crate) use server::ReadServer;

pub use client::ReadClient;
pub use data::DataBlock;
pub use data::ReadData;
pub use decoder::{Decoder, FileInfo};
pub use error::{OpenError, ReadError};

pub(crate) enum ServerToClientMsg {
    ReadIntoBlockRes {
        block_index: usize,
        block: DataBlock,
    },
    CacheRes {
        cache_index: usize,
        cache: DataBlockCache,
    },
    FatalError(ReadError),
}

pub(crate) enum ClientToServerMsg {
    ReadIntoBlock {
        block_index: usize,
        block: Option<DataBlock>,
        start_frame: usize,
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
        start_frame: usize,
    },
    DisposeCache {
        cache: DataBlockCache,
    },
}

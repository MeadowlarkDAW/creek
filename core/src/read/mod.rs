mod data;
mod decoder;
mod read_stream;
mod server;

pub mod error;

pub(crate) use data::{DataBlockCache, DataBlockCacheEntry, DataBlockEntry, HeapData};
pub(crate) use server::ReadServer;

pub use data::DataBlock;
pub use data::ReadData;
pub use decoder::{Decoder, FileInfo};
pub use error::ReadError;
pub use read_stream::{ReadDiskStream, SeekMode};

pub(crate) enum ServerToClientMsg<D: Decoder> {
    ReadIntoBlockRes {
        block_index: usize,
        block: DataBlock<D::T>,
        wanted_start_frame: usize,
    },
    CacheRes {
        cache_index: usize,
        cache: DataBlockCache<D::T>,
        wanted_start_frame: usize,
    },
    FatalError(D::FatalError),
}

pub(crate) enum ClientToServerMsg<D: Decoder> {
    ReadIntoBlock {
        block_index: usize,
        block: Option<DataBlock<D::T>>,
        start_frame: usize,
    },
    DisposeBlock {
        block: DataBlock<D::T>,
    },
    SeekTo {
        frame: usize,
    },
    Cache {
        cache_index: usize,
        cache: Option<DataBlockCache<D::T>>,
        start_frame: usize,
    },
    DisposeCache {
        cache: DataBlockCache<D::T>,
    },
}

use std::error::Error;

#[derive(Debug)]
pub enum WriteError<FatalError: Error> {
    FatalError(FatalError),
    Underflow,
    BufferTooLong {
        buffer_len: usize,
        block_size: usize,
    },
    InvalidBuffer,
    ReachedMaxSize {
        max_size_bytes: usize,
    },
    FileFinished,
    IOServerChannelFull,
    IOServerClosed,
}

impl<FatalError: Error> std::error::Error for WriteError<FatalError> {}

impl<FatalError: Error> std::fmt::Display for WriteError<FatalError> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::FatalError(e) => write!(f, "Fatal error: {:?}", e),
            WriteError::Underflow => write!(f, "Data could not be written because there are no more blocks left in the pool. Please make sure the number of write blocks allocated to this stream is sufficiently large enough."),
            WriteError::BufferTooLong { buffer_len, block_size } => write!(f, "Buffer with len {} is longer than the block size {}", buffer_len, block_size),
            WriteError::InvalidBuffer => write!(f, "Buffer does not match internal buffer layout"),
            WriteError::ReachedMaxSize { max_size_bytes } => write!(f, "File reached maximum size of {} bytes", max_size_bytes),
            WriteError::FileFinished => write!(f, "The file was either finished or discarded"),
            WriteError::IOServerChannelFull => write!(f, "The message channel to the IO server is full."),
            WriteError::IOServerClosed => write!(f, "Server closed unexpectedly"),
        }
    }
}

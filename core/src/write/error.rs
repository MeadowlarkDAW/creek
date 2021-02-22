use std::error::Error;

/// A fatal error occurred and the stream cannot continue.
#[derive(Debug)]
pub enum FatalWriteError<FatalEncoderError: Error> {
    /// The stream is closed and thus cannot continue.
    StreamClosed,
    /// A fatal encoder error occured. The stream cannot continue.
    EncoderError(FatalEncoderError),
}

/// An error writing the file.
#[derive(Debug)]
pub enum WriteError<FatalEncoderError: Error> {
    /// A fatal error occured. The stream cannot continue.
    FatalError(FatalWriteError<FatalEncoderError>),
    /// There are no more blocks left in the buffer because the server was
    /// too slow writing previous ones. Make sure there are enough write blocks
    /// available to the stream.
    ///
    /// In theory this should not happen, but if it does, try writing again
    /// later.
    ///
    /// If this is returned, then no data in the given buffer will be written
    /// to the file.
    Underflow,
    /// The given buffer is too long. The length of the buffer cannot exceed
    /// `block_size`. The value of `block_size` can be retrieved using
    /// `WriteDiskStream::block_size()`.
    ///
    /// If this is returned, then no data in the given buffer will be written
    /// to the file.
    BufferTooLong {
        buffer_len: usize,
        block_size: usize,
    },
    /// The given buffer does not match the internal layout of the stream. Check
    /// that the number of channels in both are the same.
    ///
    /// If this is returned, then no data in the given buffer will be written
    /// to the file.
    InvalidBuffer,
    /// The message channel to the IO server was full.
    ///
    /// In theory this should not happen, but if it does, try writing again
    /// later.
    ///
    /// If this is returned, then no data in the given buffer will be written
    /// to the file.
    IOServerChannelFull,
}

impl<FatalError: Error> std::error::Error for WriteError<FatalError> {}

impl<FatalError: Error> std::fmt::Display for WriteError<FatalError> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::FatalError(e) => {
                match e {
                    FatalWriteError::StreamClosed => write!(f, "Fatal error: stream is closed"),
                    FatalWriteError::EncoderError(ee) => write!(f, "Fatal encoder error: {:?}", ee),
                }
            }
            WriteError::Underflow => write!(f, "Data could not be written because there are no more blocks left in the pool. Please make sure the number of write blocks allocated to this stream is sufficiently large enough."),
            WriteError::BufferTooLong { buffer_len, block_size } => write!(f, "Buffer with len {} is longer than the block size {}", buffer_len, block_size),
            WriteError::InvalidBuffer => write!(f, "Buffer does not match internal buffer layout"),
            WriteError::IOServerChannelFull => write!(f, "The message channel to the IO server is full."),
        }
    }
}

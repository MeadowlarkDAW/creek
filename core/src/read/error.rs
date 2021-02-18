use std::error::Error;

#[derive(Debug)]
pub enum ReadError<FatalError: Error> {
    FatalError(FatalError),
    EndOfFile,
    CacheIndexOutOfRange { index: usize, caches_len: usize },
    IOServerChannelFull,
    IOServerClosed,
    InvalidBuffer,
    UnknownFatalError,
}

impl<FatalError: Error> std::error::Error for ReadError<FatalError> {}

impl<FatalError: Error> std::fmt::Display for ReadError<FatalError> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::FatalError(e) => write!(f, "Fatal error: {:?}", e),
            ReadError::EndOfFile => write!(f, "End of file"),
            ReadError::CacheIndexOutOfRange { index, caches_len } => {
                write!(
                    f,
                    "Cache index {} is out of range of the length of allocated caches {}",
                    index, caches_len
                )
            }
            ReadError::IOServerChannelFull => {
                write!(f, "The message channel to the IO server is full.")
            }
            ReadError::IOServerClosed => write!(f, "Server closed unexpectedly"),
            ReadError::InvalidBuffer => {
                write!(f, "Fill buffer does not match internal buffer layout")
            }
            ReadError::UnknownFatalError => write!(f, "An unkown fatal error occurred"),
        }
    }
}

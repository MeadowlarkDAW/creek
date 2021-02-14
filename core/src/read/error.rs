use std::error::Error;

#[derive(Debug)]
pub enum ReadError<FatalError: Error> {
    FatalError(FatalError),
    EndOfFile,
    CacheIndexOutOfRange { index: usize, caches_len: usize },
    MsgChannelFull,
    ServerClosed,
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
            ReadError::MsgChannelFull => write!(f, "The message channel to the server is full."),
            ReadError::ServerClosed => write!(f, "Server closed unexpectedly"),
            ReadError::UnknownFatalError => write!(f, "An unkown fatal error occurred"),
        }
    }
}

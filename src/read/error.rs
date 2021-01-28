use std::io;

use crate::BLOCK_SIZE;

#[derive(Debug)]
pub enum OpenError {
    Io(io::Error),
    Format(symphonia::core::errors::Error),
    NoDefaultStream,
    NoNumFrames,
    NoNumChannels,
}

impl std::error::Error for OpenError {}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Io(e) => write!(f, "IO error: {:?}", e),
            OpenError::Format(e) => write!(f, "Format error: {:?}", e),
            OpenError::NoDefaultStream => write!(f, "No default stream for codec"),
            OpenError::NoNumFrames => write!(f, "Failed to find the number of frames in the file"),
            OpenError::NoNumChannels => {
                write!(f, "Failed to find the number of channels in the file")
            }
        }
    }
}

impl From<io::Error> for OpenError {
    fn from(e: io::Error) -> Self {
        OpenError::Io(e)
    }
}

impl From<symphonia::core::errors::Error> for OpenError {
    fn from(e: symphonia::core::errors::Error) -> Self {
        OpenError::Format(e)
    }
}

#[derive(Debug)]
pub enum ReadError {
    FatalError(symphonia::core::errors::Error),
    CacheIndexOutOfRange { index: usize, caches_len: usize },
    MsgChannelFull,
    ReadLengthOutOfRange(usize),
    ServerClosed,
    UnknownFatalError,
}

impl std::error::Error for ReadError {}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::FatalError(e) => write!(f, "Fatal error: {:?}", e),
            ReadError::CacheIndexOutOfRange { index, caches_len } => {
                write!(
                    f,
                    "Cache index {} is out of range of the length of allocated caches {}",
                    index, caches_len
                )
            }
            ReadError::MsgChannelFull => write!(f, "The message channel to the server is full."),
            ReadError::ReadLengthOutOfRange(len) => write!(
                f,
                "Read length {} is out of range of maximum {}",
                len, BLOCK_SIZE
            ),
            ReadError::ServerClosed => write!(f, "Server closed unexpectedly"),
            ReadError::UnknownFatalError => write!(f, "An unkown fatal error occurred"),
        }
    }
}

impl From<symphonia::core::errors::Error> for ReadError {
    fn from(e: symphonia::core::errors::Error) -> Self {
        ReadError::FatalError(e)
    }
}

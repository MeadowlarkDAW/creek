use std::io;

use crate::Format;

#[derive(Debug)]
pub enum OpenError {
    Io(io::Error),
    CodecNotImplementedYet { num_channels: u16, format: Format },
}

impl std::error::Error for OpenError {}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Io(e) => write!(f, "IO error: {:?}", e),
            OpenError::CodecNotImplementedYet {
                num_channels,
                format,
            } => {
                write!(
                    f,
                    "Codec not implemented yet: num_channels: {}, format: {:?}",
                    num_channels, format
                )
            }
        }
    }
}

impl From<io::Error> for OpenError {
    fn from(e: io::Error) -> Self {
        OpenError::Io(e)
    }
}

#[derive(Debug)]
pub enum FatalError {
    Io(io::Error),
    ReachedMaxSize,
}

impl std::error::Error for FatalError {}

impl std::fmt::Display for FatalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FatalError::Io(e) => write!(f, "IO error: {:?}", e),
            FatalError::ReachedMaxSize => write!(f, "Reached maximum WAVE file size of 4GB"),
        }
    }
}

impl From<io::Error> for FatalError {
    fn from(e: io::Error) -> Self {
        FatalError::Io(e)
    }
}

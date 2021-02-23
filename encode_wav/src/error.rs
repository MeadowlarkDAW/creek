use std::io;

use crate::Format;

#[derive(Debug)]
pub enum WavOpenError {
    Io(io::Error),
    CodecNotImplementedYet { num_channels: u16, format: Format },
}

impl std::error::Error for WavOpenError {}

impl std::fmt::Display for WavOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WavOpenError::Io(e) => write!(f, "IO error: {:?}", e),
            WavOpenError::CodecNotImplementedYet {
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

impl From<io::Error> for WavOpenError {
    fn from(e: io::Error) -> Self {
        WavOpenError::Io(e)
    }
}

#[derive(Debug)]
pub enum WavFatalError {
    Io(io::Error),
    ReachedMaxSize,
    CouldNotGetFileName,
}

impl std::error::Error for WavFatalError {}

impl std::fmt::Display for WavFatalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WavFatalError::Io(e) => write!(f, "IO error: {:?}", e),
            WavFatalError::ReachedMaxSize => write!(f, "Reached maximum WAVE file size of 4GB"),
            WavFatalError::CouldNotGetFileName => {
                write!(f, "There was an error reading the name of the file")
            }
        }
    }
}

impl From<io::Error> for WavFatalError {
    fn from(e: io::Error) -> Self {
        WavFatalError::Io(e)
    }
}

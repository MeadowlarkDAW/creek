use std::io;

use crate::Format;

/// An error while opening a WAV file for writing.
#[derive(Debug)]
pub enum WavOpenError {
    /// IO error
    Io(io::Error),
    /// The given codec parameters are not currently supported in creek.
    CodecParamsNotSupported { num_channels: u16, format: Format },
}

impl std::error::Error for WavOpenError {}

impl std::fmt::Display for WavOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WavOpenError::Io(e) => write!(f, "IO error: {:?}", e),
            WavOpenError::CodecParamsNotSupported {
                num_channels,
                format,
            } => {
                write!(
                    f,
                    "Codec parameters are not currently supported in creek: num_channels: {}, format: {:?}",
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

/// A fatal error while running a WAV write stream.
#[derive(Debug)]
pub enum WavFatalError {
    /// IO error
    Io(io::Error),
    /// The maximum size for a WAV file was reached.
    ReachedMaxSize,
    /// There was an error reading the name of the file.
    CouldNotGetFileName,
}

impl std::error::Error for WavFatalError {}

impl std::fmt::Display for WavFatalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WavFatalError::Io(e) => write!(f, "IO error: {:?}", e),
            WavFatalError::ReachedMaxSize => write!(f, "Reached maximum WAV file size of 4GB"),
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

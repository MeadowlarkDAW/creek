use std::io;

/// An error while opening an audio file.
#[derive(Debug)]
pub enum OpenError {
    /// IO error
    Io(io::Error),
    /// Format error
    Format(symphonia::core::errors::Error),
    /// No default track was found in the file.
    NoDefaultTrack,
    /// Failed to find the number of frames in the file.
    NoNumFrames,
    /// Failed to find the number of channels in the file
    NoNumChannels,
}

impl std::error::Error for OpenError {}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Io(e) => write!(f, "IO error: {:?}", e),
            OpenError::Format(e) => write!(f, "Format error: {:?}", e),
            OpenError::NoDefaultTrack => write!(f, "No default track found for codec"),
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

use std::error::Error;

/// A fatal error occurred and the stream cannot continue.
#[derive(Debug)]
pub enum FatalReadError<FatalDecoderError: Error> {
    /// The stream is closed and thus cannot continue.
    StreamClosed,
    /// A fatal decoder error occurred. The stream cannot continue.
    DecoderError(FatalDecoderError),
}

/// An error reading the file.
#[derive(Debug)]
pub enum ReadError<FatalDecoderError: Error> {
    /// A fatal error occurred. The stream cannot continue.
    FatalError(FatalReadError<FatalDecoderError>),
    /// The end of the file was reached. The stream must be seeked to
    /// an earlier position to continue reading data. Until then, output
    /// silence.
    ///
    /// If this is returned, then the playhead of the stream did not
    /// advance.
    EndOfFile,
    /// The given cache with index `index` is out of range of the number
    /// of caches assigned to this stream. Please try a
    /// different index.
    ///
    /// If this is returned, then the playhead of the stream did not
    /// advance.
    CacheIndexOutOfRange { index: usize, num_caches: usize },
    /// The message channel to the IO server was full.
    ///
    /// In theory this should not happen, but if it does, then output silence
    /// until the channel has more slots open later.
    ///
    /// If this is returned, then the playhead of the stream did not
    /// advance.
    IOServerChannelFull,
    /// The given buffer does not match the internal layout of the stream. Check
    /// that the number of channels in both are the same.
    ///
    /// If this is returned, then the playhead of the stream did not
    /// advance.
    InvalidBuffer,
}

impl<FatalDecoderError: Error> std::error::Error for ReadError<FatalDecoderError> {}

impl<FatalDecoderError: Error> std::fmt::Display for ReadError<FatalDecoderError> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::FatalError(e) => match e {
                FatalReadError::StreamClosed => {
                    write!(f, "Fatal error: stream is closed")
                }
                FatalReadError::DecoderError(de) => {
                    write!(f, "Fatal decoder error: {:?}", de)
                }
            },
            ReadError::EndOfFile => write!(f, "End of file"),
            ReadError::CacheIndexOutOfRange { index, num_caches } => {
                write!(
                    f,
                    "Cache index {} is out of range of the number of allocated caches {}",
                    index, num_caches
                )
            }
            ReadError::IOServerChannelFull => {
                write!(f, "The message channel to the IO server is full.")
            }
            ReadError::InvalidBuffer => {
                write!(f, "Fill buffer does not match internal buffer layout")
            }
        }
    }
}

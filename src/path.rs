use std::mem::MaybeUninit;
use std::ffi::{CString, NulError};

#[derive(Debug)]
pub enum PathError<'a> {
    PathTooLong(&'a str),
}

impl<'a> std::error::Error for PathError<'a> {}

impl<'a> std::fmt::Display for PathError<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathError::PathTooLong(path) => {
                write!(f, "Path {}, is longer than {} bytes", path, PathBuffer::MAX_LEN)
            }
        }
    }
}

// We don't want to allocate memory on the client side, so use a null-terminated buffer instead.
pub struct PathBuffer {
    buffer: [u8; PathBuffer::MAX_LEN + 1],
}

impl PathBuffer {
    pub const MAX_LEN: usize = 511;

    pub fn from(path: &str) -> Result<Self, PathError> {
        if path.as_bytes().len() > PathBuffer::MAX_LEN {
            return Err(PathError::PathTooLong(path));
        }

        // Safe because string will be null-terminated.
        let mut buffer: [u8; PathBuffer::MAX_LEN + 1] = unsafe {
            MaybeUninit::uninit().assume_init()
        };

        {
            let buffer_view = &mut buffer[0..path.as_bytes().len()];

            buffer_view.copy_from_slice(path.as_bytes());
        }

        // Null-terminate string
        buffer[path.as_bytes().len()] = 0;

        Ok(PathBuffer { buffer })
    }

    pub fn as_string(&self) -> Result<CString, NulError> {
        CString::new(self.buffer)
    }
}
#![warn(rust_2018_idioms)]
#![warn(rust_2021_compatibility)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::clone_on_ref_ptr)]
#![deny(trivial_numeric_casts)]
#![forbid(unsafe_code)]

pub use creek_core::*;

#[cfg(feature = "decode")]
pub use creek_decode_symphonia::*;

#[cfg(feature = "encode-wav")]
pub use creek_encode_wav::*;

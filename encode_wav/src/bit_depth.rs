// Code modified from https://github.com/Fluhzar/WAV/blob/master,
// which is licensed under
//
// GNU Lesser General Public License v3.0
//
// https://github.com/Fluhzar/WAV/blob/master/LICENSE

use std::convert::TryFrom;

/// Enum listing the supported bit-depths and containers for the samples at each depth.
#[derive(Debug, PartialEq, Clone)]
pub enum BitDepth {
    Uint8(Vec<u8>),
    Int16(Vec<i16>),
    Int24(Vec<i32>),
    Empty,
}

impl Default for BitDepth {
    fn default() -> Self {
        BitDepth::Empty
    }
}

impl From<Vec<u8>> for BitDepth {
    fn from(v: Vec<u8>) -> Self {
        BitDepth::Uint8(v)
    }
}

impl From<Vec<i16>> for BitDepth {
    fn from(v: Vec<i16>) -> Self {
        BitDepth::Int16(v)
    }
}

impl From<Vec<i32>> for BitDepth {
    fn from(v: Vec<i32>) -> Self {
        BitDepth::Int24(v)
    }
}

impl TryFrom<BitDepth> for Vec<u8> {
    type Error = &'static str;

    /// ## Errors
    ///
    /// This function fails if `value` is not `BitDepth::Uint8`.
    fn try_from(value: BitDepth) -> Result<Self, Self::Error> {
        if let BitDepth::Uint8(v) = value {
            Ok(v)
        } else {
            Err("Bit-depth is not 8")
        }
    }
}

impl TryFrom<BitDepth> for Vec<i16> {
    type Error = &'static str;

    /// ## Errors
    ///
    /// This function fails if `value` is not `BitDepth::Int16`.
    fn try_from(value: BitDepth) -> Result<Self, Self::Error> {
        if let BitDepth::Int16(v) = value {
            Ok(v)
        } else {
            Err("Bit-depth is not 16")
        }
    }
}

impl TryFrom<BitDepth> for Vec<i32> {
    type Error = &'static str;

    /// ## Errors
    ///
    /// This function fails if `value` is not `BitDepth::Int24`.
    fn try_from(value: BitDepth) -> Result<Self, Self::Error> {
        if let BitDepth::Int24(v) = value {
            Ok(v)
        } else {
            Err("Bit-depth is not 24")
        }
    }
}

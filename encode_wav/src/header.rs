// Code modified from https://github.com/Fluhzar/WAV/blob/master,
// which is licensed under the
//
// GNU Lesser General Public License v3.0.
//
// https://github.com/Fluhzar/WAV/blob/master/LICENSE

use std::convert::TryFrom;

/// Structure for the `"fmt "` chunk of wave files, specifying key information
/// about the enclosed data. This struct supports only PCM data, which is to
/// say there is no extra members for compressed format data.
#[derive(Debug, Default, Copy, Clone, Hash, PartialEq, Eq)]
pub struct Header {
    pub audio_format: u16,
    pub channel_count: u16,
    pub sampling_rate: u32,
    pub bytes_per_second: u32,
    pub bytes_per_sample: u16,
    pub bits_per_sample: u16,
}

impl Header {
    /// Creates a new Header object.
    ///
    /// ## Note
    ///
    /// While the [`read`][0] and [`write`][1] functions only support
    /// uncompressed PCM for the audio format, the option is given here to
    /// select any audio format for custom implementations of wave features.
    ///
    /// ## Parameters
    ///
    /// * `af` - Audio format. 1 for uncompressed PCM data.
    /// * `cc` - Channel count, the number of channels each sample has. Generally 1 (mono) or 2 (stereo).
    /// * `r` - Sampling rate (e.g. 44.1kHz, 48kHz, 96kHz, etc.).
    /// * `bps` - Number of bits in each (sub-channel) sample. Generally 8, 16, or 24.
    ///
    /// ## Example
    ///
    /// ```
    /// let h = wav::Header::new(1, 2, 48_000, 16);
    /// ```
    ///
    /// [0]: crate::read
    /// [1]: crate::write
    pub fn new(af: u16, cc: u16, r: u32, bps: u16) -> Header {
        Header {
            audio_format: af,
            channel_count: cc,
            sampling_rate: r,
            bytes_per_second: (((bps >> 3) * cc) as u32) * r,
            bytes_per_sample: ((bps >> 3) * cc) as u16,
            bits_per_sample: bps,
        }
    }
}

impl From<Header> for [u8; 16] {
    fn from(h: Header) -> Self {
        let mut v: [u8; 16] = [0; 16];

        let b = h.audio_format.to_le_bytes();
        v[0] = b[0];
        v[1] = b[1];
        let b = h.channel_count.to_le_bytes();
        v[2] = b[0];
        v[3] = b[1];
        let b = h.sampling_rate.to_le_bytes();
        v[4] = b[0];
        v[5] = b[1];
        v[6] = b[2];
        v[7] = b[3];
        let b = h.bytes_per_second.to_le_bytes();
        v[8] = b[0];
        v[9] = b[1];
        v[10] = b[2];
        v[11] = b[3];
        let b = h.bytes_per_sample.to_le_bytes();
        v[12] = b[0];
        v[13] = b[1];
        let b = h.bits_per_sample.to_le_bytes();
        v[14] = b[0];
        v[15] = b[1];

        v
    }
}

impl From<[u8; 16]> for Header {
    fn from(v: [u8; 16]) -> Self {
        let audio_format = u16::from_le_bytes([v[0], v[1]]);
        let channel_count = u16::from_le_bytes([v[2], v[3]]);
        let sampling_rate = u32::from_le_bytes([v[4], v[5], v[6], v[7]]);
        let bytes_per_second = u32::from_le_bytes([v[8], v[9], v[10], v[11]]);
        let bytes_per_sample = u16::from_le_bytes([v[12], v[13]]);
        let bits_per_sample = u16::from_le_bytes([v[14], v[15]]);

        Header {
            audio_format,
            channel_count,
            sampling_rate,
            bytes_per_second,
            bytes_per_sample,
            bits_per_sample,
        }
    }
}

impl TryFrom<&[u8]> for Header {
    type Error = &'static str;

    /// ## Errors
    ///
    /// This function will return an error if the given slice is smaller than 16 bytes.
    fn try_from(v: &[u8]) -> Result<Self, Self::Error> {
        if v.len() < 16 {
            Err("Slice is smaller than the minimum-required 16 bytes")
        } else {
            let mut a: [u8; 16] = [0; 16];
            a.copy_from_slice(&v[0..16]);
            Ok(Header::from(a))
        }
    }
}

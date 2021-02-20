static RIFF_DESC: [u8; 4] = 0x52494646u32.to_be_bytes(); // The letters "RIFF" in ASCII.
static WAVE_DESC: [u8; 4] = 0x57415645u32.to_be_bytes(); // The letters "WAVE" in ASCII.
static FMT_DESC: [u8; 4] = 0x666d7420u32.to_be_bytes(); // The letters "fmt " in ASCII.
static DATA_DESC: [u8; 4] = 0x64617461u32.to_be_bytes(); // The letters "data" in ASCII.

use crate::{Format, FormatType};

pub struct Header {
    buffer: Vec<u8>,
    num_channels: u16,
    format: Format,
}

impl Header {
    pub fn new(num_channels: u16, sample_rate: u32, format: Format) -> Self {
        let bits_per_sample = format.bits_per_sample();
        let bytes_per_sample = format.bytes_per_sample();

        let nc = num_channels.to_le_bytes();
        let sr = sample_rate.to_le_bytes();
        let bps = bits_per_sample.to_le_bytes();
        let br =
            (sample_rate * u32::from(num_channels) * u32::from(bytes_per_sample)).to_le_bytes();
        let ba = (num_channels * bytes_per_sample).to_le_bytes();

        let (subchunk1_size, audio_format): (u32, u16) = if format.format_type() == FormatType::Pcm
        {
            (16, 1)
        } else {
            (16, 1) // TODO
        };
        let chunk_size = 4 + (8 + subchunk1_size);

        let sc1_size = subchunk1_size.to_le_bytes();
        let ch_size = chunk_size.to_le_bytes();
        let af = audio_format.to_le_bytes();

        #[rustfmt::skip]
        let buffer = vec![
            // RIFF chunk

            RIFF_DESC[0], RIFF_DESC[1], RIFF_DESC[2], RIFF_DESC[3],  // ChunkID
            ch_size[0],   ch_size[1],   ch_size[2],   ch_size[3],    // ChunkSize
            WAVE_DESC[0], WAVE_DESC[1], WAVE_DESC[2], WAVE_DESC[3],  // Format

            // Format subchunk

            FMT_DESC[0],  FMT_DESC[1],  FMT_DESC[2],  FMT_DESC[3],   // Subchunk1ID
            sc1_size[0],  sc1_size[1],  sc1_size[2],  sc1_size[3],   // Subchunk1Size
            af[0],        af[1],                                     // AudioFormat
            nc[0],        nc[1],                                     // NumChannels
            sr[0],        sr[1],        sr[2],        sr[3],         // SampleRate
            br[0],        br[1],        br[2],        br[3],         // ByteRate
            ba[0],        ba[1],                                     // BlockAlign
            bps[0],       bps[1],                                    // BitsPerSample

            // Data subchunk

            DATA_DESC[0], DATA_DESC[1], DATA_DESC[2], DATA_DESC[3],  // Subchunk2ID
            0,            0,            0,            0,             // Subchunk2Size
        ];

        Self {
            buffer,
            num_channels,
            format,
        }
    }

    pub fn set_num_frames(&mut self, num_frames: u32) {
        let num_bytes =
            num_frames * u32::from(self.num_channels) * u32::from(self.format.bytes_per_sample());

        if self.format.format_type() == FormatType::Pcm {
            let chunk_size = 36 + num_bytes;

            let ch_size = chunk_size.to_le_bytes();
            let da_size = num_bytes.to_le_bytes();

            self.buffer[4] = ch_size[0];
            self.buffer[5] = ch_size[1];
            self.buffer[6] = ch_size[2];
            self.buffer[7] = ch_size[3];

            self.buffer[40] = da_size[0];
            self.buffer[41] = da_size[1];
            self.buffer[42] = da_size[2];
            self.buffer[43] = da_size[3];
        } else {
            // TODO
        }
    }

    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }

    pub fn max_data_bytes(&self) -> u32 {
        if self.format.format_type() == FormatType::Pcm {
            std::u32::MAX - 36
        } else {
            std::u32::MAX - 36 // TODO
        }
    }
}

static RIFF_DESC: [u8; 4] = 0x52494646u32.to_be_bytes(); // The letters "RIFF" in ASCII.
static WAVE_DESC: [u8; 4] = 0x57415645u32.to_be_bytes(); // The letters "WAVE" in ASCII.
static FMT_DESC: [u8; 4] = 0x666d7420u32.to_be_bytes(); // The letters "fmt " in ASCII.
static DATA_DESC: [u8; 4] = 0x64617461u32.to_be_bytes(); // The letters "data" in ASCII.
static FACT_DESC: [u8; 4] = 0x66616374u32.to_be_bytes(); // The letters "fact" in ASCII.

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

        let (chunk_size, subchunk1_size, audio_format): (u32, u32, u16) = match format.format_type()
        {
            FormatType::Pcm => (36, 16, 0x1),
            FormatType::Float => (50, 18, 0x3),
        };

        let sc1_size = subchunk1_size.to_le_bytes();
        let ch_size = chunk_size.to_le_bytes();
        let af = audio_format.to_le_bytes();

        #[rustfmt::skip]
        let buffer = match format.format_type() {
            FormatType::Pcm => {
                vec![
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
                    0x0,          0x0,          0x0,          0x0,           // Subchunk2Size
                ]
            }
            FormatType::Float => {
                let sc2_size = 0x4u32.to_le_bytes();

                vec![
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
                    0x0,          0x0,                                       // ExtensionSize

                    // Fact subchunk

                    FACT_DESC[0], FACT_DESC[1], FACT_DESC[2], FACT_DESC[3],  // Subchunk2ID
                    sc2_size[0],  sc2_size[1],  sc2_size[2],  sc2_size[3],   // Subchunk2Size
                    0x0,          0x0,          0x0,          0x0,           // SamplesPerChannel

                    // Data subchunk

                    DATA_DESC[0], DATA_DESC[1], DATA_DESC[2], DATA_DESC[3],  // Subchunk3ID
                    0x0,          0x0,          0x0,          0x0,           // Subchunk3Size
                ]
            }
        };

        Self {
            buffer,
            num_channels,
            format,
        }
    }

    pub fn set_num_frames(&mut self, num_frames: u32) {
        let mut num_bytes =
            num_frames * u32::from(self.num_channels) * u32::from(self.format.bytes_per_sample());

        // If num_bytes is odd, add a padding byte.
        if num_bytes & 0x1 == 0x1 {
            num_bytes += 1;
        }

        match self.format.format_type() {
            FormatType::Pcm => {
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
            }
            FormatType::Float => {
                let chunk_size = 50 + num_bytes;

                let ch_size = chunk_size.to_le_bytes();
                let da_size = num_bytes.to_le_bytes();
                let frames = num_frames.to_le_bytes();

                self.buffer[4] = ch_size[0];
                self.buffer[5] = ch_size[1];
                self.buffer[6] = ch_size[2];
                self.buffer[7] = ch_size[3];

                self.buffer[46] = frames[0];
                self.buffer[47] = frames[1];
                self.buffer[48] = frames[2];
                self.buffer[49] = frames[3];

                self.buffer[54] = da_size[0];
                self.buffer[55] = da_size[1];
                self.buffer[56] = da_size[2];
                self.buffer[57] = da_size[3];
            }
        }
    }

    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }

    pub fn max_data_bytes(&self) -> u32 {
        if self.format.format_type() == FormatType::Pcm {
            u32::MAX - 36
        } else {
            u32::MAX - 50
        }
    }
}

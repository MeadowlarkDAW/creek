use crate::Format;
use byte_slice_cast::AsByteSlice;
use std::fs::File;
use std::io::Write;

pub trait WavBitDepth {
    type T: Copy + Clone + Default + Send;

    fn new(max_block_frames: usize, num_channels: u16) -> Self;

    fn format() -> Format;

    fn write_to_disk(&mut self, data: &[Self::T], file: &mut File) -> Result<(), std::io::Error>;
}

pub struct Uint8 {}

impl WavBitDepth for Uint8 {
    type T = u8;

    fn new(_max_block_frames: usize, _num_channels: u16) -> Self {
        Self {}
    }

    fn format() -> Format {
        Format::Uint8
    }

    fn write_to_disk(&mut self, data: &[u8], file: &mut File) -> Result<(), std::io::Error> {
        file.write_all(data)
    }
}

pub struct Int16 {}

impl WavBitDepth for Int16 {
    type T = i16;

    fn new(_max_block_frames: usize, _num_channels: u16) -> Self {
        Self {}
    }

    fn format() -> Format {
        Format::Int16
    }

    fn write_to_disk(&mut self, data: &[i16], file: &mut File) -> Result<(), std::io::Error> {
        file.write_all(data.as_byte_slice())
    }
}

pub struct Int24 {
    cram_buffer: Vec<u8>,
}

impl WavBitDepth for Int24 {
    type T = i32;

    fn new(max_block_frames: usize, num_channels: u16) -> Self {
        let cram_buffer_size = max_block_frames * usize::from(num_channels) * 3;

        Self {
            cram_buffer: Vec::with_capacity(cram_buffer_size),
        }
    }

    fn format() -> Format {
        Format::Int24
    }

    fn write_to_disk(&mut self, data: &[i32], file: &mut File) -> Result<(), std::io::Error> {
        self.cram_buffer.clear();
        let num_frames = data.len();

        let data_u8 = data.as_byte_slice();

        // Hint to compiler to optimize loop.
        assert!(num_frames * 3 <= self.cram_buffer.capacity());
        assert!(num_frames * 4 == data_u8.len());

        for f in 0..num_frames {
            self.cram_buffer.push(data_u8[f * 4]);
            self.cram_buffer.push(data_u8[f * 4 + 1]);
            self.cram_buffer.push(data_u8[f * 4 + 2]);
        }

        file.write_all(&self.cram_buffer)
    }
}

pub struct Float32 {}

impl WavBitDepth for Float32 {
    type T = f32;

    fn new(_max_block_frames: usize, _num_channels: u16) -> Self {
        Self {}
    }

    fn format() -> Format {
        Format::Float32
    }

    fn write_to_disk(&mut self, data: &[f32], file: &mut File) -> Result<(), std::io::Error> {
        file.write_all(data.as_byte_slice())
    }
}

pub struct Float64 {}

impl WavBitDepth for Float64 {
    type T = f64;

    fn new(_max_block_frames: usize, _num_channels: u16) -> Self {
        Self {}
    }

    fn format() -> Format {
        Format::Float64
    }

    fn write_to_disk(&mut self, data: &[f64], file: &mut File) -> Result<(), std::io::Error> {
        file.write_all(data.as_byte_slice())
    }
}

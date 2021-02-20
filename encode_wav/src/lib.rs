use std::path::PathBuf;
use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
};

use rt_audio_disk_stream_core::{Encoder, FileInfo, WriteBlock};

mod error;
mod header;

pub mod bit_writer;

pub use bit_writer::*;
pub use error::{FatalError, OpenError};

use header::Header;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FormatType {
    Pcm,
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Format {
    Uint8,
    Int16,
    Int24,
    Float32,
    Float64,
}

impl Format {
    pub fn bits_per_sample(&self) -> u16 {
        match self {
            Format::Uint8 => 8,
            Format::Int16 => 16,
            Format::Int24 => 24,
            Format::Float32 => 32,
            Format::Float64 => 64,
        }
    }

    pub fn bytes_per_sample(&self) -> u16 {
        match self {
            Format::Uint8 => 1,
            Format::Int16 => 2,
            Format::Int24 => 3,
            Format::Float32 => 4,
            Format::Float64 => 8,
        }
    }

    pub fn format_type(&self) -> FormatType {
        match self {
            Format::Uint8 => FormatType::Pcm,
            Format::Int16 => FormatType::Pcm,
            Format::Int24 => FormatType::Pcm,
            Format::Float32 => FormatType::Float,
            Format::Float64 => FormatType::Float,
        }
    }
}

#[derive(Clone)]
pub struct Params {
    format: Format,
}

pub struct BWavFileEncoder<B: BitWriter + 'static> {
    interleave_buf: Vec<B::T>,
    file: Option<File>,
    header: Header,
    path: PathBuf,
    bytes_per_frame: u64,
    frames_written: u32,
    max_file_bytes: u64,
    max_block_bytes: u64,
    num_channels: usize,
    bit_writer: B,
}

impl<B: BitWriter + 'static> Encoder for BWavFileEncoder<B> {
    type T = B::T;
    type AdditionalOpts = ();
    type FileParams = Params;
    type OpenError = OpenError;
    type FatalError = FatalError;

    const DEFAULT_BLOCK_SIZE: usize = 32768;
    const DEFAULT_NUM_WRITE_BLOCKS: usize = 8;

    fn new(
        path: PathBuf,
        num_channels: u16,
        sample_rate: f64,
        block_size: usize,
        _num_write_blocks: usize,
        _additional_opts: Self::AdditionalOpts,
    ) -> Result<(Self, FileInfo<Self::FileParams>), Self::OpenError> {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path.clone())?;

        let format = B::format();
        let header = Header::new(num_channels, sample_rate.round() as u32, format);

        file.write_all(header.buffer())?;
        file.flush()?;

        let buf_len = usize::from(num_channels) * block_size;
        let mut interleave_buf: Vec<B::T> = Vec::with_capacity(buf_len);
        // Safe because data will always be written to before it is read.
        unsafe {
            interleave_buf.set_len(buf_len);
        }

        let max_file_bytes = u64::from(header.max_data_bytes());
        let bytes_per_frame = u64::from(num_channels) * u64::from(format.bytes_per_sample());

        Ok((
            Self {
                interleave_buf,
                file: Some(file),
                header,
                path,
                frames_written: 0,
                bytes_per_frame,
                max_file_bytes,
                max_block_bytes: block_size as u64 * bytes_per_frame,
                num_channels: usize::from(num_channels),
                bit_writer: B::new(block_size, num_channels),
            },
            FileInfo {
                num_frames: 0,
                num_channels,
                sample_rate: Some(sample_rate),
                params: Params { format },
            },
        ))
    }

    unsafe fn encode(&mut self, write_block: &WriteBlock<Self::T>) -> Result<(), Self::FatalError> {
        if let Some(mut file) = self.file.take() {
            let written_frames = write_block.written_frames();

            if self.num_channels == 1 {
                self.bit_writer
                    .write_to_disk(&write_block.block()[0][0..written_frames], &mut file)?;
            } else {
                if self.num_channels == 2 {
                    // Hint to compiler to optimize loop
                    assert!(written_frames <= write_block.block()[0].len());
                    assert!(written_frames <= write_block.block()[1].len());
                    assert!(written_frames * 2 <= self.interleave_buf.len());

                    for frame in 0..written_frames {
                        self.interleave_buf[(frame * 2)] = write_block.block()[0][frame];
                        self.interleave_buf[(frame * 2) + 1] = write_block.block()[1][frame];
                    }
                } else {
                    // Hint to compiler to optimize loop
                    assert!(written_frames * self.num_channels <= self.interleave_buf.len());
                    for block in write_block.block() {
                        assert!(written_frames <= block.len());
                    }

                    for frame in 0..written_frames {
                        for (ch, block) in write_block.block().iter().enumerate() {
                            self.interleave_buf[(frame * self.num_channels) + ch] = block[frame];
                        }
                    }
                }

                self.bit_writer.write_to_disk(
                    &self.interleave_buf[0..written_frames * self.num_channels],
                    &mut file,
                )?;
            }

            self.frames_written += written_frames as u32;
            let bytes_written = u64::from(self.frames_written) * self.bytes_per_frame;

            self.header.set_num_frames(self.frames_written);

            // Update the header in the file.
            file.seek(SeekFrom::Start(0))?;
            file.write_all(self.header.buffer())?;
            file.seek(SeekFrom::Start(bytes_written))?;
            file.flush()?;

            // Make sure the number of written bytes does not exceed 4GB.
            if bytes_written + self.max_block_bytes >= self.max_file_bytes {
                // Drop file here.
                let _ = file;

                return Err(FatalError::ReachedMaxSize);
            }

            self.file = Some(file);
        }

        Ok(())
    }

    fn finish_file(&mut self) -> Result<(), Self::FatalError> {
        if let Some(mut file) = self.file.take() {
            self.header.set_num_frames(self.frames_written);

            file.seek(SeekFrom::Start(0))?;
            file.write_all(self.header.buffer())?;
            file.flush()?;

            // Drop file here.
            let _ = file;
        }

        Ok(())
    }

    fn discard_file(&mut self) -> Result<(), Self::FatalError> {
        if let Some(file) = self.file.take() {
            // Drop file here.
            let _ = file;

            std::fs::remove_file(self.path.clone())?;
        }

        Ok(())
    }

    fn discard_and_restart(&mut self) -> Result<(), Self::FatalError> {
        if let Some(file) = &mut self.file {
            file.set_len(0)?;

            self.frames_written = 0;
            self.header.set_num_frames(0);

            file.seek(SeekFrom::Start(0))?;
            file.write_all(self.header.buffer())?;
            file.flush()?;
        }

        Ok(())
    }
}

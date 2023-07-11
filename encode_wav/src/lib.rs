#![warn(rust_2018_idioms)]
#![warn(rust_2021_compatibility)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::clone_on_ref_ptr)]
#![deny(trivial_numeric_casts)]

use std::path::PathBuf;
use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
};

use creek_core::{write, Encoder, FileInfo, WriteBlock, WriteStatus};

pub mod error;
mod header;

#[cfg(test)]
mod tests;

pub mod wav_bit_depth;

use error::{WavFatalError, WavOpenError};
use header::Header;
use wav_bit_depth::WavBitDepth;

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
    _format: Format,
}

pub struct WavEncoder<B: WavBitDepth + 'static> {
    interleave_buf: Vec<B::T>,
    file: Option<File>,
    header: Header,
    path: PathBuf,
    bytes_per_frame: u64,
    frames_written: u32,
    max_file_bytes: u64,
    max_block_bytes: u64,
    num_channels: usize,
    num_files: u32,
    bit_depth: B,
}

impl<B: WavBitDepth + 'static> Encoder for WavEncoder<B> {
    type T = B::T;
    type AdditionalOpts = ();
    type FileParams = Params;
    type OpenError = WavOpenError;
    type FatalError = WavFatalError;

    const DEFAULT_BLOCK_SIZE: usize = 32768;
    const DEFAULT_NUM_WRITE_BLOCKS: usize = 8;

    fn new(
        path: PathBuf,
        num_channels: u16,
        sample_rate: u32,
        block_size: usize,
        _num_write_blocks: usize,
        _additional_opts: Self::AdditionalOpts,
    ) -> Result<(Self, FileInfo<Self::FileParams>), Self::OpenError> {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(path.clone())?;

        let format = B::format();
        let header = Header::new(num_channels, sample_rate, format);

        file.write_all(header.buffer())?;
        file.flush()?;

        let buf_len = usize::from(num_channels) * block_size;
        let mut interleave_buf: Vec<B::T> = Vec::with_capacity(buf_len);
        // Safe because data will always be written to before it is read.
        #[allow(clippy::uninit_vec)] // TODO
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
                num_files: 1,
                bit_depth: B::new(block_size, num_channels),
            },
            FileInfo {
                num_frames: 0,
                num_channels,
                sample_rate: Some(sample_rate),
                params: Params { _format: format },
            },
        ))
    }

    unsafe fn encode(
        &mut self,
        write_block: &WriteBlock<Self::T>,
    ) -> Result<WriteStatus, Self::FatalError> {
        let mut status = WriteStatus::Ok;

        if let Some(mut file) = self.file.take() {
            let written_frames = write_block.written_frames();

            if self.num_channels == 1 {
                self.bit_depth
                    .write_to_disk(&write_block.block()[0][0..written_frames], &mut file)?;
            } else {
                if self.num_channels == 2 {
                    // Hint to compiler to optimize loop
                    assert!(written_frames <= write_block.block()[0].len());
                    assert!(written_frames <= write_block.block()[1].len());
                    assert!(written_frames * 2 <= self.interleave_buf.len());

                    for frame in 0..written_frames {
                        self.interleave_buf[frame * 2] = write_block.block()[0][frame];
                        self.interleave_buf[frame * 2 + 1] = write_block.block()[1][frame];
                    }
                } else {
                    // Hint to compiler to optimize loop
                    assert!(written_frames * self.num_channels <= self.interleave_buf.len());
                    for block in write_block.block() {
                        assert!(written_frames <= block.len());
                    }

                    for frame in 0..written_frames {
                        for (ch, block) in write_block.block().iter().enumerate() {
                            self.interleave_buf[frame * self.num_channels + ch] = block[frame];
                        }
                    }
                }

                self.bit_depth.write_to_disk(
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
            file.seek(SeekFrom::Current(bytes_written as i64))?;
            file.flush()?;

            // Make sure the number of written bytes does not exceed 4GB.
            if bytes_written + self.max_block_bytes >= self.max_file_bytes {
                // When it does, create a new file to hold more data.

                // Drop current file here.
                let _ = file;

                self.num_files += 1;

                let mut file_name = self
                    .path
                    .file_name()
                    .ok_or_else(|| WavFatalError::CouldNotGetFileName)?
                    .to_os_string();
                file_name.push(write::num_files_to_file_name_extension(self.num_files));
                let mut new_file_path = self.path.clone();
                new_file_path.set_file_name(file_name);

                // Create new file.
                let mut file = OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .create(true)
                    .open(new_file_path)?;

                self.frames_written = 0;
                self.header.set_num_frames(0);

                file.seek(SeekFrom::Start(0))?;
                file.write_all(self.header.buffer())?;
                file.flush()?;

                status = WriteStatus::ReachedMaxSize {
                    num_files: self.num_files,
                };

                self.file = Some(file);
            } else {
                self.file = Some(file);
            }
        }

        Ok(status)
    }

    fn finish_file(&mut self) -> Result<(), Self::FatalError> {
        if let Some(mut file) = self.file.take() {
            self.header.set_num_frames(self.frames_written);

            file.seek(SeekFrom::Start(0))?;
            file.write_all(self.header.buffer())?;
            file.flush()?;

            // Drop file here.
            let _ = file;

            self.num_files = 0;
        }

        Ok(())
    }

    fn discard_file(&mut self) -> Result<(), Self::FatalError> {
        if let Some(file) = self.file.take() {
            // Drop file here.
            let _ = file;

            std::fs::remove_file(self.path.clone())?;

            // Delete any previously created files.
            if self.num_files > 1 {
                for i in 2..(self.num_files + 1) {
                    let mut file_name = self
                        .path
                        .file_name()
                        .ok_or_else(|| WavFatalError::CouldNotGetFileName)?
                        .to_os_string();
                    file_name.push(write::num_files_to_file_name_extension(i));
                    let mut new_file_path = self.path.clone();
                    new_file_path.set_file_name(file_name);

                    std::fs::remove_file(new_file_path)?;
                }
            }

            self.num_files = 0;
        }

        Ok(())
    }

    fn discard_and_restart(&mut self) -> Result<(), Self::FatalError> {
        if let Some(mut file) = self.file.take() {
            self.frames_written = 0;
            self.header.set_num_frames(0);

            if self.num_files > 1 {
                // Drop the old file here.
                let _ = file;

                // Delete any previously created files.
                for i in 2..(self.num_files + 1) {
                    let mut file_name = self
                        .path
                        .file_name()
                        .ok_or_else(|| WavFatalError::CouldNotGetFileName)?
                        .to_os_string();
                    file_name.push(write::num_files_to_file_name_extension(i));
                    let mut new_file_path = self.path.clone();
                    new_file_path.set_file_name(file_name);

                    std::fs::remove_file(new_file_path)?;
                }

                // Re-create the original file and start over.
                let mut file = OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .create(true)
                    .open(self.path.clone())?;
                file.seek(SeekFrom::Start(0))?;
                file.write_all(self.header.buffer())?;
                file.flush()?;

                self.file = Some(file);
                self.num_files = 1;
            } else {
                file.set_len(0)?;

                file.seek(SeekFrom::Start(0))?;
                file.write_all(self.header.buffer())?;
                file.flush()?;

                self.file = Some(file);
            }
        }

        Ok(())
    }
}

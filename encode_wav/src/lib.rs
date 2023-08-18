#![warn(rust_2018_idioms)]
#![warn(rust_2021_compatibility)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::clone_on_ref_ptr)]
#![deny(trivial_numeric_casts)]

use std::path::PathBuf;
use std::time::Duration;
use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
};

use crate::write::{Encoder, WriteStatus};
use creek_core::{write, AudioBlock, FileInfo};

pub mod error;
mod header;
pub mod wav_bit_depth;

#[cfg(test)]
mod tests;

use error::{WavFatalError, WavOpenError};
use header::Header;
use wav_bit_depth::WavBitDepth;

/// The type of samples used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatType {
    /// Integer sample type
    Pcm,
    /// Floating-point sample type
    Float,
}

/// The sample format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Unsigned 8-bit integer
    Uint8,
    /// Signed 16-bit integer
    Int16,
    /// Signed 24-bit integer
    Int24,
    /// 32-bit floating point
    Float32,
    /// 64-bit floating point
    Float64,
}

impl Format {
    /// The number of bits per sample.
    pub fn bits_per_sample(&self) -> u16 {
        match self {
            Format::Uint8 => 8,
            Format::Int16 => 16,
            Format::Int24 => 24,
            Format::Float32 => 32,
            Format::Float64 => 64,
        }
    }

    /// The number of bytes per sample.
    pub fn bytes_per_sample(&self) -> u16 {
        match self {
            Format::Uint8 => 1,
            Format::Int16 => 2,
            Format::Int24 => 3,
            Format::Float32 => 4,
            Format::Float64 => 8,
        }
    }

    /// The type of samples.
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

/// Further information about a WAV file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// The sample format.
    pub format: Format,
}

/// An encoder for creek that writes to a WAV file.
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

    const DEFAULT_BLOCK_FRAMES: usize = 32768;
    const DEFAULT_NUM_WRITE_BLOCKS: usize = 8;
    const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(1);

    fn new(
        path: PathBuf,
        num_channels: u16,
        sample_rate: u32,
        block_frames: usize,
        _num_blocks: usize,
        _poll_interval: Duration,
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

        let buf_len = usize::from(num_channels) * block_frames;
        let interleave_buf: Vec<B::T> = vec![Default::default(); buf_len];

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
                max_block_bytes: block_frames as u64 * bytes_per_frame,
                num_channels: usize::from(num_channels),
                num_files: 1,
                bit_depth: B::new(block_frames, num_channels),
            },
            FileInfo {
                num_frames: 0,
                num_channels,
                sample_rate: Some(sample_rate),
                params: Params { format },
            },
        ))
    }

    fn encode(&mut self, block: &AudioBlock<Self::T>) -> Result<WriteStatus, Self::FatalError> {
        let mut status = WriteStatus::Ok;

        if let Some(mut file) = self.file.take() {
            let frames_written = block.frames_written();

            if self.num_channels == 1 {
                self.bit_depth
                    .write_to_disk(&block.channels[0][0..frames_written], &mut file)?;
            } else {
                if self.num_channels == 2 {
                    // Provide efficient stereo interleaving.
                    let ch1 = &block.channels[0][0..frames_written];
                    let ch2 = &block.channels[1][0..frames_written];
                    let interleave_buf_part = &mut self.interleave_buf[0..frames_written * 2];

                    for (i, frame) in interleave_buf_part.chunks_exact_mut(2).enumerate() {
                        frame[0] = ch1[i];
                        frame[1] = ch2[i];
                    }
                } else {
                    let interleave_buf_part =
                        &mut self.interleave_buf[0..frames_written * self.num_channels];

                    for (ch_i, ch) in block.channels.iter().enumerate() {
                        let ch_slice = &ch[0..frames_written];

                        for (dst, src) in interleave_buf_part[ch_i..]
                            .iter_mut()
                            .step_by(self.num_channels)
                            .zip(ch_slice)
                        {
                            *dst = *src;
                        }
                    }
                }

                self.bit_depth.write_to_disk(
                    &self.interleave_buf[0..frames_written * self.num_channels],
                    &mut file,
                )?;
            }

            self.frames_written += frames_written as u32;
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

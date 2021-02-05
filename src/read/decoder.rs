use core::num;
use std::path::PathBuf;
use std::{fs::File, ptr::null_mut};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CodecParameters, Decoder as SymphDecoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Duration;

use super::{
    error::{self, OpenError, ReadError},
    DataBlock,
};
use crate::BLOCK_SIZE;

#[derive(Clone)]
pub struct FileInfo {
    pub params: CodecParameters,

    pub num_frames: usize,
    pub num_channels: usize,
    pub sample_rate: Option<u32>,
}

pub struct Decoder {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn SymphDecoder>,

    smp_buf: SampleBuffer<f32>,
    curr_smp_buf_i: usize,

    num_frames: usize,
    num_channels: usize,
    sample_rate: Option<u32>,

    decoder_opts: DecoderOptions,

    current_frame: usize,
    reset_smp_buffer: bool,
}

impl Decoder {
    pub fn new(
        file: PathBuf,
        mut start_frame: usize,
        verify: bool,
    ) -> Result<(Self, FileInfo), OpenError> {
        // Create a hint to help the format registry guess what format reader is appropriate.
        let mut hint = Hint::new();

        // Provide the file extension as a hint.
        if let Some(extension) = file.extension() {
            if let Some(extension_str) = extension.to_str() {
                hint.with_extension(extension_str);
            }
        }

        let source = Box::new(File::open(file)?);

        // Create the media source stream using the boxed media source from above.
        let mss = MediaSourceStream::new(source);

        // Use the default options for metadata and format readers.
        let format_opts: FormatOptions = Default::default();
        let metadata_opts: MetadataOptions = Default::default();

        let probed =
            symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

        let mut reader = probed.format;

        let decoder_opts = DecoderOptions {
            verify,
            ..Default::default()
        };

        let params = {
            // Get the default stream.
            let stream = reader
                .default_stream()
                .ok_or_else(|| OpenError::NoDefaultStream)?;

            stream.codec_params.clone()
        };

        let num_frames = params.n_frames.ok_or_else(|| OpenError::NoNumFrames)? as usize;
        let num_channels = (params.channels.ok_or_else(|| OpenError::NoNumChannels)?).count();
        let sample_rate = params.sample_rate;

        // Seek the reader to the requested position.
        if start_frame != 0 {
            start_frame = wrap_frame(start_frame, num_frames);

            let seconds = start_frame as f64 / f64::from(sample_rate.unwrap_or(44100));

            reader.seek(SeekTo::Time {
                time: seconds.into(),
            })?;
        }

        // Create a decoder for the stream.
        let mut decoder = symphonia::default::get_codecs().make(&params, &decoder_opts)?;

        // Decode the first packet to get the signal specification.
        let smp_buf = loop {
            match decoder.decode(&reader.next_packet()?) {
                Ok(decoded) => {
                    // Get the buffer spec.
                    let spec = *decoded.spec();

                    // Get the buffer capacity.
                    let capacity = Duration::from(decoded.capacity() as u64);

                    let mut smp_buf = SampleBuffer::<f32>::new(capacity, spec);

                    smp_buf.copy_interleaved_ref(decoded);

                    break smp_buf;
                }
                Err(Error::DecodeError(e)) => {
                    // Decode errors are not fatal. Send a warning and try to decode the next packet.

                    // TODO: print warning.

                    continue;
                }
                Err(e) => {
                    // Errors other than decode errors are fatal.
                    return Err(e.into());
                }
            }
        };

        let file_info = FileInfo {
            params,
            num_frames,
            num_channels,
            sample_rate,
        };

        Ok((
            Self {
                reader,
                decoder,

                smp_buf,
                curr_smp_buf_i: 0,

                num_frames,
                num_channels,
                sample_rate,

                decoder_opts,

                current_frame: start_frame,
                reset_smp_buffer: false,
            },
            file_info,
        ))
    }

    pub fn seek_to(&mut self, frame: usize) -> Result<(), ReadError> {
        self.current_frame = wrap_frame(frame, self.num_frames);

        let seconds = self.current_frame as f64 / f64::from(self.sample_rate.unwrap_or(44100));

        self.reader.seek(SeekTo::Time {
            time: seconds.into(),
        })?;

        self.reset_smp_buffer = true;

        /*
        self.decoder.close();
        self.decoder = symphonia::default::get_codecs()
            .make(self.decoder.codec_params(), &self.decoder_opts)?;
            */

        Ok(())
    }

    pub fn decode_into(&mut self, data_block: &mut DataBlock) -> Result<(), ReadError> {
        let mut reached_end_of_file = false;

        let mut block_start = 0;
        while block_start < BLOCK_SIZE {
            let num_frames_to_cpy = if self.reset_smp_buffer {
                // Get new data first.
                self.reset_smp_buffer = false;
                0
            } else if self.smp_buf.len() < self.num_channels {
                // Get new data first.
                0
            } else {
                // Find the maximum amount of frames that can be copied.
                (BLOCK_SIZE - block_start)
                    .min((self.smp_buf.len() - self.curr_smp_buf_i) / self.num_channels)
            };

            if num_frames_to_cpy != 0 {
                if self.num_channels == 1 {
                    // Mono, no need to deinterleave.
                    &mut data_block.block[0][block_start..block_start + num_frames_to_cpy]
                        .copy_from_slice(
                            &self.smp_buf.samples()
                                [self.curr_smp_buf_i..self.curr_smp_buf_i + num_frames_to_cpy],
                        );
                } else if self.num_channels == 2 {
                    // Provide efficient stereo deinterleaving.

                    let smp_buf = &self.smp_buf.samples()
                        [self.curr_smp_buf_i..self.curr_smp_buf_i + (num_frames_to_cpy * 2)];

                    let (block1, block2) = data_block.block.split_at_mut(1);
                    let block1 = &mut block1[0][block_start..block_start + num_frames_to_cpy];
                    let block2 = &mut block2[0][block_start..block_start + num_frames_to_cpy];

                    for i in 0..num_frames_to_cpy {
                        block1[i] = smp_buf[i * 2];
                        block2[i] = smp_buf[(i * 2) + 1];
                    }
                } else {
                    let smp_buf = &self.smp_buf.samples()[self.curr_smp_buf_i
                        ..self.curr_smp_buf_i + (num_frames_to_cpy * self.num_channels)];

                    for i in 0..num_frames_to_cpy {
                        for (ch, block) in data_block.block.iter_mut().enumerate() {
                            block[block_start + i] = smp_buf[(i * self.num_channels) + ch];
                        }
                    }
                }

                block_start += num_frames_to_cpy;

                self.curr_smp_buf_i += num_frames_to_cpy * self.num_channels;
                if self.curr_smp_buf_i >= self.smp_buf.len() {
                    self.reset_smp_buffer = true;
                }
            } else {
                // Decode more packets.

                loop {
                    match self.reader.next_packet() {
                        Ok(packet) => {
                            match self.decoder.decode(&packet) {
                                Ok(decoded) => {
                                    self.smp_buf.copy_interleaved_ref(decoded);
                                    self.curr_smp_buf_i = 0;
                                    break;
                                }
                                Err(Error::DecodeError(e)) => {
                                    // Decode errors are not fatal. Print a message and try to decode the next packet as
                                    // usual.

                                    // TODO: print warning.

                                    continue;
                                }
                                Err(e) => {
                                    // Errors other than decode errors are fatal.
                                    return Err(e.into());
                                }
                            }
                        }
                        Err(e) => {
                            if let Error::IoError(io_error) = &e {
                                if io_error.kind() == std::io::ErrorKind::UnexpectedEof {
                                    // End of file, stop decoding.
                                    reached_end_of_file = true;
                                    block_start = BLOCK_SIZE;
                                    break;
                                } else {
                                    return Err(e.into());
                                }
                            } else {
                                return Err(e.into());
                            }
                        }
                    }
                }
            }
        }

        data_block.start_frame = self.current_frame;

        if reached_end_of_file {
            self.current_frame = self.num_frames;
        } else {
            self.current_frame = wrap_frame(self.current_frame + BLOCK_SIZE, self.num_frames);
        }

        Ok(())
    }

    pub fn current_frame(&self) -> usize {
        self.current_frame
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.decoder.close();
    }
}

fn wrap_frame(mut frame: usize, len: usize) -> usize {
    while frame >= len {
        frame -= len;
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_new() {
        let files = vec![
            //  file | num_channels | num_frames | sample_rate
            ("./test_files/wav_u8_mono.wav", 1, 1323000, Some(44100)),
            ("./test_files/wav_i16_mono.wav", 1, 1323000, Some(44100)),
            ("./test_files/wav_i24_mono.wav", 1, 1323000, Some(44100)),
            ("./test_files/wav_i32_mono.wav", 1, 1323000, Some(44100)),
            ("./test_files/wav_f32_mono.wav", 1, 1323000, Some(44100)),
            ("./test_files/wav_i24_stereo.wav", 2, 1323000, Some(44100)),
            //"./test_files/ogg_mono.ogg",
            //"./test_files/ogg_stereo.ogg",
            //"./test_files/mp3_constant_mono.mp3",
            //"./test_files/mp3_constant_stereo.mp3",
            //"./test_files/mp3_variable_mono.mp3",
            //"./test_files/mp3_variable_stereo.mp3",
        ];

        for file in files {
            dbg!(file.0);
            let decoder = Decoder::new(file.0.into(), 0, false);
            match decoder {
                Ok((_, file_info)) => {
                    assert_eq!(file_info.num_channels, file.1);
                    assert_eq!(file_info.num_frames, file.2);
                    assert_eq!(file_info.sample_rate, file.3);
                }
                Err(e) => {
                    panic!(e)
                }
            }
        }
    }
}

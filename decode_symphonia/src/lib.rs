#![warn(rust_2018_idioms)]
#![warn(rust_2021_compatibility)]
#![warn(clippy::missing_panics_doc)]
#![warn(clippy::clone_on_ref_ptr)]
#![deny(trivial_numeric_casts)]
#![forbid(unsafe_code)]

use std::fs::File;
use std::path::PathBuf;

use symphonia::core::audio::AudioBuffer;
use symphonia::core::codecs::{CodecParameters, Decoder as SymphDecoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{Metadata, MetadataOptions, MetadataRevision};
use symphonia::core::probe::Hint;

use creek_core::{DataBlock, Decoder, FileInfo};

mod error;
pub use error::OpenError;

pub struct SymphoniaDecoder {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn SymphDecoder>,

    decode_buffer: AudioBuffer<f32>,
    decode_buffer_len: usize,
    curr_decode_buffer_frame: usize,

    num_frames: usize,
    sample_rate: Option<u32>,
    block_size: usize,

    playhead_frame: usize,
    reset_decode_buffer: bool,
    seek_diff: usize,
}

impl Decoder for SymphoniaDecoder {
    type T = f32;
    type FileParams = SymphoniaDecoderInfo;
    type OpenError = OpenError;
    type FatalError = Error;
    type AdditionalOpts = ();

    const DEFAULT_BLOCK_SIZE: usize = 16384;
    const DEFAULT_NUM_CACHE_BLOCKS: usize = 0;
    const DEFAULT_NUM_LOOK_AHEAD_BLOCKS: usize = 8;

    fn new(
        file: PathBuf,
        start_frame: usize,
        block_size: usize,
        _additional_opts: Self::AdditionalOpts,
    ) -> Result<(Self, FileInfo<Self::FileParams>), Self::OpenError> {
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
        let mss = MediaSourceStream::new(source, Default::default());

        // Use the default options for metadata and format readers.
        let format_opts: FormatOptions = Default::default();
        let metadata_opts: MetadataOptions = Default::default();

        let probed =
            symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

        let mut reader = probed.format;

        let decoder_opts = DecoderOptions {
            ..Default::default()
        };

        let params = {
            // Get the default stream.
            let stream = reader.default_track().ok_or(OpenError::NoDefaultTrack)?;

            stream.codec_params.clone()
        };
        let num_frames = params.n_frames.ok_or(OpenError::NoNumFrames)? as usize;
        let sample_rate = params.sample_rate;

        // Seek the reader to the requested position.
        if start_frame != 0 {
            reader.seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: frame_to_symphonia_time(start_frame as u64, sample_rate.unwrap_or(44100)),
                    track_id: None,
                },
            )?;
        }

        // Create a decoder for the stream.
        let mut decoder = symphonia::default::get_codecs().make(&params, &decoder_opts)?;
        debug_assert_eq!(params.n_frames, decoder.codec_params().n_frames);
        debug_assert_eq!(params.sample_rate, decoder.codec_params().sample_rate);
        debug_assert_eq!(params.channels, decoder.codec_params().channels);

        // The stream/decoder might not always provide the actual numbers
        // of channels (MP4/AAC/ALAC). In this case the number of channels
        // will be obtained from the signal spec of the first decoded packet.
        let mut channels = params.channels;

        // Decode the first packet to get the signal specification.
        let (decode_buffer, decode_buffer_len) = loop {
            match decoder.decode(&reader.next_packet()?) {
                Ok(decoded) => {
                    // Get the buffer spec.
                    let spec = *decoded.spec();
                    if let Some(channels) = channels {
                        assert_eq!(channels, spec.channels);
                    } else {
                        log::debug!(
                            "Assuming {num_channels} channel(s) according to the first decoded packet",
                            num_channels = spec.channels.count()
                        );
                        channels = Some(spec.channels);
                    }

                    let len = decoded.frames();
                    let capacity = decoded.capacity();

                    let mut decode_buffer: AudioBuffer<f32> =
                        AudioBuffer::new(capacity as u64, spec);

                    decoded.convert(&mut decode_buffer);

                    break (decode_buffer, len);
                }
                Err(Error::DecodeError(err)) => {
                    // Decode errors are not fatal.
                    log::warn!("{err}");
                    // Continue by decoding the next packet.
                    continue;
                }
                Err(e) => {
                    // Errors other than decode errors are fatal.
                    return Err(e.into());
                }
            }
        };

        let metadata = reader.metadata().skip_to_latest().cloned();
        let info = SymphoniaDecoderInfo {
            codec_params: params,
            metadata,
        };
        let num_channels = (channels.ok_or(OpenError::NoNumChannels)?).count();

        let file_info = FileInfo {
            params: info,
            num_frames,
            num_channels: num_channels as u16,
            sample_rate,
        };
        Ok((
            Self {
                reader,
                decoder,

                decode_buffer,
                decode_buffer_len,
                curr_decode_buffer_frame: 0,

                num_frames,
                sample_rate,
                block_size,

                playhead_frame: start_frame,
                reset_decode_buffer: false,
                seek_diff: 0,
            },
            file_info,
        ))
    }

    fn seek(&mut self, frame: usize) -> Result<(), Self::FatalError> {
        if frame >= self.num_frames {
            // Do nothing if out of range.
            self.playhead_frame = self.num_frames;

            return Ok(());
        }

        self.playhead_frame = frame;

        match self.reader.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time: frame_to_symphonia_time(
                    self.playhead_frame as u64,
                    self.sample_rate.unwrap_or(44100),
                ),
                track_id: None,
            },
        ) {
            Ok(res) => {
                self.seek_diff = (res.required_ts - res.actual_ts) as usize;
            }
            Err(e) => {
                return Err(e);
            }
        }

        self.reset_decode_buffer = true;
        self.curr_decode_buffer_frame = 0;

        /*
        let decoder_opts = DecoderOptions {
            verify: false,
            ..Default::default()
        };

        self.decoder.close();
        self.decoder = symphonia::default::get_codecs()
            .make(self.decoder.codec_params(), &decoder_opts)?;
            */

        Ok(())
    }

    fn decode(&mut self, data_block: &mut DataBlock<Self::T>) -> Result<(), Self::FatalError> {
        if self.playhead_frame >= self.num_frames {
            // Do nothing if reached the end of the file.
            return Ok(());
        }

        let mut reached_end_of_file = false;

        let mut block_start_frame = 0;
        while block_start_frame < self.block_size {
            let num_frames_to_cpy = if self.reset_decode_buffer {
                // Get new data first.
                self.reset_decode_buffer = false;
                0
            } else {
                // Find the maximum amount of frames that can be copied.
                (self.block_size - block_start_frame)
                    .min(self.decode_buffer_len - self.curr_decode_buffer_frame)
            };

            if num_frames_to_cpy != 0 {
                let src_planes = self.decode_buffer.planes();
                let src_channels = src_planes.planes();

                for (dst_ch, src_ch) in data_block.block.iter_mut().zip(src_channels) {
                    let src_ch_part = &src_ch[self.curr_decode_buffer_frame
                        ..self.curr_decode_buffer_frame + num_frames_to_cpy];
                    dst_ch.extend_from_slice(src_ch_part);
                }

                block_start_frame += num_frames_to_cpy;

                self.curr_decode_buffer_frame += num_frames_to_cpy;
                if self.curr_decode_buffer_frame >= self.decode_buffer_len {
                    self.reset_decode_buffer = true;
                }
            } else {
                // Decode the next packet.

                loop {
                    match self.reader.next_packet() {
                        Ok(packet) => {
                            match self.decoder.decode(&packet) {
                                Ok(decoded) => {
                                    self.decode_buffer_len = decoded.frames();
                                    if self.seek_diff < self.decode_buffer_len {
                                        let capacity = decoded.capacity();
                                        if self.decode_buffer.capacity() < capacity {
                                            self.decode_buffer =
                                                AudioBuffer::new(capacity as u64, *decoded.spec());
                                        }
                                        decoded.convert(&mut self.decode_buffer);
                                        self.curr_decode_buffer_frame = self.seek_diff;
                                        self.seek_diff = 0;
                                        break;
                                    } else {
                                        self.seek_diff -= self.decode_buffer_len;
                                    }
                                }
                                Err(Error::DecodeError(err)) => {
                                    // Decode errors are not fatal.
                                    log::warn!("{err}");
                                    // Continue by decoding the next packet.
                                    continue;
                                }
                                Err(e) => {
                                    // Errors other than decode errors are fatal.
                                    return Err(e);
                                }
                            }
                        }
                        Err(e) => {
                            if let Error::IoError(io_error) = &e {
                                if io_error.kind() == std::io::ErrorKind::UnexpectedEof {
                                    // End of file, stop decoding.
                                    reached_end_of_file = true;
                                    block_start_frame = self.block_size;
                                    break;
                                } else {
                                    return Err(e);
                                }
                            } else {
                                return Err(e);
                            }
                        }
                    }
                }
            }
        }

        if reached_end_of_file {
            self.playhead_frame = self.num_frames;
        } else {
            self.playhead_frame += self.block_size;
        }

        Ok(())
    }

    fn current_frame(&self) -> usize {
        self.playhead_frame
    }
}

impl Drop for SymphoniaDecoder {
    fn drop(&mut self) {
        let _ = self.decoder.finalize();
    }
}

impl SymphoniaDecoder {
    /// Symphonia does metadata oddly. This is more for raw access.
    ///
    /// See [`Metadata`](https://docs.rs/symphonia-core/0.5.2/symphonia_core/meta/struct.Metadata.html).
    pub fn get_metadata_raw(&mut self) -> Metadata<'_> {
        self.reader.metadata()
    }

    /// Get the latest entry in the metadata.
    pub fn get_metadata(&mut self) -> Option<MetadataRevision> {
        let mut md = self.reader.metadata();
        md.skip_to_latest().cloned()
    }
}

#[derive(Debug, Clone)]
pub struct SymphoniaDecoderInfo {
    pub codec_params: CodecParameters,
    pub metadata: Option<MetadataRevision>,
}

fn frame_to_symphonia_time(frame: u64, sample_rate: u32) -> symphonia::core::units::Time {
    // Doing it this way is more accurate for large inputs than just using f64s.
    let seconds = frame / u64::from(sample_rate);
    let fract_frames = frame % u64::from(sample_rate);
    let frac = fract_frames as f64 / f64::from(sample_rate);

    // TODO: Ask the maintainer of Symphonia to add an option to seek to an
    // exact frame.
    symphonia::core::units::Time { seconds, frac }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::*;

    #[test]
    fn decoder_new() {
        let files = vec![
            //  file | num_channels | num_frames | sample_rate
            ("../test_files/wav_u8_mono.wav", 1, 1323000, Some(44100)),
            ("../test_files/wav_i16_mono.wav", 1, 1323000, Some(44100)),
            ("../test_files/wav_i24_mono.wav", 1, 1323000, Some(44100)),
            ("../test_files/wav_i32_mono.wav", 1, 1323000, Some(44100)),
            ("../test_files/wav_f32_mono.wav", 1, 1323000, Some(44100)),
            ("../test_files/wav_i24_stereo.wav", 2, 1323000, Some(44100)),
            //"../test_files/ogg_mono.ogg",
            //"../test_files/ogg_stereo.ogg",
            //"../test_files/mp3_constant_mono.mp3",
            //"../test_files/mp3_constant_stereo.mp3",
            //"../test_files/mp3_variable_mono.mp3",
            //"../test_files/mp3_variable_stereo.mp3",
        ];

        for file in files {
            dbg!(file.0);
            let decoder =
                SymphoniaDecoder::new(file.0.into(), 0, SymphoniaDecoder::DEFAULT_BLOCK_SIZE, ());
            match decoder {
                Ok((_, file_info)) => {
                    assert_eq!(file_info.num_channels, file.1);
                    assert_eq!(file_info.num_frames, file.2);
                    //assert_eq!(file_info.sample_rate, file.3);
                }
                Err(e) => {
                    panic!("{}", e);
                }
            }
        }
    }

    #[test]
    fn decode_first_frame() {
        let block_size = 10;

        let decoder =
            SymphoniaDecoder::new("../test_files/wav_u8_mono.wav".into(), 0, block_size, ());

        let (mut decoder, file_info) = decoder.unwrap();

        let mut data_block = DataBlock::new(1, block_size);
        data_block.clear();
        decoder.decode(&mut data_block).unwrap();

        let samples = &mut data_block.block[0];
        assert_eq!(samples.len(), block_size);

        let first_frame = [
            0.0, 0.046875, 0.09375, 0.1484375, 0.1953125, 0.2421875, 0.2890625, 0.3359375,
            0.3828125, 0.421875,
        ];

        for i in 0..samples.len() {
            assert!(approx_eq!(f32, first_frame[i], samples[i], ulps = 2));
        }

        let second_frame = [
            0.46875, 0.5078125, 0.5390625, 0.578125, 0.609375, 0.640625, 0.671875, 0.6953125,
            0.71875, 0.7421875,
        ];

        data_block.clear();
        decoder.decode(&mut data_block).unwrap();

        let samples = &mut data_block.block[0];
        for i in 0..samples.len() {
            assert_approx_eq!(f32, second_frame[i], samples[i], ulps = 2);
        }

        let last_frame = [
            -0.0625, -0.046875, -0.0234375, -0.0078125, 0.015625, 0.03125, 0.046875, 0.0625,
            0.078125, 0.0859375,
        ];

        // Seek to last frame
        decoder.seek(file_info.num_frames - 1 - block_size).unwrap();

        data_block.clear();
        decoder.decode(&mut data_block).unwrap();

        let samples = &mut data_block.block[0];
        for i in 0..samples.len() {
            assert_approx_eq!(f32, last_frame[i], samples[i], ulps = 2);
        }

        assert_eq!(decoder.playhead_frame, file_info.num_frames - 1);
    }
}

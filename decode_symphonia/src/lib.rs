use std::fs::File;
use std::path::PathBuf;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CodecParameters, Decoder as SymphDecoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Duration;

use creek_core::{DataBlock, Decoder, FileInfo};

mod error;
pub use error::OpenError;

pub struct SymphoniaDecoder {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn SymphDecoder>,

    smp_buf: SampleBuffer<f32>,
    curr_smp_buf_i: usize,

    num_frames: usize,
    num_channels: usize,
    sample_rate: Option<u32>,
    block_size: usize,

    current_frame: usize,
    reset_smp_buffer: bool,
}

impl Decoder for SymphoniaDecoder {
    type T = f32;
    type FileParams = CodecParameters;
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
            let stream = reader
                .default_track()
                .ok_or_else(|| OpenError::NoDefaultTrack)?;

            stream.codec_params.clone()
        };

        let num_frames = params.n_frames.ok_or_else(|| OpenError::NoNumFrames)? as usize;
        let num_channels = (params.channels.ok_or_else(|| OpenError::NoNumChannels)?).count();
        let sample_rate = params.sample_rate;

        // Seek the reader to the requested position.
        if start_frame != 0 {
            let seconds = start_frame as f64 / f64::from(sample_rate.unwrap_or(44100));

            reader.seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: seconds.into(),
                    track_id: None,
                }
            )?;
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

                    println!("{}", e);

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
            num_channels: num_channels as u16,
            sample_rate: sample_rate.map(|s| s as u32),
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
                block_size,

                current_frame: start_frame,
                reset_smp_buffer: false,
            },
            file_info,
        ))
    }

    fn seek(&mut self, frame: usize) -> Result<(), Self::FatalError> {
        if frame >= self.num_frames {
            // Do nothing if out of range.
            self.current_frame = self.num_frames;

            return Ok(());
        }

        self.current_frame = frame;

        let seconds = self.current_frame as f64 / f64::from(self.sample_rate.unwrap_or(44100));

        match self.reader.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time: seconds.into(),
                track_id: None,
            }
        ) {
            Ok(_res) => {}
            Err(e) => {
                return Err(e);
            }
        }

        self.reset_smp_buffer = true;
        self.curr_smp_buf_i = 0;

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

    unsafe fn decode(
        &mut self,
        data_block: &mut DataBlock<Self::T>,
    ) -> Result<(), Self::FatalError> {
        if self.current_frame >= self.num_frames {
            // Do nothing if reached the end of the file.
            return Ok(());
        }

        let mut reached_end_of_file = false;

        let mut block_start = 0;
        while block_start < self.block_size {
            let num_frames_to_cpy = if self.reset_smp_buffer {
                // Get new data first.
                self.reset_smp_buffer = false;
                0
            } else if self.smp_buf.len() < self.num_channels {
                // Get new data first.
                0
            } else {
                // Find the maximum amount of frames that can be copied.
                (self.block_size - block_start)
                    .min((self.smp_buf.len() - self.curr_smp_buf_i) / self.num_channels)
            };

            if num_frames_to_cpy != 0 {
                if self.num_channels == 1 {
                    // Mono, no need to deinterleave.
                    data_block.block[0][block_start..block_start + num_frames_to_cpy]
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

                                    println!("{}", e);

                                    // TODO: print warning.

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
                                    block_start = self.block_size;
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
            self.current_frame = self.num_frames;
        } else {
            self.current_frame += self.block_size;
        }

        Ok(())
    }

    fn current_frame(&self) -> usize {
        self.current_frame
    }
}

impl Drop for SymphoniaDecoder {
    fn drop(&mut self) {
        let _ = self.decoder.finalize();
    }
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
        println!("{:?}", file_info.num_frames);

        let mut data_block = DataBlock::new(1, block_size);
        unsafe {
            decoder.decode(&mut data_block).unwrap();
        }

        let samples = &mut data_block.block[0];
        assert_eq!(samples.len(), block_size);


        let first_frame = [
            0.0,
            0.046875,
            0.09375,
            0.1484375,
            0.1953125,
            0.2421875,
            0.2890625,
            0.3359375,
            0.3828125,
            0.421875
        ];


        for i in 0..samples.len() {
            assert!(approx_eq!(f32, first_frame[i], samples[i], ulps = 2));
        }

        let second_frame = [
            0.46875,
            0.5078125,
            0.5390625,
            0.578125,
            0.609375,
            0.640625,
            0.671875,
            0.6953125,
            0.71875,
            0.7421875,
        ];

        unsafe {
            decoder.decode(&mut data_block).unwrap();
        }

        let samples = &mut data_block.block[0];
        for i in 0..samples.len() {
            assert!(approx_eq!(f32, second_frame[i], samples[i], ulps = 2));
        }

        let last_frame = [
            -0.0625,
            -0.046875,
            -0.0234375,
            -0.0078125,
            0.015625,
            0.03125,
            0.046875,
            0.0625,
            0.078125,
            0.0859375,
        ];

        // Seek to last frame
        decoder.seek(file_info.num_frames - 1 - block_size).unwrap();

        unsafe {
            decoder.decode(&mut data_block).unwrap();
        }
        let samples = &mut data_block.block[0];
        for i in 0..samples.len() {
            assert!(approx_eq!(f32, last_frame[i], samples[i], ulps = 2));
        }

        assert_eq!(decoder.current_frame, file_info.num_frames - 1);
    }
}

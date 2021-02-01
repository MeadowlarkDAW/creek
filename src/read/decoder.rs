use std::fs::File;
use std::path::PathBuf;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CodecParameters, Decoder as SymphDecoder, DecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Duration;

use super::{
    error::{OpenError, ReadError},
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

    sample_buf: SampleBuffer<f32>,

    num_frames: usize,
    num_channels: usize,
    sample_rate: Option<u32>,

    decoder_opts: DecoderOptions,

    current_frame: usize,
    did_just_sync: bool,
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

        // Create a decoder for the stream.
        let mut decoder = symphonia::default::get_codecs().make(&params, &decoder_opts)?;

        // Seek the reader to the requested position.
        if start_frame != 0 {
            start_frame = wrap_frame(start_frame, num_frames);

            let seconds = start_frame as f64 / f64::from(sample_rate.unwrap_or(44100));

            reader.seek(SeekTo::Time {
                time: seconds.into(),
            })?;
        }

        // Decode the first packet to get the signal specification.
        let sample_buf = loop {
            match decoder.decode(&reader.next_packet()?) {
                Ok(decoded) => {
                    // Get the buffer spec.
                    let spec = *decoded.spec();

                    // Get the buffer duration.
                    let duration = Duration::from(decoded.capacity() as u64);

                    let mut sample_buf = SampleBuffer::<f32>::new(duration, spec);

                    sample_buf.copy_interleaved_ref(decoded);

                    break sample_buf;
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

                sample_buf,

                num_frames,
                num_channels,
                sample_rate,

                decoder_opts,

                current_frame: start_frame,
                did_just_sync: false,
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

        self.decoder.close();
        self.decoder = symphonia::default::get_codecs()
            .make(self.decoder.codec_params(), &self.decoder_opts)?;

        Ok(())
    }

    pub fn decode_into(&mut self, data_block: &mut DataBlock) -> Result<(), ReadError> {
        let mut frames_left = BLOCK_SIZE;
        while frames_left != 0 {
            if self.sample_buf.len() != 0 && !self.did_just_sync {
                let num_new_frames = (self.sample_buf.len() / self.num_channels).min(frames_left);

                if self.num_channels == 1 {
                    // Mono, no need to deinterleave.
                    &mut data_block.block[0][0..num_new_frames]
                        .copy_from_slice(&self.sample_buf.samples()[0..num_new_frames]);
                } else if self.num_channels == 2 {
                    // Provide somewhat-efficient stereo deinterleaving.

                    let smp_buf = &self.sample_buf.samples()[0..num_new_frames * 2];

                    for i in 0..num_new_frames {
                        data_block.block[0][i] = smp_buf[i * 2];
                        data_block.block[1][i] = smp_buf[(i * 2) + 1];
                    }
                } else {
                    let smp_buf =
                        &self.sample_buf.samples()[0..num_new_frames * data_block.block.len()];

                    for i in 0..num_new_frames {
                        for (ch, block) in data_block.block.iter_mut().enumerate() {
                            block[i] = smp_buf[(i * self.num_channels) + ch];
                        }
                    }
                }

                frames_left -= num_new_frames;
            }

            if frames_left != 0 {
                // Decode more packets.

                let mut do_decode = true;
                while do_decode {
                    match self.decoder.decode(&self.reader.next_packet()?) {
                        Ok(decoded) => {
                            self.sample_buf.copy_interleaved_ref(decoded);
                            do_decode = false;
                            self.did_just_sync = false;
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
            }
        }

        data_block.start_frame = self.current_frame;

        self.current_frame = wrap_frame(self.current_frame + BLOCK_SIZE, self.num_frames);

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

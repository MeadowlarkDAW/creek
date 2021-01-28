use std::fs::File;
use std::path::PathBuf;

use rtrb::{Consumer, Producer, RingBuffer};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::Decoder;
use symphonia::core::formats::FormatReader;

use crate::{BLOCK_SIZE, SERVER_WAIT_TIME};

use super::error::OpenError;
use super::{ClientToServerMsg, DataBlock, DataBlockCache, FileInfo, HeapData, ServerToClientMsg};

pub(crate) struct ReadServer {
    to_client_tx: Producer<ServerToClientMsg>,
    from_client_rx: Consumer<ClientToServerMsg>,
    close_signal_rx: Consumer<Option<HeapData>>,

    reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,

    sample_buf: SampleBuffer<f32>,

    block_pool: Vec<DataBlock>,
    cache_pool: Vec<DataBlockCache>,

    num_frames: usize,
    num_channels: usize,
    sample_rate: u32,

    current_frame: usize,
    did_sync: bool,
    run: bool,
}

impl ReadServer {
    pub fn new(
        file: PathBuf,
        verify: bool,
        start_frame_in_file: usize,
        to_client_tx: Producer<ServerToClientMsg>,
        from_client_rx: Consumer<ClientToServerMsg>,
        close_signal_rx: Consumer<Option<HeapData>>,
    ) -> Result<FileInfo, OpenError> {
        let (mut open_tx, mut open_rx) = RingBuffer::<Result<FileInfo, OpenError>>::new(1).split();

        std::thread::spawn(move || {
            match Self::build(
                file,
                verify,
                start_frame_in_file,
                to_client_tx,
                from_client_rx,
                close_signal_rx,
            ) {
                Ok((server, file_info)) => {
                    // Push cannot fail because only one message is ever sent.
                    let _ = open_tx.push(Ok(file_info));
                    server.run();
                }
                Err(e) => {
                    // Push cannot fail because only one message is ever sent.
                    let _ = open_tx.push(Err(e));
                }
            }
        });

        loop {
            if let Ok(res) = open_rx.pop() {
                return res;
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }
    }

    pub fn build(
        file: PathBuf,
        verify: bool,
        mut start_frame_in_file: usize,
        to_client_tx: Producer<ServerToClientMsg>,
        from_client_rx: Consumer<ClientToServerMsg>,
        close_signal_rx: Consumer<Option<HeapData>>,
    ) -> Result<(Self, FileInfo), OpenError> {
        use symphonia::core::codecs::DecoderOptions;
        use symphonia::core::errors::Error;
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;
        use symphonia::core::units::Duration;

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
        let sample_rate = params.sample_rate.unwrap_or(44100);

        // Create a decoder for the stream.
        let mut decoder = symphonia::default::get_codecs().make(&params, &decoder_opts)?;

        // Seek the reader to the requested position.
        if start_frame_in_file != 0 {
            use symphonia::core::formats::SeekTo;

            start_frame_in_file = wrap_frame(start_frame_in_file, num_frames);

            let seconds = start_frame_in_file as f64 / f64::from(sample_rate);

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

                    assert_eq!(spec.channels.count(), num_channels as usize);

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
                to_client_tx,
                from_client_rx,
                close_signal_rx,

                reader,
                decoder,

                sample_buf,

                block_pool: Vec::new(),
                cache_pool: Vec::new(),

                num_frames,
                num_channels,
                sample_rate,

                current_frame: start_frame_in_file,
                did_sync: false,
                run: true,
            },
            file_info,
        ))
    }

    fn run(mut self) {
        while self.run {
            // Check for close signal.
            if let Ok(heap_data) = self.close_signal_rx.pop() {
                // Drop heap data here.
                let _ = heap_data;
                self.run = false;
                break;
            }

            while let Ok(msg) = self.from_client_rx.pop() {
                match msg {
                    ClientToServerMsg::ReadIntoBlock {
                        block_index,
                        block,
                        starting_frame_in_file,
                    } => {
                        let mut block = block.unwrap_or(
                            // Try using one in the pool if it exists.
                            self.block_pool.pop().unwrap_or(
                                // No blocks in pool. Create a new one.
                                DataBlock::new(self.num_channels),
                            ),
                        );

                        block.starting_frame_in_file = starting_frame_in_file;
                        block.wanted_start_smp = starting_frame_in_file;

                        match self.decode_into_block(&mut block) {
                            Ok(()) => {
                                self.send_msg(ServerToClientMsg::ReadIntoBlockRes {
                                    block_index,
                                    block,
                                });
                            }
                            Err(e) => {
                                self.send_msg(ServerToClientMsg::FatalError(e));
                                self.run = false;
                                break;
                            }
                        }
                    }
                    ClientToServerMsg::DisposeBlock { block } => {
                        // Store the block to be reused.
                        self.block_pool.push(block);
                    }
                    ClientToServerMsg::SeekTo { frame } => {
                        if let Err(e) = self.seek(frame) {
                            self.send_msg(ServerToClientMsg::FatalError(e));
                            self.run = false;
                            break;
                        }
                    }
                    ClientToServerMsg::Cache {
                        cache_index,
                        cache,
                        starting_frame_in_file,
                    } => {
                        let mut cache = cache.unwrap_or(
                            // Try using one in the pool if it exists.
                            self.cache_pool.pop().unwrap_or(
                                // No caches in pool. Create a new one.
                                DataBlockCache::new(self.num_channels),
                            ),
                        );

                        cache.wanted_start_smp = starting_frame_in_file;

                        let current_frame = self.current_frame;

                        // Seek to the position the client wants to cache.
                        if let Err(e) = self.seek(starting_frame_in_file) {
                            self.send_msg(ServerToClientMsg::FatalError(e));
                            self.run = false;
                            break;
                        }

                        // Fill the cache
                        for block in cache.blocks.iter_mut() {
                            if let Err(e) = self.decode_into_block(block) {
                                self.send_msg(ServerToClientMsg::FatalError(e));
                                self.run = false;
                                break;
                            }
                        }

                        // Seek back to the previous position.
                        if let Err(e) = self.seek(current_frame) {
                            self.send_msg(ServerToClientMsg::FatalError(e));
                            self.run = false;
                            break;
                        }

                        self.send_msg(ServerToClientMsg::CacheRes { cache_index, cache });
                    }
                    ClientToServerMsg::DisposeCache { cache } => {
                        // Store the cache to be reused.
                        self.cache_pool.push(cache);
                    }
                }
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }

        self.decoder.close();
    }

    fn send_msg(&mut self, msg: ServerToClientMsg) {
        // Block until message can be sent.
        loop {
            if !self.to_client_tx.is_full() {
                break;
            }

            // Check for close signal to avoid waiting forever.
            if let Ok(heap_data) = self.close_signal_rx.pop() {
                // Drop heap data here.
                let _ = heap_data;
                self.run = false;
                break;
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }

        // Push will never fail because we made sure a slot is available in the
        // previous step (or the server has closed).
        let _ = self.to_client_tx.push(msg);
    }

    fn seek(&mut self, frame: usize) -> Result<(), symphonia::core::errors::Error> {
        use symphonia::core::formats::SeekTo;

        self.current_frame = wrap_frame(frame, self.num_frames);

        let seconds = self.current_frame as f64 / f64::from(self.sample_rate);

        self.reader.seek(SeekTo::Time {
            time: seconds.into(),
        })?;

        self.did_sync = true;

        Ok(())
    }

    fn decode_into_block(
        &mut self,
        data_block: &mut DataBlock,
    ) -> Result<(), symphonia::core::errors::Error> {
        use symphonia::core::errors::Error;

        let mut frames_left = BLOCK_SIZE;
        while frames_left != 0 {
            if self.sample_buf.len() != 0 && !self.did_sync {
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
                            self.did_sync = false;
                        }
                        Err(Error::DecodeError(e)) => {
                            // Decode errors are not fatal. Print a message and try to decode the next packet as
                            // usual.

                            // TODO: print warning.

                            continue;
                        }
                        Err(e) => {
                            // Errors other than decode errors are fatal.
                            return Err(e);
                        }
                    }
                }
            }
        }

        data_block.starting_frame_in_file = self.current_frame;

        self.current_frame = wrap_frame(self.current_frame + BLOCK_SIZE, self.num_frames);

        Ok(())
    }
}

fn wrap_frame(mut frame: usize, len: usize) -> usize {
    while frame >= len {
        frame -= len;
    }
    frame
}

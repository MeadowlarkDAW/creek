use std::fs::File;
use std::io;
use std::path::PathBuf;

use rtrb::{Consumer, Producer, RingBuffer};

pub const BLOCK_SIZE: usize = 4096;
pub const NUM_PREFETCH_BLOCKS: usize = 4;
pub const MSG_CHANNEL_SIZE: usize = 64;

static SILENCE_BUFFER: [f32; BLOCK_SIZE] = [0.0; BLOCK_SIZE];

enum ServerToClientMsg {
    ReadIntoBlockRes {
        block_index: usize,
        block: DataBlock,
    },
    CacheRes {
        cache_index: usize,
        cache: DataBlockCache,
    },
    FatalError(symphonia::core::errors::Error),
}

enum ClientToServerMsg {
    ReadIntoBlock {
        block_index: usize,
        block: Option<DataBlock>,
        starting_frame_in_file: usize,
    },
    DisposeBlock {
        block: DataBlock,
    },
    SeekTo {
        frame: usize,
    },
    Cache {
        cache_index: usize,
        cache: Option<DataBlockCache>,
        starting_frame_in_file: usize,
    },
    DisposeCache {
        cache: DataBlockCache,
    },
}

#[derive(Debug)]
pub enum OpenError {
    Io(io::Error),
    Format(symphonia::core::errors::Error),
    NoDefaultStream,
    NoNumFrames,
    NoNumChannels,
}

impl std::error::Error for OpenError {}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Io(e) => write!(f, "IO error: {:?}", e),
            OpenError::Format(e) => write!(f, "Format error: {:?}", e),
            OpenError::NoDefaultStream => write!(f, "No default stream for codec"),
            OpenError::NoNumFrames => write!(f, "Failed to find the number of frames in the file"),
            OpenError::NoNumChannels => {
                write!(f, "Failed to find the number of channels in the file")
            }
        }
    }
}

impl From<io::Error> for OpenError {
    fn from(e: io::Error) -> Self {
        OpenError::Io(e)
    }
}

impl From<symphonia::core::errors::Error> for OpenError {
    fn from(e: symphonia::core::errors::Error) -> Self {
        OpenError::Format(e)
    }
}

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::Decoder;
use symphonia::core::formats::FormatReader;

struct ReadServer {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,

    sample_buf: SampleBuffer<f32>,

    num_frames: usize,
    num_channels: usize,
    sample_rate: u32,

    current_frame: usize,
    did_sync: bool,
}

impl ReadServer {
    pub fn new<P: Into<PathBuf>>(file: P, verify: bool) -> Result<Self, OpenError> {
        use symphonia::core::codecs::DecoderOptions;
        use symphonia::core::errors::Error;
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;
        use symphonia::core::units::Duration;

        let path: PathBuf = file.into();

        // Create a hint to help the format registry guess what format reader is appropriate.
        let mut hint = Hint::new();

        // Provide the file extension as a hint.
        if let Some(extension) = path.extension() {
            if let Some(extension_str) = extension.to_str() {
                hint.with_extension(extension_str);
            }
        }

        let source = Box::new(File::open(path)?);

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

        // Get the default stream.
        let stream = reader
            .default_stream()
            .ok_or_else(|| OpenError::NoDefaultStream)?;

        let params = &stream.codec_params;

        let num_frames = params.n_frames.ok_or_else(|| OpenError::NoNumFrames)? as usize;
        let num_channels = (params.channels.ok_or_else(|| OpenError::NoNumChannels)?).count();
        let sample_rate = params.sample_rate.unwrap_or(44100);

        // Create a decoder for the stream.
        let mut decoder =
            symphonia::default::get_codecs().make(&stream.codec_params, &decoder_opts)?;

        // Decode the first packet to get the signal specification.
        let sample_buf = loop {
            match decoder.decode(&reader.next_packet()?) {
                Ok(decoded) => {
                    // Get the buffer spec.
                    let spec = *decoded.spec();

                    assert_eq!(spec.channels.count(), num_channels as usize);

                    // Get the buffer duration.
                    let duration = Duration::from(decoded.capacity() as u64);

                    break SampleBuffer::<f32>::new(duration, spec);
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

        Ok(Self {
            reader,
            decoder,

            sample_buf,

            num_frames,
            num_channels,
            sample_rate,

            current_frame: 0,
            did_sync: false,
        })
    }

    pub fn seek(&mut self, frame: usize) -> Result<(), symphonia::core::errors::Error> {
        use symphonia::core::formats::SeekTo;

        self.current_frame = wrap_frame(frame, self.num_frames);

        let seconds = self.current_frame as f64 / f64::from(self.sample_rate);

        self.reader.seek(SeekTo::Time {
            time: seconds.into(),
        })?;

        self.did_sync = true;

        Ok(())
    }

    pub fn decode_into_block(
        &mut self,
        data_block: &mut DataBlock,
    ) -> Result<(), symphonia::core::errors::Error> {
        use symphonia::core::errors::Error;

        assert!(data_block.block.len() == self.num_channels);

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

pub struct ReadStream {}

impl ReadStream {
    pub fn new<P: Into<PathBuf>>(file: P, verify: bool) -> Result<Self, OpenError> {
        let mut server = ReadServer::new(file, verify)?;

        Ok(Self {})
    }
}

fn wrap_frame(mut frame: usize, len: usize) -> usize {
    while frame >= len {
        frame -= len;
    }
    frame
}

pub struct DataBlock {
    pub block: Vec<[f32; BLOCK_SIZE]>,
    pub starting_frame_in_file: usize,
    pub requested_frame_in_file: usize,
}

struct DataBlockEntry {
    pub use_cache: Option<usize>,
    pub block: Option<DataBlock>,
    pub wanted_start_smp: usize,
}

struct DataBlockCache {
    pub blocks: [DataBlock; NUM_PREFETCH_BLOCKS],
    pub requested_frame_in_file: usize,
}

struct DataBlockCacheEntry {
    pub cache: Option<DataBlockCache>,
    pub wanted_start_smp: usize,
}

pub struct ReadData<'a> {
    data: &'a Vec<Vec<f32>>,
    len: usize,
}

impl<'a> ReadData<'a> {
    pub(crate) fn new(data: &'a Vec<Vec<f32>>, len: usize) -> Self {
        Self { data, len }
    }

    pub fn read_channel(&self, channel: usize) -> &[f32] {
        &self.data[channel][0..self.len]
    }

    pub fn num_channels(&self) -> usize {
        self.data.len()
    }

    pub fn buffer_len(&self) -> usize {
        self.len
    }
}

pub struct ReadClient {
    to_server_tx: Producer<ClientToServerMsg>,
    from_server_rx: Consumer<ServerToClientMsg>,

    read_buffer: Vec<Vec<f32>>,
    prefetch_buffer: Vec<DataBlockEntry>,
    caches: Vec<DataBlockCacheEntry>,

    num_channels: usize,
    current_block_index: usize,
    next_block_index: usize,
    current_block_starting_frame_in_file: usize,
    current_frame_in_block: usize,
}

impl ReadClient {
    pub(crate) fn new(
        to_server_tx: Producer<ClientToServerMsg>,
        from_server_rx: Consumer<ServerToClientMsg>,
        prefetch_buffer: Vec<DataBlockEntry>,
        starting_frame_in_file: usize,
        num_channels: usize,
        max_num_caches: usize,
    ) -> Self {
        let mut read_buffer: Vec<Vec<f32>> = Vec::with_capacity(num_channels);
        for _ in 0..num_channels {
            let mut data: Vec<f32> = Vec::with_capacity(BLOCK_SIZE);

            // Safe because algorithm will always ensure that data can only be
            // read once it's written to.
            unsafe {
                data.set_len(BLOCK_SIZE);
            }

            read_buffer.push(data);
        }

        let mut caches: Vec<DataBlockCacheEntry> = Vec::with_capacity(max_num_caches);
        for _ in 0..max_num_caches {
            caches.push(DataBlockCacheEntry {
                cache: None,
                wanted_start_smp: 0,
            });
        }

        Self {
            to_server_tx,
            from_server_rx,

            read_buffer,
            prefetch_buffer,

            caches,

            num_channels,
            current_block_index: 0,
            next_block_index: 1,
            current_block_starting_frame_in_file: starting_frame_in_file,
            current_frame_in_block: 0,
        }
    }

    pub fn max_num_caches(&self) -> usize {
        self.caches.len()
    }

    pub fn cache(&mut self, cache_index: usize, starting_frame_in_file: usize) -> bool {
        assert!(cache_index < self.caches.len());

        let mut do_cache = false;
        if let Some(cache) = &self.caches[cache_index].cache {
            if cache.requested_frame_in_file != starting_frame_in_file {
                do_cache = true;
            }
        } else {
            do_cache = true;
        }

        if do_cache {
            self.caches[cache_index].wanted_start_smp = starting_frame_in_file;
            let cache = self.caches[cache_index].cache.take();

            self.to_server_tx
                .push(ClientToServerMsg::Cache {
                    cache_index,
                    cache,
                    starting_frame_in_file,
                })
                .expect("Client to Server channel full");

            return false;
        }

        true
    }

    pub fn seek_to_cache(&mut self, cache_index: usize, starting_frame_in_file: usize) -> bool {
        let cache_exists = self.cache(cache_index, starting_frame_in_file);

        self.current_block_starting_frame_in_file = starting_frame_in_file;
        self.current_frame_in_block = 0;

        // Request the server to start fetching blocks ahead of the cache.
        self.to_server_tx
            .push(ClientToServerMsg::SeekTo {
                frame: self.current_block_starting_frame_in_file
                    + (NUM_PREFETCH_BLOCKS * BLOCK_SIZE),
            })
            .expect("Client to Server channel full");

        // Tell each prefetch block to use the cache.
        let mut wanted_start_smp = starting_frame_in_file;
        for block in self.prefetch_buffer.iter_mut() {
            block.use_cache = Some(cache_index);
            block.wanted_start_smp = wanted_start_smp;

            wanted_start_smp += BLOCK_SIZE;
        }

        cache_exists
    }

    /// Read the next slice of data with length `length`.
    ///
    /// ## Panics
    /// This will panic if `length` > `BLOCK_SIZE` (4096).
    pub fn read(&mut self, length: usize) -> ReadData {
        assert!(length <= BLOCK_SIZE);

        // Retrieve any data sent from the server.
        while let Ok(msg) = self.from_server_rx.pop() {
            match msg {
                ServerToClientMsg::ReadIntoBlockRes { block_index, block } => {
                    let prefetch_block = &mut self.prefetch_buffer[block_index];

                    // Only use results from the latest request.
                    if block.requested_frame_in_file == prefetch_block.wanted_start_smp {
                        if let Some(prefetch_block) = prefetch_block.block.take() {
                            // Tell the IO server to deallocate the old block.
                            self.to_server_tx
                                .push(ClientToServerMsg::DisposeBlock {
                                    block: prefetch_block,
                                })
                                .expect("Client to Server channel full");
                        }

                        // Store the new block into the prefetch buffer.
                        prefetch_block.block = Some(block);
                    } else {
                        // Tell the IO server to deallocate the block.
                        self.to_server_tx
                            .push(ClientToServerMsg::DisposeBlock { block })
                            .expect("Client to Server channel full");
                    }
                }
                ServerToClientMsg::CacheRes { cache_index, cache } => {
                    let cache_entry = &mut self.caches[cache_index];

                    // Only use results from the latest request.
                    if cache.requested_frame_in_file == cache_entry.wanted_start_smp {
                        if let Some(cache_entry) = cache_entry.cache.take() {
                            // Tell the IO server to deallocate the old cache.
                            self.to_server_tx
                                .push(ClientToServerMsg::DisposeCache { cache: cache_entry })
                                .expect("Client to Server channel full");
                        }

                        // Store the new cache.
                        cache_entry.cache = Some(cache);
                    } else {
                        // Tell the IO server to deallocate the cache.
                        self.to_server_tx
                            .push(ClientToServerMsg::DisposeCache { cache })
                            .expect("Client to Server channel full");
                    }
                }
                ServerToClientMsg::FatalError(e) => {
                    // TODO: handle error
                }
            }
        }

        let (current_block_data, current_block_start_frame) = {
            let current_block = &self.prefetch_buffer[self.current_block_index];

            if let Some(cache_index) = current_block.use_cache {
                if let Some(cache) = &self.caches[cache_index].cache {
                    let start_frame = cache.blocks[self.current_block_index].starting_frame_in_file;
                    (Some(&cache.blocks[self.current_block_index]), start_frame)
                } else {
                    // If cache is empty, output silence instead.
                    (None, self.current_block_starting_frame_in_file)
                }
            } else {
                if let Some(block) = &current_block.block {
                    let start_frame = block.starting_frame_in_file;
                    (Some(block), start_frame)
                } else {
                    // TODO: warn of buffer underflow.
                    (None, self.current_block_starting_frame_in_file)
                }
            }
        };

        // Keep this from growing indefinitely.
        self.current_block_starting_frame_in_file = current_block_start_frame;

        let end_frame_in_block = self.current_frame_in_block + length;

        if end_frame_in_block > BLOCK_SIZE {
            // Data spans between two blocks, so two copies need to be performed.

            // Copy from first block.

            let first_len = BLOCK_SIZE - self.current_frame_in_block;
            let second_len = length - first_len;

            for i in 0..self.read_buffer.len() {
                let read_buffer_part = &mut self.read_buffer[i][0..first_len];

                let from_buffer_part = if let Some(block) = current_block_data {
                    &block.block[i]
                        [self.current_frame_in_block..self.current_frame_in_block + first_len]
                } else {
                    // Output silence.
                    &SILENCE_BUFFER[0..first_len]
                };

                read_buffer_part.copy_from_slice(from_buffer_part);
            }

            self.advance_to_next_block();

            // Copy from next block.

            let (next_block_data, next_block_start_frame) = {
                let next_block = &self.prefetch_buffer[self.current_block_index];

                if let Some(cache_index) = next_block.use_cache {
                    if let Some(cache) = &self.caches[cache_index].cache {
                        let start_frame =
                            cache.blocks[self.current_block_index].starting_frame_in_file;
                        (Some(&cache.blocks[self.current_block_index]), start_frame)
                    } else {
                        // If cache is empty, output silence instead.
                        (None, self.current_block_starting_frame_in_file)
                    }
                } else {
                    if let Some(block) = &next_block.block {
                        let start_frame = block.starting_frame_in_file;
                        (Some(block), start_frame)
                    } else {
                        // TODO: warn of buffer underflow.
                        (None, self.current_block_starting_frame_in_file)
                    }
                }
            };

            // Copy from second block
            for i in 0..self.read_buffer.len() {
                let read_buffer_part = &mut self.read_buffer[i][first_len..first_len + second_len];

                let from_buffer_part = if let Some(block) = next_block_data {
                    &block.block[i][0..second_len]
                } else {
                    // Output silence.
                    &SILENCE_BUFFER[0..first_len]
                };

                read_buffer_part.copy_from_slice(from_buffer_part);
            }

            // Advance.
            self.current_block_starting_frame_in_file = next_block_start_frame;
            self.current_frame_in_block = second_len;
        } else {
            // Only need to copy from current block.
            for i in 0..self.read_buffer.len() {
                // Safe because data blocks will always have the same number of channels.
                let read_buffer_part = &mut self.read_buffer[i][0..length];

                let from_buffer_part = if let Some(block) = current_block_data {
                    &block.block[i]
                        [self.current_frame_in_block..self.current_frame_in_block + length]
                } else {
                    // Output silence.
                    &SILENCE_BUFFER[0..length]
                };

                read_buffer_part.copy_from_slice(from_buffer_part);
            }

            // Advance.
            self.current_frame_in_block = end_frame_in_block;
            if self.current_frame_in_block == BLOCK_SIZE {
                self.advance_to_next_block();

                // Safe because indexes are always constrained to be in-bounds inside `advance_to_next_block()`.
                self.current_block_starting_frame_in_file = if let Some(next_block) =
                    &self.prefetch_buffer[self.current_block_index].block
                {
                    next_block.starting_frame_in_file
                } else {
                    self.current_block_starting_frame_in_file + BLOCK_SIZE
                };
                self.current_frame_in_block = 0;
            }
        }

        ReadData::new(&self.read_buffer, length)
    }

    fn advance_to_next_block(&mut self) {
        // Safe because indexes are always constrained to be in-bounds inside `advance_to_next_block()`.
        let entry = unsafe {
            self.prefetch_buffer
                .get_unchecked_mut(self.current_block_index)
        };

        // Request a new block of data that is one block ahead of the
        // latest block in the prefetch buffer.
        let wanted_start_smp =
            self.current_block_starting_frame_in_file + (NUM_PREFETCH_BLOCKS * BLOCK_SIZE);

        entry.use_cache = None;
        entry.wanted_start_smp = wanted_start_smp;

        self.to_server_tx
            .push(ClientToServerMsg::ReadIntoBlock {
                block_index: self.current_block_index,
                // Send block to be re-used by the IO server.
                block: entry.block.take(),
                starting_frame_in_file: wanted_start_smp,
            })
            .expect("Client to Server channel full");

        self.current_block_index += 1;
        if self.current_block_index >= NUM_PREFETCH_BLOCKS {
            self.current_block_index = 0;
        }

        self.next_block_index += 1;
        if self.next_block_index >= NUM_PREFETCH_BLOCKS {
            self.next_block_index = 0;
        }

        self.current_block_starting_frame_in_file += BLOCK_SIZE;
    }

    pub fn current_file_sample(&self) -> usize {
        self.current_block_starting_frame_in_file + self.current_frame_in_block
    }

    pub fn num_channels(&self) -> usize {
        self.num_channels
    }
}

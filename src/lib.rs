use std::fs::File;
use std::io;
use std::path::PathBuf;
use std::time;

use rtrb::{Consumer, Producer, RingBuffer};

pub const BLOCK_SIZE: usize = 4096;
pub const NUM_PREFETCH_BLOCKS: usize = 4;
pub const MSG_CHANNEL_SIZE: usize = 64;
pub const SERVER_WAIT_TIME: time::Duration = time::Duration::from_millis(1);

static SILENCE_BUFFER: [f32; BLOCK_SIZE] = [0.0; BLOCK_SIZE];

pub struct FileInfo {
    pub params: symphonia::core::codecs::CodecParameters,

    pub num_frames: usize,
    pub num_channels: usize,
    pub sample_rate: u32,
}

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
    DisposeCacheEntries {
        caches: Vec<DataBlockCacheEntry>,
    },
    Close,
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
    to_client_tx: Producer<ServerToClientMsg>,
    from_client_rx: Consumer<ClientToServerMsg>,

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
}

impl ReadServer {
    pub fn new(
        file: PathBuf,
        verify: bool,
        start_frame_in_file: usize,
        to_client_tx: Producer<ServerToClientMsg>,
        from_client_rx: Consumer<ClientToServerMsg>,
    ) -> Result<FileInfo, OpenError> {
        let (mut open_tx, mut open_rx) = RingBuffer::<Result<FileInfo, OpenError>>::new(1).split();

        std::thread::spawn(move || {
            match Self::build(
                file,
                verify,
                start_frame_in_file,
                to_client_tx,
                from_client_rx,
            ) {
                Ok((server, file_info)) => {
                    open_tx.push(Ok(file_info)).unwrap();
                    server.run();
                }
                Err(e) => {
                    open_tx.push(Err(e)).unwrap();
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
            },
            file_info,
        ))
    }

    fn run(mut self) {
        let mut run = true;
        while run {
            while let Ok(msg) = self.from_client_rx.pop() {
                match msg {
                    ClientToServerMsg::ReadIntoBlock {
                        block_index,
                        mut block,
                        starting_frame_in_file,
                    } => {
                        if block.is_none() {
                            // Try using one in the pool if it exists.
                            block = Some(self.block_pool.pop().unwrap_or(
                                // No blocks in pool. Create a new one.
                                DataBlock::new(self.num_channels),
                            ));
                        }

                        let mut block = block.unwrap();
                        block.starting_frame_in_file = starting_frame_in_file;
                        block.requested_frame_in_file = starting_frame_in_file;

                        match self.decode_into_block(&mut block) {
                            Ok(()) => {
                                self.to_client_tx
                                    .push(ServerToClientMsg::ReadIntoBlockRes {
                                        block_index,
                                        block,
                                    })
                                    .expect("Server to client channel full");
                            }
                            Err(e) => {
                                self.to_client_tx
                                    .push(ServerToClientMsg::FatalError(e))
                                    .expect("Server to client channel full");
                                run = false;
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
                            self.to_client_tx
                                .push(ServerToClientMsg::FatalError(e))
                                .expect("Server to client channel full");
                            run = false;
                            break;
                        }
                    }
                    ClientToServerMsg::Cache {
                        cache_index,
                        mut cache,
                        starting_frame_in_file,
                    } => {
                        if cache.is_none() {
                            // Try using one in the pool if it exists.
                            cache = Some(self.cache_pool.pop().unwrap_or(
                                // No caches in pool. Create a new one.
                                DataBlockCache::new(self.num_channels),
                            ));
                        }

                        let mut cache = cache.unwrap();
                        cache.requested_frame_in_file = starting_frame_in_file;

                        let current_frame = self.current_frame;

                        // Seek to the position the client wants to cache.
                        if let Err(e) = self.seek(starting_frame_in_file) {
                            self.to_client_tx
                                .push(ServerToClientMsg::FatalError(e))
                                .expect("Server to client channel full");
                            run = false;
                            break;
                        }

                        // Fill the cache
                        for block in cache.blocks.iter_mut() {
                            if let Err(e) = self.decode_into_block(block) {
                                self.to_client_tx
                                    .push(ServerToClientMsg::FatalError(e))
                                    .expect("Server to client channel full");
                                run = false;
                                break;
                            }
                        }

                        // Seek back to the previous position.
                        if let Err(e) = self.seek(current_frame) {
                            self.to_client_tx
                                .push(ServerToClientMsg::FatalError(e))
                                .expect("Server to client channel full");
                            run = false;
                            break;
                        }

                        self.to_client_tx
                            .push(ServerToClientMsg::CacheRes { cache_index, cache })
                            .expect("Server to client channel full");
                    }
                    ClientToServerMsg::DisposeCache { cache } => {
                        // Store the cache to be reused.
                        self.cache_pool.push(cache);
                    }
                    ClientToServerMsg::DisposeCacheEntries { caches } => {
                        // The caches will be dropped here.
                    }
                    ClientToServerMsg::Close => {
                        run = false;
                        break;
                    }
                }
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }

        self.decoder.close();
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
    pub fn new<P: Into<PathBuf>>(
        file: P,
        start_frame_in_file: usize,
        max_num_caches: usize,
        decode_verify: bool,
    ) -> Result<ReadClient, OpenError> {
        let (to_server_tx, from_client_rx) =
            RingBuffer::<ClientToServerMsg>::new(MSG_CHANNEL_SIZE).split();
        let (to_client_tx, from_server_rx) =
            RingBuffer::<ServerToClientMsg>::new(MSG_CHANNEL_SIZE).split();

        let file: PathBuf = file.into();

        match ReadServer::new(
            file,
            decode_verify,
            start_frame_in_file,
            to_client_tx,
            from_client_rx,
        ) {
            Ok(file_info) => {
                let client = ReadClient::new(
                    to_server_tx,
                    from_server_rx,
                    start_frame_in_file,
                    max_num_caches,
                    file_info,
                );

                Ok(client)
            }
            Err(e) => Err(e),
        }
    }
}

fn wrap_frame(mut frame: usize, len: usize) -> usize {
    while frame >= len {
        frame -= len;
    }
    frame
}

struct DataBlock {
    pub block: Vec<[f32; BLOCK_SIZE]>,
    pub starting_frame_in_file: usize,
    pub requested_frame_in_file: usize,
}

impl DataBlock {
    pub(crate) fn new(num_channels: usize) -> Self {
        let mut block: Vec<[f32; BLOCK_SIZE]> = Vec::with_capacity(num_channels);
        for _ in 0..num_channels {
            // Safe because block will be always filled before it is sent to be read by the client.
            let data: [f32; BLOCK_SIZE] =
                unsafe { std::mem::MaybeUninit::<[f32; BLOCK_SIZE]>::uninit().assume_init() };
            block.push(data);
        }

        DataBlock {
            block,
            starting_frame_in_file: 0,
            requested_frame_in_file: 0,
        }
    }
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

impl DataBlockCache {
    pub(crate) fn new(num_channels: usize) -> Self {
        // Safe because we initialize this in the next step.
        let mut blocks: [DataBlock; NUM_PREFETCH_BLOCKS] = unsafe {
            std::mem::MaybeUninit::<[DataBlock; NUM_PREFETCH_BLOCKS]>::uninit().assume_init()
        };

        for block in blocks.iter_mut() {
            *block = DataBlock::new(num_channels);
        }

        Self {
            blocks,
            requested_frame_in_file: 0,
        }
    }
}

struct DataBlockCacheEntry {
    pub cache: Option<DataBlockCache>,
    pub wanted_start_smp: usize,
}

pub struct ReadData<'a> {
    data: &'a DataBlock,
    len: usize,
}

impl<'a> ReadData<'a> {
    pub(crate) fn new(data: &'a DataBlock, len: usize) -> Self {
        Self { data, len }
    }

    pub fn read_channel(&self, channel: usize) -> &[f32] {
        &self.data.block[channel][0..self.len]
    }

    pub fn num_channels(&self) -> usize {
        self.data.block.len()
    }

    pub fn buffer_len(&self) -> usize {
        self.len
    }
}

#[derive(Debug)]
pub enum ReadError {
    FatalError(symphonia::core::errors::Error),
    ServerClosed,
}

impl std::error::Error for ReadError {}

impl std::fmt::Display for ReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::FatalError(e) => write!(f, "Fatal error: {:?}", e),
            ReadError::ServerClosed => write!(f, "Server closed unexpectedly"),
        }
    }
}

impl From<symphonia::core::errors::Error> for ReadError {
    fn from(e: symphonia::core::errors::Error) -> Self {
        ReadError::FatalError(e)
    }
}

pub struct ReadClient {
    to_server_tx: Producer<ClientToServerMsg>,
    from_server_rx: Consumer<ServerToClientMsg>,

    read_buffer: Option<DataBlock>,
    prefetch_buffer: [DataBlockEntry; NUM_PREFETCH_BLOCKS],
    caches: Option<Vec<DataBlockCacheEntry>>,

    current_block_index: usize,
    next_block_index: usize,
    current_block_starting_frame_in_file: usize,
    current_frame_in_block: usize,

    file_info: FileInfo,
    error: bool,
}

impl ReadClient {
    pub(crate) fn new(
        to_server_tx: Producer<ClientToServerMsg>,
        from_server_rx: Consumer<ServerToClientMsg>,
        starting_frame_in_file: usize,
        max_num_caches: usize,
        file_info: FileInfo,
    ) -> Self {
        let read_buffer = Some(DataBlock::new(file_info.num_channels));

        let mut caches: Vec<DataBlockCacheEntry> = Vec::with_capacity(max_num_caches);
        for _ in 0..max_num_caches {
            caches.push(DataBlockCacheEntry {
                cache: None,
                wanted_start_smp: 0,
            });
        }

        // Safe because we initialize the values in the next step.
        let mut prefetch_buffer: [DataBlockEntry; NUM_PREFETCH_BLOCKS] = unsafe {
            std::mem::MaybeUninit::<[DataBlockEntry; NUM_PREFETCH_BLOCKS]>::uninit().assume_init()
        };
        let mut wanted_start_smp = starting_frame_in_file;
        for entry in prefetch_buffer.iter_mut() {
            *entry = DataBlockEntry {
                use_cache: None,
                block: None,
                wanted_start_smp,
            };

            wanted_start_smp += BLOCK_SIZE;
        }

        Self {
            to_server_tx,
            from_server_rx,

            read_buffer,
            prefetch_buffer,

            caches: Some(caches),

            current_block_index: 0,
            next_block_index: 1,
            current_block_starting_frame_in_file: starting_frame_in_file,
            current_frame_in_block: 0,

            file_info,
            error: false,
        }
    }

    pub fn max_num_caches(&self) -> usize {
        self.caches.as_ref().unwrap().len()
    }

    pub fn cache(
        &mut self,
        cache_index: usize,
        starting_frame_in_file: usize,
    ) -> Result<bool, ReadError> {
        if self.error {
            return Err(ReadError::ServerClosed);
        }

        let caches = self.caches.as_mut().unwrap();

        assert!(cache_index < caches.len());

        let mut do_cache = false;
        if let Some(cache) = &caches[cache_index].cache {
            if cache.requested_frame_in_file != starting_frame_in_file {
                do_cache = true;
            }
        } else {
            do_cache = true;
        }

        if do_cache {
            caches[cache_index].wanted_start_smp = starting_frame_in_file;
            let cache = caches[cache_index].cache.take();

            self.to_server_tx
                .push(ClientToServerMsg::Cache {
                    cache_index,
                    cache,
                    starting_frame_in_file,
                })
                .expect("Client to Server channel full");

            return Ok(false);
        }

        Ok(true)
    }

    pub fn seek_to_cache(
        &mut self,
        cache_index: usize,
        starting_frame_in_file: usize,
    ) -> Result<bool, ReadError> {
        if self.error {
            return Err(ReadError::ServerClosed);
        }

        let cache_exists = self.cache(cache_index, starting_frame_in_file)?;

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

        Ok(cache_exists)
    }

    /// Returns true if there is data to be read, false otherwise.
    ///
    /// Note the `read()` method can still be called if this returns false,
    /// it will just output silence instead.
    pub fn is_ready(&mut self) -> Result<bool, ReadError> {
        self.poll()?;

        // Check if the next two blocks are ready.

        if let Some(cache_index) = self.prefetch_buffer[self.current_block_index].use_cache {
            if self.caches.as_ref().unwrap()[cache_index].cache.is_none() {
                // Cache has not been recieved yet.
                return Ok(false);
            }
        } else if self.prefetch_buffer[self.current_block_index]
            .block
            .is_none()
        {
            // Block has not been recieved yet.
            return Ok(false);
        }

        if let Some(cache_index) = self.prefetch_buffer[self.next_block_index].use_cache {
            if self.caches.as_ref().unwrap()[cache_index].cache.is_none() {
                // Cache has not been recieved yet.
                return Ok(false);
            }
        } else if self.prefetch_buffer[self.next_block_index].block.is_none() {
            // Block has not been recieved yet.
            return Ok(false);
        }

        Ok(true)
    }

    pub fn block_until_ready(&mut self) -> Result<(), ReadError> {
        loop {
            if self.is_ready()? {
                break;
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }

        Ok(())
    }

    fn poll(&mut self) -> Result<(), ReadError> {
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
                        // Tell the server to deallocate the block.
                        self.to_server_tx
                            .push(ClientToServerMsg::DisposeBlock { block })
                            .expect("Client to Server channel full");
                    }
                }
                ServerToClientMsg::CacheRes { cache_index, cache } => {
                    let cache_entry = &mut self.caches.as_mut().unwrap()[cache_index];

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
                        // Tell the server to deallocate the cache.
                        self.to_server_tx
                            .push(ClientToServerMsg::DisposeCache { cache })
                            .expect("Client to Server channel full");
                    }
                }
                ServerToClientMsg::FatalError(e) => {
                    self.error = true;
                    return Err(e.into());
                }
            }
        }

        Ok(())
    }

    /// Read the next slice of data with length `length`.
    ///
    /// ## Panics
    /// This will panic if `length` > `BLOCK_SIZE` (4096).
    pub fn read(&mut self, length: usize) -> Result<ReadData, ReadError> {
        assert!(length <= BLOCK_SIZE);

        if self.error {
            return Err(ReadError::ServerClosed);
        }

        self.poll()?;

        // Get the first block of data.
        let (current_block_data, current_block_start_frame) = {
            let current_block = &self.prefetch_buffer[self.current_block_index];

            if let Some(cache_index) = current_block.use_cache {
                if let Some(cache) = &self.caches.as_ref().unwrap()[cache_index].cache {
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
            {
                let read_buffer = self.read_buffer.as_mut().unwrap();

                for i in 0..read_buffer.block.len() {
                    let read_buffer_part = &mut read_buffer.block[i][0..first_len];

                    let from_buffer_part = if let Some(block) = current_block_data {
                        &block.block[i]
                            [self.current_frame_in_block..self.current_frame_in_block + first_len]
                    } else {
                        // Output silence.
                        &SILENCE_BUFFER[0..first_len]
                    };

                    read_buffer_part.copy_from_slice(from_buffer_part);
                }
            }

            self.advance_to_next_block();

            // Get the next block of data.
            let (next_block_data, next_block_start_frame) = {
                let next_block = &self.prefetch_buffer[self.current_block_index];

                if let Some(cache_index) = next_block.use_cache {
                    if let Some(cache) = &self.caches.as_ref().unwrap()[cache_index].cache {
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
            {
                let read_buffer = self.read_buffer.as_mut().unwrap();

                for i in 0..read_buffer.block.len() {
                    let read_buffer_part =
                        &mut read_buffer.block[i][first_len..first_len + second_len];

                    let from_buffer_part = if let Some(block) = next_block_data {
                        &block.block[i][0..second_len]
                    } else {
                        // Output silence.
                        &SILENCE_BUFFER[0..second_len]
                    };

                    read_buffer_part.copy_from_slice(from_buffer_part);
                }
            }

            // Advance.
            self.current_block_starting_frame_in_file = next_block_start_frame;
            self.current_frame_in_block = second_len;
        } else {
            // Only need to copy from current block.
            {
                let read_buffer = self.read_buffer.as_mut().unwrap();

                for i in 0..read_buffer.block.len() {
                    let read_buffer_part = &mut read_buffer.block[i][0..length];

                    let from_buffer_part = if let Some(block) = current_block_data {
                        &block.block[i]
                            [self.current_frame_in_block..self.current_frame_in_block + length]
                    } else {
                        // Output silence.
                        &SILENCE_BUFFER[0..length]
                    };

                    read_buffer_part.copy_from_slice(from_buffer_part);
                }
            }

            // Advance.
            self.current_frame_in_block = end_frame_in_block;
            if self.current_frame_in_block == BLOCK_SIZE {
                self.advance_to_next_block();

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

        Ok(ReadData::new(self.read_buffer.as_ref().unwrap(), length))
    }

    fn advance_to_next_block(&mut self) {
        let entry = &mut self.prefetch_buffer[self.current_block_index];

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

    pub fn info(&self) -> &FileInfo {
        &self.file_info
    }
}

impl Drop for ReadClient {
    fn drop(&mut self) {
        // Tell the server to deallocate any heap data.

        for entry in self.prefetch_buffer.iter_mut() {
            if let Some(block) = entry.block.take() {
                self.to_server_tx
                    .push(ClientToServerMsg::DisposeBlock { block })
                    .expect("Client to Server channel full");
            }
        }

        if let Some(block) = self.read_buffer.take() {
            self.to_server_tx
                .push(ClientToServerMsg::DisposeBlock { block })
                .expect("Client to Server channel full");
        }

        if let Some(caches) = self.caches.take() {
            self.to_server_tx
                .push(ClientToServerMsg::DisposeCacheEntries { caches })
                .expect("Client to Server channel full");
        }

        self.to_server_tx
            .push(ClientToServerMsg::Close)
            .expect("Client to Server channel full");
    }
}

# rt-audio-disk-stream
Realtime disk streaming IO for audio

This crate is currently experimental and incomplete.


# How it Works

<div><img src="how_it_works.svg", alt="how it works"></div>

The stream has two types of buffers: a `cache` buffer and `look-ahead` buffer.

A `cache` buffer is a user-defined range (of a fixed user-defined number of blocks) starting from any frame in the file. Caches must be loaded before they can be used. The default size for a block is 16384 frames in length.

There are a number of `look-ahead` blocks ahead of the currently used cache block/playhead. These automatically load-in to ensure that data will always be ready even in the worse-case IO latency scenerio. A number of `look-ahead` blocks are added to the end of every `cache` buffer to ensure enough data is always available.

The stream can have as many `cache` buffers as desired. When seeking to a frame in the file, the stream searches for a cache that contains that frame. If one does, then it uses it and playback can resume immediately. A common use case is to cache the start of a file or loop region for seamless looping.

If a suitable cache is not found (or the cache is not loaded yet), then the `look-ahead` buffer will need to fill-up before any more data can be read. In this case, you may choose to either continue playback (which will output silence) or to temporarily pause playback.

# Format and Codec Support Roadmap

Default decoding of files is provided by the [`Symphonia`] crate. (So far only wav is verified to work).

In addition to the default decoders, you may define your own using the `Decoder` trait.

### Formats (Demux)

| Format   | Status                       | Default |
|----------|------------------------------|---------|
| ISO/MP4  | :x:                          | No      |
| MKV/WebM | :x:                          | Yes     |
| OGG      | :x:                          | Yes     |
| Wav      | :heavy_check_mark: Compliant | Yes     |

### Codecs (Decode)

| Codec                        | Status                       | Default |
|------------------------------|------------------------------|---------|
| AAC-LC                       | :x:                          | No      |
| HE-AAC (AAC+, aacPlus)       | :x:                          | No      |
| HE-AACv2 (eAAC+, aacPlus v2) | :x:                          | No      |
| FLAC                         | :x:                          | Yes     |
| MP1                          | :x:                          | No      |
| MP2                          | :x:                          | No      |
| MP3                          | :x:                          | No      |
| Opus                         | :x:                          | Yes     |
| PCM                          | :x:                          | Yes     |
| Vorbis                       | :x:                          | Yes     |
| WavPack                      | :x:                          | Yes     |
| Wav                          | :heavy_check_mark: Compliant | Yes     |

### Codecs (Encode)
| Codec                        | Status                       | Default |
|------------------------------|------------------------------|---------|
| FLAC                         | :x:                          | Yes     |
| MP3                          | :x:                          | No      |
| Opus                         | :x:                          | Yes     |
| PCM                          | :x:                          | Yes     |
| Vorbis                       | :x:                          | Yes     |
| Wav                          | :x:                          | Yes     |

# Examples

## Simple Usage Example
```rust
use rt_audio_disk_stream::{Decoder, SymphoniaDecoder, SeekMode};

// Open the read stream.
let mut read_disk_stream = rt_audio_disk_stream::open_read::<SymphoniaDecoder, _>(
    "./test_files/wav_i24_stereo.wav",  // Path to file.
    0,  // The frame in the file to start reading from.
    Default::default(),  // Use default read stream options.
).unwrap();

// Cache the start of the file into cache with index `0`.
let _ = read_disk_stream.cache(0, 0);

// Tell the stream to seek to the beginning of file. This will also alert the stream to the existence
// of the cache with index `0`.
read_disk_stream.seek(0, Default::default()).unwrap();

// Wait until the buffer is filled before sending it to the process thread.
//
// NOTE: Do ***not*** use this method in a real-time thread.
read_disk_stream.block_until_ready().unwrap();

// (Send `read_stream` to the audio processing thread)


// -------------------------------------------------------------

// In the realtime audio processing thread:


// Update client and check if it is ready.
//
// NOTE: You should avoid using `unwrap()` in realtime code.
if !read_disk_stream.is_ready().unwrap() {
    // If the look-ahead buffer is still buffering, We can choose to either continue
    // reading (which will return silence), or pause playback until the buffer is filled.
}

let read_data = read_disk_stream.read(num_frames_in_output_buffer).unwrap();

println!("{}", read_data.num_frames());
println!("{}", read_data.num_channels());

// Seek to a new position in the file.
read_disk_stream.seek(50000, SeekMode::Auto};

assert_eq!(read_dist_stream.playhead(), 50000);
```

## Demo Audio Player
Here is a basic [`looping demo player`] that plays a single wav file with adjustable loop regions.

[`Symphonia`]: https://github.com/pdeljanov/Symphonia
[`looping demo player`]: https://github.com/RustyDAW/rt-audio-disk-stream/tree/main/examples/demo_player

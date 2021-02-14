# rt-audio-disk-stream
Realtime disk streaming IO for audio

This crate is currently experimental and incomplete.

# Format and Codec Support Roadmap

Default decoding of files is provided by the [`Symphonia`] crate. (So far only wav is verified to work).

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

[`Symphonia`]: https://github.com/pdeljanov/Symphonia

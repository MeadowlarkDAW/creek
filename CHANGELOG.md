# Version History

## Version 1.2.0 (2023-12-28)

### Breaking changes:

- The trait methods `Decoder::decode()` and `Encoder::encode()` are no longer marked unsafe. Refer to the new documentation on how these trait methods should be implemented.

### Other changes:

- Removed all unsafe code
- Bumped `rtrb` to version "0.3.0"
- Updated demos to use latest version of `egui`

## Version 1.1.0 (2023-07-11)

- Added the ability to decode MP4/AAC/ALAC files
- Changed `println` statements to actual `log` entries in `creek-decode-symphonia`
- Updated Minimum Supported Rust Version (MSRV) from 1.56 to 1.62
- Updated to Rust edition 2021

## Version 1.0 (2023-06-08)

- Added metadata to `SymphoniaDecoder`

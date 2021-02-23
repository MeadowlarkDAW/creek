use crate::*;
use std::{f64, path::PathBuf};

#[test]
fn encode_wav() {
    // Generate a fading sine wave for output.

    let hz: f64 = 44100.0;
    let freq: f64 = 261.626; // midde C
    let max_amp: f64 = 0.9;
    let mut sine_f64: Vec<f64> = Vec::with_capacity(32768);
    for n in 0..32768 {
        let amp = (n as f64 / 32768.0) * max_amp;
        let val = (f64::consts::TAU * n as f64 * freq / hz).sin();
        sine_f64.push(amp * val);
    }
    let mut sine_f64_r = sine_f64.clone();
    sine_f64_r.reverse();

    // Convert to u8
    let mut sine_u8_l: Vec<u8> = Vec::with_capacity(32768);
    for i in 0..32768 {
        let val_f64 = sine_f64[i];
        let val_u8 = (((val_f64 + 1.0) / 2.0) * std::u8::MAX as f64).round() as u8;
        sine_u8_l.push(val_u8)
    }
    let mut sine_u8_r = sine_u8_l.clone();
    sine_u8_r.reverse();

    // Convert to i16
    let mut sine_i16_l: Vec<i16> = Vec::with_capacity(32768);
    for i in 0..32768 {
        let val_f64 = sine_f64[i];
        let val_i16 = (val_f64 * std::i16::MAX as f64).round() as i16;
        sine_i16_l.push(val_i16)
    }
    let mut sine_i16_r = sine_i16_l.clone();
    sine_i16_r.reverse();

    // Convert to i24
    let mut sine_i24_l: Vec<i32> = Vec::with_capacity(32768);
    for i in 0..32768 {
        let val_f64 = sine_f64[i];
        let val_i24 = (val_f64 * 0x7FFFFF as f64).round() as i32;
        sine_i24_l.push(val_i24)
    }
    let mut sine_i24_r = sine_i24_l.clone();
    sine_i24_r.reverse();

    // Convert to f32
    let mut sine_f32_l: Vec<f32> = Vec::with_capacity(32768);
    for i in 0..32768 {
        sine_f32_l.push(sine_f64[i] as f32);
    }
    let mut sine_f32_r = sine_f32_l.clone();
    sine_f32_r.reverse();

    // u8 mono

    let path: PathBuf = "./test_files/wav_u8_out.wav".into();

    let mut block = WriteBlock::<u8>::new(1, 32768);
    block.block[0].copy_from_slice(&sine_u8_l);
    block.written_frames = 32768;

    let (mut encoder, _info) = WavEncoder::<Uint8>::new(path, 1, 44100.0, 32768, 8, ()).unwrap();

    unsafe {
        encoder.encode(&block).unwrap();
    }

    encoder.finish_file().unwrap();

    // u8 stereo

    let path: PathBuf = "./test_files/wav_u8_out_stereo.wav".into();

    let mut block = WriteBlock::<u8>::new(2, 32768);
    block.block[0].copy_from_slice(&sine_u8_l);
    block.block[1].copy_from_slice(&sine_u8_r);
    block.written_frames = 32768;

    let (mut encoder, _info) = WavEncoder::<Uint8>::new(path, 2, 44100.0, 32768, 8, ()).unwrap();

    unsafe {
        encoder.encode(&block).unwrap();
    }

    encoder.finish_file().unwrap();

    // i16 mono

    let path: PathBuf = "./test_files/wav_i16_out.wav".into();

    let mut block = WriteBlock::<i16>::new(1, 32768);
    block.block[0].copy_from_slice(&sine_i16_l);
    block.written_frames = 32768;

    let (mut encoder, _info) = WavEncoder::<Int16>::new(path, 1, 44100.0, 32768, 8, ()).unwrap();

    unsafe {
        encoder.encode(&block).unwrap();
    }

    encoder.finish_file().unwrap();

    // i16 stereo

    let path: PathBuf = "./test_files/wav_i16_out_stereo.wav".into();

    let mut block = WriteBlock::<i16>::new(2, 32768);
    block.block[0].copy_from_slice(&sine_i16_l);
    block.block[1].copy_from_slice(&sine_i16_r);
    block.written_frames = 32768;

    let (mut encoder, _info) = WavEncoder::<Int16>::new(path, 2, 44100.0, 32768, 8, ()).unwrap();

    unsafe {
        encoder.encode(&block).unwrap();
    }

    encoder.finish_file().unwrap();

    // i24 mono

    let path: PathBuf = "./test_files/wav_i24_out.wav".into();

    let mut block = WriteBlock::<i32>::new(1, 32768);
    block.block[0].copy_from_slice(&sine_i24_l);
    block.written_frames = 32768;

    let (mut encoder, _info) = WavEncoder::<Int24>::new(path, 1, 44100.0, 32768, 8, ()).unwrap();

    unsafe {
        encoder.encode(&block).unwrap();
    }

    encoder.finish_file().unwrap();

    // i24 stereo

    let path: PathBuf = "./test_files/wav_i24_out_stereo.wav".into();

    let mut block = WriteBlock::<i32>::new(2, 32768);
    block.block[0].copy_from_slice(&sine_i24_l);
    block.block[1].copy_from_slice(&sine_i24_r);
    block.written_frames = 32768;

    let (mut encoder, _info) = WavEncoder::<Int24>::new(path, 2, 44100.0, 32768, 8, ()).unwrap();

    unsafe {
        encoder.encode(&block).unwrap();
    }

    encoder.finish_file().unwrap();

    // f32 mono

    let path: PathBuf = "./test_files/wav_f32_out.wav".into();

    let mut block = WriteBlock::<f32>::new(1, 32768);
    block.block[0].copy_from_slice(&sine_f32_l);
    block.written_frames = 32768;

    let (mut encoder, _info) = WavEncoder::<Float32>::new(path, 1, 44100.0, 32768, 8, ()).unwrap();

    unsafe {
        encoder.encode(&block).unwrap();
    }

    encoder.finish_file().unwrap();

    // f32 stereo

    let path: PathBuf = "./test_files/wav_f32_out_stereo.wav".into();

    let mut block = WriteBlock::<f32>::new(2, 32768);
    block.block[0].copy_from_slice(&sine_f32_l);
    block.block[1].copy_from_slice(&sine_f32_r);
    block.written_frames = 32768;

    let (mut encoder, _info) = WavEncoder::<Float32>::new(path, 2, 44100.0, 32768, 8, ()).unwrap();

    unsafe {
        encoder.encode(&block).unwrap();
    }

    encoder.finish_file().unwrap();

    // f64 mono

    let path: PathBuf = "./test_files/wav_f64_out.wav".into();

    let mut block = WriteBlock::<f64>::new(1, 32768);
    block.block[0].copy_from_slice(&sine_f64);
    block.written_frames = 32768;

    let (mut encoder, _info) = WavEncoder::<Float64>::new(path, 1, 44100.0, 32768, 8, ()).unwrap();

    unsafe {
        encoder.encode(&block).unwrap();
    }

    encoder.finish_file().unwrap();

    // f64 stereo

    let path: PathBuf = "./test_files/wav_f64_out_stereo.wav".into();

    let mut block = WriteBlock::<f64>::new(2, 32768);
    block.block[0].copy_from_slice(&sine_f64);
    block.block[1].copy_from_slice(&sine_f64_r);
    block.written_frames = 32768;

    let (mut encoder, _info) = WavEncoder::<Float64>::new(path, 2, 44100.0, 32768, 8, ()).unwrap();

    unsafe {
        encoder.encode(&block).unwrap();
    }

    encoder.finish_file().unwrap();
}

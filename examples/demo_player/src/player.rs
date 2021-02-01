use cpal::SampleFormat;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleRate, StreamConfig,
};
use rt_audio_disk_stream::ReadClient;
use rtrb::{Consumer, Producer};
use std::{fs::File, path::Path, time::Duration};

use crate::{GuiToPlayerMsg, PlayerToGuiMsg};

static SLEEP_DURATION: Duration = Duration::from_millis(1);

pub struct Player {
    read_client: Option<ReadClient>,

    to_gui_tx: Producer<PlayerToGuiMsg>,
    from_gui_rx: Consumer<GuiToPlayerMsg>,

    output_config: StreamConfig,
}

impl Player {
    pub fn new(
        to_gui_tx: Producer<PlayerToGuiMsg>,
        from_gui_rx: Consumer<GuiToPlayerMsg>,
    ) -> cpal::Stream {
        // Setup cpal audio output

        let host = cpal::default_host();

        let device = host
            .default_output_device()
            .expect("no output device available");

        let mut supported_configs_range = device
            .supported_output_configs()
            .expect("error while querying configs");
        let supported_config = supported_configs_range
            .next()
            .expect("no supported config?!")
            .with_sample_rate(SampleRate(44100));

        let err_fn = |err| eprintln!("an error occurred on the output audio stream: {}", err);

        let sample_format = supported_config.sample_format();
        let config: StreamConfig = supported_config.into();

        dbg!(config.channels);

        let mut _player = Self {
            read_client: None,
            to_gui_tx,
            from_gui_rx,
            output_config: config.clone(),
        };

        let buffer_size = if let cpal::BufferSize::Fixed(s) = config.buffer_size {
            s as usize
        } else {
            // Use a large buffer size just in case.
            48000
        };

        let stream = match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    process(data, &mut _player);
                },
                err_fn,
            ),
            SampleFormat::I16 => {
                // Use intermittent f32 buffer.
                let mut buffer: Vec<f32> = Vec::with_capacity(buffer_size);
                buffer.resize(buffer_size, 0.0);

                device.build_output_stream(
                    &config,
                    move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                        process_i16(data, &mut _player, buffer.as_mut_slice());
                    },
                    err_fn,
                )
            }
            SampleFormat::U16 => {
                // Use intermittent f32 buffer.
                let mut buffer: Vec<f32> = Vec::with_capacity(buffer_size);
                buffer.resize(buffer_size, 0.0);

                device.build_output_stream(
                    &config,
                    move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                        process_u16(data, &mut _player, buffer.as_mut_slice());
                    },
                    err_fn,
                )
            }
        }
        .unwrap();

        stream.play().unwrap();

        stream
    }

    fn update(&mut self) {
        while let Ok(msg) = self.from_gui_rx.pop() {
            match msg {
                GuiToPlayerMsg::PlayStream(read_client) => {
                    self.read_client = Some(read_client);
                }
            }
        }
    }
}

fn process(mut data: &mut [f32], player: &mut Player) {
    player.update();

    if let Some(read_client) = &mut player.read_client {
        while data.len() > 0 {
            let read_frames = (data.len() / player.output_config.channels as usize)
                .min(rt_audio_disk_stream::BLOCK_SIZE);

            match read_client.read(read_frames) {
                Ok(read_data) => {
                    // Only support stereo output for this demo.
                    if player.output_config.channels == 2 {
                        if read_data.num_channels() == 1 {
                            let ch = read_data.read_channel(0);

                            for i in 0..read_frames {
                                data[i * 2] = ch[i];
                                data[(i * 2) + 1] = ch[i];
                            }
                        } else if read_data.num_channels() == 2 {
                            let ch1 = read_data.read_channel(0);
                            let ch2 = read_data.read_channel(1);

                            for i in 0..read_frames {
                                data[i * 2] = ch1[i];
                                data[(i * 2) + 1] = ch2[i];
                            }
                        }
                    } else {
                    }
                }
                Err(e) => {
                    // Stop playback on error.
                    println!("{:?}", e);
                    break;
                }
            }

            data = &mut data[read_frames..];
        }
    } else {
        // Output silence until file is recieved.
        for sample in data.iter_mut() {
            *sample = 0.0;
        }
    }
}

fn process_i16(data: &mut [i16], player: &mut Player, buf: &mut [f32]) {
    let len = data.len().min(buf.len());

    process(&mut buf[0..len], player);

    for i in 0..len {
        data[i] = (buf[i] * std::i16::MAX as f32) as i16;
    }
}

fn process_u16(data: &mut [u16], player: &mut Player, buf: &mut [f32]) {
    let len = data.len().min(buf.len());

    process(&mut buf[0..len], player);

    for i in 0..len {
        data[i] = (((buf[i] + 1.0) / 2.0) * std::u16::MAX as f32) as u16;
    }
}

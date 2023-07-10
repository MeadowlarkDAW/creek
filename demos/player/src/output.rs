use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Consumer, Producer};

use crate::process::Process;
use crate::{GuiToProcessMsg, ProcessToGuiMsg};

pub struct Output;

impl Output {
    #[allow(clippy::new_ret_no_self)] // TODO: Rename?
    pub fn new(
        to_gui_tx: Producer<ProcessToGuiMsg>,
        from_gui_rx: Consumer<GuiToProcessMsg>,
    ) -> cpal::Stream {
        // Setup cpal audio output

        let host = cpal::default_host();

        let device = host
            .default_output_device()
            .expect("no output device available");

        let sample_rate = device.default_output_config().unwrap().sample_rate();

        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let mut process = Process::new(to_gui_tx, from_gui_rx);

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| process.process(data),
                move |err| {
                    eprintln!("{}", err);
                },
            )
            .unwrap();

        stream.play().unwrap();

        stream
    }
}

use std::path::PathBuf;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::{FileInfo, SERVER_WAIT_TIME};

use super::{ClientToServerMsg, Encoder, HeapData, ServerToClientMsg, WriteStatus};

pub(crate) struct WriteServerOptions<E: Encoder> {
    pub file: PathBuf,
    pub num_write_blocks: usize,
    pub block_size: usize,
    pub num_channels: u16,
    pub sample_rate: u32,
    pub additional_opts: E::AdditionalOpts,
}

pub(crate) struct WriteServer<E: Encoder> {
    to_client_tx: Producer<ServerToClientMsg<E>>,
    from_client_rx: Consumer<ClientToServerMsg<E>>,
    close_signal_rx: Consumer<Option<HeapData<E::T>>>,

    encoder: E,

    restart_count: usize,
    file_finished: bool,
    fatal_error: bool,

    run: bool,
    client_closed: bool,
}

impl<E: Encoder> WriteServer<E> {
    pub(crate) fn spawn(
        opts: WriteServerOptions<E>,
        to_client_tx: Producer<ServerToClientMsg<E>>,
        from_client_rx: Consumer<ClientToServerMsg<E>>,
        close_signal_rx: Consumer<Option<HeapData<E::T>>>,
    ) -> Result<FileInfo<E::FileParams>, E::OpenError> {
        let (mut open_tx, mut open_rx) =
            RingBuffer::<Result<FileInfo<E::FileParams>, E::OpenError>>::new(1);

        std::thread::spawn(move || {
            match E::new(
                opts.file,
                opts.num_channels,
                opts.sample_rate,
                opts.block_size,
                opts.num_write_blocks,
                opts.additional_opts,
            ) {
                Ok((encoder, file_info)) => {
                    // Push cannot fail because only one message is ever sent.
                    let _ = open_tx.push(Ok(file_info));

                    WriteServer::run(Self {
                        to_client_tx,
                        from_client_rx,
                        close_signal_rx,
                        encoder,
                        restart_count: 0,
                        file_finished: false,
                        fatal_error: false,
                        run: true,
                        client_closed: false,
                    });
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

    fn run(mut self) {
        while self.run {
            let mut do_sleep = true;

            while let Ok(msg) = self.from_client_rx.pop() {
                match msg {
                    ClientToServerMsg::WriteBlock { mut block } => {
                        // Don't use this block if it is from a previous discarded stream.
                        if block.restart_count != self.restart_count {
                            // Clear and send block to be re-used by client.
                            block.clear();
                            self.send_msg(ServerToClientMsg::NewWriteBlock { block });
                        } else {
                            let write_res = self.encoder.encode(&block);

                            match write_res {
                                Ok(status) => {
                                    if let WriteStatus::ReachedMaxSize { num_files } = status {
                                        self.send_msg(ServerToClientMsg::ReachedMaxSize {
                                            num_files,
                                        });
                                    }

                                    // Clear and send block to be re-used by client.
                                    block.clear();
                                    self.send_msg(ServerToClientMsg::NewWriteBlock { block });
                                }
                                Err(e) => {
                                    self.send_msg(ServerToClientMsg::FatalError(e));
                                    self.fatal_error = true;
                                    self.run = false;
                                    do_sleep = false;
                                    break;
                                }
                            }
                        }
                    }
                    ClientToServerMsg::FinishFile => match self.encoder.finish_file() {
                        Ok(()) => {
                            self.send_msg(ServerToClientMsg::Finished);
                            self.file_finished = true;
                            self.run = false;
                            do_sleep = false;
                            break;
                        }
                        Err(e) => {
                            self.send_msg(ServerToClientMsg::FatalError(e));
                            self.file_finished = true;
                            self.fatal_error = true;
                            self.run = false;
                            do_sleep = false;
                            break;
                        }
                    },
                    ClientToServerMsg::DiscardFile => match self.encoder.discard_file() {
                        Ok(()) => {
                            self.send_msg(ServerToClientMsg::Finished);
                            self.file_finished = true;
                            self.run = false;
                            do_sleep = false;
                            break;
                        }
                        Err(e) => {
                            self.send_msg(ServerToClientMsg::FatalError(e));
                            self.file_finished = true;
                            self.fatal_error = true;
                            self.run = false;
                            do_sleep = false;
                            break;
                        }
                    },
                    ClientToServerMsg::DiscardAndRestart => {
                        self.restart_count += 1;

                        match self.encoder.discard_and_restart() {
                            Ok(()) => {}
                            Err(e) => {
                                self.send_msg(ServerToClientMsg::FatalError(e));
                                self.fatal_error = true;
                                self.run = false;
                                do_sleep = false;
                                break;
                            }
                        }
                    }
                }
            }

            // Check for close signal.
            if let Ok(heap_data) = self.close_signal_rx.pop() {
                // Drop heap data here.
                let _ = heap_data;
                self.run = false;
                self.client_closed = true;
                break;
            }

            if do_sleep {
                std::thread::sleep(SERVER_WAIT_TIME);
            }
        }

        // Attempt to finish the file if it was not already.
        if !self.file_finished && !self.fatal_error {
            let _ = self.encoder.finish_file();
        }

        // If client has not closed yet, wait until it does before closing.
        if !self.client_closed {
            loop {
                if let Ok(heap_data) = self.close_signal_rx.pop() {
                    // Drop heap data here.
                    let _ = heap_data;
                    break;
                }

                std::thread::sleep(SERVER_WAIT_TIME);
            }
        }
    }

    fn send_msg(&mut self, msg: ServerToClientMsg<E>) {
        // Do nothing if stream has been closed.
        if !self.run {
            return;
        }

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
                self.client_closed = true;
                break;
            }

            std::thread::sleep(SERVER_WAIT_TIME);
        }

        // Push will never fail because we made sure a slot is available in the
        // previous step (or the stream has closed, in which case an error doesn't
        // matter).
        let _ = self.to_client_tx.push(msg);
    }
}

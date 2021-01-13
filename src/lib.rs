mod path;

pub use path::{PathBuffer, PathError};

pub enum AcessMode {
    ReadOnly,
    ReadWrite,
}

pub struct DataBlock {}

pub struct FileHandle {}

pub struct OpenFileError {}
pub struct ReadBlockError {}
pub struct AllocateWriteBlockError {}

pub enum ClientToServerMsg {
    OpenFile {
        path: PathBuffer,
        access_mode: AcessMode,
    },
    CloseFile {
        file: FileHandle,
    },
    ReadBlock {
        file: FileHandle,
        position: usize,
    },
    ReleaseReadBlock {
        file: FileHandle,
        data_block: DataBlock,
    },
    AllocateWriteBlock {
        file: FileHandle,
        position: usize,
    },
    CommitModifiedWriteBlock {
        file: FileHandle,
        data_block: DataBlock,
    },
    ReleaseUnmodifiedWriteBlock {
        file: FileHandle,
        data_block: DataBlock,
    }
}

pub enum ServerToClientMsg {
    OpenFileResult(Result<FileHandle, OpenFileError>),
    ReadBlockResult(Result<DataBlock, ReadBlockError>),
    AllocateWriteBlockResult(Result<DataBlock, AllocateWriteBlockError>),
}

pub struct AudioDiskStream;
impl AudioDiskStream {
    pub fn new(channel_capacity: usize) -> (Client, Server) {
        let (to_server_tx, from_client_rx) = rtrb::RingBuffer::new(channel_capacity).split();
        let (to_client_tx, from_server_rx) = rtrb::RingBuffer::new(channel_capacity).split();

        (
            Client::new(to_server_tx, from_server_rx),
            Server::new(to_client_tx, from_client_rx),
        )
    }
}

pub struct Client {
    to_server_tx: rtrb::Producer<ClientToServerMsg>,
    from_server_rx: rtrb::Consumer<ServerToClientMsg>,
}

pub struct Server {
    to_client_tx: rtrb::Producer<ServerToClientMsg>,
    from_client_rx: rtrb::Consumer<ClientToServerMsg>,
}

impl Client {
    pub(crate) fn new(
        to_server_tx: rtrb::Producer<ClientToServerMsg>,
        from_server_rx: rtrb::Consumer<ServerToClientMsg>,
    ) -> Self {
        Self {
            to_server_tx,
            from_server_rx,
        }
    }
}

impl Server {
    pub(crate) fn new(
        to_client_tx: rtrb::Producer<ServerToClientMsg>,
        from_client_rx: rtrb::Consumer<ClientToServerMsg>,
    ) -> Self {
        Self {
            to_client_tx,
            from_client_rx,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}

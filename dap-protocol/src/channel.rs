use std::pin::Pin;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};

use crate::error::ProtocolError;
use crate::message::Message;
use crate::transport::{read_message, write_message};

const DAP_BUFFER_SIZE: usize = 64 * 1024;

/// Buffered reader for inbound DAP messages.
pub struct ReadChannel {
    reader: BufReader<Pin<Box<dyn AsyncRead + Send + Unpin>>>,
}

impl ReadChannel {
    pub fn new(reader: impl AsyncRead + Send + Unpin + 'static) -> Self {
        Self {
            reader: BufReader::with_capacity(DAP_BUFFER_SIZE, Box::pin(reader)),
        }
    }

    pub async fn recv(&mut self) -> Result<Option<Message>, ProtocolError> {
        read_message(&mut self.reader).await
    }
}

/// Buffered writer for outbound DAP messages.
pub struct WriteChannel {
    writer: BufWriter<Pin<Box<dyn AsyncWrite + Send + Unpin>>>,
}

impl WriteChannel {
    pub fn new(writer: impl AsyncWrite + Send + Unpin + 'static) -> Self {
        Self {
            writer: BufWriter::with_capacity(DAP_BUFFER_SIZE, Box::pin(writer)),
        }
    }

    pub async fn send(&mut self, message: &Message) -> Result<(), ProtocolError> {
        write_message(&mut self.writer, message).await
    }

    pub async fn flush(&mut self) -> Result<(), ProtocolError> {
        self.writer.flush().await.map_err(ProtocolError::Io)
    }
}

/// Bidirectional DAP transport (e.g. stdio or adapter process pipes).
pub struct DuplexChannel {
    read: ReadChannel,
    write: WriteChannel,
}

impl DuplexChannel {
    pub fn from_streams(
        reader: impl AsyncRead + Send + Unpin + 'static,
        writer: impl AsyncWrite + Send + Unpin + 'static,
    ) -> Self {
        Self {
            read: ReadChannel::new(reader),
            write: WriteChannel::new(writer),
        }
    }

    pub fn from_stdio() -> Self {
        Self::from_streams(tokio::io::stdin(), tokio::io::stdout())
    }

    pub fn into_channels(self) -> (ReadChannel, WriteChannel) {
        (self.read, self.write)
    }
}

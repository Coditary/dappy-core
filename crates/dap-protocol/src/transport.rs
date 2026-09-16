use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::error::ProtocolError;
use crate::message::Message;

/// Read one DAP message using Content-Length framing.
///
/// Returns `Ok(None)` on a clean end-of-stream before any message bytes.
pub async fn read_message<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<Option<Message>, ProtocolError> {
    let mut content_length: Option<usize> = None;
    let mut line_buf = String::new();
    let mut saw_header = false;

    while {
        line_buf.clear();
        let n = reader.read_line(&mut line_buf).await?;
        if n == 0 {
            if !saw_header {
                return Ok(None);
            }
            return Err(ProtocolError::InvalidHeader("unexpected eof".into()));
        }
        let trimmed = line_buf.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            false
        } else if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            saw_header = true;
            let len = rest.trim().parse::<usize>().map_err(|_| {
                ProtocolError::InvalidHeader(format!("bad Content-Length: {}", trimmed))
            })?;
            content_length = Some(len);
            true
        } else {
            return Err(ProtocolError::InvalidHeader(trimmed.to_owned()));
        }
    } {}

    let len = content_length
        .ok_or_else(|| ProtocolError::InvalidHeader("missing Content-Length".into()))?;
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    let msg: Message = serde_json::from_slice(&body)?;
    Ok(Some(msg))
}

/// Write one DAP message with Content-Length framing.
pub async fn write_message<W: AsyncWrite + Unpin>(
    writer: &mut W,
    message: &Message,
) -> Result<(), ProtocolError> {
    let body = serde_json::to_vec(message)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

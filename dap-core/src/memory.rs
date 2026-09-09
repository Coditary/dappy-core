use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::Value;
use std::fmt::Write as _;

pub const MAX_READ_BYTES: i64 = 1 << 20;
pub const MAX_WRITE_BYTES: usize = 1 << 20;
pub const DEFAULT_READ_COUNT: i64 = 256;

/// Validate a readMemory byte count.
pub fn validate_read_count(count: i64) -> Result<i64> {
    if count <= 0 {
        bail!("count must be > 0, got {count}");
    }
    if count > MAX_READ_BYTES {
        bail!("count {count} exceeds maximum of {MAX_READ_BYTES} bytes");
    }
    Ok(count)
}

/// Parse a user-provided hex string into raw bytes for writeMemory.
pub fn hex_string_to_bytes(data: &str) -> Result<Vec<u8>> {
    let data = data.trim();
    if data.is_empty() {
        bail!("data must not be empty");
    }
    if !data.len().is_multiple_of(2) {
        bail!("hex data must have an even number of characters");
    }
    let mut out = Vec::with_capacity(data.len() / 2);
    for idx in (0..data.len()).step_by(2) {
        let pair = &data[idx..idx + 2];
        let byte = u8::from_str_radix(pair, 16)
            .with_context(|| format!("invalid hex byte '{pair}'"))?;
        out.push(byte);
    }
    if out.len() > MAX_WRITE_BYTES {
        bail!(
            "write payload {} bytes exceeds maximum of {} bytes",
            out.len(),
            MAX_WRITE_BYTES
        );
    }
    Ok(out)
}

/// Format a readMemory response body as a hex dump with ASCII sidebar.
pub fn format_memory_read(body: &Value) -> Result<String> {
    let address = body
        .get("address")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let unreadable_bytes = body.get("unreadableBytes").and_then(Value::as_i64);

    let data = match body.get("data").and_then(Value::as_str) {
        Some(b64) => base64::engine::general_purpose::STANDARD
            .decode(b64)
            .with_context(|| format!("decode memory data at {address}"))?,
        None => {
            return Ok(match unreadable_bytes {
                Some(n) => format!("Address: {}\n{n} byte(s) unreadable.", address),
                None => format!("Address: {}\nNo data returned.", address),
            });
        }
    };

    let base_addr = parse_address(address);
    let mut output = format!("Memory at {} ({} bytes):\n", address, data.len());
    if let Some(n) = unreadable_bytes {
        let _ = writeln!(output, "({n} byte(s) unreadable)");
    }

    for (i, chunk) in data.chunks(16).enumerate() {
        match base_addr {
            Some(base) => {
                let addr = base.wrapping_add((i * 16) as u64);
                let _ = write!(output, "0x{:016X}: ", addr);
            }
            None => {
                let _ = write!(output, "+0x{:08X}:        ", i * 16);
            }
        }

        for (j, byte) in chunk.iter().enumerate() {
            if j == 8 {
                output.push(' ');
            }
            let _ = write!(output, "{:02X} ", byte);
        }
        for j in chunk.len()..16 {
            if j == 8 {
                output.push(' ');
            }
            output.push_str("   ");
        }

        output.push(' ');
        for byte in chunk {
            output.push(if byte.is_ascii_graphic() || *byte == b' ' {
                char::from(*byte)
            } else {
                '.'
            });
        }
        output.push('\n');
    }

    Ok(output)
}

pub fn parse_address(s: &str) -> Option<u64> {
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(rest, 16).ok()
    } else {
        s.parse::<u64>().ok()
    }
}

pub fn encode_write_payload(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hex_string_to_bytes_parses_pairs() {
        assert_eq!(hex_string_to_bytes("48656C6C6F").unwrap(), b"Hello");
    }

    #[test]
    fn hex_string_rejects_odd_length() {
        assert!(hex_string_to_bytes("ABC").is_err());
    }

    #[test]
    fn format_memory_read_renders_dump() {
        let body = json!({
            "address": "0x1000",
            "data": base64::engine::general_purpose::STANDARD.encode([0x48, 0x65, 0x6C, 0x6C, 0x6F])
        });
        let text = format_memory_read(&body).unwrap();
        assert!(text.contains("Memory at 0x1000"));
        assert!(text.contains("Hello"));
    }
}

//! Embedded binary resource codec (4-byte big-endian length + zlib + standard Base64).

use crate::MAX_RESOURCE_BYTES;
use base64::Engine as _;
use std::{
    cell::Cell,
    io::{self, Read, Write},
};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EmbeddedError {
    #[error("{0}")]
    Damaged(String),
    #[error("embedded resource exceeds the remaining material resource budget")]
    Budget,
    #[error("embedded resource exceeds the material inflation work budget")]
    WorkBudget,
}

impl From<String> for EmbeddedError {
    fn from(message: String) -> Self {
        Self::Damaged(message)
    }
}

impl From<&str> for EmbeddedError {
    fn from(message: &str) -> Self {
        Self::Damaged(message.into())
    }
}

fn base64_engine() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

pub fn decode(strdata: &str) -> Result<Vec<u8>, String> {
    decode_bounded(strdata, MAX_RESOURCE_BYTES, MAX_RESOURCE_BYTES, &Cell::new(MAX_RESOURCE_BYTES + 1)).map_err(|error| error.to_string())
}

pub fn decode_bounded(strdata: &str, max_bytes: u64, remaining_bytes: u64, inflation_work: &Cell<u64>) -> Result<Vec<u8>, EmbeddedError> {
    let work_before = inflation_work.get();
    if work_before == 0 {
        return Err(EmbeddedError::WorkBudget);
    }

    let raw = match strdata.split_once(char::is_whitespace) {
        Some((prefix, rest)) => base64_engine().decode(std::iter::once(prefix).chain(rest.split_whitespace()).collect::<String>()),
        None => base64_engine().decode(strdata),
    }
    .map_err(|error| format!("bad base64: {error}"))?;
    let (length, compressed) = raw.split_at_checked(4).ok_or("too short")?;
    let declared_bytes = u64::from(u32::from_be_bytes([length[0], length[1], length[2], length[3]]));
    if declared_bytes > max_bytes {
        return Err("embedded resource exceeds the 256 MiB limit".into());
    }

    // Read one probe byte beyond the tighter bound to detect overruns
    let probe_limit = declared_bytes.min(remaining_bytes) + 1;
    let mut decoder = flate2::read::ZlibDecoder::new(compressed);
    let mut limited = decoder.by_ref().take(probe_limit.min(work_before));
    let mut output = Vec::new();
    let result = if declared_bytes > remaining_bytes || declared_bytes >= work_before {
        // Discard probe output that cannot become a retained asset
        io::copy(&mut limited, &mut io::sink())
    } else {
        limited.read_to_end(&mut output).map(|bytes| bytes as u64)
    };

    // Charge every emitted byte, including output produced before a checksum error
    inflation_work.set(work_before.saturating_sub(decoder.total_out()));
    let actual_bytes = result.map_err(|error| format!("bad compressed data: {error}"))?;
    if actual_bytes > declared_bytes {
        return Err(format!("{actual_bytes} bytes where {declared_bytes} were declared").into());
    }

    if work_before < probe_limit && actual_bytes == work_before {
        return Err(EmbeddedError::WorkBudget);
    }

    if actual_bytes > remaining_bytes {
        return Err(EmbeddedError::Budget);
    }

    if actual_bytes != declared_bytes {
        return Err(format!("{actual_bytes} bytes where {declared_bytes} were declared").into());
    }

    output.shrink_to_fit();
    Ok(output)
}

pub fn encode(bytes: &[u8]) -> Result<String, String> {
    let length = u32::try_from(bytes.len()).map_err(|_| String::from("resource too large"))?;
    let mut encoder = flate2::write::ZlibEncoder::new(length.to_be_bytes().to_vec(), flate2::Compression::default());
    encoder.write_all(bytes).map_err(|e| e.to_string())?;
    let raw = encoder.finish().map_err(|e| e.to_string())?;

    Ok(craft_mime::transfer::base64(&raw))
}

#[cfg(test)]
mod tests {
    use super::{EmbeddedError, MAX_RESOURCE_BYTES, decode, decode_bounded, encode};
    use std::cell::Cell;

    #[test]
    fn roundtrips_and_enforces_budgets() {
        let encoded = encode(b"four").expect("encode");
        assert_eq!(decode(&encoded).expect("decode"), b"four");
        assert_eq!(decode_bounded(&encoded, MAX_RESOURCE_BYTES, 3, &Cell::new(16)), Err(EmbeddedError::Budget));
        assert_eq!(decode_bounded(&encoded, MAX_RESOURCE_BYTES, 4, &Cell::new(16)), Ok(b"four".to_vec()));
        assert_eq!(decode_bounded(&encoded, MAX_RESOURCE_BYTES, 4, &Cell::new(0)), Err(EmbeddedError::WorkBudget));
    }
}

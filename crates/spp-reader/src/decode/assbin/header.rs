//! The fixed 512-byte header of an Assimp binary dump, and access to the scene bytes behind it.

use super::RECORD;
use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;
use std::borrow::Cow;

const SIGNATURE: &[u8] = b"ASSIMP.binary-dump.";
const SIGNATURE_FIELD: usize = 44;
const SOURCE_NAME_FIELD: usize = 256;
const COMMAND_LINE_FIELD: usize = 128;
const RESERVED_FIELD: usize = 64;
/// Total header length, fixed by the format.
pub const HEADER_LEN: usize = 512;
/// Upper bound for a declared decompressed size, against corrupt input.
const MAX_UNCOMPRESSED: usize = 1 << 30;

/// Header fields. The reference app writes version 1.5 (2019), 1.6 (2020–21) and 1.7 (2023).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssbinHeader {
    pub version_major: u32,
    pub version_minor: u32,
    pub revision: u32,
    pub compile_flags: u32,
    /// Shortened dumps replace vertex data by hashes; they carry no usable mesh.
    pub shortened: bool,
    /// The scene behind the header is a zlib stream.
    pub compressed: bool,
    /// Path of the file the dump was made from, as stored (NUL padding removed).
    pub source_name: String,
}

impl AssbinHeader {
    pub fn parse(data: &[u8]) -> Result<Self, DecodeError> {
        let mut r = ByteReader::new(data);
        let signature = r.bytes(SIGNATURE_FIELD).map_err(DecodeError::truncated(RECORD))?;
        if !signature.starts_with(SIGNATURE) {
            return Err(DecodeError::BadMagic { record: RECORD });
        }
        let num = |r: &mut ByteReader| r.u32_le().map_err(DecodeError::truncated(RECORD));
        let version_major = num(&mut r)?;
        let version_minor = num(&mut r)?;
        let revision = num(&mut r)?;
        let compile_flags = num(&mut r)?;
        let shortened = r.u16_le().map_err(DecodeError::truncated(RECORD))? != 0;
        let compressed = r.u16_le().map_err(DecodeError::truncated(RECORD))? != 0;
        let source = r.bytes(SOURCE_NAME_FIELD).map_err(DecodeError::truncated(RECORD))?;
        r.skip(COMMAND_LINE_FIELD + RESERVED_FIELD).map_err(DecodeError::truncated(RECORD))?;
        Ok(AssbinHeader { version_major, version_minor, revision, compile_flags, shortened, compressed, source_name: nul_terminated(source) })
    }

    /// The scene bytes after the header, decompressed if needed.
    pub fn scene_bytes<'a>(&self, data: &'a [u8]) -> Result<Cow<'a, [u8]>, DecodeError> {
        let body = data.get(HEADER_LEN..).ok_or_else(|| DecodeError::invalid(RECORD, "shorter than its header"))?;
        if !self.compressed {
            return Ok(Cow::Borrowed(body));
        }
        let mut r = ByteReader::new(body);
        let size = r.u32_le().map_err(DecodeError::truncated(RECORD))? as usize;
        if size > MAX_UNCOMPRESSED {
            return Err(DecodeError::invalid(RECORD, format!("declared size {size} is too large")));
        }
        // zlib = 2-byte header + raw DEFLATE + Adler-32; the inflater wants the raw stream.
        r.skip(2).map_err(DecodeError::truncated(RECORD))?;
        let out = crate::inflate::inflate(r.rest()).map_err(|e| DecodeError::invalid(RECORD, e))?;
        if out.len() != size {
            return Err(DecodeError::invalid(RECORD, format!("inflated to {} bytes, header says {size}", out.len())));
        }
        Ok(Cow::Owned(out))
    }
}

fn nul_terminated(bytes: &[u8]) -> String {
    // `split` always yields at least one piece: everything before the first NUL, or all of `bytes`.
    let text = bytes.split(|&b| b == 0).next().unwrap_or_default();
    String::from_utf8_lossy(text).into_owned()
}

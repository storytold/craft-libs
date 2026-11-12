//! `canvas.fig`: the header, the chunks and their compression.

use std::io::{Read, Write};

use craft_kiwi::Schema;

use crate::{Error, corrupt};

/// The canvas header of a design file.
pub const DESIGN_HEADER: &[u8] = b"fig-kiwi";
/// The canvas header of a FigJam whiteboard (a `.jam`); a `.fig` has [`DESIGN_HEADER`].
pub const JAM_HEADER: &[u8] = b"fig-jam.";
/// The version [`encode_canvas`] writes after the header.
pub const VERSION: u32 = 101;
/// Largest `canvas.fig` read from an archive or written.
pub const MAX_CANVAS: u64 = 256 * 1024 * 1024;
/// Largest decompressed chunk.
pub const MAX_CHUNK: u64 = 512 * 1024 * 1024;
/// Largest decompressed schema chunk: real schemas unpack to well under 1 MB, so a schema that
/// inflates past this is refused before its definitions are decoded.
pub const MAX_SCHEMA: u64 = 16 * 1024 * 1024;
/// Most chunks read from `canvas.fig`.
pub const MAX_CHUNKS: usize = 16;

/// Value budget per compressed canvas byte. Real files hold up to 0.75 values per compressed byte
/// without their single scalar fields; counting those too adds 15–43% on generated files and up
/// to 3.2 times on a rectangle carrying only geometry and paint, so a dense file may reach about
/// 2.4 per byte. Four leaves two-thirds more room than that. The trade-off: from 32 MiB of canvas
/// (8 MiB on 32-bit targets) a file gets all of the ceiling, and above the floor it may decode
/// four values for every byte it carries.
const VALUES_PER_PACKED_BYTE: usize = 4;
/// 32-bit targets (wasm32) have a 4 GB address space in all, so their ceiling is lower: past it
/// the file is refused cleanly instead of running out of memory.
#[cfg(not(target_pointer_width = "32"))]
const MAX_VALUES: usize = 128 << 20;
#[cfg(target_pointer_width = "32")]
const MAX_VALUES: usize = 32 << 20;

/// Most values to decode ([`Schema::decode_message_within`]) from a file whose compressed canvas
/// is `packed` bytes: at least [`craft_kiwi::DEFAULT_MAX_VALUES`], four per byte, and at most 128
/// million (32 million on 32-bit targets). A large real file decodes while a small one can't
/// expand into a huge tree, however far its chunks inflate.
pub fn value_budget(packed: usize) -> usize {
    packed.saturating_mul(VALUES_PER_PACKED_BYTE).clamp(craft_kiwi::DEFAULT_MAX_VALUES, MAX_VALUES)
}

/// The schema and the raw (decompressed) message of a `canvas.fig`.
pub fn decode_canvas(canvas: &[u8]) -> Result<(Schema, Vec<u8>), Error> {
    let chunks = chunks(canvas)?;
    let mut it = chunks.into_iter();
    let (Some(schema), Some(message)) = (it.next(), it.next()) else {
        return Err(corrupt("canvas.fig needs a schema and a message chunk"));
    };
    let schema = Schema::decode(&inflate(schema, MAX_SCHEMA)?).map_err(|e| corrupt(e.to_string()))?;
    Ok((schema, inflate(message, MAX_CHUNK)?))
}

/// The length-prefixed chunks after the 12-byte header (at most [`MAX_CHUNKS`]), still compressed.
pub fn chunks(canvas: &[u8]) -> Result<Vec<&[u8]>, Error> {
    if canvas.len() < 12 || !canvas.starts_with(b"fig-") {
        return Err(corrupt("canvas.fig has no header"));
    }
    let mut out = Vec::new();
    let mut pos = 12usize;
    while pos < canvas.len() && out.len() < MAX_CHUNKS {
        let start = pos.checked_add(4).ok_or_else(|| corrupt("truncated chunk"))?;
        let len = canvas.get(pos..start).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).ok_or_else(|| corrupt("truncated chunk"))?;
        let end = start.checked_add(len as usize).filter(|e| *e <= canvas.len()).ok_or_else(|| corrupt("truncated chunk"))?;
        out.push(canvas.get(start..end).unwrap_or_default());
        pos = end;
    }
    Ok(out)
}

/// Decompress a chunk: zstd (by its magic number), else raw deflate, else zlib. A chunk that
/// unpacks to more than `max` bytes is refused.
pub fn inflate(data: &[u8], max: u64) -> Result<Vec<u8>, Error> {
    let capped = |r: &mut dyn Read| -> Option<Vec<u8>> {
        let mut out = Vec::new();
        r.take(max.saturating_add(1)).read_to_end(&mut out).ok()?;
        (out.len() as u64 <= max).then_some(out)
    };
    if data.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        let mut src = data;
        let mut dec = ruzstd::decoding::StreamingDecoder::new(&mut src).map_err(|e| corrupt(format!("zstd: {e}")))?;
        return capped(&mut dec).ok_or_else(|| corrupt("damaged or oversized zstd chunk"));
    }
    capped(&mut flate2::read::DeflateDecoder::new(data))
        .or_else(|| capped(&mut flate2::read::ZlibDecoder::new(data)))
        .ok_or_else(|| corrupt("damaged or oversized deflate chunk"))
}

/// `canvas.fig` of a design file: [`DESIGN_HEADER`], [`VERSION`] and the two chunks (the encoded
/// schema and message), each raw-deflated after its length.
pub fn encode_canvas(schema: &[u8], message: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = DESIGN_HEADER.to_vec();
    out.extend_from_slice(&VERSION.to_le_bytes());
    for chunk in [schema, message] {
        let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(chunk).map_err(|e| Error::Io(e.to_string()))?;
        let c = z.finish().map_err(|e| Error::Io(e.to_string()))?;
        let len = u32::try_from(c.len()).map_err(|_| Error::TooLarge)?;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&c);
    }
    if out.len() as u64 > MAX_CANVAS {
        return Err(Error::TooLarge);
    }
    Ok(out)
}

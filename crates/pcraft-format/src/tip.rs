//! PhotoCraft's PCTIP1 brush-tip envelope and lossless grayscale sample serialization.
//!
//! Extracted from PhotoCraft preset_store.rs at 19aec5eb44131de44361189d64910e7b3e19e57c.
//! Original implementation: Brandon Thomas; subsequent codec changes: David Mann and FinnNk.
//! Group schemas, file naming, content hashes and storage policy remain with the application.

/// Largest accepted tip side.
pub const MAX_SIDE: u32 = 16384;
/// Largest accepted sample count.
pub const MAX_SAMPLES: u64 = 1 << 26;
const TIP_MAGIC: &[u8; 6] = b"PCTIP1";
const TIP_HEADER: usize = 6 + 1 + 4 + 4;

/// Decoded grayscale samples. Eight-bit samples expand losslessly by multiplication by 257.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tip {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major grayscale samples, `width * height` of them.
    pub data: Vec<u16>,
}

/// `PCTIP1`, bits (8|16), width, height (u32 LE), then the zlib-deflated samples.
/// Returns an empty vector for invalid dimensions/sample counts or compression failure.
pub fn encode(width: u32, height: u32, data: &[u16]) -> Vec<u8> {
    use std::io::Write;
    let count = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || count > MAX_SAMPLES || count != data.len() as u64 {
        return Vec::new();
    }
    let eight = data.iter().all(|v| v % 257 == 0);
    let mut out = Vec::with_capacity(TIP_HEADER + data.len() / 4);
    out.extend_from_slice(TIP_MAGIC);
    out.push(if eight { 8 } else { 16 });
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    // Level 3: zlib-rs level 1 writes static Huffman blocks only, 13-30% bigger than miniz's level 1.
    let mut z = flate2::write::ZlibEncoder::new(out, flate2::Compression::new(3));
    let mut buf = Vec::with_capacity(8192);
    let mut ok = true;
    for chunk in data.chunks(4096) {
        buf.clear();
        if eight {
            buf.extend(chunk.iter().map(|v| (v / 257) as u8));
        } else {
            for v in chunk {
                buf.extend_from_slice(&v.to_le_bytes());
            }
        }
        ok &= z.write_all(&buf).is_ok();
    }
    match z.finish() {
        Ok(v) if ok => v,
        _ => Vec::new(),
    }
}

/// Inverse of [`encode`]; any malformed, truncated or oversized input is an error.
pub fn decode(bytes: &[u8]) -> Result<Tip, String> {
    decode_with_max_side(bytes, MAX_SIDE)
}

/// [`decode`] refusing (before inflating anything) a bitmap with a side over `max_side`.
pub fn decode_with_max_side(bytes: &[u8], max_side: u32) -> Result<Tip, String> {
    let head = bytes.get(..TIP_HEADER).ok_or("truncated tip header")?;
    if &head[..6] != TIP_MAGIC {
        return Err("not a PhotoCraft tip".into());
    }
    let bits = head[6];
    let u32_at = |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
    let (w, h) = (u32_at(7), u32_at(11));
    if w == 0 || h == 0 || w > MAX_SIDE.min(max_side) || h > MAX_SIDE.min(max_side) {
        return Err(format!("bad tip size {w}×{h}"));
    }
    let n = u64::from(w) * u64::from(h);
    if n > MAX_SAMPLES {
        return Err(format!("tip too large ({w}×{h})"));
    }
    let bytes_per = match bits {
        8 => 1u64,
        16 => 2,
        b => return Err(format!("unsupported tip depth {b}")),
    };
    let want = n * bytes_per;
    // One byte beyond the declared size detects overlong output, without allocating for
    // an attacker-controlled expansion. Require the zlib trailer, not merely enough samples.
    let mut raw = Vec::with_capacity(want as usize + 1);
    let status = flate2::Decompress::new(true)
        .decompress_vec(&bytes[TIP_HEADER..], &mut raw, flate2::FlushDecompress::Finish)
        .map_err(|e| format!("tip data: {e}"))?;
    if status != flate2::Status::StreamEnd {
        return Err("tip data: truncated or oversized zlib stream".into());
    }
    if raw.len() as u64 != want {
        return Err(format!("tip data has {} bytes, expected {want}", raw.len()));
    }
    let data: Vec<u16> = if bits == 8 {
        raw.iter().map(|&v| u16::from(v) * 257).collect()
    } else {
        raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect()
    };
    Ok(Tip { width: w, height: h, data })
}

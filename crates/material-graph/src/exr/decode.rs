//! Reading: header, line-offset table, then every chunk, each placed by the y coordinate it carries.

use super::header::Header;
use super::{ExrChannel, ExrImage, SampleKind};
use crate::half::f16_to_f32;

/// Decode a single-part scanline OpenEXR file (NONE, RLE, ZIPS or ZIP compression). Channels come back sorted by
/// name; the data window becomes the image (its origin is dropped).
pub fn decode(b: &[u8]) -> Result<ExrImage, String> {
    let (header, table_at) = Header::read(b)?;
    // `Header::read` bounds the data window (65536 per side, 2^28 pixels), so `w * h` cannot overflow.
    let (w, h) = (header.width(), header.height());
    let lines = header.compression.lines_per_block();
    let blocks = h.div_ceil(lines);

    // Refuse sizes the file cannot possibly hold before allocating anything.
    let line_bytes = header.line_bytes();
    let raw_total = line_bytes.checked_mul(h).ok_or("image too large")?;
    if raw_total / header.compression.max_expansion() > b.len() {
        return Err("image larger than the file can hold".into());
    }
    let table_end = blocks.checked_mul(8).and_then(|n| n.checked_add(table_at)).ok_or("image too large")?;
    let table = b.get(table_at..table_end).ok_or("truncated line offset table")?;

    let mut channels = Vec::with_capacity(header.channels.len());
    for (name, kind) in &header.channels {
        let mut data = Vec::new();
        data.try_reserve_exact(w * h).map_err(|_| "image too large")?;
        data.resize(w * h, 0.0);
        channels.push(ExrChannel::new(name, *kind, data));
    }
    let mut seen = vec![false; blocks];
    for &entry in table.as_chunks::<8>().0 {
        let offset = u64::from_le_bytes(entry);
        let at = usize::try_from(offset).map_err(|_| "chunk offset out of range")?;
        let Some(&[y0, y1, y2, y3, s0, s1, s2, s3]) = b.get(at..at.checked_add(8).ok_or("chunk offset out of range")?) else {
            return Err("chunk offset out of range".into());
        };
        let y = i32::from_le_bytes([y0, y1, y2, y3]) as i64;
        let size = usize::try_from(i32::from_le_bytes([s0, s1, s2, s3])).map_err(|_| "negative chunk size")?;
        // at + 8 cannot overflow: those eight bytes exist.
        let data = b.get(at + 8..(at + 8).checked_add(size).ok_or("truncated chunk")?).ok_or("truncated chunk")?;

        let rel = y - header.data_window[1] as i64;
        if rel < 0 || rel as usize >= h || !(rel as usize).is_multiple_of(lines) {
            return Err(format!("chunk has invalid y {y}"));
        }
        let block = rel as usize / lines;
        let Some(seen) = seen.get_mut(block) else {
            return Err(format!("chunk has invalid y {y}"));
        };
        if std::mem::replace(seen, true) {
            return Err(format!("duplicate chunk for y {y}"));
        }
        let y0 = rel as usize;
        let n_lines = lines.min(h - y0);
        // n_lines * line_bytes <= raw_total, checked above.
        let raw = header.compression.decompress(data, n_lines * line_bytes)?;
        let mut pos = 0;
        for line in y0..y0 + n_lines {
            for ch in channels.iter_mut() {
                let bytes = w * ch.sample.size();
                // `raw` holds exactly n_lines * line_bytes bytes and every line of `data` exists (line < h).
                let (Some(src), Some(dst)) = (raw.get(pos..pos + bytes), ch.data.get_mut(line * w..(line + 1) * w)) else {
                    return Err("block decompressed to the wrong size".into());
                };
                read_samples(src, ch.sample, dst);
                pos += bytes;
            }
        }
    }
    Ok(ExrImage { width: w, height: h, channels })
}

fn read_samples(src: &[u8], kind: SampleKind, dst: &mut [f32]) {
    match kind {
        SampleKind::Half => src.as_chunks::<2>().0.iter().zip(dst).for_each(|(&s, d)| *d = f16_to_f32(u16::from_le_bytes(s))),
        SampleKind::Float => src.as_chunks::<4>().0.iter().zip(dst).for_each(|(&s, d)| *d = f32::from_le_bytes(s)),
        SampleKind::Uint => src.as_chunks::<4>().0.iter().zip(dst).for_each(|(&s, d)| *d = u32::from_le_bytes(s) as f32),
    }
}

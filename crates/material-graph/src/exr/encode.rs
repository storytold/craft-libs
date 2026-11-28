//! Writing: header, line-offset table, then one chunk (y, size, data) per block of scanlines.

use super::header::Header;
use super::{Compression, ExrChannel, ExrImage, SampleKind};
use crate::half::f32_to_f16;

/// Encode with ZIP compression (lossless; the usual choice for HDR maps).
pub fn encode(img: &ExrImage) -> Result<Vec<u8>, String> {
    encode_with(img, Compression::Zip)
}

/// Encode with the given compression (`Rle` is read-only and rejected).
pub fn encode_with(img: &ExrImage, compression: Compression) -> Result<Vec<u8>, String> {
    validate(img)?;
    if compression == Compression::Rle {
        return Err("writing RLE-compressed OpenEXR is not supported".into());
    }
    let mut order: Vec<&ExrChannel> = img.channels.iter().collect();
    order.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    let header = Header {
        channels: order.iter().map(|c| (c.name.clone(), c.sample)).collect(),
        compression,
        data_window: [0, 0, img.width as i32 - 1, img.height as i32 - 1],
    };
    let mut out = Vec::new();
    header.write(&mut out);

    let lines = compression.lines_per_block();
    let blocks = img.height.div_ceil(lines);
    let table_at = out.len();
    out.resize(table_at + blocks * 8, 0);
    let mut raw = Vec::with_capacity(lines * header.line_bytes());
    for block in 0..blocks {
        let y0 = block * lines;
        raw.clear();
        for y in y0..(y0 + lines).min(img.height) {
            for ch in &order {
                // `validate` checked every channel holds width * height samples.
                let row = ch.data.get(y * img.width..(y + 1) * img.width).ok_or("channel too short")?;
                write_samples(row, ch.sample, &mut raw);
            }
        }
        let data = compression.compress(&raw);
        let offset = out.len() as u64;
        let slot = table_at + block * 8;
        out.get_mut(slot..slot + 8).ok_or("line offset table")?.copy_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(y0 as i32).to_le_bytes());
        out.extend_from_slice(&(data.len() as i32).to_le_bytes());
        out.extend_from_slice(&data);
    }
    Ok(out)
}

fn validate(img: &ExrImage) -> Result<(), String> {
    if img.width == 0 || img.height == 0 || img.width > i32::MAX as usize || img.height > i32::MAX as usize {
        return Err(format!("bad image size {}x{}", img.width, img.height));
    }
    if img.channels.is_empty() {
        return Err("image has no channels".into());
    }
    let n = img.width.checked_mul(img.height).ok_or("image too large")?;
    for (i, c) in img.channels.iter().enumerate() {
        if c.name.is_empty() || c.name.len() > 255 || c.name.contains('\0') {
            return Err(format!("bad channel name '{}'", c.name));
        }
        if img.channels.iter().take(i).any(|o| o.name == c.name) {
            return Err(format!("duplicate channel '{}'", c.name));
        }
        if c.data.len() != n {
            return Err(format!("channel '{}' has {} samples, expected {n}", c.name, c.data.len()));
        }
    }
    Ok(())
}

fn write_samples(row: &[f32], kind: SampleKind, out: &mut Vec<u8>) {
    match kind {
        SampleKind::Half => row.iter().for_each(|&v| out.extend_from_slice(&f32_to_f16(v).to_le_bytes())),
        SampleKind::Float => row.iter().for_each(|&v| out.extend_from_slice(&v.to_le_bytes())),
        // Saturating cast: negatives and NaN become 0.
        SampleKind::Uint => row.iter().for_each(|&v| out.extend_from_slice(&(v as u32).to_le_bytes())),
    }
}

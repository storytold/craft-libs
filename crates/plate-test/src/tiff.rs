//! A minimal reader for the plates Ghostscript's `tiffsep` device writes: one uncompressed
//! 8-bit gray channel in strips, either byte order.
//!
//! `tiffsep` writes plates as BlackIsZero, so a sample of 0 is solid ink and 255 is paper.

use crate::Error;

/// One separation plate, as ink levels (0 = paper, 255 = solid ink), row-major.
#[derive(Debug, Clone, PartialEq)]
pub struct Plate {
    width: usize,
    height: usize,
    ink: Vec<u8>,
}

impl Plate {
    /// A plate from ink levels; `None` when `ink` doesn't hold `width × height` values.
    pub fn new(width: usize, height: usize, ink: Vec<u8>) -> Option<Self> {
        (width.checked_mul(height) == Some(ink.len())).then_some(Self { width, height, ink })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// Ink percentage (0–100) at a pixel; `None` outside the plate.
    pub fn ink(&self, x: usize, y: usize) -> Option<f32> {
        if x >= self.width {
            return None;
        }
        self.ink.get(y.checked_mul(self.width)?.checked_add(x)?).map(|level| f32::from(*level) * 100.0 / 255.0)
    }
}

const WIDTH: u16 = 256;
const HEIGHT: u16 = 257;
const BITS_PER_SAMPLE: u16 = 258;
const COMPRESSION: u16 = 259;
const PHOTOMETRIC: u16 = 262;
const STRIP_OFFSETS: u16 = 273;
const SAMPLES_PER_PIXEL: u16 = 277;
const ROWS_PER_STRIP: u16 = 278;

/// Read a plate from TIFF bytes.
pub fn read(data: &[u8]) -> Result<Plate, Error> {
    let bad = |what: &str| Error::Tiff(what.to_owned());
    let big = match data.get(..4) {
        Some(b"II*\0") => false,
        Some(b"MM\0*") => true,
        _ => return Err(bad("not a TIFF file")),
    };
    let file = File { data, big };
    let ifd = file.u32(4).ok_or_else(|| bad("truncated header"))? as usize;
    let count = file.u16(ifd).ok_or_else(|| bad("truncated directory"))? as usize;
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let at = ifd + 2 + 12 * i;
        let (Some(tag), Some(kind), Some(n)) = (file.u16(at), file.u16(at + 2), file.u32(at + 4)) else {
            return Err(bad("truncated directory"));
        };
        entries.push(Entry { tag, kind, count: n as usize, value_at: at + 8 });
    }
    let get = |tag: u16| -> Result<Option<Vec<usize>>, Error> {
        entries.iter().find(|e| e.tag == tag).map(|e| file.values(e).ok_or_else(|| bad(&format!("bad value for tag {tag}")))).transpose()
    };
    let one = |tag: u16, default: Option<usize>| -> Result<usize, Error> {
        match get(tag)?.as_deref() {
            Some([value]) => Ok(*value),
            Some(_) => Err(bad(&format!("tag {tag} must hold one value"))),
            None => default.ok_or_else(|| bad(&format!("missing tag {tag}"))),
        }
    };
    let (width, height) = (one(WIDTH, None)?, one(HEIGHT, None)?);
    if one(BITS_PER_SAMPLE, Some(1))? != 8 || one(SAMPLES_PER_PIXEL, Some(1))? != 1 {
        return Err(bad("only 8-bit single-channel plates are supported"));
    }
    if one(COMPRESSION, Some(1))? != 1 {
        return Err(bad("only uncompressed plates are supported (-sCompression=none)"));
    }
    let white_is_zero = match one(PHOTOMETRIC, None)? {
        0 => true,
        1 => false,
        other => return Err(bad(&format!("photometric interpretation {other} is not gray"))),
    };
    let rows_per_strip = one(ROWS_PER_STRIP, Some(height))?.clamp(1, height.max(1));
    let offsets = get(STRIP_OFFSETS)?.ok_or_else(|| bad("missing strip offsets"))?;
    let size = width.checked_mul(height).ok_or_else(|| bad("plate too large"))?;
    let mut ink = Vec::with_capacity(size);
    for (strip, offset) in offsets.iter().enumerate() {
        let rows = rows_per_strip.min(height.saturating_sub(strip * rows_per_strip));
        let bytes = data.get(*offset..offset.saturating_add(rows * width)).ok_or_else(|| bad("strip outside the file"))?;
        ink.extend(bytes.iter().map(|v| if white_is_zero { *v } else { 255 - v }));
    }
    Plate::new(width, height, ink).ok_or_else(|| bad("strips don't cover the plate"))
}

struct Entry {
    tag: u16,
    kind: u16,
    count: usize,
    /// Where the value (four bytes or less) or the offset to it sits.
    value_at: usize,
}

struct File<'a> {
    data: &'a [u8],
    big: bool,
}

impl File<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b: [u8; 2] = self.data.get(at..at + 2)?.try_into().ok()?;
        Some(if self.big { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b: [u8; 4] = self.data.get(at..at + 4)?.try_into().ok()?;
        Some(if self.big { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) })
    }

    /// SHORT or LONG values, inline when they fit in four bytes.
    fn values(&self, entry: &Entry) -> Option<Vec<usize>> {
        let size = match entry.kind {
            3 => 2,
            4 => 4,
            _ => return None,
        };
        let start = if entry.count * size <= 4 { entry.value_at } else { self.u32(entry.value_at)? as usize };
        (0..entry.count)
            .map(|i| {
                let at = start.checked_add(i.checked_mul(size)?)?;
                if size == 2 { self.u16(at).map(usize::from) } else { self.u32(at).map(|v| v as usize) }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// A gray TIFF as `tiffsep` writes it (BlackIsZero samples), in strips of `rows` rows.
    fn encode(width: u16, height: u16, samples: &[u8], rows: u16, big: bool) -> Vec<u8> {
        let p16 = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let p32 = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
        let strips: Vec<&[u8]> = samples.chunks(usize::from(width) * usize::from(rows)).collect();
        let mut out = if big { b"MM\0*".to_vec() } else { b"II*\0".to_vec() };
        out.extend(p32(8));
        let entries = 8u16;
        let dir_end = 8 + 2 + 12 * u32::from(entries) + 4;
        let offsets_at = dir_end;
        let data_at = offsets_at + 4 * strips.len() as u32;
        let mut dir = p16(entries).to_vec();
        let mut entry = |tag: u16, kind: u16, count: u32, value: u32| {
            dir.extend(p16(tag));
            dir.extend(p16(kind));
            dir.extend(p32(count));
            if kind == 3 && count == 1 {
                dir.extend(p16(value as u16));
                dir.extend([0, 0]);
            } else {
                dir.extend(p32(value));
            }
        };
        entry(WIDTH, 3, 1, u32::from(width));
        entry(HEIGHT, 3, 1, u32::from(height));
        entry(BITS_PER_SAMPLE, 3, 1, 8);
        entry(COMPRESSION, 3, 1, 1);
        entry(PHOTOMETRIC, 3, 1, 1);
        let first = if strips.len() == 1 { data_at } else { offsets_at };
        entry(STRIP_OFFSETS, 4, strips.len() as u32, first);
        entry(SAMPLES_PER_PIXEL, 3, 1, 1);
        entry(ROWS_PER_STRIP, 3, 1, u32::from(rows));
        dir.extend(p32(0));
        out.extend(dir);
        let mut at = data_at;
        for strip in &strips {
            out.extend(p32(at));
            at += strip.len() as u32;
        }
        for strip in strips {
            out.extend(strip);
        }
        out
    }

    #[test]
    fn reads_strips_in_both_byte_orders() {
        let samples: Vec<u8> = (0..15).map(|i| (i * 17) as u8).collect();
        for big in [false, true] {
            for rows in [1, 2, 3] {
                let plate = read(&encode(5, 3, &samples, rows, big)).unwrap();
                assert_eq!((plate.width(), plate.height()), (5, 3));
                assert_eq!(plate.ink(0, 0), Some(100.0), "sample 0 is solid ink");
                assert_eq!(plate.ink(4, 2), Some((255.0 - 238.0) * 100.0 / 255.0));
                assert_eq!(plate.ink(5, 0), None);
                assert_eq!(plate.ink(0, 3), None);
            }
        }
    }

    #[test]
    fn rejects_what_it_cannot_read() {
        let good = encode(2, 2, &[0, 0, 0, 0], 2, false);
        assert!(read(b"GIF89a").is_err());
        assert!(read(&good[..40]).is_err());
        assert!(read(&good[..good.len() - 1]).is_err(), "truncated strip");
        let mut packed = good.clone();
        // Compression (the fourth entry's value) set to PackBits.
        packed[8 + 2 + 12 * 3 + 8] = 5;
        assert!(read(&packed).unwrap_err().to_string().contains("uncompressed"));
    }
}

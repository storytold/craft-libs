//! BMP header reading and decoding shared by the page builder, the software 1-bit conversion
//! ([`crate::bw`]) and the GUI preview.
//!
//! One parser, [`parse`], answers "is this a BMP, how big, how deep, how are the rows stored, where
//! are the palette and the pixels". It never trusts a size it has not checked against the buffer, so
//! a hostile header (zero or negative width, 100000 x 100000 pixels in a 1 KB file, a palette
//! count of four billion, `bfOffBits` past the end) gives `None` or a refusal without allocating.
//!
//! Headers: the 12-byte OS/2 core header and the 40 to 124 byte Windows family (info, V4, V5).
//! Row order follows the sign of the height (negative is top-down). Palette entries are 3 bytes
//! (core) or 4 bytes (everything else). `bfOffBits` is authoritative; 0 means "right after the
//! palette", as several writers leave it.

/// Most pixels [`to_rgba`] decodes (1 GiB of RGBA).
pub const MAX_PREVIEW_PIXELS: usize = 1 << 28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    Rgb,
    Rle8,
    Rle4,
    Bitfields,
    /// JPEG, PNG, alpha bit fields, CMYK and so on: the header is fine but the pixels are not ours.
    Other(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BmpInfo {
    pub width: u32,
    /// Absolute height; see `top_down` for the row order.
    pub height: u32,
    pub top_down: bool,
    pub bits: u16,
    pub compression: Compression,
    pub header_size: usize,
    /// 3 for the OS/2 core header, else 4.
    pub palette_entry: usize,
    /// Palette entries the header declares (clamped to what the depth allows; 0 above 8 bits).
    pub declared_colours: usize,
    /// `bfOffBits` as written.
    pub offset_field: usize,
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// The header of a BMP file, or `None` when `bytes` is not one this crate can describe.
pub fn parse(bytes: &[u8]) -> Option<BmpInfo> {
    if bytes.get(..2)? != b"BM" {
        return None;
    }
    let header_size = usize::try_from(le32(bytes, 14)?).ok()?;
    let core = header_size == 12;
    if !core && !(40..=124).contains(&header_size) {
        return None;
    }
    // The whole header must be there.
    bytes.get(..14usize.checked_add(header_size)?)?;
    let (width, height, top_down, planes, bits, compression, colours) = if core {
        (u32::from(le16(bytes, 18)?), u32::from(le16(bytes, 20)?), false, le16(bytes, 22)?, le16(bytes, 24)?, 0, 0)
    } else {
        let raw_width = le32(bytes, 18)? as i32;
        let raw_height = le32(bytes, 22)? as i32;
        if raw_width <= 0 || raw_height == i32::MIN {
            return None;
        }
        (raw_width.unsigned_abs(), raw_height.unsigned_abs(), raw_height < 0, le16(bytes, 26)?, le16(bytes, 28)?, le32(bytes, 30)?, le32(bytes, 46)?)
    };
    if width == 0 || height == 0 || planes != 1 || !matches!(bits, 1 | 4 | 8 | 16 | 24 | 32) {
        return None;
    }
    let compression = match (compression, bits) {
        (0, _) => Compression::Rgb,
        (1, 8) => Compression::Rle8,
        (2, 4) => Compression::Rle4,
        (3, 16 | 32) => Compression::Bitfields,
        (1..=3, _) => return None,
        (other, _) => Compression::Other(other),
    };
    let full = if bits <= 8 { 1usize << bits } else { 0 };
    let colours = usize::try_from(colours).ok()?;
    let declared_colours = if core || colours == 0 || colours > full { full } else { colours };
    Some(BmpInfo {
        width,
        height,
        top_down,
        bits,
        compression,
        header_size,
        palette_entry: if core { 3 } else { 4 },
        declared_colours,
        offset_field: usize::try_from(le32(bytes, 10)?).ok()?,
    })
}

impl BmpInfo {
    /// Where the palette starts.
    pub fn palette_at(&self) -> usize {
        14 + self.header_size
    }

    /// Where the pixels start: `bfOffBits`, or the end of the palette when the field is 0.
    /// `None` when the field points into the headers.
    pub fn data_at(&self) -> Option<usize> {
        if self.offset_field == 0 {
            // Bit-field masks sit between an info header and the pixels.
            let masks = if self.header_size == 40 && self.compression == Compression::Bitfields { 12 } else { 0 };
            return self.palette_at().checked_add(masks)?.checked_add(self.declared_colours.checked_mul(self.palette_entry)?);
        }
        (self.offset_field >= self.palette_at()).then_some(self.offset_field)
    }

    /// Palette entries actually stored: the declared count, but never past the start of the pixels.
    pub fn stored_colours(&self) -> usize {
        let Some(data) = self.data_at() else { return 0 };
        self.declared_colours.min(data.saturating_sub(self.palette_at()) / self.palette_entry)
    }

    /// One palette entry as `[r, g, b]`; indices past the stored table give black.
    pub fn colour(&self, bytes: &[u8], index: usize) -> [u8; 3] {
        let black = [0, 0, 0];
        if index >= self.stored_colours() {
            return black;
        }
        let Some(at) = index.checked_mul(self.palette_entry).and_then(|o| o.checked_add(self.palette_at())) else { return black };
        match bytes.get(at..).and_then(|b| b.get(..3)) {
            Some([blue, green, red]) => [*red, *green, *blue],
            _ => black,
        }
    }

    /// Bytes per stored row (rows pad to 4 bytes), uncompressed forms.
    pub fn stride(&self) -> Option<usize> {
        usize::try_from(self.width).ok()?.checked_mul(usize::from(self.bits))?.div_ceil(32).checked_mul(4)
    }

    /// The uncompressed pixel array, only when all of it is in `bytes`.
    pub fn pixels<'a>(&self, bytes: &'a [u8]) -> Option<&'a [u8]> {
        if self.compression != Compression::Rgb {
            return None;
        }
        let size = self.stride()?.checked_mul(usize::try_from(self.height).ok()?)?;
        bytes.get(self.data_at()?..)?.get(..size)
    }

    /// Index into the stored rows for visual row `row` (0 is the top of the picture).
    pub fn file_row(&self, row: usize) -> Option<usize> {
        let rows = usize::try_from(self.height).ok()?;
        if self.top_down { Some(row) } else { rows.checked_sub(1)?.checked_sub(row) }
    }
}

/// Width and height of a BMP, if `bytes` starts like one.
pub fn size(bytes: &[u8]) -> Option<(u32, u32)> {
    parse(bytes).map(|info| (info.width, info.height))
}

/// Palette indices of an RLE4 or RLE8 body in file row order (`cols` x `rows`). Pixels the stream
/// skips stay index 0; a run past the image edge is clipped. `None` for a truncated stream.
fn rle_indices(data: &[u8], cols: usize, rows: usize, four_bit: bool) -> Option<Vec<u8>> {
    let total = cols.checked_mul(rows)?;
    let mut out = Vec::new();
    out.try_reserve_exact(total).ok()?;
    out.resize(total, 0u8);
    let (mut x, mut y, mut pos) = (0usize, 0usize, 0usize);
    let put = |out: &mut Vec<u8>, x: usize, y: usize, index: u8| {
        if x < cols
            && y < rows
            && let Some(slot) = out.get_mut(y * cols + x)
        {
            *slot = index;
        }
    };
    while pos < data.len() {
        let count = usize::from(*data.get(pos)?);
        let value = *data.get(pos + 1)?;
        pos += 2;
        if count > 0 {
            for i in 0..count {
                let index = if !four_bit {
                    value
                } else if i % 2 == 0 {
                    value >> 4
                } else {
                    value & 0x0f
                };
                put(&mut out, x, y, index);
                x += 1;
            }
            continue;
        }
        match value {
            0 => {
                x = 0;
                y += 1;
            }
            1 => break,
            2 => {
                x += usize::from(*data.get(pos)?);
                y += usize::from(*data.get(pos + 1)?);
                pos += 2;
            }
            n => {
                let n = usize::from(n);
                let used = if four_bit { n.div_ceil(2) } else { n };
                let run = data.get(pos..pos.checked_add(used)?)?;
                for i in 0..n {
                    let index = if !four_bit {
                        *run.get(i)?
                    } else if i % 2 == 0 {
                        *run.get(i / 2)? >> 4
                    } else {
                        *run.get(i / 2)? & 0x0f
                    };
                    put(&mut out, x, y, index);
                    x += 1;
                }
                // Absolute runs are padded to a 16-bit boundary.
                pos += used + (used % 2);
            }
        }
    }
    Some(out)
}

/// BMP (1, 4, 8, 24 or 32 bits uncompressed; 4 and 8 bits also RLE) as top-down RGBA: row 0 is the
/// top of the picture. Bit fields, 16 bits, embedded JPEG/PNG and truncated files give `None`, so the
/// caller can try another decoder. Palette indices past the stored table paint black.
pub fn to_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let info = parse(bytes)?;
    let (cols, rows) = (usize::try_from(info.width).ok()?, usize::try_from(info.height).ok()?);
    let count = cols.checked_mul(rows).filter(|n| *n <= MAX_PREVIEW_PIXELS)?;
    let mut table = [[0u8, 0, 0, 255]; 256];
    if info.bits <= 8 {
        for (index, slot) in table.iter_mut().enumerate().take(1 << info.bits) {
            let [r, g, b] = info.colour(bytes, index);
            *slot = [r, g, b, 255];
        }
    }
    let rgba_len = count.checked_mul(4)?;
    match info.compression {
        Compression::Rgb => {
            if !matches!(info.bits, 1 | 4 | 8 | 24 | 32) {
                return None;
            }
            let (stride, body) = (info.stride()?, info.pixels(bytes)?);
            let mut rgba = Vec::new();
            rgba.try_reserve_exact(rgba_len).ok()?;
            for row in 0..rows {
                let line = body.get(info.file_row(row)?.checked_mul(stride)?..)?.get(..stride)?;
                for col in 0..cols {
                    let px = match info.bits {
                        24 => {
                            let p = line.get(col * 3..col * 3 + 3)?;
                            [p[2], p[1], p[0], 255]
                        }
                        32 => {
                            let p = line.get(col * 4..col * 4 + 3)?;
                            [p[2], p[1], p[0], 255]
                        }
                        8 => table[usize::from(*line.get(col)?)],
                        4 => {
                            let byte = *line.get(col / 2)?;
                            table[usize::from(if col % 2 == 0 { byte >> 4 } else { byte & 0x0f })]
                        }
                        _ => table[usize::from((*line.get(col / 8)? >> (7 - col % 8)) & 1)],
                    };
                    rgba.extend_from_slice(&px);
                }
            }
            Some((info.width, info.height, rgba))
        }
        Compression::Rle8 | Compression::Rle4 => {
            let data = bytes.get(info.data_at()?..)?;
            // Every pair of bytes paints at most 255 pixels, and a delta skips at most 255 x 255: a
            // stream this short cannot account for a picture this large.
            if count > data.len().saturating_mul(1 << 14) {
                return None;
            }
            let indices = rle_indices(data, cols, rows, info.compression == Compression::Rle4)?;
            let mut rgba = Vec::new();
            rgba.try_reserve_exact(rgba_len).ok()?;
            for row in 0..rows {
                let line = indices.get(info.file_row(row)?.checked_mul(cols)?..)?.get(..cols)?;
                for index in line {
                    rgba.extend_from_slice(&table[usize::from(*index)]);
                }
            }
            Some((info.width, info.height, rgba))
        }
        Compression::Bitfields | Compression::Other(_) => None,
    }
}

#[cfg(test)]
mod tests;

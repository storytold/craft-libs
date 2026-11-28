//! Apple Macintosh PICT (QuickDraw picture) reader, written from the picture format Apple
//! publishes in *Inside Macintosh: Imaging With QuickDraw*, appendix A ("Picture Opcodes").
//!
//! A picture is a 10-byte header (a 16-bit size and the frame rectangle) followed by opcodes:
//! one byte each in version 1 pictures, and two bytes, word-aligned, in version 2 and extended
//! version 2 pictures. Many files start with a 512-byte application header, which is skipped.
//!
//! Read: the pixel images of BitsRect, BitsRgn, PackBitsRect and PackBitsRgn (a 1-bit bitmap, or
//! a 1, 2, 4 or 8-bit indexed pixel map with its colour table) and of DirectBitsRect and
//! DirectBitsRgn (16-bit 5-5-5 and 32-bit RGB, with an alpha plane when one holds values), stored
//! or PackBits compressed (pack types 0 to 4). Several pixel images, as in a banded picture, are
//! drawn in picture order where the picture places them, on white. Every other opcode is skipped
//! by its documented length. Vector drawing (lines, shapes, regions, text) and QuickTime-compressed
//! images are not drawn: a picture with only those is a clear `Unsupported` error, and one that
//! also has a pixel image opens with [`DecodeWarning::DrawingSkipped`]. The mask regions of the
//! `...Rgn` opcodes and the transfer modes are not applied. Read-only: there is no PICT writer.

use crate::error::CodecError;
use crate::format::Format;
use crate::image::{ChannelLayout, DecodeWarning, Image, SampleType};
use crate::options::Limits;

const F: Format = Format::Pict;
/// The application header some PICT files start with; it is not part of the picture.
const FILE_HEADER: usize = 512;
/// The most opcodes read from one picture.
const MAX_OPCODES: u32 = 1 << 24;
/// The most pixel images drawn into one picture.
const MAX_IMAGES: usize = 4096;
/// Several pixel images are drawn on a canvas at most this many times the size of their pixels,
/// so a few small images placed far apart can't ask for a huge canvas.
const MAX_CANVAS_RATIO: u64 = 4;

fn bad(msg: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, msg)
}

fn unsupported(msg: impl Into<String>) -> CodecError {
    CodecError::unsupported(F, msg)
}

fn short() -> CodecError {
    bad("the picture data ends early")
}

/// Byte opcodes (version 1) or word opcodes (version 2 and extended version 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Version {
    V1,
    V2,
}

/// Where the picture starts (after a 512-byte file header, or at the first byte) and its version.
pub(crate) fn sniff(b: &[u8]) -> Option<(usize, Version)> {
    [FILE_HEADER, 0].into_iter().find_map(|at| {
        let after = b.get(at.checked_add(10)?..)?;
        if after.starts_with(&[0x00, 0x11, 0x02, 0xFF, 0x0C, 0x00]) {
            Some((at, Version::V2))
        } else if after.starts_with(&[0x11, 0x01]) && v1_plausible(b, at) {
            Some((at, Version::V1))
        } else {
            None
        }
    })
}

/// A version 1 signature is only two bytes, so the frame must also be a rectangle and the next
/// byte a version 1 opcode before the bytes count as a picture.
fn v1_plausible(b: &[u8], at: usize) -> bool {
    let word = |i: usize| b.get(at + i..at + i + 2).and_then(|s| s.try_into().ok()).map(i16::from_be_bytes);
    let (Some(top), Some(left), Some(bottom), Some(right)) = (word(2), word(4), word(6), word(8)) else {
        return false;
    };
    top < bottom && left < right && b.get(at + 12).is_some_and(|&op| op <= 0xA1 || op == 0xFF)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    top: i16,
    left: i16,
    bottom: i16,
    right: i16,
}

impl Rect {
    fn width(self) -> i32 {
        i32::from(self.right) - i32::from(self.left)
    }

    fn height(self) -> i32 {
        i32::from(self.bottom) - i32::from(self.top)
    }

    fn intersect(self, o: Rect) -> Rect {
        Rect { top: self.top.max(o.top), left: self.left.max(o.left), bottom: self.bottom.min(o.bottom), right: self.right.min(o.right) }
    }

    fn union(self, o: Rect) -> Rect {
        Rect { top: self.top.min(o.top), left: self.left.min(o.left), bottom: self.bottom.max(o.bottom), right: self.right.max(o.right) }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        let end = self.pos.checked_add(n).ok_or_else(short)?;
        let s = self.b.get(self.pos..end).ok_or_else(short)?;
        self.pos = end;
        Ok(s)
    }

    fn skip(&mut self, n: usize) -> Result<(), CodecError> {
        self.take(n).map(drop)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        self.take(N)?.try_into().map_err(|_| short())
    }

    fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(u8::from_be_bytes(self.array()?))
    }

    fn u16(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_be_bytes(self.array()?))
    }

    fn i16(&mut self) -> Result<i16, CodecError> {
        Ok(i16::from_be_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_be_bytes(self.array()?))
    }

    fn rect(&mut self) -> Result<Rect, CodecError> {
        Ok(Rect { top: self.i16()?, left: self.i16()?, bottom: self.i16()?, right: self.i16()? })
    }

    /// A region or polygon, whose first word is its size in bytes, the word included.
    fn skip_sized(&mut self) -> Result<(), CodecError> {
        let size = usize::from(self.u16()?);
        self.skip(size.checked_sub(2).ok_or_else(|| bad("a region or polygon is shorter than its size word"))?)
    }

    /// Data that a 16-bit byte count precedes.
    fn skip_counted(&mut self) -> Result<(), CodecError> {
        let n = self.u16()?;
        self.skip(n.into())
    }

    /// Data that a 32-bit byte count precedes.
    fn skip_long_counted(&mut self) -> Result<(), CodecError> {
        let n = usize::try_from(self.u32()?).map_err(|_| short())?;
        self.skip(n)
    }

    /// The text of a text opcode: a count byte and that many characters.
    fn skip_text(&mut self) -> Result<(), CodecError> {
        let n = self.u8()?;
        self.skip(n.into())
    }
}

/// A 16.16 fixed-point number.
fn fixed(v: u32) -> f32 {
    (f64::from(v) / 65536.0) as f32
}

/// A 16-bit QuickDraw colour component as 8 bits, rounded.
fn channel(v: u16) -> u8 {
    u8::try_from((u32::from(v) * 255 + 32767) / 65535).unwrap_or(u8::MAX)
}

/// What decoding needs from a bitmap or pixel map header.
struct PixMap {
    row_bytes: usize,
    bounds: Rect,
    pack_type: u16,
    pixel_size: u16,
    cmp_count: u16,
    /// Horizontal and vertical resolution in pixels per inch.
    res: (f32, f32),
}

/// A pixel map after its row-bytes word (and, for direct pixels, the base address before it).
fn pixmap(r: &mut Reader, row_bytes: u16) -> Result<PixMap, CodecError> {
    let bounds = r.rect()?;
    r.skip(2)?; // pmVersion
    let pack_type = r.u16()?;
    r.skip(4)?; // packSize
    let res = (fixed(r.u32()?), fixed(r.u32()?));
    r.skip(2)?; // pixelType
    let pixel_size = r.u16()?;
    let cmp_count = r.u16()?;
    r.skip(2 + 4 + 4 + 4)?; // cmpSize, planeBytes, pmTable, pmReserved
    Ok(PixMap { row_bytes: usize::from(row_bytes & 0x3FFF), bounds, pack_type, pixel_size, cmp_count, res })
}

/// A colour table: the colour of every index (black where the table sets none) and how many
/// leading indices it sets.
struct Palette {
    colors: Vec<[u8; 3]>,
    len: usize,
}

fn color_table(r: &mut Reader) -> Result<Palette, CodecError> {
    r.skip(4)?; // ctSeed
    let flags = r.u16()?;
    // ctSize is the number of entries less one, so 0xFFFF is an empty table.
    let count = usize::from(r.u16()?.wrapping_add(1));
    if count > 256 {
        return Err(bad(format!("a colour table of {count} entries (at most 256)")));
    }
    let mut colors = vec![[0u8; 3]; 256];
    let mut len = 0;
    for i in 0..count {
        let value = usize::from(r.u16()?);
        let rgb = [channel(r.u16()?), channel(r.u16()?), channel(r.u16()?)];
        // A device table (the flags' high bit) lists its colours in index order and ignores `value`.
        let index = if flags & 0x8000 != 0 { i } else { value };
        if let Some(slot) = colors.get_mut(index) {
            *slot = rgb;
            len = len.max(index + 1);
        }
    }
    Ok(Palette { colors, len })
}

/// How one unpacked row holds its pixels.
#[derive(Clone, Copy)]
enum Pixels {
    /// 1, 2, 4 or 8 bits per pixel, looked up in the colour table.
    Indexed(u16),
    /// A QuickDraw bitmap: 1 bit per pixel, 0 white and 1 black.
    Bitmap,
    /// 16 bits: a pad bit, then 5 bits each of red, green and blue.
    Rgb555,
    /// 32 bits as they are in memory: alpha (a pad byte with fewer than four components), red,
    /// green, blue.
    Argb { alpha: bool },
    /// 32 bits stored without the pad byte (pack type 2): red, green, blue.
    Rgb24,
    /// 32 bits as one plane per component (pack type 4): alpha (with four components), red,
    /// green, blue.
    Planar { alpha: bool },
}

/// How the rows are stored.
#[derive(Clone, Copy)]
enum Rows {
    /// Stored as they are, this many bytes each.
    Stored(usize),
    /// PackBits compressed, each row after its byte count, in units of one byte or (pack type 3)
    /// one 16-bit word, unpacking to at most `max` bytes.
    Packed { unit: usize, max: usize },
}

struct Layout {
    rows: Rows,
    pixels: Pixels,
    /// The unpacked bytes a row needs for its pixels.
    need: usize,
}

impl Layout {
    fn new(rows: Rows, pixels: Pixels, need: usize) -> Result<Self, CodecError> {
        let room = match rows {
            Rows::Stored(n) => n,
            Rows::Packed { max, .. } => max,
        };
        if need > room {
            return Err(bad(format!("rows of {room} bytes are too short for their {need} bytes of pixels")));
        }
        Ok(Layout { rows, pixels, need })
    }
}

fn row_size(width: usize, bits: usize) -> Result<usize, CodecError> {
    Ok(width.checked_mul(bits).ok_or_else(|| bad("a row size overflows"))?.div_ceil(8))
}

/// The rows of a bitmap or indexed pixel map. PackBits applies only when rows are 8 bytes or more.
fn indexed_layout(pm: &PixMap, packed: bool, bitmap: bool, width: usize) -> Result<Layout, CodecError> {
    let bits = pm.pixel_size;
    if !matches!(bits, 1 | 2 | 4 | 8) {
        return Err(unsupported(format!("{bits}-bit indexed pixels (1, 2, 4 or 8)")));
    }
    let rows = if packed && pm.row_bytes >= 8 { Rows::Packed { unit: 1, max: pm.row_bytes } } else { Rows::Stored(pm.row_bytes) };
    Layout::new(rows, if bitmap { Pixels::Bitmap } else { Pixels::Indexed(bits) }, row_size(width, usize::from(bits))?)
}

/// The rows of a direct pixel map, by pixel size and pack type (0 is the default packing: 3 for
/// 16-bit pixels, 4 for 32-bit ones). Rows under 8 bytes are always stored.
fn direct_layout(pm: &PixMap, width: usize) -> Result<Layout, CodecError> {
    let stored = pm.row_bytes < 8 || pm.pack_type == 1;
    let (size, pack) = (pm.pixel_size, pm.pack_type);
    match size {
        16 if stored => Layout::new(Rows::Stored(pm.row_bytes), Pixels::Rgb555, row_size(width, 16)?),
        16 if matches!(pack, 0 | 3) => Layout::new(Rows::Packed { unit: 2, max: pm.row_bytes }, Pixels::Rgb555, row_size(width, 16)?),
        32 if stored => Layout::new(Rows::Stored(pm.row_bytes), Pixels::Argb { alpha: pm.cmp_count == 4 }, row_size(width, 32)?),
        32 if pack == 2 => {
            let n = row_size(width, 24)?;
            Layout::new(Rows::Stored(n), Pixels::Rgb24, n)
        }
        32 if matches!(pack, 0 | 4) => {
            if !matches!(pm.cmp_count, 3 | 4) {
                return Err(unsupported(format!("{} components in a 32-bit pixel map (3 or 4)", pm.cmp_count)));
            }
            let n = row_size(width, 8 * usize::from(pm.cmp_count))?;
            Layout::new(Rows::Packed { unit: 1, max: n }, Pixels::Planar { alpha: pm.cmp_count == 4 }, n)
        }
        16 | 32 => Err(unsupported(format!("pack type {pack} for {size}-bit pixels"))),
        _ => Err(unsupported(format!("{size}-bit direct pixels (16 or 32)"))),
    }
}

/// Unpack PackBits data in units of `unit` bytes, appending at most `max` bytes to `out`.
/// Returns how many bytes it appended.
fn unpack(src: &[u8], unit: usize, max: usize, out: &mut Vec<u8>) -> Result<usize, CodecError> {
    let start = out.len();
    let mut i = 0;
    while let Some(&flag) = src.get(i) {
        i += 1;
        let flag = i8::from_be_bytes([flag]);
        if flag == -128 {
            continue; // a no-op
        }
        // 0..=127: that many plus one literal units; -1..=-127: one unit repeated 1 - flag times.
        let count = usize::from(flag.unsigned_abs()) + 1;
        let literal = flag >= 0;
        let n = if literal { count * unit } else { unit };
        let data = src.get(i..i + n).ok_or_else(|| bad("a PackBits run ends early"))?;
        i += n;
        if out.len() - start + count * unit > max {
            return Err(bad("a row unpacks to more bytes than it holds"));
        }
        if literal {
            out.extend_from_slice(data);
        } else {
            for _ in 0..count {
                out.extend_from_slice(data);
            }
        }
    }
    Ok(out.len() - start)
}

/// Read one row; with `row`, unpack it there (replacing what it held).
fn read_row(r: &mut Reader, layout: &Layout, long_count: bool, row: Option<&mut Vec<u8>>) -> Result<(), CodecError> {
    match layout.rows {
        Rows::Stored(n) => {
            let data = r.take(n)?;
            if let Some(row) = row {
                row.clear();
                row.extend_from_slice(data);
            }
        }
        Rows::Packed { unit, max } => {
            // The byte count is a word when rows are over 250 bytes, otherwise a byte.
            let count = if long_count { usize::from(r.u16()?) } else { usize::from(r.u8()?) };
            let data = r.take(count)?;
            if let Some(row) = row {
                row.clear();
                row.try_reserve(max).map_err(|_| CodecError::LimitExceeded("a PICT row does not fit in memory".into()))?;
                if unpack(data, unit, max, row)? < layout.need {
                    return Err(bad("a row unpacks to fewer bytes than its pixels need"));
                }
            }
        }
    }
    Ok(())
}

fn index(row: &[u8], bits: usize, x: usize) -> Option<usize> {
    let bit = x.checked_mul(bits)?;
    let byte = *row.get(bit / 8)?;
    // `bits` divides 8, so the pixel never crosses a byte.
    let shift = 8usize.checked_sub(bits)?.checked_sub(bit % 8)?;
    Some(usize::from(byte >> shift) & ((1 << bits) - 1))
}

fn five(v: u16) -> u8 {
    let v = u8::try_from(v & 0x1F).unwrap_or(0);
    (v << 3) | (v >> 2)
}

/// Pixel `x` of an unpacked row as RGBA (`width` is the row's width, for planar rows).
fn pixel(row: &[u8], pixels: Pixels, width: usize, x: usize, colors: &[[u8; 3]]) -> Option<[u8; 4]> {
    Some(match pixels {
        Pixels::Bitmap => {
            if index(row, 1, x)? == 0 {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        }
        Pixels::Indexed(bits) => {
            let [r, g, b] = colors.get(index(row, usize::from(bits), x)?).copied().unwrap_or([0, 0, 0]);
            [r, g, b, 255]
        }
        Pixels::Rgb555 => {
            let v = u16::from_be_bytes([*row.get(2 * x)?, *row.get(2 * x + 1)?]);
            [five(v >> 10), five(v >> 5), five(v), 255]
        }
        Pixels::Argb { alpha } => {
            let p = row.get(4 * x..4 * x + 4)?;
            [p[1], p[2], p[3], if alpha { p[0] } else { 255 }]
        }
        Pixels::Rgb24 => {
            let p = row.get(3 * x..3 * x + 3)?;
            [p[0], p[1], p[2], 255]
        }
        Pixels::Planar { alpha } => {
            let at = |plane: usize| row.get(plane * width + x).copied();
            if alpha { [at(1)?, at(2)?, at(3)?, at(0)?] } else { [at(0)?, at(1)?, at(2)?, 255] }
        }
    })
}

/// One pixel image of the picture: the pixels of its source rectangle as RGBA (none when only
/// the picture's structure is read), the rectangle the picture draws them in, and its colour
/// table when it has one.
struct Placed {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    alpha: bool,
    dst: Rect,
    res: (f32, f32),
    palette: Option<Vec<[u8; 3]>>,
}

/// A pixel image's header, read up to its pixel data.
struct Source<'a> {
    pm: PixMap,
    layout: Layout,
    colors: &'a [[u8; 3]],
    src: Rect,
    dst: Rect,
}

fn dims(bounds: Rect) -> Result<(usize, usize), CodecError> {
    match (usize::try_from(bounds.width()), usize::try_from(bounds.height())) {
        (Ok(w), Ok(h)) if w > 0 && h > 0 => Ok((w, h)),
        _ => Err(bad(format!("a pixel image of {}x{} pixels", bounds.width(), bounds.height()))),
    }
}

/// The source and destination rectangles, the transfer mode and, for the `...Rgn` opcodes, the
/// mask region (not applied).
fn placement(r: &mut Reader, op: u16) -> Result<(Rect, Rect), CodecError> {
    let (src, dst) = (r.rect()?, r.rect()?);
    r.skip(2)?; // transfer mode
    if op & 1 != 0 {
        r.skip_sized()?;
    }
    Ok((src, dst))
}

/// Read a pixel image's rows, keeping (with `decode`) the pixels of its source rectangle.
fn read_image(r: &mut Reader, s: &Source, decode: bool, limits: &Limits) -> Result<Placed, CodecError> {
    let bounds = s.pm.bounds;
    let (width, height) = dims(bounds)?;
    let crop = s.src.intersect(bounds);
    let (Ok(cw), Ok(ch)) = (u32::try_from(crop.width()), u32::try_from(crop.height())) else {
        return Err(bad("the source rectangle lies outside the pixel image"));
    };
    if cw == 0 || ch == 0 {
        return Err(bad("the source rectangle lies outside the pixel image"));
    }
    let x0 = usize::try_from(i32::from(crop.left) - i32::from(bounds.left)).map_err(|_| bad("a source rectangle is out of range"))?;
    let rows = i32::from(crop.top) - i32::from(bounds.top)..i32::from(crop.bottom) - i32::from(bounds.top);
    if decode {
        limits.check(cw, ch, ChannelLayout::Rgba, SampleType::U8)?;
    }
    let (mut out, mut row) = (Vec::new(), Vec::new());
    let long_count = s.pm.row_bytes > 250;
    for y in 0..height {
        let keep = decode && i32::try_from(y).is_ok_and(|y| rows.contains(&y));
        read_row(r, &s.layout, long_count, keep.then_some(&mut row))?;
        if keep {
            out.try_reserve(cw as usize * 4).map_err(|_| CodecError::LimitExceeded("the PICT image does not fit in memory".into()))?;
            for x in x0..x0 + cw as usize {
                out.extend_from_slice(&pixel(&row, s.layout.pixels, width, x, s.colors).ok_or_else(|| bad("a row is too short for its pixels"))?);
            }
        }
    }
    let alpha = matches!(s.layout.pixels, Pixels::Argb { alpha: true } | Pixels::Planar { alpha: true }) && real_alpha(&mut out);
    Ok(Placed { width: cw, height: ch, rgba: out, alpha, dst: s.dst, res: s.pm.res, palette: None })
}

/// Whether an alpha plane holds anything: one that is all 0 (unused, as many writers leave the
/// fourth component) or all 255 is made opaque.
fn real_alpha(rgba: &mut [u8]) -> bool {
    let Some(first) = rgba.get(3).copied() else { return false };
    if matches!(first, 0 | 255) && rgba.as_chunks::<4>().0.iter().all(|p| p[3] == first) {
        rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
        return false;
    }
    true
}

/// BitsRect, BitsRgn, PackBitsRect, PackBitsRgn: a bitmap, or (version 2) an indexed pixel map.
fn bits_op(r: &mut Reader, op: u16, version: Version, decode: bool, limits: &Limits) -> Result<Placed, CodecError> {
    let word = r.u16()?;
    let (pm, table) = if word & 0x8000 != 0 {
        if version == Version::V1 {
            return Err(bad("a version 1 picture holds a colour pixel map"));
        }
        let pm = pixmap(r, word)?;
        (pm, Some(color_table(r)?))
    } else {
        let bounds = r.rect()?;
        (PixMap { row_bytes: usize::from(word), bounds, pack_type: 0, pixel_size: 1, cmp_count: 1, res: (72.0, 72.0) }, None)
    };
    let layout = indexed_layout(&pm, op & 0x08 != 0, table.is_none(), dims(pm.bounds)?.0)?;
    let (src, dst) = placement(r, op)?;
    let colors = table.as_ref().map_or(&[][..], |t| &t.colors);
    let mut placed = read_image(r, &Source { pm, layout, colors, src, dst }, decode, limits)?;
    placed.palette = Some(match &table {
        Some(t) => t.colors.get(..t.len).unwrap_or_default().to_vec(),
        None => vec![[255, 255, 255], [0, 0, 0]],
    });
    Ok(placed)
}

/// DirectBitsRect, DirectBitsRgn: a 16 or 32-bit pixel map.
fn direct_op(r: &mut Reader, op: u16, decode: bool, limits: &Limits) -> Result<Placed, CodecError> {
    r.skip(4)?; // baseAddr
    let word = r.u16()?;
    let pm = pixmap(r, word)?;
    let layout = direct_layout(&pm, dims(pm.bounds)?.0)?;
    let (src, dst) = placement(r, op)?;
    read_image(r, &Source { pm, layout, colors: &[], src, dst }, decode, limits)
}

/// BkPixPat, PnPixPat, FillPixPat: a pattern, skipped (a full-colour one holds a pixel map).
fn skip_pix_pat(r: &mut Reader) -> Result<(), CodecError> {
    let kind = r.u16()?;
    r.skip(8)?; // the 1-bit pattern
    match kind {
        1 => {
            let word = r.u16()?;
            let pm = pixmap(r, word)?;
            color_table(r)?;
            let (width, height) = dims(pm.bounds)?;
            let layout = indexed_layout(&pm, true, false, width)?;
            for _ in 0..height {
                read_row(r, &layout, pm.row_bytes > 250, None)?;
            }
        }
        2 => r.skip(6)?, // the RGB colour of a dithered pattern
        _ => {}
    }
    Ok(())
}

/// The data length of the opcodes whose data has a fixed size.
fn fixed_len(op: u16) -> usize {
    match op {
        0x0004 => 1,
        0x0003 | 0x0005 | 0x0008 | 0x000D | 0x0011 | 0x0015 | 0x0016 | 0x0023 | 0x00A0 => 2,
        0x0006 | 0x0007 | 0x000B | 0x000C | 0x000E | 0x000F | 0x0021 | 0x0068..=0x006F => 4,
        0x001A | 0x001B | 0x001D | 0x001F | 0x0022 => 6,
        0x0002 | 0x0009 | 0x000A | 0x0010 | 0x0020 | 0x0030..=0x0037 | 0x0040..=0x0047 | 0x0050..=0x0057 => 8,
        0x0060..=0x0067 => 12,
        // The version opcode's data, met as an opcode, is 2 bytes; reserved opcodes from 0x0100
        // to 0x7FFF hold twice their high byte.
        0x0100..=0x7FFF if op == 0x02FF => 2,
        0x0100..=0x7FFF => usize::from(op >> 8) * 2,
        // NOP, the opcodes that repeat the last shape, and the reserved 0x00B0-0x00CF and
        // 0x8000-0x80FF have no data.
        _ => 0,
    }
}

/// Skip an opcode this reader doesn't draw, by its documented data length. Returns whether it
/// draws something (lines, shapes, regions, text), which the decoded image then lacks.
fn skip(r: &mut Reader, op: u16, version: Version) -> Result<bool, CodecError> {
    match op {
        0x0011 if version == Version::V1 => r.skip(1)?,
        0x0001 | 0x0070..=0x0077 | 0x0080..=0x0087 => r.skip_sized()?,
        0x0024..=0x0027 | 0x002C..=0x002F | 0x0092..=0x0097 | 0x009C..=0x009F | 0x00A2..=0x00AF => r.skip_counted()?,
        0x0028 => {
            r.skip(4)?;
            r.skip_text()?;
        }
        0x0029 | 0x002A => {
            r.skip(1)?;
            r.skip_text()?;
        }
        0x002B => {
            r.skip(2)?;
            r.skip_text()?;
        }
        0x00A1 => {
            r.skip(2)?;
            r.skip_counted()?;
        }
        0x00D0..=0x00FE | 0x8100..=0xFFFF => r.skip_long_counted()?,
        _ => r.skip(fixed_len(op))?,
    }
    Ok(matches!(op, 0x0020..=0x0023 | 0x0028..=0x002B | 0x0030..=0x008F))
}

/// The extended version 2 header's resolution; the version 2 header has none.
fn header(r: &mut Reader) -> Result<Option<(f32, f32)>, CodecError> {
    let version = r.u16()?;
    r.skip(2)?; // reserved
    if version != 0xFFFE {
        r.skip(20)?;
        return Ok(None);
    }
    let res = (fixed(r.u32()?), fixed(r.u32()?));
    r.skip(8 + 4)?; // the optimal source rectangle, reserved
    Ok(Some(res).filter(|(h, v)| *h > 0.0 && *v > 0.0))
}

#[derive(Default)]
struct Picture {
    images: Vec<Placed>,
    /// Drawing opcodes and QuickTime images skipped.
    skipped: u32,
    quicktime: bool,
    ended: bool,
    dpi: Option<(f32, f32)>,
    bytes: u64,
}

impl Picture {
    fn add(&mut self, image: Placed, limits: &Limits) -> Result<(), CodecError> {
        if self.images.len() >= MAX_IMAGES {
            return Err(CodecError::LimitExceeded(format!("more than {MAX_IMAGES} pixel images in one picture")));
        }
        self.bytes = self.bytes.saturating_add(u64::try_from(image.rgba.len()).unwrap_or(u64::MAX));
        if self.bytes > limits.max_alloc {
            return Err(CodecError::LimitExceeded(format!("{} bytes of pixel images exceed max_alloc {}", self.bytes, limits.max_alloc)));
        }
        self.images.push(image);
        Ok(())
    }

    fn opcode(&mut self, r: &mut Reader, op: u16, version: Version, decode: bool, limits: &Limits) -> Result<(), CodecError> {
        match op {
            0x0090 | 0x0091 | 0x0098 | 0x0099 => {
                let image = bits_op(r, op, version, decode, limits)?;
                self.add(image, limits)?;
            }
            0x009A | 0x009B if version == Version::V2 => {
                let image = direct_op(r, op, decode, limits)?;
                self.add(image, limits)?;
            }
            0x009A | 0x009B => return Err(bad("a version 1 picture holds direct pixels")),
            0x0C00 if version == Version::V2 => self.dpi = header(r)?,
            0x0012..=0x0014 if version == Version::V2 => skip_pix_pat(r)?,
            // CompressedQuickTime, UncompressedQuickTime: an image only QuickTime decodes.
            0x8200 | 0x8201 => {
                r.skip_long_counted()?;
                self.quicktime = true;
                self.skipped = self.skipped.saturating_add(1);
            }
            _ => {
                if skip(r, op, version)? {
                    self.skipped = self.skipped.saturating_add(1);
                }
            }
        }
        Ok(())
    }

    /// Why a picture without a pixel image doesn't open.
    fn no_image(&self) -> CodecError {
        if self.quicktime {
            unsupported("QuickTime-compressed images (such as JPEG inside a PICT) are not supported")
        } else if self.skipped > 0 {
            unsupported(format!(
                "the picture holds only QuickDraw drawing ({} operations: lines, shapes, regions or text), which is not rendered; only pictures with a pixel image open",
                self.skipped
            ))
        } else if !self.ended {
            bad("the picture ends before its first image")
        } else {
            bad("the picture holds no image")
        }
    }
}

/// Read the picture's opcodes; with `decode`, keep its pixel images' pixels.
fn parse(b: &[u8], decode: bool, limits: &Limits) -> Result<Picture, CodecError> {
    let (start, version) = sniff(b).ok_or_else(|| bad("no version 1 or version 2 picture header"))?;
    // The picture size, the frame and the version opcode with its data.
    let mut r = Reader { b, pos: start + if version == Version::V2 { 14 } else { 12 } };
    let mut pic = Picture::default();
    for _ in 0..MAX_OPCODES {
        // Version 2 opcodes start on a word boundary.
        if version == Version::V2 && (r.pos - start) % 2 == 1 {
            r.pos += 1;
        }
        if r.pos >= b.len() {
            return Ok(pic);
        }
        let op = match version {
            Version::V2 => r.u16()?,
            Version::V1 => u16::from(r.u8()?),
        };
        if op == 0x00FF {
            pic.ended = true;
            return Ok(pic);
        }
        pic.opcode(&mut r, op, version, decode, limits)?;
    }
    Err(CodecError::LimitExceeded(format!("more than {MAX_OPCODES} opcodes in one picture")))
}

/// Draw several pixel images where the picture places them, on white. They must share one scale
/// from picture coordinates to pixels (as the bands of a banded picture do).
fn compose(images: &[Placed], limits: &Limits) -> Result<(u32, u32, Vec<u8>), CodecError> {
    let (Some(first), Some(all)) = (images.first(), images.iter().map(|i| i.dst).reduce(Rect::union)) else {
        return Err(bad("the picture holds no image"));
    };
    let (nx, dx, ny, dy) = (i64::from(first.width), i64::from(first.dst.width()), i64::from(first.height), i64::from(first.dst.height()));
    if dx <= 0 || dy <= 0 {
        return Err(bad("a pixel image is drawn into an empty rectangle"));
    }
    let scaled = |v: i32, n: i64, d: i64| {
        let p = i64::from(v) * n;
        if p % d == 0 { Ok(p / d) } else { Err(unsupported("pixel images drawn at different scales")) }
    };
    let (Ok(cw), Ok(ch)) = (u32::try_from(scaled(all.width(), nx, dx)?), u32::try_from(scaled(all.height(), ny, dy)?)) else {
        return Err(bad("the pixel images span no canvas"));
    };
    limits.check(cw, ch, ChannelLayout::Rgba, SampleType::U8)?;
    let size = u64::from(cw) * u64::from(ch) * 4;
    let decoded: u64 = images.iter().map(|i| u64::try_from(i.rgba.len()).unwrap_or(u64::MAX)).fold(0, u64::saturating_add);
    Limits::check_plausible(size, usize::try_from(decoded).unwrap_or(usize::MAX), MAX_CANVAS_RATIO)?;
    let size = usize::try_from(size).map_err(|_| CodecError::LimitExceeded("the PICT canvas does not fit in memory".into()))?;
    let mut canvas = Vec::new();
    canvas.try_reserve_exact(size).map_err(|_| CodecError::LimitExceeded("the PICT canvas does not fit in memory".into()))?;
    canvas.resize(size, 255);
    for image in images {
        if i64::from(image.width) * dx != i64::from(image.dst.width()) * nx || i64::from(image.height) * dy != i64::from(image.dst.height()) * ny {
            return Err(unsupported("pixel images drawn at different scales"));
        }
        let x = scaled(i32::from(image.dst.left) - i32::from(all.left), nx, dx)?;
        let y = scaled(i32::from(image.dst.top) - i32::from(all.top), ny, dy)?;
        blit(&mut canvas, cw, image, x, y);
    }
    Ok((cw, ch, canvas))
}

fn blit(canvas: &mut [u8], cw: u32, image: &Placed, x: i64, y: i64) {
    let (w, cw) = (image.width as usize, cw as usize);
    let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) else { return };
    if w == 0 || x >= cw {
        return;
    }
    let n = w.min(cw - x) * 4;
    for (row, src) in image.rgba.chunks_exact(w * 4).enumerate() {
        let at = ((y + row) * cw + x) * 4;
        if let (Some(dst), Some(src)) = (canvas.get_mut(at..at + n), src.get(..n)) {
            dst.copy_from_slice(src);
        }
    }
}

/// RGBA samples as RGB, in place.
fn drop_alpha(mut rgba: Vec<u8>) -> Vec<u8> {
    let n = rgba.len() / 4;
    for i in 0..n {
        rgba.copy_within(i * 4..i * 4 + 3, i * 3);
    }
    rgba.truncate(n * 3);
    rgba
}

pub(crate) fn decode(b: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let mut pic = parse(b, true, limits)?;
    if pic.images.is_empty() {
        return Err(pic.no_image());
    }
    let alpha = pic.images.iter().any(|i| i.alpha);
    let res = pic.images.first().map(|i| i.res);
    let (w, h, rgba) = match pic.images.len() {
        1 => match pic.images.pop() {
            Some(only) => (only.width, only.height, only.rgba),
            None => return Err(pic.no_image()),
        },
        _ => compose(&pic.images, limits)?,
    };
    let (layout, data) = if alpha { (ChannelLayout::Rgba, rgba) } else { (ChannelLayout::Rgb, drop_alpha(rgba)) };
    let mut image = Image::from_u8(w, h, layout, data)?;
    image.meta.dpi = pic.dpi.or(res.filter(|(h, v)| *h > 0.0 && *v > 0.0));
    if pic.skipped > 0 {
        image.warnings.push(DecodeWarning::DrawingSkipped { format: F, count: pic.skipped });
    }
    if !pic.ended {
        image.warnings.push(DecodeWarning::Truncated { format: F });
    }
    Ok(image)
}

/// The colour table of a picture made of one indexed pixel image (a bitmap's is white and
/// black), read without decoding its pixels.
pub(crate) fn palette(b: &[u8]) -> Result<Option<Vec<[u8; 3]>>, CodecError> {
    let pic = parse(b, false, &Limits::default())?;
    Ok(match pic.images.as_slice() {
        [only] => only.palette.clone().filter(|p| !p.is_empty()),
        _ => None,
    })
}

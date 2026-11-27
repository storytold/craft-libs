//! SGI image files (`.sgi`, `.rgb`, `.rgba`, `.bw`): 8- and 16-bit gray, gray + alpha, RGB and
//! RGBA, verbatim or run-length encoded.
//!
//! Read and write follow the published SGI image file format description: a 512-byte big-endian
//! header, then one plane per channel with the rows stored bottom to top. Palette-mapped files
//! (colormap ID other than 0) are rejected. The writer uses the run-length encoding.

use crate::error::CodecError;
use crate::fidelity::Plan;
use crate::format::Format;
use crate::image::{ChannelLayout, Image, SampleType};
use crate::options::Limits;

const F: Format = Format::Sgi;
const HEADER: usize = 512;
const MAGIC: u16 = 474;

fn bad(reason: impl std::fmt::Display) -> CodecError {
    CodecError::malformed(F, reason)
}

fn be16(b: &[u8], i: usize) -> u16 {
    u16::from_be_bytes([b[i], b[i + 1]])
}

fn be32(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// The fields of the header that decoding needs, already checked for plausibility.
struct Header {
    rle: bool,
    wide: bool,
    width: u32,
    height: u32,
    planes: usize,
}

/// Parse the 512-byte header. `None` when the bytes are not a plausible SGI header.
fn header(b: &[u8]) -> Option<Header> {
    if b.len() < HEADER || be16(b, 0) != MAGIC {
        return None;
    }
    let (storage, bpc, dimension) = (b[2], b[3], be16(b, 4));
    let (width, height, planes) = (u32::from(be16(b, 6)), u32::from(be16(b, 8)), usize::from(be16(b, 10)));
    if storage > 1 || !(1..=2).contains(&bpc) || !(1..=3).contains(&dimension) {
        return None;
    }
    // A one-dimensional image has one row, a two-dimensional one a single plane.
    let (height, planes) = match dimension {
        1 => (1, 1),
        2 => (height, 1),
        _ => (height, planes),
    };
    if width == 0 || height == 0 || !(1..=4).contains(&planes) {
        return None;
    }
    Some(Header { rle: storage == 1, wide: bpc == 2, width, height, planes })
}

/// Whether `b` starts with a plausible SGI header (the magic number plus consistent fields).
pub(crate) fn looks_like_sgi(b: &[u8]) -> bool {
    header(b).is_some()
}

fn layout_of(planes: usize) -> ChannelLayout {
    match planes {
        1 => ChannelLayout::Gray,
        2 => ChannelLayout::GrayA,
        3 => ChannelLayout::Rgb,
        _ => ChannelLayout::Rgba,
    }
}

/// Expand one run-length encoded row into exactly `out.len()` samples.
fn read_rle_row(src: &[u8], wide: bool, out: &mut [u16]) -> Result<(), CodecError> {
    let step = if wide { 2 } else { 1 };
    let sample = |pos: usize| -> Result<u16, CodecError> {
        let s = src.get(pos..pos + step).ok_or_else(|| bad("truncated RLE row"))?;
        Ok(if wide { u16::from_be_bytes([s[0], s[1]]) } else { u16::from(s[0]) })
    };
    let (mut pos, mut n) = (0usize, 0usize);
    while n < out.len() {
        let code = sample(pos)?;
        pos += step;
        let count = usize::from(code & 0x7f);
        if count == 0 {
            return Err(bad("RLE row ends before the image width"));
        }
        if count > out.len() - n {
            return Err(bad("RLE packet runs past the end of the row"));
        }
        if code & 0x80 != 0 {
            for slot in &mut out[n..n + count] {
                *slot = sample(pos)?;
                pos += step;
            }
        } else {
            let value = sample(pos)?;
            pos += step;
            out[n..n + count].fill(value);
        }
        n += count;
    }
    Ok(())
}

pub(crate) fn decode(b: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    let h = header(b).ok_or_else(|| bad("invalid SGI header"))?;
    if be32(b, 104) != 0 {
        return Err(CodecError::unsupported(F, "palette-mapped SGI files are not supported"));
    }
    let layout = layout_of(h.planes);
    let sample = if h.wide { SampleType::U16 } else { SampleType::U8 };
    limits.check(h.width, h.height, layout, sample)?;
    let (w, ht, planes) = (h.width as usize, h.height as usize, h.planes);
    let rows = ht.checked_mul(planes).ok_or_else(|| bad("image size overflow"))?;
    let bps = if h.wide { 2 } else { 1 };
    let total = w.checked_mul(rows).ok_or_else(|| bad("image size overflow"))?;
    // Reject inputs that cannot hold the declared pixels before allocating anything for them.
    let table = if h.rle { rows.checked_mul(8).ok_or_else(|| bad("image size overflow"))? } else { 0 };
    let min_len = if h.rle {
        // Both offset tables must fit; rows may share compressed data, so the data itself is
        // checked row by row against the offsets below.
        HEADER + table
    } else {
        HEADER + total.checked_mul(bps).ok_or_else(|| bad("image size overflow"))?
    };
    if b.len() < min_len {
        return Err(bad("the file is too short for the declared image"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(total).map_err(|_| bad("pixel buffer is too large"))?;
    out.resize(total, 0u16);
    let mut row = vec![0u16; w];
    for z in 0..planes {
        for y in 0..ht {
            let index = y + z * ht;
            if h.rle {
                let start = be32(b, HEADER + index * 4) as usize;
                let len = be32(b, HEADER + rows * 4 + index * 4) as usize;
                let end = start
                    .checked_add(len)
                    .filter(|&e| e <= b.len() && start >= HEADER + table)
                    .ok_or_else(|| bad("RLE row lies outside the file"))?;
                read_rle_row(&b[start..end], h.wide, &mut row)?;
            } else {
                let start = HEADER + index * w * bps;
                let s = &b[start..start + w * bps];
                for (x, v) in row.iter_mut().enumerate() {
                    *v = if h.wide { u16::from_be_bytes([s[2 * x], s[2 * x + 1]]) } else { u16::from(s[x]) };
                }
            }
            // Rows are stored bottom to top; planes are separate.
            let top_y = ht - 1 - y;
            for (x, &v) in row.iter().enumerate() {
                out[(top_y * w + x) * planes + z] = v;
            }
        }
    }
    if h.wide {
        Image::from_u16(h.width, h.height, layout, &out)
    } else {
        Image::from_u8(h.width, h.height, layout, out.iter().map(|&v| v as u8).collect())
    }
}

/// Append one run-length encoded row (one packet list and a terminator).
fn write_rle_row(row: &[u16], wide: bool, out: &mut Vec<u8>) {
    let put = |out: &mut Vec<u8>, v: u16| {
        if wide {
            out.extend_from_slice(&v.to_be_bytes());
        } else {
            out.push(v as u8);
        }
    };
    let mut i = 0;
    while i < row.len() {
        let mut run = 1;
        while i + run < row.len() && run < 127 && row[i + run] == row[i] {
            run += 1;
        }
        if run >= 2 {
            put(out, run as u16);
            put(out, row[i]);
            i += run;
        } else {
            // A literal packet extends until the next run of two or more.
            let mut end = i + 1;
            while end < row.len() && end - i < 127 && !(end + 1 < row.len() && row[end] == row[end + 1]) {
                end += 1;
            }
            put(out, 0x80 | (end - i) as u16);
            for &v in &row[i..end] {
                put(out, v);
            }
            i = end;
        }
    }
    put(out, 0);
}

/// Writes a run-length encoded SGI file: 8 or 16 bits per channel, gray, gray + alpha, RGB or RGBA.
pub(crate) fn encode(src: &Image, plan: Plan) -> Result<Vec<u8>, CodecError> {
    let enc = |msg: String| CodecError::encode(F, msg);
    let img = src.converted(plan.layout, plan.sample);
    let wide = match img.sample_type() {
        SampleType::U8 => false,
        SampleType::U16 => true,
        s => return Err(enc(format!("SGI stores 8 or 16 bits per channel, not {s:?}"))),
    };
    let planes = img.layout().channels();
    if !matches!(img.layout(), ChannelLayout::Gray | ChannelLayout::GrayA | ChannelLayout::Rgb | ChannelLayout::Rgba) {
        return Err(enc(format!("no SGI layout for {:?}", img.layout())));
    }
    let (w, ht) = img.dimensions();
    let (Ok(w16), Ok(h16)) = (u16::try_from(w), u16::try_from(ht)) else {
        return Err(enc(format!("dimensions exceed 65535x65535: {w}x{ht}")));
    };
    let (w, ht) = (w as usize, ht as usize);
    let bps = if wide { 2 } else { 1 };
    let data = img.data();
    if data.len() != w * ht * planes * bps {
        return Err(enc("the image has the wrong number of samples".into()));
    }
    let sample_at = |i: usize| -> u16 { if wide { u16::from_ne_bytes([data[2 * i], data[2 * i + 1]]) } else { u16::from(data[i]) } };
    let rows = ht * planes;
    let table = HEADER;
    let mut starts = Vec::with_capacity(rows);
    let mut lens = Vec::with_capacity(rows);
    let mut body = Vec::new();
    let mut row = vec![0u16; w];
    let base = u32::try_from(HEADER + rows * 8).map_err(|_| enc("image too large".into()))?;
    for z in 0..planes {
        for y in 0..ht {
            let src_y = ht - 1 - y; // stored bottom to top
            for (x, v) in row.iter_mut().enumerate() {
                *v = sample_at((src_y * w + x) * planes + z);
            }
            let before = body.len();
            write_rle_row(&row, wide, &mut body);
            let start = u32::try_from(base as usize + before).map_err(|_| enc("image too large".into()))?;
            starts.push(start);
            lens.push((body.len() - before) as u32);
        }
    }
    let mut out = Vec::with_capacity(table + rows * 8 + body.len());
    out.extend_from_slice(&MAGIC.to_be_bytes());
    out.push(1); // RLE
    out.push(bps as u8);
    out.extend_from_slice(&(if planes > 1 { 3u16 } else { 2u16 }).to_be_bytes());
    out.extend_from_slice(&w16.to_be_bytes());
    out.extend_from_slice(&h16.to_be_bytes());
    out.extend_from_slice(&(planes as u16).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes()); // pixmin
    out.extend_from_slice(&(if wide { 65535u32 } else { 255 }).to_be_bytes()); // pixmax
    out.resize(HEADER, 0); // dummy, image name, colormap 0, padding
    for s in &starts {
        out.extend_from_slice(&s.to_be_bytes());
    }
    for l in &lens {
        out.extend_from_slice(&l.to_be_bytes());
    }
    out.extend_from_slice(&body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32, layout: ChannelLayout) -> Image {
        let ch = layout.channels();
        let data = (0..w as usize * h as usize * ch).map(|i| ((i * 7 + i / 5) % 251) as u8).collect();
        Image::from_u8(w, h, layout, data).unwrap()
    }

    #[test]
    fn feature_config_rle_round_trips_every_layout_in_8_bits() {
        for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
            let img = gradient(37, 11, layout);
            let plan = Plan { layout, sample: SampleType::U8 };
            let bytes = encode(&img, plan).unwrap();
            assert!(looks_like_sgi(&bytes));
            let back = decode(&bytes, &Limits::default()).unwrap();
            assert_eq!(back.layout(), layout);
            assert_eq!(back.data(), img.data());
        }
    }

    #[test]
    fn feature_config_rle_round_trips_16_bits_with_long_runs() {
        let (w, h) = (300u32, 4u32);
        let samples: Vec<u16> = (0..w * h * 3).map(|i| if (i / 3) % 200 < 150 { 4242 } else { (i * 31) as u16 }).collect();
        let img = Image::from_u16(w, h, ChannelLayout::Rgb, &samples).unwrap();
        let bytes = encode(&img, Plan { layout: ChannelLayout::Rgb, sample: SampleType::U16 }).unwrap();
        let back = decode(&bytes, &Limits::default()).unwrap();
        assert_eq!(back.sample_type(), SampleType::U16);
        assert_eq!(back.data(), img.data());
    }

    /// A hand-built verbatim file: two rows, one plane, stored bottom to top.
    #[test]
    fn feature_config_verbatim_files_read_bottom_to_top() {
        let mut f = vec![0u8; HEADER];
        f[0..2].copy_from_slice(&MAGIC.to_be_bytes());
        f[2] = 0; // verbatim
        f[3] = 1;
        f[4..6].copy_from_slice(&2u16.to_be_bytes());
        f[6..8].copy_from_slice(&3u16.to_be_bytes()); // x
        f[8..10].copy_from_slice(&2u16.to_be_bytes()); // y
        f[10..12].copy_from_slice(&1u16.to_be_bytes()); // z
        f.extend_from_slice(&[10, 11, 12, 20, 21, 22]); // bottom row first
        let img = decode(&f, &Limits::default()).unwrap();
        assert_eq!(img.layout(), ChannelLayout::Gray);
        assert_eq!(img.data(), &[20, 21, 22, 10, 11, 12]);
    }

    #[test]
    fn feature_config_hostile_input_is_an_error_not_a_panic() {
        let img = gradient(16, 8, ChannelLayout::Rgb);
        let good = encode(&img, Plan { layout: ChannelLayout::Rgb, sample: SampleType::U8 }).unwrap();
        // Every truncation fails cleanly.
        for len in 0..good.len() {
            let _ = decode(&good[..len], &Limits::default());
        }
        // A row table pointing outside the file, and a packet that overruns its row.
        let mut bad_table = good.clone();
        bad_table[HEADER..HEADER + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(decode(&bad_table, &Limits::default()).is_err());
        let mut overrun = good.clone();
        let start = be32(&overrun, HEADER) as usize;
        overrun[start] = 127; // a run longer than the 16-pixel row
        assert!(decode(&overrun, &Limits::default()).is_err());
        // A huge declared size is stopped by the limits or the length check, before allocating.
        let mut huge = good;
        huge[6..8].copy_from_slice(&u16::MAX.to_be_bytes());
        huge[8..10].copy_from_slice(&u16::MAX.to_be_bytes());
        assert!(decode(&huge, &Limits::default()).is_err());
    }

    /// A 1-plane, 8-bit RLE file whose `rows` all start where `starts[i]` says, each `len` bytes long.
    fn rle_file(w: u16, starts: &[u32], len: u32, payload: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; HEADER];
        f[0..2].copy_from_slice(&MAGIC.to_be_bytes());
        f[2] = 1;
        f[3] = 1;
        f[4..6].copy_from_slice(&2u16.to_be_bytes());
        f[6..8].copy_from_slice(&w.to_be_bytes());
        f[8..10].copy_from_slice(&(starts.len() as u16).to_be_bytes());
        f[10..12].copy_from_slice(&1u16.to_be_bytes());
        for s in starts {
            f.extend_from_slice(&s.to_be_bytes());
        }
        for _ in starts {
            f.extend_from_slice(&len.to_be_bytes());
        }
        f.extend_from_slice(payload);
        f
    }

    /// The format lets table entries share one compressed row: eight rows, one payload.
    #[test]
    fn feature_config_rows_may_share_compressed_data() {
        let table = HEADER + 8 * 8;
        let f = rle_file(1, &[table as u32; 8], 3, &[0x81, 255, 0]);
        assert_eq!(f.len(), table + 3);
        let img = decode(&f, &Limits::default()).unwrap();
        assert_eq!((img.dimensions(), img.data()), ((1, 8), &[255u8; 8][..]));
    }

    /// Compressed rows start after both tables: an offset into a table is refused.
    #[test]
    fn feature_config_rows_may_not_start_inside_the_offset_tables() {
        // 3x1: the tables end at 520; 515 reads the table's own bytes as a packet list.
        let f = rle_file(3, &[515], 3, &[0, 0, 0, 0, 0]);
        assert!(decode(&f, &Limits::default()).is_err());
    }

    /// FFmpeg's decoder, run as an oracle, also reads a row that ends with its last packet and no
    /// zero terminator; so does this one.
    #[test]
    fn feature_config_a_row_without_its_terminator_still_reads() {
        let f = rle_file(1, &[520], 2, &[0x81, 42]);
        assert_eq!(decode(&f, &Limits::default()).unwrap().data(), &[42]);
    }

    #[test]
    fn feature_config_palette_mapped_files_are_unsupported() {
        let img = gradient(4, 4, ChannelLayout::Gray);
        let mut f = encode(&img, Plan { layout: ChannelLayout::Gray, sample: SampleType::U8 }).unwrap();
        f[104..108].copy_from_slice(&1u32.to_be_bytes());
        assert!(matches!(decode(&f, &Limits::default()), Err(CodecError::Unsupported { .. })));
    }
}

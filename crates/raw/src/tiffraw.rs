//! Reading the pixel data of a TIFF IFD (strips or tiles; uncompressed, lossless/lossy JPEG, Deflate, JPEG XL), in parallel.

#[cfg(feature = "dng")]
use crate::unpack::inflate;
use crate::unpack::{f16_to_f32, f24_to_f32, read_u16s, undo_diff_u16, undo_float_predictor, unpack_lsb, unpack_msb, unpack_words32_msb};
use crate::{Limits, RawData, RawError, Result, ljpeg};
use craft_tiff::image::{Chunk, ImageInfo, chunk_bytes};
use craft_tiff::{ByteOrder, tags::compression as comp};
use rayon::prelude::*;

/// Bit packing for uncompressed integer data with bit depths other than 8/16.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // vendor decoders use the other variants
pub enum Packing {
    /// TIFF standard: MSB-first, rows start on byte boundaries.
    Msb,
    /// LSB-first little-endian bit stream (some vendor raws), rows byte aligned.
    Lsb,
    /// Each sample stored in 16 bits in file byte order, regardless of `bits`.
    Word16,
    /// MSB-first bit stream inside 32-bit words stored in file byte order ([`unpack_words32_msb`]); rows start on a
    /// word boundary.
    Words32Msb,
}

pub(crate) enum ChunkPx {
    U16(Vec<u16>),
    F32(Vec<f32>),
}

/// The checks [`read_image`] makes before decoding anything (sample format, size limits, data
/// present); returns the number of samples.
pub(crate) fn check_image(data: &[u8], info: &ImageInfo, limits: &Limits) -> Result<usize> {
    let (w, h) = (info.width as usize, info.height as usize);
    let cpp = info.samples_per_pixel as usize;
    let total = w.checked_mul(h).and_then(|v| v.checked_mul(cpp)).ok_or(RawError::Limit("image too large"))?;
    if total > limits.samples() || total == 0 {
        return Err(RawError::Limit("image too large"));
    }
    let bits = info.bits() as u32;
    let float = info.sample_format == 3;
    if float && !matches!(bits, 16 | 24 | 32) {
        return Err(RawError::Unsupported(format!("{bits}-bit floating point samples")));
    }
    if !float && !(1..=16).contains(&bits) {
        return Err(RawError::Unsupported(format!("{bits}-bit integer samples")));
    }
    let chunks = info.chunks(data.len() as u64);
    if chunks.is_empty() {
        return Err(RawError::Corrupt("no image data chunks".into()));
    }
    // every visible sample needs a chunk: a short offsets table would leave zero-filled (black) regions
    let (across, down) = info.grid();
    let planes = if info.planar == 2 { cpp } else { 1 };
    let expected = (across as usize).saturating_mul(down as usize).saturating_mul(planes);
    if chunks.len() < expected {
        return Err(RawError::Corrupt(format!("{} image data chunks, expected {expected}", chunks.len())));
    }
    // Tile sizes don't depend on the image size, and every tile is decoded at its nominal size and held until the copy:
    // a grid of tiles no larger than the image covers less than twice its width and height, so more than four times
    // its samples (or one tile of more than TILE_SAMPLES_FLOOR on a tiny image) is no picture (storytold/lightcraft#2554).
    let ccpp = if info.planar == 2 { 1 } else { cpp as u64 };
    let nominal = chunks.iter().fold(0u64, |sum, c| sum.saturating_add(u64::from(c.width).saturating_mul(u64::from(c.height)).saturating_mul(ccpp)));
    if nominal > (total as u64).saturating_mul(4).max(TILE_SAMPLES_FLOOR) {
        return Err(RawError::Limit("image data chunks far larger than the image"));
    }
    // Plausibility bound before allocating: no supported coding stores more than ~2000 samples per byte
    // (Deflate of constant data is the extreme), so tiny files cannot trigger huge allocations. The
    // bytes are counted once however many chunks point at them: a grid of tiles sharing one small
    // payload holds that payload's samples, not the grid's.
    let mut spans: Vec<(u64, u64)> =
        chunks.iter().filter_map(|c| Some((c.offset, c.offset.checked_add(chunk_bytes(data, c)?.len() as u64)?))).collect();
    spans.sort_unstable();
    let (mut available, mut end) = (0u64, 0u64);
    for (a, b) in spans {
        let a = a.max(end);
        if b > a {
            available = available.saturating_add(b - a);
            end = b;
        }
    }
    if available.saturating_mul(2048) < total as u64 {
        return Err(RawError::Corrupt(format!("{available} bytes of image data cannot hold {total} samples")));
    }
    // Lossless JPEG codes every sample in a bit at least: ljpeg's per-frame check allows 8 samples per entropy byte plus
    // 64 bytes of slack, which a frame's own headers (over 32 bytes with its Huffman table) cover twice over, so 16
    // coded samples per byte, and twice that for 4:2:0 YCbCr (half the samples it fills are coded) (storytold/lightcraft#2554). The
    // bytes are counted once per chunk that points at them here: tiles may legitimately share one payload (identical
    // tiles written once), and each copy is decoded in full, so every reference brings its payload's samples. The
    // unique-byte bound above still caps what a small payload shared by a large grid can make the output reserve.
    let referenced = chunks.iter().fold(0u64, |sum, c| sum.saturating_add(chunk_bytes(data, c).map_or(0, |b| b.len() as u64)));
    if info.compression == comp::JPEG && referenced.saturating_mul(32) < total as u64 {
        return Err(RawError::Corrupt(format!("{referenced} bytes of lossless JPEG data cannot hold {total} samples")));
    }
    Ok(total)
}

/// Read the samples of `info` in `mode`: [`Mode::Header`] only checks them ([`check_image`]) and
/// returns no samples (floating-point data as an empty `F32`).
pub(crate) fn read_image_in(
    mode: crate::Mode,
    data: &[u8],
    info: &ImageInfo,
    order: ByteOrder,
    packing: Packing,
    limits: &Limits,
) -> Result<RawData> {
    match mode {
        crate::Mode::Full => read_image(data, info, order, packing, limits),
        crate::Mode::Header => {
            check_image(data, info, limits)?;
            // a lossless-JPEG layout the decoder rejects (subsampled components, e.g. Sony's
            // lossless M/S sizes) must fail here too, as the full decode will; likewise a JPEG XL
            // chunk whose headers don't match the IFD
            let first = info.chunks(data.len() as u64).first().copied();
            if let Some(c) = first
                && let Some(src) = chunk_bytes(data, &c)
            {
                match info.compression {
                    7 => {
                        ljpeg::frame_info(src)?;
                    }
                    comp::JPEG_XL => {
                        let planar = info.planar == 2 && info.samples_per_pixel > 1;
                        let cpp = if planar { 1 } else { info.samples_per_pixel as usize };
                        let (cw, ch, vw, vh) = chunk_dims(info, &c);
                        jxl_chunk_check(src, cw, ch, vw, vh, cpp, info.bits() as u32, info.sample_format == 3)?;
                    }
                    _ => {}
                }
            }
            Ok(if info.sample_format == 3 { RawData::F32(Vec::new()) } else { RawData::U16(Vec::new()) })
        }
    }
}

/// The nominal samples of all chunks [`check_image`] allows whatever the image size: one 2048 × 2048 tile.
const TILE_SAMPLES_FLOOR: u64 = 1 << 22;

/// Decode all chunks of `info` into one buffer of `width × height × cpp` samples.
pub(crate) fn read_image(data: &[u8], info: &ImageInfo, order: ByteOrder, packing: Packing, limits: &Limits) -> Result<RawData> {
    let (w, h) = (info.width as usize, info.height as usize);
    let total = check_image(data, info, limits)?;
    let cpp = info.samples_per_pixel as usize;
    let bits = info.bits() as u32;
    let float = info.sample_format == 3;
    let chunks = info.chunks(data.len() as u64);
    let planar = info.planar == 2 && cpp > 1;
    let ccpp = if planar { 1 } else { cpp };
    // The chunks are decoded and copied into the output in groups whose combined nominal size stays within
    // a few times the output buffer's: everything decoded at once would retain the whole image twice, and
    // a hostile tile table's chosen padding could stretch that much further (storytold/lightcraft#1517). One group
    // still decodes in parallel; the grid order groups make, the copies stay disjoint.
    // The tile grid decides how much is decoded, whatever the file's bytes: the nominal sizes of all
    // the chunks stay within a few times the image (edge tiles pad it) before any is decoded.
    let nominal = chunks.iter().fold(0usize, |sum, c| sum.saturating_add(chunk_samples(c, ccpp)));
    if nominal > total.saturating_mul(4).saturating_add(1 << 24) {
        return Err(RawError::Corrupt("the tile grid is much larger than the image".into()));
    }
    let group_samples = (total / 4).max(1 << 20);
    let mut out_u16 = if float { Vec::new() } else { vec![0u16; total] };
    let mut out_f32 = if float { vec![0f32; total] } else { Vec::new() };
    let mut decode_from = 0usize;
    while decode_from < chunks.len() {
        let mut to = decode_from;
        let mut taken = 0usize;
        for (k, c) in chunks[decode_from..].iter().enumerate() {
            let n = chunk_samples(c, ccpp);
            if taken > 0 && taken.saturating_add(n) > group_samples {
                break;
            }
            taken = taken.saturating_add(n);
            to = decode_from + k + 1;
        }
        let decoded: Vec<Result<(Chunk, ChunkPx)>> = chunks[decode_from..to]
            .par_iter()
            .map(|c| decode_chunk(data, info, c, order, packing, ccpp, bits, float, limits).map(|p| (*c, p)))
            .collect();
        for r in decoded {
            // a chunk that cannot be decoded is not a partial image: the file stays preview-only
            let (c, px) = r?;
            let (cw, ch) = (c.width as usize, c.height as usize);
            let (x0, y0) = (c.x as usize, c.y as usize);
            if x0 >= w || y0 >= h {
                continue;
            }
            let copy_w = cw.min(w - x0);
            let copy_h = ch.min(h - y0);
            for y in 0..copy_h {
                for x in 0..copy_w {
                    for s in 0..ccpp {
                        let src = (y * cw + x) * ccpp + s;
                        let dst_s = if planar { c.plane as usize } else { s };
                        if dst_s >= cpp {
                            continue;
                        }
                        let dst = ((y0 + y) * w + x0 + x) * cpp + dst_s;
                        match &px {
                            ChunkPx::U16(v) => {
                                if let (Some(o), Some(&s)) = (out_u16.get_mut(dst), v.get(src)) {
                                    *o = s;
                                }
                            }
                            ChunkPx::F32(v) => {
                                if let (Some(o), Some(&s)) = (out_f32.get_mut(dst), v.get(src)) {
                                    *o = s;
                                }
                            }
                        }
                    }
                }
            }
        }
        decode_from = to;
    }
    Ok(if float { RawData::F32(out_f32) } else { RawData::U16(out_u16) })
}

/// Bytes that `n` samples take in a row of an uncompressed or inflated chunk, laid out as
/// [`unpack_chunk`] reads them: whole bytes or 16-bit words, or 32-bit words for `Words32Msb`.
fn packed_row_bytes(n: u64, bits: u32, packing: Packing, float: bool) -> u64 {
    if float {
        return n.saturating_mul(u64::from(bits.div_ceil(8)));
    }
    let bit_len = n.saturating_mul(u64::from(bits));
    match (bits, packing) {
        (8, _) => n,
        (16, _) | (_, Packing::Word16) => n.saturating_mul(2),
        (_, Packing::Msb | Packing::Lsb) => bit_len.div_ceil(8),
        (_, Packing::Words32Msb) => bit_len.div_ceil(32).saturating_mul(4),
    }
}

/// A chunk's nominal decoded size in samples (per plane, as the decoder builds it).
fn chunk_samples(c: &Chunk, ccpp: usize) -> usize {
    (c.width as usize).saturating_mul(c.height as usize).saturating_mul(ccpp)
}

/// Consecutive index ranges of `chunks` whose combined nominal samples (`ccpp` per pixel) stay within the group size
/// [`read_image`] uses for an image of `total` samples (at least one chunk per range). A decoder that holds its
/// decoded tiles until it copies them decodes one range at a time, so it holds a group's frames beside the output,
/// not the whole grid's (storytold/lightcraft#1517, storytold/lightcraft#2554).
#[cfg(feature = "arw")]
pub(crate) fn decode_groups(chunks: &[Chunk], ccpp: usize, total: usize) -> Vec<std::ops::Range<usize>> {
    let budget = (total / 4).max(1 << 20);
    let mut groups = Vec::new();
    let (mut from, mut taken) = (0usize, 0usize);
    for (i, c) in chunks.iter().enumerate() {
        let n = chunk_samples(c, ccpp);
        if i > from && taken.saturating_add(n) > budget {
            groups.push(from..i);
            (from, taken) = (i, 0);
        }
        taken = taken.saturating_add(n);
    }
    if from < chunks.len() {
        groups.push(from..chunks.len());
    }
    groups
}

#[allow(clippy::too_many_arguments)]
fn decode_chunk(
    data: &[u8],
    info: &ImageInfo,
    c: &Chunk,
    order: ByteOrder,
    packing: Packing,
    cpp: usize,
    bits: u32,
    float: bool,
    limits: &Limits,
) -> Result<ChunkPx> {
    let src = chunk_bytes(data, c).ok_or_else(|| RawError::Corrupt("chunk offset past end of file".into()))?;
    let (cw, ch, vw, vh) = chunk_dims(info, c);
    let n = cw.checked_mul(ch).and_then(|v| v.checked_mul(cpp)).ok_or(RawError::Limit("chunk too large"))?;
    if n > limits.samples() {
        return Err(RawError::Limit("chunk too large"));
    }
    match info.compression {
        comp::NONE => {
            // uncompressed: missing rows are filled with zeros, but a chunk holding a small fraction of what its
            // samples need is no picture: refused before the nominal chunk is allocated (storytold/lightcraft#1512)
            let need = (n as u64).saturating_mul(u64::from(bits)) / 8;
            if (src.len() as u64).saturating_mul(64) < need {
                return Err(RawError::Corrupt(format!("an uncompressed chunk of {n} samples has only {} bytes", src.len())));
            }
            unpack_chunk(src, order, packing, cw, ch, cpp, bits, float, 1)
        }
        comp::JPEG => {
            let f = ljpeg::decode(src, n.saturating_mul(2).max(1 << 16))?;
            if f.data.len() < n {
                // Some writers encode edge tiles smaller than nominal: accept a frame that covers the rows inside the image.
                if f.data.is_empty() || f.data.len() % (cw * cpp) != 0 || f.data.len() / (cw * cpp) < vh {
                    return Err(RawError::Corrupt(format!("lossless JPEG tile has {} samples, expected {n}", f.data.len())));
                }
            }
            let mut v = f.data;
            v.resize(n, 0);
            Ok(ChunkPx::U16(v))
        }
        #[cfg(feature = "dng")]
        comp::LOSSY_JPEG => {
            // lossy DNG: each chunk is a baseline (DCT) JPEG of 8-bit samples
            // a JPEG decoded for a tile may hold no more samples than the tile (a real one is exactly the tile):
            // many small tiles pointing at one big JPEG would otherwise each decode all of it
            // (the JPEG codes whole MCUs, at most 32 × 32 pixels each, so its size is the tile's rounded up to them)
            let mcu = |v: usize| v.div_ceil(32).saturating_mul(32);
            let tile_budget =
                mcu(cw).saturating_mul(mcu(ch)).saturating_mul(if cpp == 1 { 1 } else { 3 }).clamp(1, limits.max_lossy_tile_samples.max(1));
            let (px, jw, jh, jc) = lossy_jpeg(src, cpp, tile_budget)?;
            // An edge tile's JPEG may be smaller than the nominal tile, but it has to cover the part of the tile
            // inside the image: one that doesn't would leave a black region in a picture reported as decoded
            // (PhotoCraft's and FilmCraft's readers refuse a JPEG smaller than its tile; this keeps the edge case).
            if jw < vw || jh < vh {
                return Err(RawError::Corrupt(format!("a lossy JPEG of {jw} × {jh} does not cover the {vw} × {vh} of its tile inside the image")));
            }
            // the buffer holds the rows the decoded JPEG covers (the caller zero-fills the tile's
            // remainder and reads only rows it fills): a tile-many-larger decoded JPEG does not
            // get the nominal tile's full buffer (storytold/lightcraft#1517)
            let rows = ch.min(jh);
            let mut v = vec![0u16; rows.max(1).saturating_mul(cw).saturating_mul(cpp)];
            for y in 0..rows {
                for x in 0..cw.min(jw) {
                    for k in 0..cpp {
                        v[(y * cw + x) * cpp + k] = px[(y * jw + x) * jc + k.min(jc - 1)] as u16;
                    }
                }
            }
            Ok(ChunkPx::U16(v))
        }
        #[cfg(feature = "dng")]
        comp::ADOBE_DEFLATE | comp::DEFLATE => {
            // row bytes in u64: a wrapping `cw · cpp · …` in usize could come out small on 32-bit targets and slip past the gate
            let row_bytes = packed_row_bytes(cw as u64 * cpp as u64, bits, packing, float);
            // the gate caps at MAX_SAMPLES (≤ 2^30), so the `usize` cast below is lossless on 32-bit targets too
            let inflate_len =
                crate::checked_pixel_count(row_bytes, ch as u64).ok_or_else(|| RawError::Corrupt("bad TIFF chunk dimensions".into()))?;
            // Deflate compresses at most about 1:1032, so a source this far under half of its
            // nominal length cannot reconstruct the chunk: refused before the nominal buffer is
            // inflated and predicted, which padding-only work would make gigabytes.
            // Nor may the nominal chunk be mostly padding outside the image (a tile far larger than the picture):
            // its allocation and predictor pass would be spent on samples nobody reads.
            if n > vw
                .saturating_mul(vh)
                .saturating_mul(cpp)
                .saturating_mul(MAX_PADDING_FACTOR)
                .saturating_add(MAX_WRITER_TILE_SAMPLES.saturating_mul(cpp))
            {
                return Err(RawError::Limit("deflated chunk is mostly padding outside the image"));
            }
            if (src.len() as u64).saturating_mul(2048) < inflate_len {
                return Err(RawError::Corrupt(format!("a deflated chunk of {inflate_len} bytes is packed into only {} bytes", src.len())));
            }
            let raw = inflate(src, inflate_len as usize)?;
            // The stream has to reach the last row inside the image (edge strips may stop there);
            // it is laid out in nominal rows, so shorter is a truncated chunk, not a picture.
            let last = packed_row_bytes(vw as u64 * cpp as u64, bits, packing, float);
            let visible = row_bytes.saturating_mul(vh.saturating_sub(1) as u64).saturating_add(last);
            if (raw.len() as u64) < visible {
                return Err(RawError::Corrupt(format!(
                    "a deflated chunk holds {} bytes, fewer than the {visible} bytes its in-image samples span",
                    raw.len()
                )));
            }
            unpack_chunk(&raw, order, packing, cw, ch, cpp, bits, float, info.predictor)
        }
        comp::JPEG_XL => jxl_chunk(src, cw, ch, vw, vh, cpp, bits, float),
        other => Err(RawError::Unsupported(format!("TIFF compression {other}"))),
    }
}

/// A chunk's nominal size and the part of it inside the image (smaller for edge tiles).
fn chunk_dims(info: &ImageInfo, c: &Chunk) -> (usize, usize, usize, usize) {
    let (cw, ch) = (c.width as usize, c.height as usize);
    let vw = cw.min(info.width.saturating_sub(c.x) as usize);
    let vh = ch.min(info.height.saturating_sub(c.y) as usize);
    (cw, ch, vw, vh)
}

#[cfg(feature = "jxl")]
use crate::jxl::{check as jxl_chunk_check, decode as jxl_chunk};

#[cfg(not(feature = "jxl"))]
#[allow(clippy::too_many_arguments)]
fn jxl_chunk(_: &[u8], _: usize, _: usize, _: usize, _: usize, _: usize, _: u32, _: bool) -> Result<ChunkPx> {
    Err(RawError::Unsupported("JPEG XL DNG (built without the `jxl` feature)".into()))
}

#[cfg(not(feature = "jxl"))]
#[allow(clippy::too_many_arguments)]
fn jxl_chunk_check(src: &[u8], cw: usize, ch: usize, vw: usize, vh: usize, cpp: usize, bits: u32, float: bool) -> Result<()> {
    jxl_chunk(src, cw, ch, vw, vh, cpp, bits, float).map(|_| ())
}

/// How many times its in-image samples a deflated chunk's nominal size may reach (plus a writer tile of slack):
/// edge tiles of real files are mostly inside the image.
#[cfg(feature = "dng")]
const MAX_PADDING_FACTOR: usize = 32;
/// Samples per plane of the largest Deflate tile the DNG writer pads to (2048 × 2048, `dngwrite::MAX_DEFLATE_TILE`),
/// which a small image's one tile is nearly all padding of: such files still decode.
#[cfg(feature = "dng")]
const MAX_WRITER_TILE_SAMPLES: usize = 2048 * 2048;

#[cfg(feature = "dng")]
/// Decode one baseline JPEG chunk to 8-bit samples: (samples, width, height, channels).
pub(crate) fn lossy_jpeg(src: &[u8], cpp: usize, max_samples: usize) -> Result<(Vec<u8>, usize, usize, usize)> {
    use zune_core::bytestream::ZCursor;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let cs = if cpp == 1 { ColorSpace::Luma } else { ColorSpace::RGB };
    let opts = DecoderOptions::default().set_max_width(1 << 16).set_max_height(1 << 16).set_strict_mode(false).jpeg_set_out_colorspace(cs);
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(src), opts);
    // The header is read first: the JPEG's own dimensions, not the enclosing tile's, size every
    // allocation `decode` makes, and they may claim billions of pixels.
    d.decode_headers().map_err(|e| RawError::Corrupt(format!("lossy JPEG tile: {e}")))?;
    let info = d.info().ok_or_else(|| RawError::Corrupt("lossy JPEG tile without a header".into()))?;
    let (w, h) = (info.width as usize, info.height as usize);
    let c = if cpp == 1 { 1 } else { 3 };
    let out = w as u64 * h as u64 * c as u64;
    if w == 0 || h == 0 || out > max_samples as u64 {
        return Err(RawError::Corrupt(format!("a lossy JPEG of {w} × {h} × {c} would exceed the {max_samples} output samples allowed")));
    }
    let px = d.decode().map_err(|e| RawError::Corrupt(format!("lossy JPEG tile: {e}")))?;
    if px.len() < w * h * c {
        return Err(RawError::Corrupt("lossy JPEG tile: short output".into()));
    }
    Ok((px, w, h, c))
}

#[allow(clippy::too_many_arguments)]
fn unpack_chunk(
    src: &[u8],
    order: ByteOrder,
    packing: Packing,
    cw: usize,
    ch: usize,
    cpp: usize,
    bits: u32,
    float: bool,
    predictor: u16,
) -> Result<ChunkPx> {
    let row_n = cw * cpp;
    let (factor, fp) = match predictor {
        1 => (0, false),
        2 => (1, false),
        3 => (1, true),
        34892 => (2, false),
        34893 => (4, false),
        34894 => (2, true),
        34895 => (4, true),
        p => return Err(RawError::Unsupported(format!("predictor {p}"))),
    };
    if float {
        let bp = bits.div_ceil(8) as usize;
        let row_bytes = row_n * bp;
        let mut out = vec![0f32; row_n * ch];
        let mut rowbuf = vec![0u8; row_bytes];
        for y in 0..ch {
            let s = src.get(y * row_bytes..).unwrap_or(&[]);
            let len = s.len().min(row_bytes);
            rowbuf[..len].copy_from_slice(&s[..len]);
            rowbuf[len..].fill(0);
            let big = if fp {
                undo_float_predictor(&mut rowbuf, row_n, bp, cpp * factor);
                true
            } else {
                if factor > 0 {
                    return Err(RawError::Unsupported("integer predictor on float data".into()));
                }
                order == ByteOrder::Big
            };
            for (i, o) in out[y * row_n..(y + 1) * row_n].iter_mut().enumerate() {
                let b = &rowbuf[i * bp..(i + 1) * bp];
                *o = match (bp, big) {
                    (2, true) => f16_to_f32(u16::from_be_bytes([b[0], b[1]])),
                    (2, false) => f16_to_f32(u16::from_le_bytes([b[0], b[1]])),
                    (3, true) => f24_to_f32(u32::from_be_bytes([0, b[0], b[1], b[2]])),
                    (3, false) => f24_to_f32(u32::from_le_bytes([b[0], b[1], b[2], 0])),
                    (4, true) => f32::from_be_bytes([b[0], b[1], b[2], b[3]]),
                    _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                };
            }
        }
        return Ok(ChunkPx::F32(out));
    }
    if fp {
        return Err(RawError::Unsupported("floating-point predictor on integer data".into()));
    }
    let mut out = vec![0u16; row_n * ch];
    let (row_bytes, kind) = match (bits, packing) {
        (8, _) => (row_n, 0),
        (16, _) | (_, Packing::Word16) => (row_n * 2, 1),
        (_, Packing::Msb) => ((row_n * bits as usize).div_ceil(8), 2),
        (_, Packing::Lsb) => ((row_n * bits as usize).div_ceil(8), 3),
        (_, Packing::Words32Msb) => ((row_n * bits as usize).div_ceil(32) * 4, 4),
    };
    for y in 0..ch {
        let s = src.get(y * row_bytes..).unwrap_or(&[]);
        let s = &s[..s.len().min(row_bytes)];
        let row = &mut out[y * row_n..(y + 1) * row_n];
        match kind {
            0 => row.iter_mut().zip(s).for_each(|(o, &b)| *o = b as u16),
            1 => read_u16s(s, order, row),
            2 => unpack_msb(s, bits, row),
            4 => unpack_words32_msb(s, order, bits, row),
            _ => unpack_lsb(s, bits, row),
        }
        if factor > 0 {
            if bits == 8 {
                for i in cpp * factor..row.len() {
                    row[i] = (row[i] as u8).wrapping_add(row[i - cpp * factor] as u8) as u16;
                }
            } else {
                undo_diff_u16(row, cpp * factor);
            }
        }
    }
    Ok(ChunkPx::U16(out))
}

#[cfg(test)]
#[cfg(feature = "dng")]
mod lossy_tests {
    use super::*;
    use craft_tiff::image::Layout;

    /// 65 536 tiles all pointing at the same 256 bytes: the image's samples can't come from them,
    /// and nothing of its size is allocated to find that out.
    #[test]
    fn tiles_sharing_one_payload_count_its_bytes_once() {
        // (longer than its tile count: craft-tiff builds at most one chunk per byte of file, and a shorter
        // file is refused for its missing chunks before the bytes are counted)
        let file = vec![0u8; 65_536 + 8];
        let info = ImageInfo {
            width: 4096,
            height: 4096,
            bits_per_sample: vec![8],
            samples_per_pixel: 1,
            compression: comp::NONE,
            photometric: 34892,
            planar: 1,
            predictor: 1,
            sample_format: 1,
            new_subfile_type: 0,
            layout: Layout::Tiles { tile_width: 16, tile_height: 16 },
            offsets: vec![8; 65_536],
            byte_counts: vec![256; 65_536],
        };
        let err = check_image(&file, &info, &Limits::default()).unwrap_err().to_string();
        assert!(err.contains("cannot hold"), "{err}");
    }

    /// A lossy-DNG-style image: two 16×8 tiles, each a baseline JPEG, the right one cut short
    /// by the image edge.
    #[test]
    fn lossy_jpeg_tiles_decode() {
        let (tw, th) = (16usize, 8usize);
        let tile = |shade: u8| {
            let px: Vec<u8> = (0..tw * th).flat_map(|i| [shade, (i % tw * 15) as u8, 200]).collect();
            let mut out = Vec::new();
            jpeg_encoder::Encoder::new(&mut out, 100).encode(&px, tw as u16, th as u16, jpeg_encoder::ColorType::Rgb).unwrap();
            out
        };
        let (a, b) = (tile(40), tile(220));
        let mut file = vec![0u8; 8];
        let oa = file.len() as u64;
        file.extend_from_slice(&a);
        let ob = file.len() as u64;
        file.extend_from_slice(&b);
        let info = ImageInfo {
            width: 24,
            height: 8,
            bits_per_sample: vec![8, 8, 8],
            samples_per_pixel: 3,
            compression: comp::LOSSY_JPEG,
            photometric: 34892,
            planar: 1,
            predictor: 1,
            sample_format: 1,
            new_subfile_type: 0,
            layout: Layout::Tiles { tile_width: tw as u32, tile_height: th as u32 },
            offsets: vec![oa, ob],
            byte_counts: vec![a.len() as u64, b.len() as u64],
        };
        let RawData::U16(v) = read_image(&file, &info, ByteOrder::Little, Packing::Msb, &Limits::default()).unwrap() else {
            panic!("integer samples")
        };
        assert_eq!(v.len(), 24 * 8 * 3);
        let px = |x: usize, y: usize| &v[(y * 24 + x) * 3..(y * 24 + x) * 3 + 3];
        assert!((px(2, 3)[0] as i32 - 40).abs() <= 3 && (px(2, 3)[2] as i32 - 200).abs() <= 3, "{:?}", px(2, 3));
        assert!((px(20, 3)[0] as i32 - 220).abs() <= 3, "second tile: {:?}", px(20, 3));
        assert!((px(10, 5)[1] as i32 - 150).abs() <= 6, "green ramp: {:?}", px(10, 5));
    }

    /// A small tile may not decode a JPEG much larger than itself (a table of tiny tiles all pointing at one big
    /// JPEG would decode it once per tile); a JPEG that fills the tile's whole 32 × 32 MCU block still decodes.
    #[test]
    fn a_lossy_tile_may_not_decode_a_much_larger_jpeg() {
        let encode = |w: usize, h: usize| {
            let mut out = Vec::new();
            jpeg_encoder::Encoder::new(&mut out, 90).encode(&vec![128u8; w * h * 3], w as u16, h as u16, jpeg_encoder::ColorType::Rgb).unwrap();
            out
        };
        let image = |jpeg: &[u8]| {
            let mut file = vec![0u8; 8];
            file.extend_from_slice(jpeg);
            let info = ImageInfo {
                width: 4,
                height: 4,
                bits_per_sample: vec![8, 8, 8],
                samples_per_pixel: 3,
                compression: comp::LOSSY_JPEG,
                photometric: 34892,
                planar: 1,
                predictor: 1,
                sample_format: 1,
                new_subfile_type: 0,
                layout: Layout::Tiles { tile_width: 16, tile_height: 16 },
                offsets: vec![8],
                byte_counts: vec![jpeg.len() as u64],
            };
            read_image(&file, &info, ByteOrder::Little, Packing::Msb, &Limits::default())
        };
        assert!(image(&encode(32, 32)).is_ok());
        assert!(matches!(image(&encode(256, 256)), Err(RawError::Corrupt(_))));
    }

    /// A Deflate tile that is nearly all padding outside the image (a 4096 × 4096 tile holding one column of a
    /// 4097-wide picture) is refused before its nominal buffer is inflated; the writer's largest padded tile
    /// (2048 × 2048) is within the slack (`dngwrite`'s clamp test).
    #[test]
    fn a_deflate_tile_that_is_mostly_padding_is_refused() {
        let file = vec![0u8; 8 + 8192];
        let info = ImageInfo {
            width: 4097,
            height: 4096,
            bits_per_sample: vec![8],
            samples_per_pixel: 1,
            compression: comp::ADOBE_DEFLATE,
            photometric: 32803,
            planar: 1,
            predictor: 1,
            sample_format: 1,
            new_subfile_type: 0,
            layout: Layout::Tiles { tile_width: 4096, tile_height: 4096 },
            offsets: vec![8, 8],
            byte_counts: vec![8192, 8192],
        };
        let chunks = info.chunks(file.len() as u64);
        let edge = decode_chunk(&file, &info, &chunks[1], ByteOrder::Little, Packing::Msb, 1, 8, false, &Limits::default());
        assert!(matches!(edge, Err(RawError::Limit(_))), "{:?}", edge.err());
    }

    fn strip_info(compression: u16, offsets: Vec<u64>, byte_counts: Vec<u64>) -> ImageInfo {
        ImageInfo {
            width: 4,
            height: 4,
            bits_per_sample: vec![8],
            samples_per_pixel: 1,
            compression,
            photometric: 1,
            planar: 1,
            predictor: 1,
            sample_format: 1,
            new_subfile_type: 0,
            layout: Layout::Strips { rows_per_strip: 2 },
            offsets,
            byte_counts,
        }
    }

    /// A missing strip or a strip that fails to decode is an error, not a black band in a "successful" image.
    #[test]
    fn missing_or_failed_chunks_are_errors() {
        let file = vec![9u8; 8 + 8];
        // both strips present
        let ok = strip_info(comp::NONE, vec![0, 8], vec![8, 8]);
        assert!(read_image(&file, &ok, ByteOrder::Little, Packing::Msb, &Limits::default()).is_ok());
        // the second strip has no offset
        let missing = strip_info(comp::NONE, vec![0], vec![8]);
        assert!(read_image(&file, &missing, ByteOrder::Little, Packing::Msb, &Limits::default()).is_err());
        // the header-only read rejects it too, as the full decode will
        assert!(read_image_in(crate::Mode::Header, &file, &missing, ByteOrder::Little, Packing::Msb, &Limits::default()).is_err());
        // the second strip starts past the end of the file
        let past_end = strip_info(comp::NONE, vec![0, 100], vec![8, 8]);
        assert!(read_image(&file, &past_end, ByteOrder::Little, Packing::Msb, &Limits::default()).is_err());
    }

    /// A small image whose uncompressed tile claims to be enormous (2048 x 2048 nominal, a few bytes of data) is
    /// refused before that tile is allocated (storytold/lightcraft#1512); an edge tile stored at its full nominal size decodes. (A
    /// tile larger still is refused by `check_image` before any decode: `tiles_far_larger_than_the_image_are_refused`.)
    #[test]
    fn a_huge_nominal_uncompressed_tile_with_little_data_is_refused() {
        let info = |tile: u32, bytes: u64| ImageInfo {
            width: 8,
            height: 8,
            bits_per_sample: vec![16],
            samples_per_pixel: 1,
            compression: comp::NONE,
            photometric: 32803,
            planar: 1,
            predictor: 1,
            sample_format: 1,
            new_subfile_type: 0,
            layout: Layout::Tiles { tile_width: tile, tile_height: tile },
            offsets: vec![0],
            byte_counts: vec![bytes],
        };
        let small = vec![1u8; 2 * 16 * 16];
        assert!(read_image(&small, &info(16, small.len() as u64), ByteOrder::Little, Packing::Msb, &Limits::default()).is_ok());
        let data = vec![1u8; 4096];
        assert!(matches!(
            read_image(&data, &info(2048, data.len() as u64), ByteOrder::Little, Packing::Msb, &Limits::default()),
            Err(RawError::Corrupt(_))
        ));
    }

    /// A 16 × 16 image whose lossless-JPEG tile claims to be 16384 × 16384 (coded only as tall as the image, so the
    /// file is small) is refused before the tile is allocated, by the header check too (storytold/lightcraft#2554); a 256 × 256
    /// tile on the same image, as a writer with a fixed tile size stores it, still decodes.
    #[test]
    fn tiles_far_larger_than_the_image_are_refused() {
        let read = |tile: u32, mode: crate::Mode| {
            let t = ljpeg::encode(&vec![300u16; tile as usize * 16], tile as usize, 16, 1, 12, 1, 0);
            let info = ImageInfo {
                width: 16,
                height: 16,
                bits_per_sample: vec![12],
                samples_per_pixel: 1,
                compression: comp::JPEG,
                photometric: 32803,
                planar: 1,
                predictor: 1,
                sample_format: 1,
                new_subfile_type: 0,
                layout: Layout::Tiles { tile_width: tile, tile_height: tile },
                offsets: vec![0],
                byte_counts: vec![t.len() as u64],
            };
            read_image_in(mode, &t, &info, ByteOrder::Little, Packing::Msb, &Limits::default())
        };
        assert_eq!(read(256, crate::Mode::Full), Ok(RawData::U16(vec![300; 16 * 16])));
        for mode in [crate::Mode::Header, crate::Mode::Full] {
            assert!(matches!(read(16384, mode), Err(RawError::Limit(_))));
        }
    }

    /// A lossless-JPEG tile may be coded only as tall as the part inside the image, never shorter.
    #[test]
    fn short_lossless_jpeg_tiles_are_errors() {
        let tile = |rows: usize| ljpeg::encode(&vec![300u16; 8 * rows], 8, rows, 1, 12, 1, 0);
        let decode = |height: u32, rows: usize| {
            let t = tile(rows);
            let info = ImageInfo {
                width: 8,
                height,
                bits_per_sample: vec![12],
                samples_per_pixel: 1,
                compression: comp::JPEG,
                photometric: 32803,
                planar: 1,
                predictor: 1,
                sample_format: 1,
                new_subfile_type: 0,
                layout: Layout::Tiles { tile_width: 8, tile_height: 8 },
                offsets: vec![0],
                byte_counts: vec![t.len() as u64],
            };
            read_image(&t, &info, ByteOrder::Little, Packing::Msb, &Limits::default())
        };
        // an edge tile coded at its in-image height, and a full tile
        assert_eq!(decode(6, 6), Ok(RawData::U16(vec![300; 8 * 6])));
        assert_eq!(decode(8, 8), Ok(RawData::U16(vec![300; 8 * 8])));
        // half of the visible rows are missing
        assert!(decode(8, 4).is_err());
    }

    /// A lossy JPEG whose header alone claims more samples than any real tile or preview (here
    /// 40000 × 40000 × 3) is refused from the header, before the decoder allocates its output
    /// from the JPEG's own dimensions.
    #[test]
    fn an_oversized_lossy_jpeg_header_is_refused_before_decoding() {
        // A real 8 × 8 baseline JPEG, whose frame header then claims other dimensions: the decoder
        // parses the header, and the size check is what refuses it.
        let px = vec![128u8; 8 * 8 * 3];
        let mut j = Vec::new();
        jpeg_encoder::Encoder::new(&mut j, 90).encode(&px, 8, 8, jpeg_encoder::ColorType::Rgb).unwrap();
        // SOF0: marker, length, precision, then the height and the width
        let sof = j.windows(2).position(|w| w == [0xff, 0xc0]).expect("a baseline frame header");
        let claim = |j: &mut Vec<u8>, size: u16| {
            j[sof + 5..sof + 7].copy_from_slice(&size.to_be_bytes());
            j[sof + 7..sof + 9].copy_from_slice(&size.to_be_bytes());
        };
        claim(&mut j, 40_000);
        let RawError::Corrupt(refused) = lossy_jpeg(&j, 3, 1 << 26).unwrap_err() else { panic!("a corrupt-data error") };
        assert!(refused.contains("would exceed"), "{refused}");
        // a header inside the budget passes the size check (and then fails to decode only for lack of data)
        claim(&mut j, 1024);
        match lossy_jpeg(&j, 3, 1 << 26) {
            Ok(_) => {}
            Err(RawError::Corrupt(m)) => assert!(!m.contains("would exceed"), "the smaller header was refused for its size: {m}"),
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    /// A lossy tile whose JPEG decodes far less than the nominal tile is fine: the rows the JPEG
    /// covers hold its pixels, the rest of the output stays zero (and the tile's intermediate buffer
    /// is sized by the JPEG, not by the nominal tile).
    #[test]
    fn a_lossy_jpeg_covers_its_tiles_part_of_the_image_or_is_refused() {
        let px: Vec<u8> = (0..8 * 8).flat_map(|_| [30u8, 90, 150]).collect();
        let mut j = Vec::new();
        jpeg_encoder::Encoder::new(&mut j, 100).encode(&px, 8, 8, jpeg_encoder::ColorType::Rgb).unwrap();
        let info = |width: u32, height: u32| ImageInfo {
            width,
            height,
            bits_per_sample: vec![8, 8, 8],
            samples_per_pixel: 3,
            compression: comp::LOSSY_JPEG,
            photometric: 34892,
            planar: 1,
            predictor: 1,
            sample_format: 1,
            new_subfile_type: 0,
            layout: Layout::Tiles { tile_width: 16, tile_height: 16 },
            offsets: vec![0],
            byte_counts: vec![j.len() as u64],
        };
        // an 8 × 8 image in one nominal 16 × 16 tile: the JPEG covers what is inside the image
        let RawData::U16(v) = read_image(&j, &info(8, 8), ByteOrder::Little, Packing::Msb, &Limits::default()).unwrap() else {
            panic!("integer samples")
        };
        assert_eq!(v.len(), 8 * 8 * 3);
        let at = |x: usize, y: usize| &v[(y * 8 + x) * 3..(y * 8 + x) * 3 + 3];
        assert!((at(3, 3)[0] as i32 - 30).abs() <= 3 && (at(7, 7)[2] as i32 - 150).abs() <= 3, "{:?}", at(3, 3));
        // a 16 × 16 image: the same JPEG would leave three quarters of it black
        for (w, h) in [(16, 16), (9, 8), (8, 9)] {
            let err = read_image(&j, &info(w, h), ByteOrder::Little, Packing::Msb, &Limits::default()).unwrap_err();
            assert!(matches!(&err, RawError::Corrupt(m) if m.contains("does not cover")), "{w} × {h}: {err}");
        }
    }

    /// A deflated tile is refused when its packed bytes are too few for deflate to have produced
    /// the nominal chunk, or when the stream stops short of the rows inside the image; a fully
    /// covered flat tile still decodes.
    #[test]
    fn a_short_or_undercovering_deflated_tile_is_refused() {
        let info = |byte_counts: u64| ImageInfo {
            width: 512,
            height: 512,
            bits_per_sample: vec![16],
            samples_per_pixel: 1,
            compression: comp::ADOBE_DEFLATE,
            photometric: 32803,
            planar: 1,
            predictor: 1,
            sample_format: 1,
            new_subfile_type: 0,
            layout: Layout::Tiles { tile_width: 512, tile_height: 512 },
            offsets: vec![0],
            byte_counts: vec![byte_counts],
        };
        // a complete flat tile: 512 KiB of zeros compresses to a few hundred bytes (well within
        // deflate's packing bound) and inflates to every nominal byte the tile claims
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; 512 * 512 * 2], 6);
        assert_eq!(
            read_image(&zlib, &info(zlib.len() as u64), ByteOrder::Little, Packing::Msb, &Limits::default()),
            Ok(RawData::U16(vec![0; 512 * 512]))
        );
        // the same 1 KiB packed, but the stream claims a whole tile: a few hundred bytes cannot
        // reconstruct 512 KiB from a decoder that compresses no further than about 1:1032
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; 1024], 6);
        assert!(
            matches!(read_image(&zlib, &info(512 * 512 * 2), ByteOrder::Little, Packing::Msb, &Limits::default()), Err(RawError::Corrupt(_))),
            "{zlib:?}"
        );
        // 500 KiB of zeros compress to a stream within the packing bound and inflating to more
        // bytes than the in-image samples span, but 24 KiB short of the whole nominal tile:
        // refused as truncated rather than filled with the predictor's padding
        let zlib = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0u8; 500 * 1000], 6);
        assert!(matches!(
            read_image(&zlib, &info(zlib.len() as u64), ByteOrder::Little, Packing::Msb, &Limits::default()),
            Err(RawError::Corrupt(_))
        ));
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use crate::ljpeg;
    use craft_tiff::image::Layout;
    use craft_tiff::tags::photometric;

    /// `w`×`h` CFA image in `tile`×`tile` lossless-JPEG tiles that all point at the same `payload` bytes at offset 0.
    fn aliased(w: u32, h: u32, tile: u32, payload: u64) -> ImageInfo {
        let n = (w.div_ceil(tile) * h.div_ceil(tile)) as usize;
        ImageInfo {
            width: w,
            height: h,
            bits_per_sample: vec![14],
            samples_per_pixel: 1,
            compression: 7,
            photometric: photometric::CFA,
            planar: 1,
            predictor: 1,
            sample_format: 1,
            new_subfile_type: 0,
            layout: Layout::Tiles { tile_width: tile, tile_height: tile },
            offsets: vec![0; n],
            byte_counts: vec![payload; n],
        }
    }

    /// storytold/lightcraft#2554: 16 tiles of 256×256 sharing one 1 KiB payload pass the generic 2048-samples-per-byte bound (1 Mi samples
    /// from 1 KiB) but no lossless JPEG holds that many samples in those bytes.
    #[test]
    fn lossless_jpeg_tiles_sharing_a_payload_are_refused() {
        let data = vec![0u8; 1024];
        let info = aliased(1024, 1024, 256, 1024);
        assert!(matches!(check_image(&data, &info, &Limits::default()), Err(RawError::Corrupt(_))));
        // the same grid declared as Deflate (~2000 samples per byte at most) still passes this check
        let deflate = ImageInfo { compression: 8, ..info };
        assert!(check_image(&data, &deflate, &Limits::default()).is_ok());
    }

    /// Tiles may share one payload (a writer storing identical tiles once): each reference decodes its payload in full,
    /// so 32 constant 256×256 lossless-JPEG tiles pointing at one tile's bytes decode (header check and full decode),
    /// though the shared bytes alone are far fewer than one bit per sample of the image.
    #[test]
    fn lossless_jpeg_tiles_sharing_a_real_tile_decode() {
        let tile = ljpeg::encode(&vec![300u16; 256 * 256], 256, 256, 1, 12, 1, 0);
        let info = ImageInfo { bits_per_sample: vec![12], byte_counts: vec![tile.len() as u64; 32], ..aliased(2048, 1024, 256, 0) };
        // one copy of the bytes holds fewer than 32 samples per byte of the whole grid: a unique-byte bound refused it
        assert!((tile.len() as u64) * 32 < 2048 * 1024, "{} bytes", tile.len());
        assert!(read_image_in(crate::Mode::Header, &tile, &info, ByteOrder::Little, Packing::Msb, &Limits::default()).is_ok());
        assert_eq!(read_image(&tile, &info, ByteOrder::Little, Packing::Msb, &Limits::default()), Ok(RawData::U16(vec![300; 2048 * 1024])));
    }

    /// The unique-byte bound still caps a large grid of tiles sharing a small payload: 256 tiles of 256×256 (16 Mi
    /// samples) referencing the same 8000 bytes pass the per-reference lossless-JPEG bound (2 MB referenced) but those
    /// 8000 bytes cannot hold 16 Mi samples, so nothing is reserved.
    #[test]
    fn a_huge_grid_sharing_a_small_payload_is_still_refused() {
        let data = vec![0u8; 8000];
        let info = aliased(4096, 4096, 256, 8000);
        for mode in [crate::Mode::Header, crate::Mode::Full] {
            let err = read_image_in(mode, &data, &info, ByteOrder::Little, Packing::Msb, &Limits::default()).unwrap_err();
            assert!(matches!(&err, RawError::Corrupt(m) if m.contains("8000 bytes of image data")), "{err:?}");
        }
    }

    /// storytold/lightcraft#2554: tiled decoders decode the grid in groups of at most a quarter of the image (1 Mi samples at
    /// least), each group at least one chunk, covering every chunk once and in order.
    #[cfg(feature = "arw")]
    #[test]
    fn decode_groups_split_the_grid() {
        let info = aliased(4096, 4096, 512, 64);
        let chunks = info.chunks(64);
        assert_eq!(chunks.len(), 64);
        let groups = decode_groups(&chunks, 1, 4096 * 4096);
        // a quarter of 16 Mi samples is 4 Mi: 16 tiles of 256 Ki samples per group
        assert_eq!(groups.len(), 4);
        assert!(groups.iter().all(|g| g.len() == 16));
        assert_eq!(groups.first().map(|g| g.start), Some(0));
        assert!(groups.windows(2).all(|p| p[0].end == p[1].start));
        assert_eq!(groups.last().map(|g| g.end), Some(64));
        // a tile larger than the budget is still a group of its own
        let big = aliased(8192, 8192, 8192, 64);
        assert_eq!(decode_groups(&big.chunks(64), 1, 1 << 20), vec![0..1]);
    }
}

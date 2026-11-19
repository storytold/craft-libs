//! Deep OpenEXR (`deepscanline` / `deeptile`): a variable-length list of samples per pixel, each
//! with its own colour, alpha and depth. The `exr` crate refuses deep headers, so the chunk stream
//! is walked here, per the OpenEXR file layout specification: a deep chunk carries its
//! coordinates, the packed size of the sample-count table, the packed and unpacked sizes of the
//! sample data, then the compressed count table (cumulative within each row of the block) and the
//! compressed sample data (row by row, each row channel by channel). The layout was checked
//! against files written by the OpenEXR library. Deep data permits NONE, RLE, ZIPS and ZIP
//! compression; the first three are read (one scan line per block keeps the count table
//! unambiguous) and ZIP is [`Error::Unsupported`].
//!
//! [`decode_deep`] returns the samples ([`DeepImage`]); [`flatten`] composites them into a flat
//! picture: samples sorted by Z, front-to-back `over` with premultiplied colour, un-premultiplied
//! into straight alpha (HDR values are kept, not clamped).
//!
//! Every allocation is admitted against [`Limits::max_bytes`] first: the per-pixel sample-count
//! index, each count table with its decompression scratch, and the samples.

use std::io::{Cursor, Read, Seek, SeekFrom};

use exr::meta::attribute::{LevelMode, SampleType as ExrSample};
use exr::meta::{BlockDescription, MetaData};
use exr::prelude::Compression;
use half::f16;

use crate::channels::ColorLayout;
use crate::decode::{Error, Limits, Result, SampleType, check_previews};

fn err(e: impl std::fmt::Display) -> Error {
    Error::Malformed(e.to_string())
}

fn unsup(reason: impl Into<String>) -> Error {
    Error::Unsupported(reason.into())
}

/// A zeroed buffer of `len` elements, or an error when the allocator refuses it.
fn alloc_zeroed<T: Copy + Default>(len: usize) -> Result<Vec<T>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| Error::LimitExceeded(format!("cannot allocate {len} elements")))?;
    v.resize(len, T::default());
    Ok(v)
}

/// One channel of a [`DeepImage`].
#[derive(Debug, Clone, PartialEq)]
pub struct DeepChannel {
    /// The channel name as in the file (`R`, `G`, `B`, `A`, `Z`, …).
    pub name: String,
    /// The sample type as stored; `samples` always holds the value as f32 (half and float
    /// exactly, unsigned integers exactly up to 2^24).
    pub sample: SampleType,
    /// Every sample of the channel, in pixel order; pixel `i`'s are `counts[i]..counts[i + 1]`.
    pub samples: Vec<f32>,
}

/// The samples of a deep part. All channels share the sample layout, so a sample is one index
/// into every channel.
#[derive(Debug, Clone, PartialEq)]
pub struct DeepImage {
    pub width: u32,
    pub height: u32,
    /// The stage the picture opens on: the display window size, when the file declares one the
    /// data region fits in and the limits allow (each side at most 32768). `None` otherwise: the
    /// picture is the data window alone.
    pub stage: Option<(u32, u32)>,
    /// Where the data window's (0, 0) sits on the stage; `(0, 0)` without a stage.
    pub origin: (i32, i32),
    /// The sample channels in file order.
    pub channels: Vec<DeepChannel>,
    /// Cumulative sample counts, `width × height + 1` entries: pixel `i` holds the samples
    /// `counts[i]..counts[i + 1]` of every channel.
    pub counts: Vec<u64>,
}

impl DeepImage {
    /// A channel by its exact name.
    pub fn channel(&self, name: &str) -> Option<&DeepChannel> {
        self.channels.iter().find(|c| c.name == name)
    }

    /// The samples of one pixel, as indices into every channel's `samples`.
    pub fn sample_range(&self, pixel: usize) -> std::ops::Range<u64> {
        let end = self.counts.get(pixel + 1).copied().unwrap_or(0);
        self.counts.get(pixel).copied().unwrap_or(end)..end
    }

    /// The total number of samples over all pixels.
    pub fn total_samples(&self) -> u64 {
        self.counts.last().copied().unwrap_or(0)
    }
}

/// A flattened deep picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Flat {
    pub width: u32,
    pub height: u32,
    pub layout: ColorLayout,
    /// `width × height × layout.channels()` straight-alpha samples, top row first.
    pub samples: Vec<f32>,
    /// `Some(origin)` when the picture is the display-window stage with the data placed at `origin`.
    pub stage_origin: Option<(i32, i32)>,
    /// The most samples one pixel had.
    pub max_samples_per_pixel: u32,
}

/// Reads the samples of the first deep part. `Err(Unsupported)` when the file has none.
pub fn decode_deep(bytes: &[u8], limits: &Limits) -> Result<DeepImage> {
    check_previews(bytes)?;
    let meta = MetaData::read_from_buffered(Cursor::new(bytes), false).map_err(err)?;
    decode_deep_meta(&meta, bytes, limits)
}
// --- checked little-endian reads -------------------------------------------------------------
// Every length and offset comes from the file, so every read is bounds-checked and a short
// read is a malformed file, never a panic.

fn read_exact<'a>(cur: &mut Cursor<&'a [u8]>, n: usize, what: &str) -> Result<&'a [u8]> {
    // Borrow packed payloads from the input. Copying them here allocates before count-table
    // admission and needlessly duplicates the table even when the second pass just skips it.
    let truncated = || err(format!("truncated file while reading {what}"));
    let start = usize::try_from(cur.position()).map_err(|_| truncated())?;
    let end = start.checked_add(n).ok_or_else(truncated)?;
    let bytes = *cur.get_ref();
    let v = bytes.get(start..end).ok_or_else(truncated)?;
    cur.set_position(end as u64);
    Ok(v)
}

fn i32(cur: &mut Cursor<&[u8]>) -> Result<i32> {
    let b = read_exact(cur, 4, "an i32")?;
    Ok(i32::from_le_bytes([
        b.first().copied().unwrap_or(0),
        b.get(1).copied().unwrap_or(0),
        b.get(2).copied().unwrap_or(0),
        b.get(3).copied().unwrap_or(0),
    ]))
}

fn u64(cur: &mut Cursor<&[u8]>) -> Result<u64> {
    let b = read_exact(cur, 8, "a u64")?;
    let a: [u8; 8] = b.try_into().unwrap_or([0; 8]);
    Ok(u64::from_le_bytes(a))
}

fn seek_to(cur: &mut Cursor<&[u8]>, pos: u64) -> Result<()> {
    if pos > cur.get_ref().len() as u64 {
        return Err(err(format!("chunk offset {pos} points past the end of the file")));
    }
    cur.seek(SeekFrom::Start(pos)).map_err(err)?;
    Ok(())
}

// --- decompression ----------------------------------------------------------------------------

/// Undo the ZIP predictor: each byte was stored as its difference to the previous byte plus
/// 128 (so a zero difference is 0x80), modulo 256. Applies to the sample-count table and the
/// sample data alike.
fn differences_to_samples(buf: &mut [u8]) {
    if let Some(&first) = buf.first() {
        let mut prev = i16::from(first);
        for b in buf.iter_mut().skip(1) {
            let s = (prev + i16::from(*b) - 128) as u8;
            prev = i16::from(s);
            *b = s;
        }
    }
}

/// Undo the ZIP half-split: the stream is `[first half][second half]` with the second half
/// holding every second byte of the original; weave them back together. The scratch copy is
/// allocated fallibly, so a buffer that passed `max_bytes` but not the allocator is an error.
fn interleave_byte_blocks(buf: &mut [u8]) -> Result<()> {
    let half = buf.len().div_ceil(2);
    let mut out = alloc_zeroed::<u8>(buf.len())?;
    let mut k = 0usize;
    for i in 0..buf.len() / 2 {
        let (a, b) = (buf.get(i).copied(), buf.get(half + i).copied());
        if let (Some(a), Some(b), Some(pair)) = (a, b, out.get_mut(k..k + 2)) {
            if let Some(byte) = pair.first_mut() {
                *byte = a;
            }
            if let Some(byte) = pair.get_mut(1) {
                *byte = b;
            }
        }
        k += 2;
    }
    if buf.len() % 2 == 1 && half > 0 {
        let last = buf.get(half - 1).copied();
        if let (Some(last), Some(slot)) = (last, out.get_mut(k)) {
            *slot = last;
        }
    }
    buf.copy_from_slice(&out);
    Ok(())
}

/// Make room in `out` for `extra` more bytes, `limit` in all. Grows by doubling like `Vec`,
/// but never reserves past `limit`: callers admit exactly that size against `max_bytes`, and
/// plain doubling reserved up to twice as much. Fallible, so a failed allocation is an error.
fn reserve_up_to(out: &mut Vec<u8>, extra: usize, limit: usize, what: &str) -> Result<()> {
    let need = out.len().saturating_add(extra);
    if need > limit {
        return Err(err(format!("{what}: data expands past {limit} bytes")));
    }
    if need > out.capacity() {
        let target = out.capacity().saturating_mul(2).max(need).min(limit);
        out.try_reserve_exact(target.saturating_sub(out.len()))
            .map_err(|_| Error::LimitExceeded(format!("cannot allocate {target} bytes for {what}")))?;
    }
    Ok(())
}

/// OpenEXR RLE tokens: a negative byte `n` means `-n` literal bytes follow, a non-negative
/// byte `n` means the next byte repeats `n + 1` times (as `ImfRle.cpp` and the `exr` crate).
fn rle_decode(src: &[u8], expected: usize, what: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    reserve_up_to(&mut out, expected.min(1 << 20), expected, what)?;
    let mut i = 0;
    while i < src.len() {
        let count = src[i] as i8;
        i += 1;
        let n = if count < 0 { (-(count as i32)) as usize } else { count as usize + 1 };
        if out.len().saturating_add(n) > expected {
            return Err(err(format!("{what}: RLE data expands past {expected} bytes")));
        }
        reserve_up_to(&mut out, n, expected, what)?;
        if count < 0 {
            let end = i.saturating_add(n);
            let run = src.get(i..end.min(src.len())).ok_or_else(|| err(format!("{what}: RLE run points past the compressed data")))?;
            if run.len() != n {
                return Err(err(format!("{what}: RLE run points past the compressed data")));
            }
            out.extend_from_slice(run);
            i = end;
        } else {
            let b = src.get(i).copied().ok_or_else(|| err(format!("{what}: RLE run points past the compressed data")))?;
            i += 1;
            out.resize(out.len() + n, b);
        }
    }
    if out.len() != expected {
        return Err(err(format!("{what}: RLE data expands to {} bytes, expected {expected}", out.len())));
    }
    Ok(out)
}

/// Decompress `what` to exactly `expected` bytes. Like flat blocks, a payload that already
/// has the uncompressed length is stored raw whatever the declared compression.
fn decompress(compression: Compression, packed: &[u8], expected: usize, what: &str) -> Result<Vec<u8>> {
    if packed.len() == expected {
        let mut out = Vec::new();
        reserve_up_to(&mut out, expected, expected, what)?;
        out.extend_from_slice(packed);
        return Ok(out);
    }
    match compression {
        Compression::Uncompressed => Err(err(format!("{what}: expected {expected} uncompressed bytes, found {}", packed.len()))),
        Compression::RLE => {
            // Like ZIPS, RLE runs over the predicted, byte-split stream.
            let mut out = rle_decode(packed, expected, what)?;
            differences_to_samples(&mut out);
            interleave_byte_blocks(&mut out)?;
            Ok(out)
        }
        Compression::ZIP1 => {
            // Inflate straight into `out`, grown with `reserve_up_to` rather than by
            // `read_to_end`, whose doubling reserves past `expected`. Once `expected` bytes are
            // in, a one-byte read checks that the stream ends there.
            let malformed = || err(format!("{what}: zlib data is malformed"));
            let mut zlib = flate2::bufread::ZlibDecoder::new(packed);
            let mut out = Vec::new();
            let mut filled = 0;
            while filled < expected {
                if filled == out.len() {
                    let grow = out.len().max(1 << 13).min(expected - filled);
                    reserve_up_to(&mut out, grow, expected, what)?;
                    out.resize(filled + grow, 0);
                }
                let n = zlib.read(out.get_mut(filled..).unwrap_or_default()).map_err(|_| malformed())?;
                if n == 0 {
                    break;
                }
                filled += n;
            }
            out.truncate(filled);
            if filled == expected && zlib.read(&mut [0u8; 1]).map_err(|_| malformed())? != 0 {
                return Err(err(format!("{what}: zlib data expands past {expected} bytes")));
            }
            if out.len() != expected {
                return Err(err(format!("{what}: zlib data expands to {} bytes, expected {expected}", out.len())));
            }
            differences_to_samples(&mut out);
            interleave_byte_blocks(&mut out)?;
            Ok(out)
        }
        other => Err(unsup(format!("deep data with {other:?} compression (NONE, RLE and ZIPS are supported)"))),
    }
}

// --- the deep chunk stream ---------------------------------------------------------------------

/// One parsed chunk: the block it covers, plus where to find the compressed sample data in
/// the second pass. Its pixels' sample counts live in the decode's global index, so a chunk
/// holds no per-pixel data.
struct DeepChunk {
    /// The block's rows within the data window.
    rows: std::ops::Range<usize>,
    /// The block's columns within the data window (clipped to it for an edge tile).
    cols: std::ops::Range<usize>,
    /// File offset of the chunk start.
    pos: u64,
    /// Size of the compressed sample data in the file.
    packed_data: u64,
    /// The decompressed sample-data size the file declares (validated against the table).
    unpacked_data: u64,
}

fn sample_bytes(t: ExrSample) -> usize {
    match t {
        ExrSample::U32 | ExrSample::F32 => 4,
        ExrSample::F16 => 2,
    }
}

/// One stored sample as f32. `b` holds [`sample_bytes`] bytes; a short slice reads as zero.
fn sample_value(t: ExrSample, b: &[u8]) -> f32 {
    match t {
        ExrSample::F16 => b.try_into().map(|a: [u8; 2]| f16::from_le_bytes(a).to_f32()).unwrap_or(0.0),
        ExrSample::F32 => b.try_into().map(f32::from_le_bytes).unwrap_or(0.0),
        ExrSample::U32 => b.try_into().map(|a: [u8; 4]| u32::from_le_bytes(a) as f32).unwrap_or(0.0),
    }
}

/// [`decode_deep`] on headers read without validation (deep headers fail it).
fn decode_deep_meta(meta: &MetaData, bytes: &[u8], limits: &Limits) -> Result<DeepImage> {
    let header_index = meta.headers.iter().position(|h| h.deep).ok_or_else(|| unsup("the file has no deep part"))?;
    let header = meta.headers.get(header_index).ok_or_else(|| unsup("the file has no deep part"))?;
    let compression = header.compression;
    if !matches!(compression, Compression::Uncompressed | Compression::RLE | Compression::ZIP1) {
        return Err(unsup(format!("deep data with {compression:?} compression (NONE, RLE and ZIPS are supported)")));
    }

    let dw = header.data_window();
    let (w, h) = (dw.size.0, dw.size.1);
    if w == 0 || h == 0 {
        return Err(err("the data window is empty"));
    }
    if w > u32::MAX as usize || h > u32::MAX as usize {
        return Err(err("the data window is too large"));
    }
    let y0 = dw.position.1;
    let npixels = w.checked_mul(h).ok_or_else(|| err("the data window is too large"))?;
    limits.check_bytes(w as u32, h as u32, (header.channels.list.len().max(1) as u64) * 4)?;
    // The per-pixel bookkeeping is the cumulative sample-count index the result returns
    // (`DeepImage::counts`: one u64 per pixel, plus one), live from the first chunk to the end.
    // Charge it before anything is allocated; the count tables and the samples are admitted
    // against what it leaves. The data-window check above budgets only four bytes a pixel.
    let index_len = npixels.checked_add(1).ok_or_else(|| err("the data window is too large"))?;
    let index_bytes = (index_len as u64).checked_mul(8).ok_or_else(|| err("the data window is too large"))?;
    if index_bytes > limits.max_bytes {
        return Err(Error::LimitExceeded(format!(
            "the sample-count index of {w}x{h} deep pixels needs {index_bytes} bytes, exceeding max_bytes {}",
            limits.max_bytes
        )));
    }
    let budget = limits.max_bytes - index_bytes;

    // The display window is the full stage the renderer composed onto (a render pass's data
    // window is often its crop). Open on the stage when it plausibly holds the data region:
    // sane size for the limits, and the data region inside it. Otherwise fall back to the
    // data window alone, as before.
    let display = header.shared_attributes.display_window;
    let display_size = (display.size.0.max(1) as u32, display.size.1.max(1) as u32);
    let (stage, origin) = match display_size {
        (sw, sh)
            if sw <= 1 << 15
                && sh <= 1 << 15
                && limits.check_bytes(sw, sh, 4).is_ok()
                && dw.position.0 >= display.position.0
                && dw.position.1 >= display.position.1
                && dw.position.0.saturating_add(w as i32) <= display.position.0.saturating_add(sw as i32)
                && dw.position.1.saturating_add(h as i32) <= display.position.1.saturating_add(sh as i32) =>
        {
            (Some((sw, sh)), (dw.position.0 - display.position.0, dw.position.1 - display.position.1))
        }
        _ => (None, (0, 0)),
    };

    let exr_types: Vec<ExrSample> = header.channels.list.iter().map(|c| c.sample_type).collect();
    let bytes_per_sample: usize = exr_types.iter().map(|t| sample_bytes(*t)).sum();
    let multipart = meta.requirements.has_multiple_layers;

    // The offset tables follow the headers: `chunk_count` u64 per header, in header order.
    // Our part's table starts after the tables of all earlier parts.
    let mut table_pos = header_end(bytes)?;
    for h_ in meta.headers.iter().take(header_index) {
        table_pos = table_pos.saturating_add((h_.chunk_count.min(1 << 24) as u64).saturating_mul(8));
    }
    let mut cur = Cursor::new(bytes);
    seek_to(&mut cur, table_pos)?;
    let mut offsets = Vec::with_capacity(header.chunk_count.min(1 << 20));
    for _ in 0..header.chunk_count {
        offsets.push(u64(&mut cur)?);
    }
    let tables_end = cur.position();

    // Block geometry.
    let (tiles_x, tile_w, tile_h, slots) = match &header.blocks {
        BlockDescription::ScanLines => (0usize, w, 1usize, h), // one line per block for NONE/RLE/ZIPS
        BlockDescription::Tiles(t) => {
            if t.level_mode != LevelMode::Singular {
                return Err(unsup("deep mip-map or rip-map tiles (single-level tiles are supported)"));
            }
            let (tw, th) = (t.tile_size.0.max(1), t.tile_size.1.max(1));
            let tx = w.saturating_add(tw - 1) / tw;
            let ty = h.saturating_add(th - 1) / th;
            (tx, tw, th, tx.checked_mul(ty).ok_or_else(|| err("tile count overflows"))?)
        }
    };

    // Each block has exactly one chunk (a duplicate or a missing one is malformed), so a table
    // of any other length is refused here, before the per-block bookkeeping is sized by the
    // header's geometry: `seen` and `chunks` stay proportional to the offset table in the file.
    if offsets.len() != slots {
        return Err(err(format!("the offset table lists {} chunks, the data window needs {slots}", offsets.len())));
    }
    // Per-pixel sample counts in `counts[p + 1]`, made cumulative after the first pass.
    let mut counts = alloc_zeroed::<u64>(index_len)?;
    let mut seen = vec![false; slots];
    let mut chunks: Vec<DeepChunk> = Vec::with_capacity(offsets.len());

    for &off in &offsets {
        if off < tables_end || off >= bytes.len() as u64 {
            return Err(err(format!("chunk offset {off} is outside the chunk area of the file")));
        }
        seek_to(&mut cur, off)?;
        if multipart {
            let part = i32(&mut cur)?;
            if part < 0 || part as usize != header_index {
                return Err(err(format!("chunk declares part {part}, expected {header_index}")));
            }
        }
        // Block coordinates → the block's pixel bounds and its slot.
        let (block_rows, block_cols, slot): (std::ops::Range<usize>, std::ops::Range<usize>, usize) = match &header.blocks {
            BlockDescription::ScanLines => {
                let y = i32(&mut cur)?;
                let row = match y.checked_sub(y0) {
                    Some(d) if d >= 0 && (d as usize) < h => d as usize,
                    _ => return Err(err(format!("scan line block y={y} is outside the data window"))),
                };
                (row..row + 1, 0..w, row)
            }
            BlockDescription::Tiles(_) => {
                let (tx, ty, lx, ly) = (i32(&mut cur)?, i32(&mut cur)?, i32(&mut cur)?, i32(&mut cur)?);
                if (lx, ly) != (0, 0) {
                    return Err(err(format!("deep tile at level ({lx},{ly}); only level 0 exists in a single-level file")));
                }
                if tx < 0 || ty < 0 || (tx as usize) >= tiles_x || (ty as usize) * tile_h >= h {
                    return Err(err(format!("tile ({tx},{ty}) is outside the data window")));
                }
                let (bx, by) = ((tx as usize) * tile_w, (ty as usize) * tile_h);
                let rows = (by + tile_h).min(h);
                let cols = (bx + tile_w).min(w);
                let slot = (ty as usize).saturating_mul(tiles_x) + tx as usize;
                (by..rows, bx..cols, slot)
            }
        };
        let block_len = block_rows.len().saturating_mul(block_cols.len());
        if seen.get(slot).copied().unwrap_or(true) {
            return Err(err(format!("two chunks cover block {slot}")));
        }
        if let Some(s) = seen.get_mut(slot) {
            *s = true;
        }

        let packed_table = u64(&mut cur)?;
        let packed_data = u64(&mut cur)?;
        let unpacked_data = u64(&mut cur)?;
        let remaining = (bytes.len() as u64).saturating_sub(cur.position());
        if packed_table > remaining || packed_data > remaining.saturating_sub(packed_table) {
            return Err(err("deep chunk sizes point past the end of the file"));
        }
        let what = "the sample-count table";
        let table_bytes = read_exact(&mut cur, packed_table as usize, what)?;
        let expected_table = block_len.checked_mul(4).ok_or_else(|| err("count table overflows"))?;
        // A compressed clipped table also needs its scratch copy. The data-window check only
        // budgets four bytes per channel per pixel, insufficient for a one-channel image.
        // An equal-sized payload takes decompress's raw path, which allocates no scratch.
        // Both are admitted against what the sample-count index leaves of `max_bytes`.
        let buffers = if table_bytes.len() == expected_table { 1 } else { 2 };
        let clipped_need = (expected_table as u64).saturating_mul(buffers);
        if clipped_need > budget {
            return Err(Error::LimitExceeded(format!(
                "{what} needs {clipped_need} bytes including decompression scratch, on top of the {index_bytes} byte sample-count index, exceeding max_bytes {}",
                limits.max_bytes
            )));
        }
        // An edge tile's table may still be stored at the full tile size (its pixels first, in
        // rows of the clipped width, then unused entries), as the OpenEXR library writes it.
        // That size comes from the header's tile size alone, which nothing ties to the data
        // window, so it is tried only when `max_bytes` has room for the table and for the copy
        // that undoes its byte split. Otherwise only the clipped size is read.
        let full_table = tile_w.checked_mul(tile_h).and_then(|n| n.checked_mul(4));
        let raw = match full_table {
            // Scan lines and interior tiles: the full size is the clipped size.
            Some(full) if full <= expected_table => decompress(compression, table_bytes, expected_table, what)?,
            Some(full) if (full as u64).saturating_mul(2) <= budget => {
                decompress(compression, table_bytes, full, what).or_else(|_| decompress(compression, table_bytes, expected_table, what))?
            }
            _ => decompress(compression, table_bytes, expected_table, what).map_err(|_| {
                // Four bytes per pixel of the tile, twice: the table and its scratch copy.
                let need = (tile_w as u64).saturating_mul(tile_h as u64).saturating_mul(8);
                Error::LimitExceeded(format!(
                    "{what} of a {tile_w}x{tile_h} deep tile does not decode at the clipped size, and at the full tile size it needs {need} bytes with its scratch copy, on top of the {index_bytes} byte sample-count index, exceeding max_bytes {}",
                    limits.max_bytes
                ))
            })?,
        };

        // The table is cumulative within each row of the block (a tile restarts at every row);
        // each pixel's own count is the difference to its left neighbour's entry.
        let mut entries = raw.as_chunks::<4>().0.iter();
        let mut total = 0u64;
        for yy in block_rows.clone() {
            let mut prev = 0u32;
            for xx in block_cols.clone() {
                let v = u32::from_le_bytes(*entries.next().ok_or_else(|| err("the count table is short"))?);
                if v < prev {
                    return Err(err("the sample-count table decreases (malformed cumulative counts)"));
                }
                let p = yy.saturating_mul(w).saturating_add(xx).saturating_add(1);
                if let Some(c) = counts.get_mut(p) {
                    *c = u64::from(v - prev);
                }
                prev = v;
            }
            total = total.checked_add(u64::from(prev)).ok_or_else(|| err("the block's sample count overflows"))?;
        }
        let expected_data = total.checked_mul(bytes_per_sample as u64).ok_or_else(|| err("the sample data size overflows"))?;
        if expected_data != unpacked_data {
            return Err(err(format!("the chunk declares {unpacked_data} bytes of sample data, the count table implies {expected_data}")));
        }
        chunks.push(DeepChunk { rows: block_rows, cols: block_cols, pos: off, packed_data, unpacked_data });
    }
    if seen.iter().any(|s| !s) {
        let missing = seen.iter().filter(|s| !**s).count();
        return Err(err(format!("{missing} of {slots} chunks are missing from the offset table")));
    }

    // Global cumulative counts in raster order, in place: pixel `p` holds the samples
    // `counts[p]..counts[p + 1]`.
    let mut acc = 0u64;
    for c in counts.iter_mut() {
        acc = acc.checked_add(*c).ok_or_else(|| err("the total sample count overflows"))?;
        *c = acc;
    }
    let total_samples = acc;
    let sample_bytes_total = total_samples.checked_mul(exr_types.len() as u64 * 4).ok_or_else(|| err("the sample buffer size overflows"))?;
    if sample_bytes_total > budget {
        return Err(Error::LimitExceeded(format!(
            "deep samples need {sample_bytes_total} bytes on top of the {index_bytes} byte sample-count index, exceeding max_bytes {}",
            limits.max_bytes
        )));
    }
    let total_samples =
        usize::try_from(total_samples).map_err(|_| Error::LimitExceeded(format!("{total_samples} deep samples do not fit in memory")))?;

    let mut channels: Vec<DeepChannel> = Vec::with_capacity(header.channels.list.len());
    for c in &header.channels.list {
        channels.push(DeepChannel { name: c.name.to_string(), sample: SampleType::of(c.sample_type), samples: alloc_zeroed::<f32>(total_samples)? });
    }

    // Second pass: the compressed sample data, placed row by row (a block's channel data is
    // contiguous in the file, but a tile's rows interleave with other tiles' in raster order).
    for chunk in &chunks {
        seek_to(&mut cur, chunk.pos)?;
        if multipart {
            let _part = i32(&mut cur)?;
        }
        match &header.blocks {
            BlockDescription::ScanLines => {
                let _y = i32(&mut cur)?;
            }
            BlockDescription::Tiles(_) => {
                let _coords = (i32(&mut cur)?, i32(&mut cur)?, i32(&mut cur)?, i32(&mut cur)?);
            }
        }
        let packed_table = u64(&mut cur)?;
        let _packed_data = u64(&mut cur)?;
        let _unpacked_data = u64(&mut cur)?;
        // The count table sits between the sizes and the sample data; skip its bytes.
        read_exact(&mut cur, packed_table as usize, "the sample-count table")?;
        let data_bytes = read_exact(&mut cur, chunk.packed_data as usize, "the deep sample data")?;
        let raw = decompress(compression, data_bytes, chunk.unpacked_data as usize, "the deep sample data")?;
        // Row by row (a scan line block is one row), and within a row channel by channel, as
        // flat tiles are laid out: each pixel's samples are contiguous within its channel. A
        // block row is a run of consecutive pixels in raster order, so its samples are the
        // contiguous range `counts[first]..counts[last + 1]` of every output channel.
        let mut in_off = 0usize;
        for yy in chunk.rows.clone() {
            let first = yy.saturating_mul(w).saturating_add(chunk.cols.start);
            let end = yy.saturating_mul(w).saturating_add(chunk.cols.end);
            let index = |p: usize| counts.get(p).copied().and_then(|c| usize::try_from(c).ok()).ok_or_else(|| err("pixel index out of range"));
            let (row_from, row_to) = (index(first)?, index(end)?);
            let row_total = row_to.checked_sub(row_from).ok_or_else(|| err("the sample-count index decreases"))?;
            for (ci, t) in exr_types.iter().enumerate() {
                let sb = sample_bytes(*t);
                let ch_bytes = row_total.checked_mul(sb).ok_or_else(|| err("channel size overflows"))?;
                let stop = in_off.checked_add(ch_bytes).ok_or_else(|| err("channel size overflows"))?;
                let src = raw.get(in_off..stop).ok_or_else(|| err("the sample data is short"))?;
                in_off = stop;
                let out = channels.get_mut(ci).ok_or_else(|| err("channel index out of range"))?;
                let dst = out.samples.get_mut(row_from..row_to).ok_or_else(|| err("the sample data is short"))?;
                for (slot, b) in dst.iter_mut().zip(src.chunks_exact(sb)) {
                    *slot = sample_value(*t, b);
                }
            }
        }
    }

    Ok(DeepImage { width: w as u32, height: h as u32, stage, origin, channels, counts })
}

/// Where the headers end and the offset tables begin. [`MetaData::read_from_buffered`]
/// consumes exactly the version field and the headers, so re-reading with a fresh cursor
/// and taking its end position yields it.
fn header_end(bytes: &[u8]) -> Result<u64> {
    let mut cur = Cursor::new(bytes);
    MetaData::read_from_buffered(&mut cur, false).map_err(err)?;
    Ok(cur.position())
}

/// Composite the deep samples into a flat image: per pixel, samples sorted by Z (stable; the
/// file is assumed messy), front-to-back `over` with premultiplied colour, un-premultiplied
/// into straight alpha at the end. Without any alpha channel the front-most sample wins,
/// opaque. Z, ZBack and other deep channels influence the flat result only through the
/// ordering; volume samples (with ZBack) are approximated as point samples.
pub fn flatten(deep: &DeepImage, limits: &Limits) -> Result<Flat> {
    let npx = deep.width as usize * deep.height as usize;
    let has = |n: &str| deep.channel(n).is_some();
    let rgb = has("R") && has("G") && has("B");
    let gray = !rgb && (has("Y") || deep.channels.len() == 1);
    let layout = if rgb || gray {
        if has("A") || has("AR") || has("AG") || has("AB") {
            if rgb { ColorLayout::Rgba } else { ColorLayout::YA }
        } else if rgb {
            ColorLayout::Rgb
        } else {
            ColorLayout::Y
        }
    } else {
        return Err(unsup("no R/G/B or Y channels in the deep part"));
    };
    let nc = layout.channels();
    // Admit the flattened output at its true size: nc f32 samples per pixel. The deep decode
    // admitted the data window at four bytes per pixel, before the flattened channel layout
    // was known, so re-check here with the actual layout and f32 sample size.
    limits.check_bytes(deep.width, deep.height, nc as u64 * 4)?;
    let samples = |n: &str| -> &[f32] { deep.channel(n).map(|c| c.samples.as_slice()).unwrap_or(&[]) };
    let (r, g, b) = (samples("R"), samples("G"), samples("B"));
    let (ar, ag, ab) = (samples("AR"), samples("AG"), samples("AB"));
    let (a, z) = (samples("A"), samples("Z"));
    let y = samples("Y");
    let lum = if y.is_empty() { deep.channels.first().map(|c| c.samples.as_slice()).unwrap_or(&[]) } else { y };

    let mut out = alloc_zeroed::<f32>(npx * nc)?;
    let mut order: Vec<u32> = Vec::new();
    let mut max_samples = 0u32;
    for p in 0..npx {
        let range = deep.sample_range(p);
        let (s0, n) = (range.start as usize, (range.end - range.start) as usize);
        max_samples = max_samples.max(u32::try_from(n).unwrap_or(u32::MAX));
        if n == 0 {
            continue; // no samples: transparent black
        }
        order.clear();
        order.extend(0u32..n as u32);
        if !z.is_empty() {
            // Stable sort by Z; a NaN depth (invalid) sorts last, deterministically.
            order.sort_by(|&i, &j| {
                let zi = z.get(s0 + i as usize).copied().unwrap_or(f32::NAN);
                let zj = z.get(s0 + j as usize).copied().unwrap_or(f32::NAN);
                zi.total_cmp(&zj)
            });
        }
        let at = |ch: &[f32], i: u32| ch.get(s0 + i as usize).copied().unwrap_or(0.0);
        // Per-colour-channel alpha: AR/AG/AB when present, else A, else opaque.
        let alpha = |i: u32| -> [f32; 3] {
            let s = at(a, i);
            [
                if !a.is_empty() && ar.is_empty() {
                    s
                } else if !ar.is_empty() {
                    at(ar, i)
                } else {
                    1.0
                },
                if !a.is_empty() && ag.is_empty() {
                    s
                } else if !ag.is_empty() {
                    at(ag, i)
                } else {
                    1.0
                },
                if !a.is_empty() && ab.is_empty() {
                    s
                } else if !ab.is_empty() {
                    at(ab, i)
                } else {
                    1.0
                },
            ]
        };
        let mut cacc = [0f32; 3]; // premultiplied colour
        let mut aacc = [0f32; 3]; // accumulated alpha
        for &i in &order {
            let ai = alpha(i);
            let src = if rgb { [at(r, i), at(g, i), at(b, i)] } else { [at(lum, i); 3] };
            let mut covered = true;
            for k in 0..3 {
                let t = (1.0 - aacc[k]).max(0.0);
                cacc[k] += src[k] * t;
                aacc[k] += t * ai[k];
                covered &= aacc[k] >= 1.0;
            }
            if covered {
                break; // everything behind is hidden
            }
        }
        let a_out = if a.is_empty() && ar.is_empty() && ag.is_empty() && ab.is_empty() { 1.0 } else { (aacc[0] + aacc[1] + aacc[2]) / 3.0 };
        let a_clamped = a_out.clamp(0.0, 1.0);
        // Un-premultiply into straight alpha (the deep colours are premultiplied).
        let straight = |c: f32| if a_clamped > 1e-6 { c / a_clamped } else { 0.0 };
        let base = p * nc;
        // Written by layout: RGB(A), gray(+A) — a fixed RGBA quadruple would put a luminance
        // copy into a GrayA image's alpha slot.
        // A fixed array plus a length: this runs once per pixel, so no allocation.
        let (vals, len) = match (rgb, nc) {
            (true, 4) => ([straight(cacc[0]), straight(cacc[1]), straight(cacc[2]), a_clamped], 4),
            (true, _) => ([straight(cacc[0]), straight(cacc[1]), straight(cacc[2]), 0.0], 3),
            (false, 2) => ([straight(cacc[0]), a_clamped, 0.0, 0.0], 2),
            (false, _) => ([straight(cacc[0]), 0.0, 0.0, 0.0], 1),
        };
        for (k, v) in vals.into_iter().take(len).enumerate() {
            if let Some(slot) = out.get_mut(base + k) {
                *slot = v;
            }
        }
    }
    // Open on the full display-window stage when the file declares one: the renderer's data
    // window is its crop, and the composition lives on the whole stage. The composed region
    // pads to the stage at its offset, transparent elsewhere.
    match deep.stage {
        Some((sw, sh)) => {
            // The stage can be larger than the data window: admit it at its real size first.
            limits.check_bytes(sw, sh, nc as u64 * 4)?;
            let mut padded = alloc_zeroed::<f32>(sw as usize * sh as usize * nc)?;
            let row_len = deep.width as usize * nc;
            let dx = deep.origin.0.max(0) as usize;
            for row in 0..deep.height as usize {
                let src = row * row_len;
                let dst = (row + deep.origin.1.max(0) as usize) * sw as usize + dx;
                if let (Some(d), Some(r)) = (padded.get_mut(dst * nc..dst * nc + row_len), out.get(src..src + row_len)) {
                    d.copy_from_slice(r);
                }
            }
            Ok(Flat { width: sw, height: sh, layout, samples: padded, stage_origin: Some(deep.origin), max_samples_per_pixel: max_samples })
        }
        None => Ok(Flat { width: deep.width, height: deep.height, layout, samples: out, stage_origin: None, max_samples_per_pixel: max_samples }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use std::io::Write as _;

    #[test]
    fn packed_reads_borrow_input_and_reject_bad_sizes_before_allocation() {
        let bytes = [1u8, 2, 3, 4];
        let mut cur = Cursor::new(bytes.as_slice());
        let prefix = read_exact(&mut cur, 3, "x").expect("three input bytes");
        assert_eq!(prefix, &[1, 2, 3]);
        assert_eq!(prefix.as_ptr(), bytes.as_ptr());
        assert_eq!(cur.position(), 3);
        assert!(matches!(read_exact(&mut cur, usize::MAX, "x"), Err(Error::Malformed(_))));
        assert_eq!(cur.position(), 3);
        assert_eq!(read_exact(&mut cur, 1, "x").expect("last byte"), &[4]);
        assert!(matches!(read_exact(&mut cur, 1, "x"), Err(Error::Malformed(_))));
        cur.set_position(u64::MAX);
        assert!(matches!(read_exact(&mut cur, 1, "x"), Err(Error::Malformed(_))));
    }

    /// What the writer stores before compressing: the byte split, then the +128 predictor.
    fn predicted(raw: &[u8]) -> Vec<u8> {
        let half = raw.len().div_ceil(2);
        let mut out = vec![0u8; raw.len()];
        for (i, &v) in raw.iter().enumerate() {
            out[if i % 2 == 0 { i / 2 } else { half + i / 2 }] = v;
        }
        let mut prev = 0i16;
        for (i, b) in out.iter_mut().enumerate() {
            let v = i16::from(*b);
            if i > 0 {
                *b = (v - prev + 128) as u8;
            }
            prev = v;
        }
        out
    }

    /// #4958: callers admit `expected` bytes for a decompressed buffer (twice that for a count
    /// table, with the copy that undoes its byte split), so decompression must not reserve
    /// more. `Vec`'s doubling gave a 4 MiB ZIPS table 8 MiB of capacity and a 3 MiB RLE table
    /// 4 MiB, so the "admitted" table and its copy took up to three times the table size.
    #[test]
    fn decompressed_buffers_reserve_no_more_than_expected() {
        for n in [1usize << 20, 2 << 20, (3 << 20) + 1, 4 << 20, 5 << 20] {
            let raw = vec![0u8; n];
            let p = predicted(&raw);
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
            z.write_all(&p).expect("write to a Vec");
            let zips = z.finish().expect("finish a Vec");
            assert!(decompress(Compression::ZIP1, &zips[..zips.len() - 1], n, "the table").is_err());
            // RLE: the first byte as a literal, then runs of up to 128 copies of the next.
            let mut rle = vec![0xFF, p[0]];
            for run in p[1..].chunks(128) {
                rle.extend_from_slice(&[(run.len() - 1) as u8, run[0]]);
            }
            for (compression, packed) in [(Compression::ZIP1, &zips), (Compression::RLE, &rle)] {
                let out = decompress(compression, packed, n, "the table").expect("decompresses");
                assert_eq!(out, raw, "{compression:?}, {n} bytes");
                assert!(out.capacity() <= n, "{compression:?}: a {n} byte buffer reserved {} bytes", out.capacity());
            }
            // One byte short, the same data expands past the expected size and is refused.
            for (compression, packed) in [(Compression::ZIP1, &zips), (Compression::RLE, &rle)] {
                assert!(decompress(compression, packed, n - 1, "the table").is_err(), "{compression:?}, {n} bytes");
            }
        }
    }

    /// A size past the limit is malformed data, and a reservation the allocator refuses is an
    /// error rather than an abort.
    #[test]
    fn reservations_past_the_limit_or_the_allocator_are_errors() {
        let mut v = Vec::new();
        assert!(matches!(reserve_up_to(&mut v, 2, 1, "x"), Err(Error::Malformed(_))));
        assert!(matches!(reserve_up_to(&mut v, isize::MAX as usize + 1, usize::MAX, "x"), Err(Error::LimitExceeded(_))));
        assert_eq!(v.capacity(), 0);
    }
}

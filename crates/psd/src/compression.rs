//! Channel image data codecs: Raw, RLE (PackBits), ZIP, and ZIP with prediction.
//!
//! All decoded data is *planar* bytes in the file's native big-endian sample
//! order: `planes × height × row_bytes` where `row_bytes` depends on depth
//! (depth 1 packs 8 pixels per byte, MSB first).

use std::io::{Read, Write};

use crate::error::{PsdError, Result};
use crate::header::{Version, row_bytes};
use crate::io::Reader;

/// Maximum number of decoded bytes a single decode call may produce: 8 GiB on
/// 64-bit targets, 2 GiB elsewhere.
///
/// The decoders bound their output by their input on their own (RLE expands
/// at most 64x, Raw needs every byte present), so this is a backstop against
/// decompression bombs, not the main guard. It has to fit real PSBs: a
/// 30000² RGB merged image is 2.7 GB, and a 33000² 16-bit layer channel is
/// 2.2 GB, both of which a 2 GiB cap turned into empty layers (photocraft#375).
pub const MAX_DECODED_BYTES: u64 = if cfg!(target_pointer_width = "64") { 8 << 30 } else { 2 << 30 };

/// Compression method stored before channel / image data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Compression {
    /// 0: uncompressed.
    Raw,
    /// 1: PackBits RLE with a per-row byte-count table.
    Rle,
    /// 2: zlib without prediction.
    Zip,
    /// 3: zlib with horizontal delta prediction.
    ZipPrediction,
    /// Unknown method; data is preserved but cannot be decoded.
    Unknown(u16),
}

impl Compression {
    /// All four decodable methods.
    pub const ALL: [Compression; 4] = [Compression::Raw, Compression::Rle, Compression::Zip, Compression::ZipPrediction];

    /// From the stored value.
    pub fn from_u16(v: u16) -> Self {
        match v {
            0 => Compression::Raw,
            1 => Compression::Rle,
            2 => Compression::Zip,
            3 => Compression::ZipPrediction,
            v => Compression::Unknown(v),
        }
    }
    /// Stored value.
    pub fn as_u16(self) -> u16 {
        match self {
            Compression::Raw => 0,
            Compression::Rle => 1,
            Compression::Zip => 2,
            Compression::ZipPrediction => 3,
            Compression::Unknown(v) => v,
        }
    }
}

/// PackBits (Apple / TIFF) run-length coding as used by Photoshop RLE.
pub mod packbits {
    use crate::error::{PsdError, Result};

    /// Encodes one row, appending to `out`. Never emits the `-128` no-op.
    pub fn encode(src: &[u8], out: &mut Vec<u8>) {
        // Constant 16-bit samples with different bytes contain only literal packets.
        if src.len() >= 128
            && let Some(&[a, b]) = src.first_chunk::<2>()
            && a != b
        {
            let (blocks, tail) = src.as_chunks::<8>();
            if blocks.iter().all(|v| *v == [a, b, a, b, a, b, a, b]) && tail.iter().zip([a, b].into_iter().cycle()).all(|(&v, p)| v == p) {
                for chunk in src.chunks(128) {
                    out.push((chunk.len() - 1) as u8);
                    out.extend_from_slice(chunk);
                }
                return;
            }
        }
        packets(src, |header, bytes| {
            out.push(header);
            out.extend_from_slice(bytes);
        });
    }

    /// The encoded row length, without allocating or writing its payload.
    pub fn encoded_len(src: &[u8]) -> usize {
        let mut len = 0;
        packets(src, |_, bytes| len += 1 + bytes.len());
        len
    }

    /// Visit PackBits run and literal packets in their deterministic encoding order.
    fn packets(src: &[u8], mut emit: impl FnMut(u8, &[u8])) {
        let n = src.len();
        let mut i = 0;
        while i < n {
            // Length of the run starting at i.
            let mut run = 1;
            while i + run < n && run < 8 && src[i + run] == src[i] {
                run += 1;
            }
            if run == 8 {
                while run + 8 <= 128 && src.get(i + run..).and_then(|s| s.first_chunk::<8>()) == Some(&[src[i]; 8]) {
                    run += 8;
                }
                while i + run < n && run < 128 && src[i + run] == src[i] {
                    run += 1;
                }
            }
            // A literal never stops before a 2-byte run, so any run of >= 2
            // seen here starts a packet boundary: a run packet is never larger.
            if run >= 2 {
                emit((1i16 - run as i16) as i8 as u8, &src[i..i + 1]);
                i += run;
                continue;
            }
            // Literal: extend until a run of >= 3 begins or 128 bytes.
            let start = i;
            let mut len = 0;
            // Short packets keep the scalar scan; lookahead pays only on longer literals.
            while i < n && len < 16 {
                if i + 2 < n && src[i] == src[i + 1] && src[i] == src[i + 2] {
                    break;
                }
                i += 1;
                len += 1;
            }
            if len == 16 {
                while len + 8 <= 128 {
                    let Some(&[a, b, c, d, e, f, g, h, j, k]) = src.get(i..).and_then(|s| s.first_chunk::<10>()) else { break };
                    let word = u64::from_le_bytes([a, b, c, d, e, f, g, h]);
                    let different = (word ^ u64::from_le_bytes([b, c, d, e, f, g, h, j])) | (word ^ u64::from_le_bytes([c, d, e, f, g, h, j, k]));
                    // A zero byte marks a triple; scalar scanning keeps the first boundary.
                    if different.wrapping_sub(0x0101010101010101) & !different & 0x8080808080808080 != 0 {
                        break;
                    }
                    i += 8;
                    len += 8;
                }
                while i < n && len < 128 {
                    if i + 2 < n && src[i] == src[i + 1] && src[i] == src[i + 2] {
                        break;
                    }
                    i += 1;
                    len += 1;
                }
            }
            emit((len - 1) as u8, &src[start..start + len]);
        }
    }

    /// Returns the encoded form of `src`.
    pub fn encode_vec(src: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(src.len() + src.len() / 128 + 1);
        encode(src, &mut v);
        v
    }

    /// Decodes `src` into exactly `expected` bytes appended to `out`.
    ///
    /// Errors if the input is exhausted early or a packet would overflow the
    /// row. Trailing input after the row is complete is ignored except for
    /// `-128` no-op bytes (which are always permitted). Errors, rather than
    /// overflowing, when `out.len() + expected` does not fit in `usize`.
    pub(crate) fn decode_into(src: &[u8], expected: usize, out: &mut Vec<u8>) -> Result<()> {
        let target = out.len().checked_add(expected).ok_or(PsdError::LimitExceeded("PackBits output length overflows usize"))?;
        let mut i = 0;
        while out.len() < target {
            let Some(&h) = src.get(i) else {
                return Err(PsdError::Decompress("PackBits row ended early".into()));
            };
            i += 1;
            let h = h as i8;
            if h >= 0 {
                let len = h as usize + 1;
                let Some(lit) = src.get(i..i + len) else {
                    return Err(PsdError::Decompress("PackBits literal truncated".into()));
                };
                if len > target - out.len() {
                    return Err(PsdError::Decompress("PackBits literal overflows row".into()));
                }
                out.extend_from_slice(lit);
                i += len;
            } else if h != -128 {
                let len = 1 - h as isize;
                let len = len as usize;
                let Some(&b) = src.get(i) else {
                    return Err(PsdError::Decompress("PackBits run truncated".into()));
                };
                i += 1;
                if len > target - out.len() {
                    return Err(PsdError::Decompress("PackBits run overflows row".into()));
                }
                out.extend(std::iter::repeat_n(b, len));
            }
        }
        Ok(())
    }

    /// Decodes into a new vector of exactly `expected` bytes.
    pub fn decode(src: &[u8], expected: usize) -> Result<Vec<u8>> {
        let mut v = Vec::with_capacity(expected.min(src.len().saturating_mul(64)));
        decode_into(src, expected, &mut v)?;
        Ok(v)
    }
}

/// Geometry of a set of planes to encode/decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaneLayout {
    /// Number of planes (1 for a layer channel, channel count for the merged image).
    pub planes: usize,
    /// Pixels per row.
    pub width: usize,
    /// Rows per plane.
    pub height: usize,
    /// Bits per sample (1, 8, 16, 32).
    pub depth: u16,
    /// PSD vs PSB (affects RLE count width).
    pub version: Version,
}

impl PlaneLayout {
    /// Bytes per row.
    pub fn row_bytes(&self) -> usize {
        row_bytes(self.width, self.depth)
    }
    /// Total rows over all planes.
    pub fn rows(&self) -> Result<usize> {
        self.planes.checked_mul(self.height).ok_or(PsdError::LimitExceeded("row count overflow"))
    }
    /// Total decoded bytes without the [`MAX_DECODED_BYTES`] cap.
    pub fn total_bytes(&self) -> Result<u64> {
        (self.planes as u64)
            .checked_mul(self.height as u64)
            .and_then(|v| v.checked_mul(self.row_bytes() as u64))
            .ok_or(PsdError::LimitExceeded("decoded size overflow"))
    }

    /// Total decoded bytes, checked against [`MAX_DECODED_BYTES`].
    pub fn decoded_len(&self) -> Result<usize> {
        let total = self.total_bytes()?;
        if total > MAX_DECODED_BYTES {
            return Err(PsdError::LimitExceeded("decoded channel data exceeds MAX_DECODED_BYTES"));
        }
        usize::try_from(total).map_err(|_| PsdError::LimitExceeded("decoded size exceeds address space"))
    }
    fn count_size(&self) -> usize {
        if self.version.is_psb() { 4 } else { 2 }
    }
}

/// Decodes planar data (excluding the 2-byte compression field).
pub fn decode_planes(compression: Compression, data: &[u8], layout: &PlaneLayout) -> Result<Vec<u8>> {
    let total = layout.decoded_len()?;
    match compression {
        Compression::Raw => {
            if data.len() < total {
                return Err(PsdError::UnexpectedEof { offset: data.len(), needed: total - data.len() });
            }
            let mut out = Vec::new();
            out.try_reserve_exact(total).map_err(|_| PsdError::OutOfMemory("not enough memory for the decoded channel data"))?;
            out.extend_from_slice(&data[..total]);
            Ok(out)
        }
        Compression::Rle => decode_rle(data, layout, total),
        Compression::Zip => inflate_checked(data, total as u64, true),
        Compression::ZipPrediction => {
            let mut v = inflate_checked(data, total as u64, true)?;
            unpredict(&mut v, layout)?;
            Ok(v)
        }
        Compression::Unknown(c) => Err(PsdError::Unsupported(format!("compression method {c}"))),
    }
}

/// Bytes a PSD RLE stream (two-byte row counts) of `rows` rows grows by when written as PSB
/// (four-byte row counts); an error when `data` is too short to hold the count table.
pub(crate) fn rle_widening(data: &[u8], rows: usize) -> Result<u64> {
    rle_widening_len(data.len() as u64, rows)
}

/// [`rle_widening`] from the stream's length alone, so data still in its source (or staged on
/// the scratch disk) is not read to measure it.
pub(crate) fn rle_widening_len(len: u64, rows: usize) -> Result<u64> {
    let table = rows.checked_mul(2).ok_or(PsdError::LimitExceeded("row count overflow"))? as u64;
    if len < table {
        let offset = usize::try_from(len).unwrap_or(usize::MAX);
        return Err(PsdError::UnexpectedEof { offset, needed: usize::try_from(table - len).unwrap_or(usize::MAX) });
    }
    Ok(table)
}

/// Encodes planar data (excluding the 2-byte compression field).
pub fn encode_planes(compression: Compression, decoded: &[u8], layout: &PlaneLayout) -> Result<Vec<u8>> {
    let total = layout.decoded_len()?;
    if decoded.len() != total {
        return Err(PsdError::invalid(format!("decoded buffer has {} bytes, layout requires {}", decoded.len(), total)));
    }
    match compression {
        Compression::Raw => Ok(decoded.to_vec()),
        Compression::Rle => encode_rle(decoded, layout),
        Compression::Zip => Ok(zip_compress(decoded)),
        Compression::ZipPrediction => {
            let mut v = decoded.to_vec();
            predict(&mut v, layout)?;
            Ok(zip_compress(&v))
        }
        Compression::Unknown(c) => Err(PsdError::Unsupported(format!("compression method {c}"))),
    }
}

/// Validates encoded sizes and, for ZIP, the complete stream and checksum.
///
/// A ZIP stream whose declared size is over [`MAX_DECODED_BYTES`] is not
/// decompressed at all (photocraft#2926): [`decode_planes`] refuses that layout with
/// `LimitExceeded` anyway, so inflating it here would only be work on an
/// untrusted file before the same error. The bytes are kept verbatim, like an
/// unknown compression method, so a layered file whose merged composite is
/// too large still opens.
pub(crate) fn validate_planes(compression: Compression, data: &[u8], layout: &PlaneLayout) -> Result<()> {
    let total = layout.total_bytes()?;
    match compression {
        Compression::Raw => {
            if (data.len() as u64) < total {
                return Err(PsdError::UnexpectedEof { offset: data.len(), needed: usize::try_from(total - data.len() as u64).unwrap_or(usize::MAX) });
            }
            Ok(())
        }
        Compression::Rle => {
            let rows = layout.rows()?;
            let mut r = Reader::new(data);
            r.check_count(rows as u64, layout.count_size() as u64)?;
            let mut sum: u64 = 0;
            for _ in 0..rows {
                sum += read_count(&mut r, layout.version)? as u64;
            }
            if sum > r.remaining() as u64 {
                return Err(PsdError::UnexpectedEof {
                    offset: data.len(),
                    needed: usize::try_from(sum - r.remaining() as u64).unwrap_or(usize::MAX),
                });
            }
            Ok(())
        }
        Compression::Zip | Compression::ZipPrediction if total > MAX_DECODED_BYTES => Ok(()),
        Compression::Zip | Compression::ZipPrediction => inflate_checked(data, total, false).map(|_| ()),
        Compression::Unknown(_) => Ok(()),
    }
}

#[cfg(test)]
thread_local! {
    /// ZIP streams inflated on this thread (tests count passes over the merged image).
    pub(crate) static INFLATES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

// Keep only the requested pixels, but consume the trailer to verify Adler-32.
// A bounded surplus tolerates padding without unbounded decompression work.
fn inflate_checked(data: &[u8], expected: u64, keep: bool) -> Result<Vec<u8>> {
    const OOM: PsdError = PsdError::OutOfMemory("not enough memory for the decoded channel data");
    #[cfg(test)]
    INFLATES.with(|n| n.set(n.get() + 1));
    let mut decoder = flate2::Decompress::new(true);
    let mut out = Vec::new();
    if keep {
        out.try_reserve_exact(expected.min(data.len().saturating_mul(1032) as u64) as usize).map_err(|_| OOM)?;
    }
    let mut buf = vec![0; expected.saturating_add(1).clamp(8192, 1 << 20) as usize];
    loop {
        let before = (decoder.total_in(), decoder.total_out());
        let input = data.get(before.0 as usize..).ok_or_else(|| PsdError::Decompress("zlib input overflow".into()))?;
        let status = decoder.decompress(input, &mut buf, flate2::FlushDecompress::None).map_err(|e| PsdError::Decompress(e.to_string()))?;
        let got = decoder.total_out();
        if got > expected.saturating_add(1 << 20) {
            return Err(PsdError::LimitExceeded("zlib output exceeds the declared size and padding allowance"));
        }
        if keep {
            let n = (got - before.1).min(expected.saturating_sub(out.len() as u64)) as usize;
            out.try_reserve(n).map_err(|_| OOM)?;
            out.extend_from_slice(buf.get(..n).ok_or_else(|| PsdError::Decompress("zlib output overflow".into()))?);
        }
        if status == flate2::Status::StreamEnd {
            if got < expected {
                return Err(PsdError::Decompress(format!("zlib stream produced {got} of {expected} bytes")));
            }
            return Ok(out);
        }
        if before == (decoder.total_in(), got) {
            return Err(PsdError::Decompress("incomplete zlib stream".into()));
        }
    }
}

fn read_count(r: &mut Reader<'_>, v: Version) -> Result<usize> {
    Ok(if v.is_psb() { r.u32()? as usize } else { r.u16()? as usize })
}

fn decode_rle(data: &[u8], layout: &PlaneLayout, total: usize) -> Result<Vec<u8>> {
    let rows = layout.rows()?;
    let rb = layout.row_bytes();
    let mut r = Reader::new(data);
    r.check_count(rows as u64, layout.count_size() as u64)?;
    let mut counts = Vec::with_capacity(rows);
    for _ in 0..rows {
        counts.push(read_count(&mut r, layout.version)?);
    }
    // PackBits expands at most 64x (2 input bytes -> 128 output bytes).
    if (total as u64) > (r.remaining() as u64).saturating_mul(64) {
        return Err(PsdError::Decompress("RLE data too short for declared size".into()));
    }
    // Up to MAX_DECODED_BYTES: report a machine without that much memory instead of aborting.
    let mut out = Vec::new();
    out.try_reserve_exact(total).map_err(|_| PsdError::OutOfMemory("not enough memory for the decoded channel data"))?;
    for c in counts {
        let row = r.bytes(c)?;
        packbits::decode_into(row, rb, &mut out)?;
    }
    Ok(out)
}

/// Encode independent row chunks and join their count tables and payloads in order.
fn encode_rle(decoded: &[u8], layout: &PlaneLayout) -> Result<Vec<u8>> {
    let rows = layout.rows()?;
    if rows == 0 {
        return Ok(Vec::new());
    }
    let rb = layout.row_bytes();
    let cs = layout.count_size();
    let threads = if cfg!(target_arch = "wasm32") { 1 } else { std::thread::available_parallelism().map_or(1, |v| v.get()).clamp(1, 32) };
    let step = rows.div_ceil(threads).max(1);
    let ranges: Vec<_> = (0..rows).step_by(step).map(|start| start..start.saturating_add(step).min(rows)).collect();
    // Scan packets once. Each worker owns its count table and payload; joining copies each
    // payload once, trading temporary encoded bytes for the former second packet scan.
    let mut chunks: Vec<_> = map_chunks(threads, ranges.clone(), |range| -> Result<Vec<u8>> {
        let table = range.len().checked_mul(cs).ok_or(PsdError::LimitExceeded("RLE table overflow"))?;
        let hint =
            if cfg!(target_arch = "wasm32") { table } else { table.saturating_add(range.len().saturating_mul(rb.saturating_add(rb.div_ceil(128)))) };
        encode_rle_chunk(decoded, layout, range, hint)
    })?
    .into_iter()
    .collect::<Result<_>>()?;
    if chunks.len() == 1 {
        return Ok(chunks.remove(0).into_boxed_slice().into_vec());
    }
    let total = chunks.iter().try_fold(0usize, |n, c| n.checked_add(c.len()).ok_or(PsdError::LimitExceeded("RLE size overflow")))?;
    let table = rows.checked_mul(cs).ok_or(PsdError::LimitExceeded("RLE table overflow"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(total).map_err(|_| PsdError::OutOfMemory("not enough memory for encoded RLE"))?;
    out.resize(table, 0);
    for (range, chunk) in ranges.into_iter().zip(chunks) {
        let counts = range.len() * cs;
        out[range.start * cs..range.end * cs].copy_from_slice(&chunk[..counts]);
        out.extend_from_slice(&chunk[counts..]);
    }
    Ok(out)
}

/// Encode one worker's rows, measuring packets only when the spare capacity cannot bound them.
fn encode_rle_chunk(decoded: &[u8], layout: &PlaneLayout, range: std::ops::Range<usize>, hint: usize) -> Result<Vec<u8>> {
    const OOM: PsdError = PsdError::OutOfMemory("not enough memory for encoded RLE");
    let rb = layout.row_bytes();
    let cs = layout.count_size();
    let table = range.len().checked_mul(cs).ok_or(PsdError::LimitExceeded("RLE table overflow"))?;
    let mut out = Vec::new();
    out.try_reserve_exact(hint.max(table)).or_else(|_| out.try_reserve_exact(table)).map_err(|_| OOM)?;
    out.resize(table, 0);
    let row_bound = rb.saturating_add(rb.div_ceil(128));
    for (i, row) in range.enumerate() {
        let src = &decoded[row * rb..(row + 1) * rb];
        if out.capacity() - out.len() < row_bound {
            let needed = packbits::encoded_len(src);
            out.try_reserve(needed).or_else(|_| out.try_reserve_exact(needed)).map_err(|_| OOM)?;
        }
        let start = out.len();
        packbits::encode(src, &mut out);
        let n = out.len() - start;
        let count = &mut out[i * cs..(i + 1) * cs];
        if cs == 2 {
            count.copy_from_slice(&u16::try_from(n).map_err(|_| PsdError::LimitExceeded("RLE row exceeds 65535 bytes (use PSB)"))?.to_be_bytes());
        } else {
            count.copy_from_slice(&u32::try_from(n).map_err(|_| PsdError::LimitExceeded("RLE row too large"))?.to_be_bytes());
        }
    }
    Ok(out)
}

/// Ordered work on the caller's pool, including nested channel and row encoding.
fn map_chunks<T: Send, R: Send>(threads: usize, items: Vec<T>, f: impl Fn(T) -> R + Sync) -> Result<Vec<R>> {
    if threads == 1 {
        return Ok(items.into_iter().map(f).collect());
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| items.into_par_iter().map(&f).collect()))
            .map_err(|_| PsdError::LimitExceeded("an RLE worker panicked"))
    }
    #[cfg(target_arch = "wasm32")]
    Ok(items.into_iter().map(f).collect())
}

/// zlib-compresses `data` at the default level.
pub fn zip_compress(data: &[u8]) -> Vec<u8> {
    #[cfg(not(target_arch = "wasm32"))]
    let mut e = zip_encoder();
    #[cfg(target_arch = "wasm32")]
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    // The in-memory output always accepts the complete write.
    let _ = e.write_all(data);
    #[cfg(not(target_arch = "wasm32"))]
    {
        if e.try_finish().is_err() {
            return Vec::new();
        }
        let output = std::mem::take(e.get_mut());
        // Return only the finished codec; the slot stays empty during compression.
        let _ = ZIP_ENCODER.try_with(|slot| slot.set(Some(e)));
        output.into_vec()
    }
    #[cfg(target_arch = "wasm32")]
    {
        e.finish().unwrap_or_default()
    }
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    /// One finished codec per thread; encoded output moves out before it is retained.
    static ZIP_ENCODER: std::cell::Cell<Option<flate2::write::ZlibEncoder<ZipOutput>>> = const { std::cell::Cell::new(None) };
}

/// Reset a thread's finished encoder without reallocating its fixed-size working buffers.
/// Nested calls and thread teardown can use a fresh encoder without accessing a busy slot.
#[cfg(not(target_arch = "wasm32"))]
fn zip_encoder() -> flate2::write::ZlibEncoder<ZipOutput> {
    if let Some(mut encoder) = ZIP_ENCODER.try_with(std::cell::Cell::take).ok().flatten()
        && encoder.reset(ZipOutput::default()).is_ok()
    {
        return encoder;
    }
    flate2::write::ZlibEncoder::new(ZipOutput::default(), flate2::Compression::default())
}

/// Bounded native output blocks avoid reallocating large ZIP buffers as they grow.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct ZipOutput {
    blocks: Vec<Vec<u8>>,
    current: Vec<u8>,
}

#[cfg(not(target_arch = "wasm32"))]
impl std::io::Write for ZipOutput {
    /// Accept all input bytes into bounded output blocks without reserving raw image size.
    fn write(&mut self, mut bytes: &[u8]) -> std::io::Result<usize> {
        let len = bytes.len();
        while !bytes.is_empty() {
            if self.current.len() == self.current.capacity() {
                if !self.current.is_empty() {
                    self.blocks.push(std::mem::take(&mut self.current));
                }
                self.current = Vec::with_capacity(if self.blocks.is_empty() { 8192 } else { 256 << 10 });
            }
            let part = &mut self.current;
            let (head, rest) = bytes.split_at((part.capacity() - part.len()).min(bytes.len()));
            part.extend_from_slice(head);
            bytes = rest;
        }
        Ok(len)
    }

    /// Complete a no-op flush because every write is already retained in memory.
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ZipOutput {
    /// Join encoded ZIP blocks into one vector sized to the compressed payload.
    fn into_vec(mut self) -> Vec<u8> {
        self.blocks.push(self.current);
        if self.blocks.len() == 1 {
            return self.blocks.pop().unwrap_or_default().into_boxed_slice().into_vec();
        }
        let len = self.blocks.iter().fold(0usize, |n, c| n.saturating_add(c.len()));
        let mut out = Vec::with_capacity(len);
        for part in self.blocks {
            out.extend_from_slice(&part);
        }
        out
    }
}

/// Decompresses a zlib stream, requiring at least `expected` output bytes and
/// returning exactly `expected`.
///
/// The output is reserved fallibly and read in chunks, never past `expected`
/// bytes, so a stream declaring a huge size reports an error on a machine
/// without that much memory instead of aborting.
pub fn zip_decompress(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    const OOM: PsdError = PsdError::OutOfMemory("not enough memory for the decoded channel data");
    const CHUNK: usize = 1 << 20;
    let mut dec = flate2::read::ZlibDecoder::new(data).take(u64::try_from(expected).unwrap_or(u64::MAX));
    let mut out = Vec::new();
    // Deflate expands at most ~1032:1, so this is the most a valid stream can need.
    out.try_reserve_exact(expected.min(data.len().saturating_mul(1032))).map_err(|_| OOM)?;
    let mut buf = vec![0u8; CHUNK.min(expected).max(1)];
    loop {
        let n = match dec.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(PsdError::Decompress(e.to_string())),
        };
        let chunk = buf.get(..n).ok_or_else(|| PsdError::Decompress("zlib reader overran its buffer".into()))?;
        out.try_reserve(n).map_err(|_| OOM)?;
        out.extend_from_slice(chunk);
    }
    if out.len() < expected {
        return Err(PsdError::Decompress(format!("zlib stream produced {} of {} bytes", out.len(), expected)));
    }
    Ok(out)
}

/// Applies horizontal delta prediction in place (encoder side).
///
/// * 1/8-bit: byte-wise delta per row.
/// * 16-bit: big-endian u16 sample delta per row.
/// * 32-bit: each row's samples are split into four byte planes (MSB plane
///   first) and the resulting row is byte-wise delta coded, as Photoshop does.
pub fn predict(buf: &mut [u8], layout: &PlaneLayout) -> Result<()> {
    let rb = layout.row_bytes();
    if rb == 0 {
        return Ok(());
    }
    #[cfg(not(target_arch = "wasm32"))]
    if buf.len() >= 1 << 20 && matches!(layout.depth, 1 | 8 | 16 | 32) {
        use rayon::prelude::*;
        let threads = rayon::current_num_threads().min(32);
        let rows = buf.len() / rb;
        let step = rows.div_ceil(threads).max(1) * rb;
        return buf[..rows * rb].par_chunks_mut(step).try_for_each(|part| predict_rows(part, layout));
    }
    predict_rows(buf, layout)
}

/// Apply per-row prediction, reusing scratch for float byte-plane shuffling.
fn predict_rows(buf: &mut [u8], layout: &PlaneLayout) -> Result<()> {
    let rb = layout.row_bytes();
    let w = layout.width;
    let mut scratch = Vec::new();
    for row in buf.chunks_exact_mut(rb) {
        match layout.depth {
            1 | 8 => {
                for i in (1..row.len()).rev() {
                    row[i] = row[i].wrapping_sub(row[i - 1]);
                }
            }
            16 => {
                // One forward pass: the previous *original* sample stays in a
                // register, so nothing is re-read after being overwritten.
                let mut prev = 0u16;
                for s in row.as_chunks_mut::<2>().0 {
                    let cur = u16::from_be_bytes(*s);
                    *s = cur.wrapping_sub(prev).to_be_bytes();
                    prev = cur;
                }
            }
            32 => {
                scratch.resize(rb, 0);
                // Split into four byte planes (MSB first) and delta the whole
                // plane-major row. Plane k's first delta uses the last byte of
                // plane k - 1, so seed each plane's "previous" with it.
                let mut prev = [0u8; 4];
                if let Some(l) = row.as_chunks::<4>().0.last() {
                    prev = [0, l[0], l[1], l[2]];
                }
                let (p0, rest) = scratch.split_at_mut(w);
                let (p1, rest) = rest.split_at_mut(w);
                let (p2, p3) = rest.split_at_mut(w);
                let planes = p0.iter_mut().zip(p1.iter_mut()).zip(p2.iter_mut().zip(p3.iter_mut()));
                for (s, ((o0, o1), (o2, o3))) in row.as_chunks::<4>().0.iter().zip(planes) {
                    *o0 = s[0].wrapping_sub(prev[0]);
                    *o1 = s[1].wrapping_sub(prev[1]);
                    *o2 = s[2].wrapping_sub(prev[2]);
                    *o3 = s[3].wrapping_sub(prev[3]);
                    prev = [s[0], s[1], s[2], s[3]];
                }
                row.copy_from_slice(&scratch);
            }
            d => return Err(PsdError::Unsupported(format!("prediction for depth {d}"))),
        }
    }
    Ok(())
}

/// Reverses [`predict`] in place (decoder side).
pub fn unpredict(buf: &mut [u8], layout: &PlaneLayout) -> Result<()> {
    let rb = layout.row_bytes();
    if rb == 0 {
        return Ok(());
    }
    let w = layout.width;
    let mut scratch = Vec::new();
    for row in buf.chunks_exact_mut(rb) {
        match layout.depth {
            1 | 8 => {
                for i in 1..row.len() {
                    row[i] = row[i].wrapping_add(row[i - 1]);
                }
            }
            16 => {
                // Running sum carried in a register; one pass over the row.
                let mut acc = 0u16;
                for s in row.as_chunks_mut::<2>().0 {
                    acc = acc.wrapping_add(u16::from_be_bytes(*s));
                    *s = acc.to_be_bytes();
                }
            }
            32 => {
                scratch.clear();
                scratch.extend_from_slice(row);
                // The byte-wise running sum runs across the four planes, so
                // plane k starts at the wrapping total of planes 0..k.
                let (p0, rest) = scratch.split_at(w);
                let (p1, rest) = rest.split_at(w);
                let (p2, p3) = rest.split_at(w);
                let sum = |p: &[u8]| p.iter().fold(0u8, |a, &b| a.wrapping_add(b));
                let b1 = sum(p0);
                let b2 = b1.wrapping_add(sum(p1));
                let b3 = b2.wrapping_add(sum(p2));
                let mut acc = [0u8, b1, b2, b3];
                let planes = p0.iter().zip(p1).zip(p2.iter().zip(p3));
                for (o, ((&s0, &s1), (&s2, &s3))) in row.as_chunks_mut::<4>().0.iter_mut().zip(planes) {
                    acc[0] = acc[0].wrapping_add(s0);
                    acc[1] = acc[1].wrapping_add(s1);
                    acc[2] = acc[2].wrapping_add(s2);
                    acc[3] = acc[3].wrapping_add(s3);
                    *o = acc;
                }
            }
            d => return Err(PsdError::Unsupported(format!("prediction for depth {d}"))),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod parallel_rle_equivalence {
    use super::*;

    /// The parallel row packer's output must be byte-equal to a straightforward sequential
    /// encoding of the same plane, across depths, row counts around the chunking and data
    /// shapes (runs, noise, all-default).
    #[test]
    fn parallel_rle_matches_sequential() {
        let mut data: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move || {
            data ^= data << 13;
            data ^= data >> 7;
            data ^= data << 17;
            data
        };
        let layout_for = |rows: usize, rb: usize, version: Version| PlaneLayout { planes: 1, width: rb, height: rows, depth: 8, version };
        for (rows, rb, version) in [
            (0usize, 4usize, Version::Psd),
            (1, 4, Version::Psd),
            (7, 4, Version::Psb),
            (64, 4, Version::Psd),
            (65, 4, Version::Psb),  // chunk boundary
            (513, 4, Version::Psd), // more rows than threads on most runners
            (2049, 4, Version::Psb),
            (2, 4, Version::Psd), // fewer rows than threads
            (33, 3000, Version::Psd),
            (31, 20000, Version::Psb),
        ] {
            let pattern: Vec<u8> = match rows % 3 {
                0 => vec![7u8; rows * rb],                                   // long runs
                1 => (0..rows * rb).map(|i| (i * 31 % 251) as u8).collect(), // noise-ish
                _ => vec![0u8; rows * rb],                                   // all default
            };
            let pattern = if pattern.len() != rows * rb { vec![7u8; rows * rb] } else { pattern };
            let mut data = pattern.clone();
            if rows % 4 == 0 {
                // sprinkle runs into the noise
                for chunk in data.chunks_mut(97) {
                    if let Some(f) = chunk.first_mut() {
                        *f = 200;
                    }
                }
            }
            let _ = next();
            let layout = layout_for(rows, rb, version);
            let parallel = encode_rle(&data, &layout).expect("parallel encode");
            let sequential = encode_rle_sequential(&data, &layout).expect("sequential encode");
            assert_eq!(parallel, sequential, "rows {rows} rb {rb}");
            // photocraft#6607: measured first, then packed in place: one allocation, exactly the file's size.
            assert_eq!(parallel.capacity(), parallel.len(), "rows {rows} rb {rb}: the output grew or was over-allocated");
        }
    }

    /// A row too long for a PSD's 16-bit RLE count is the same error from the two-pass encoder.
    #[test]
    fn rle_rows_past_sixteen_bits_are_refused_in_psd() {
        let layout_for = |rows: usize, rb: usize, version: Version| PlaneLayout { planes: 1, width: rb, height: rows, depth: 8, version };
        let rb = 70_000;
        let layout = layout_for(3, rb, Version::Psd);
        let noise: Vec<u8> = (0..3 * rb).map(|i| (i * 31 % 251) as u8).collect();
        assert_eq!(encode_rle(&noise, &layout), Err(PsdError::LimitExceeded("RLE row exceeds 65535 bytes (use PSB)")));
        assert_eq!(encode_rle(&noise, &layout_for(3, rb, Version::Psb)), encode_rle_sequential(&noise, &layout_for(3, rb, Version::Psb)));
    }

    /// The pre-parallel encoder, kept as the oracle.
    fn encode_rle_sequential(decoded: &[u8], layout: &PlaneLayout) -> Result<Vec<u8>> {
        let rows = layout.rows()?;
        let rb = layout.row_bytes();
        let cs = layout.count_size();
        let mut out = vec![0u8; rows * cs];
        let mut enc = Vec::new();
        for row in 0..rows {
            let start = enc.len();
            packbits::encode(&decoded[row * rb..(row + 1) * rb], &mut enc);
            let n = enc.len() - start;
            if cs == 2 {
                let n = u16::try_from(n).map_err(|_| PsdError::LimitExceeded("RLE row exceeds 65535 bytes (use PSB)"))?;
                out[row * 2..row * 2 + 2].copy_from_slice(&n.to_be_bytes());
            } else {
                let n = u32::try_from(n).map_err(|_| PsdError::LimitExceeded("RLE row too large"))?;
                out[row * 4..row * 4 + 4].copy_from_slice(&n.to_be_bytes());
            }
        }
        out.extend_from_slice(&enc);
        Ok(out)
    }
}

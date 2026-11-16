#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Radiance HDR (`.hdr`, `.pic`) pictures, read from Greg Ward's public RGBE format description
//! ("Real Pixels", Graphics Gems II): a text header (`#?RADIANCE` or `#?RGBE`,
//! `FORMAT=32-bit_rle_rgbe`), a blank line, a resolution line (`-Y h +X w` and the seven other
//! axis orders), then one RGBE pixel (shared exponent) per sample, flat, in the old run-length
//! form or in the "new" per-channel run-length form (scan lines of 8 to 32767 pixels).
//!
//! [`decode`] returns scene-linear float RGB, top row first, so values above 1.0 survive. The
//! `EXPOSURE` and `PRIMARIES` header lines are not applied, which is what other readers do too;
//! the XYZE colour format is refused.
//!
//! Input is treated as hostile: the header length and the picture size are capped by [`Limits`],
//! and every scan line is validated before the float picture is allocated, so a short file cannot
//! ask for a large allocation. Whatever the limits, no buffer is requested past the target's
//! largest allocation (`isize::MAX` bytes), so raised limits on a 32-bit target such as `wasm32`
//! give [`Error::LimitExceeded`] instead of a panic.

#[cfg(test)]
mod tests;

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes do not start with `#?RADIANCE` or `#?RGBE`.
    #[error("not a Radiance HDR picture")]
    NotHdr,
    /// The header or the pixel data is damaged or cut short.
    #[error("Radiance HDR: {0}")]
    Malformed(&'static str),
    /// A valid picture this reader does not cover (the XYZE colour format, an unknown `FORMAT`).
    #[error("unsupported Radiance HDR: {0}")]
    Unsupported(String),
    /// The file declares more than [`Limits`] allow.
    #[error("Radiance HDR exceeds a limit: {0}")]
    LimitExceeded(&'static str),
}

/// Result of this crate's functions.
pub type Result<T> = std::result::Result<T, Error>;

/// Caps on what a file may ask the reader to allocate, checked before anything is allocated.
/// `#[non_exhaustive]`: start from [`Limits::default`] and set the fields you change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Most pixels (width × height) a picture may have; each takes 12 bytes decoded. Default 2^26.
    /// A picture over the target's largest allocation is refused at any setting.
    pub max_pixels: u64,
    /// Longest header read, in bytes (real ones are a few hundred). Default 64 KiB.
    pub max_header: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_pixels: 1 << 26, max_header: 64 << 10 }
    }
}

/// A decoded picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// Scene-linear RGB, three floats a pixel, top row first.
    pub rgb: Vec<f32>,
}

/// The largest allocation in bytes that a `Vec` or `String` can make on the target.
const MAX_ALLOC_BYTES: u64 = isize::MAX as u64;

/// Bytes taken by `count` elements of `element_bytes` each, or `None` when the count overflows `u64`.
fn checked_byte_len(count: u64, element_bytes: usize) -> Option<u64> {
    count.checked_mul(element_bytes as u64)
}

/// `count` as a length when `count` elements of `element_bytes` each fit in `max_alloc_bytes`.
fn alloc_len(count: u64, element_bytes: usize, max_alloc_bytes: u64) -> Option<usize> {
    checked_byte_len(count, element_bytes).filter(|&bytes| bytes <= max_alloc_bytes).and_then(|_| usize::try_from(count).ok())
}

/// Bytes of `String::from_utf8_lossy(bytes)`: each invalid sequence becomes one U+FFFD (3 bytes).
fn lossy_len(bytes: &[u8]) -> Option<u64> {
    bytes.utf8_chunks().try_fold(0u64, |n, chunk| {
        let invalid = if chunk.invalid().is_empty() { 0 } else { '\u{FFFD}'.len_utf8() };
        n.checked_add(chunk.valid().len() as u64)?.checked_add(invalid as u64)
    })
}

/// Whether `bytes` starts like a Radiance picture.
pub fn sniff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"#?RADIANCE") || bytes.starts_with(b"#?RGBE")
}

/// One resolution-line axis: `(is_x, positive)`.
fn axis(tok: &str) -> Option<(bool, bool)> {
    match tok {
        "+X" => Some((true, true)),
        "-X" => Some((true, false)),
        "+Y" => Some((false, true)),
        "-Y" => Some((false, false)),
        _ => None,
    }
}

/// RGBE to linear floats: `mantissa × 2^(e − 136)`, zero for exponent 0. (Radiance's own reader
/// adds half a mantissa step; other common readers do not, which also makes 1.0 =
/// (128, 128, 128, 129) read back as exactly 1.0.)
pub fn rgbe_to_linear(p: [u8; 4]) -> [f32; 3] {
    if p[3] == 0 {
        return [0.0; 3];
    }
    let scale = 2f32.powi(i32::from(p[3]) - 136);
    [f32::from(p[0]) * scale, f32::from(p[1]) * scale, f32::from(p[2]) * scale]
}

/// Reads the pixels of one scan line into `line`.
fn scanline(b: &[u8], pos: &mut usize, line: &mut [[u8; 4]]) -> Result<()> {
    let n = line.len();
    let take = |pos: &mut usize, k: usize| -> Result<&[u8]> {
        let s = pos.checked_add(k).and_then(|end| b.get(*pos..end)).ok_or(Error::Malformed("truncated pixel data"))?;
        *pos += k;
        Ok(s)
    };
    let head = pos.checked_add(4).and_then(|end| b.get(*pos..end));
    if (8..=32767).contains(&n)
        && let Some(&[2, 2, hi, lo]) = head
        && hi & 0x80 == 0
    {
        if usize::from(hi) << 8 | usize::from(lo) != n {
            return Err(Error::Malformed("scan line length does not match the picture"));
        }
        *pos += 4;
        for c in 0..4 {
            let mut x = 0;
            while x < n {
                let code = take(pos, 1)?.first().copied().unwrap_or(0);
                if code > 128 {
                    let run = usize::from(code - 128);
                    let v = take(pos, 1)?.first().copied().unwrap_or(0);
                    let dst = line.get_mut(x..x + run).ok_or(Error::Malformed("run past the end of the scan line"))?;
                    for p in dst {
                        p[c] = v;
                    }
                    x += run;
                } else {
                    let run = usize::from(code);
                    if run == 0 {
                        return Err(Error::Malformed("empty literal run"));
                    }
                    let src = take(pos, run)?;
                    let dst = line.get_mut(x..x + run).ok_or(Error::Malformed("literal past the end of the scan line"))?;
                    for (p, &v) in dst.iter_mut().zip(src) {
                        p[c] = v;
                    }
                    x += run;
                }
            }
        }
        return Ok(());
    }
    // Flat pixels, with the old run-length form: (1, 1, 1, k) repeats the previous pixel k << shift times.
    let (mut x, mut shift) = (0usize, 0u32);
    while x < n {
        let p: [u8; 4] = take(pos, 4)?.try_into().map_err(|_| Error::Malformed("truncated pixel data"))?;
        if p[..3] == [1, 1, 1] {
            let count = usize::from(p[3]).checked_shl(shift).unwrap_or(usize::MAX);
            let prev = if x > 0 { line.get(x - 1).copied() } else { None }.ok_or(Error::Malformed("run with no previous pixel"))?;
            let dst = line.get_mut(x..x.saturating_add(count).min(n)).ok_or(Error::Malformed("run past the end of the scan line"))?;
            dst.fill(prev);
            x += dst.len();
            shift = shift.saturating_add(8).min(24);
        } else {
            if let Some(d) = line.get_mut(x) {
                *d = p;
            }
            x += 1;
            shift = 0;
        }
    }
    Ok(())
}

/// Decodes a Radiance picture to linear float RGB, top row first.
pub fn decode(b: &[u8], limits: &Limits) -> Result<Picture> {
    decode_within(b, limits, MAX_ALLOC_BYTES)
}

/// [`decode`] with `max_alloc_bytes` as the largest allocation the target allows.
fn decode_within(b: &[u8], limits: &Limits, max_alloc_bytes: u64) -> Result<Picture> {
    if !sniff(b) {
        return Err(Error::NotHdr);
    }
    // Header: lines up to the first empty one, then the resolution line.
    let mut pos = 0;
    let line = |pos: &mut usize| -> Result<String> {
        let rest = b.get(*pos..).ok_or(Error::Malformed("truncated header"))?;
        let end = rest.iter().take(limits.max_header).position(|&c| c == b'\n').ok_or(Error::Malformed("truncated header"))?;
        let raw = rest.get(..end).unwrap_or_default();
        if lossy_len(raw).is_none_or(|n| n > max_alloc_bytes) {
            return Err(Error::LimitExceeded("header length"));
        }
        let s = String::from_utf8_lossy(raw).trim_end_matches('\r').to_string();
        *pos += end + 1;
        Ok(s)
    };
    loop {
        let l = line(&mut pos)?;
        if l.is_empty() {
            break;
        }
        if let Some(f) = l.strip_prefix("FORMAT=") {
            match f.trim() {
                "32-bit_rle_rgbe" => {}
                "32-bit_rle_xyze" => return Err(Error::Unsupported("the XYZE colour format".into())),
                other => return Err(Error::Unsupported(format!("format {other}"))),
            }
        }
        if pos > limits.max_header {
            return Err(Error::LimitExceeded("header length"));
        }
    }
    let res = line(&mut pos)?;
    let bad = Error::Malformed("bad resolution line");
    // Tokens after the fourth are ignored, so they are not collected.
    let mut t = res.split_whitespace();
    let (Some(a1), Some(n1), Some(a2), Some(n2)) = (t.next(), t.next(), t.next(), t.next()) else { return Err(bad) };
    let ((x1, p1), (x2, p2)) = (axis(a1).ok_or(bad.clone())?, axis(a2).ok_or(bad.clone())?);
    if x1 == x2 {
        return Err(Error::Malformed("resolution line names one axis twice"));
    }
    let n1: usize = n1.parse().map_err(|_| bad.clone())?;
    let n2: usize = n2.parse().map_err(|_| bad.clone())?;
    let (w, h) = if x1 { (n1, n2) } else { (n2, n1) };
    if w == 0 || h == 0 {
        return Err(Error::Malformed("empty picture"));
    }
    let (Ok(width), Ok(height)) = (u32::try_from(w), u32::try_from(h)) else { return Err(Error::LimitExceeded("pixel count")) };
    let pixels = u64::from(width) * u64::from(height);
    if pixels > limits.max_pixels {
        return Err(Error::LimitExceeded("pixel count"));
    }
    // One scan line of RGBE pixels and the float picture, each within the target's largest allocation.
    let scan_len = alloc_len(u64::from(if x1 { height } else { width }), size_of::<[u8; 4]>(), max_alloc_bytes);
    let rgb_len = pixels.checked_mul(3).and_then(|floats| alloc_len(floats, size_of::<f32>(), max_alloc_bytes));
    let (Some(scan_len), Some(total)) = (scan_len, rgb_len) else { return Err(Error::LimitExceeded("pixel count")) };
    // A scan line takes at least a few bits of file, so a header cannot ask for more than the data has.
    if (b.len().saturating_sub(pos) as u64).saturating_mul(4096) < pixels {
        return Err(Error::Malformed("truncated pixel data"));
    }
    // Validate every encoded scan line before allocating the float picture: a short file could
    // otherwise ask for the whole limit of zeroed pixels and only then fail on its first line.
    let pixel_data = pos;
    let mut scan = vec![[0u8; 4]; scan_len];
    for _ in 0..n1 {
        scanline(b, &mut pos, &mut scan)?;
    }

    let mut rgb = vec![0f32; total];
    pos = pixel_data;
    for i in 0..n1 {
        scanline(b, &mut pos, &mut scan)?;
        for (j, &p) in scan.iter().enumerate() {
            // The first axis counts scan lines, the second runs along one.
            let (x, y) = if x1 {
                (if p1 { i } else { w - 1 - i }, if p2 { h - 1 - j } else { j })
            } else {
                (if p2 { j } else { w - 1 - j }, if p1 { h - 1 - i } else { i })
            };
            let at = (y * w + x) * 3;
            if let Some(o) = rgb.get_mut(at..at + 3) {
                o.copy_from_slice(&rgbe_to_linear(p));
            }
        }
    }
    Ok(Picture { width, height, rgb })
}

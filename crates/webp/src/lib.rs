#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! The WebP container (the RIFF layout of Google's WebP Container Specification), read and
//! written. The codecs inside it (VP8, VP8L) are not part of this crate.
//!
//! Reading: [`scan`] walks the chunk headers and reports the canvas size, whether the file has
//! alpha, and, for an animation, every frame's position, size, duration, blending and disposal
//! and the loop count, without decoding any pixels. That tells an animation from a still and
//! gives its timing and memory needs before a decoder runs.
//!
//! Writing: [`still`] wraps one encoded VP8 key frame, as the simple form when there is nothing
//! else, or the extended form (`VP8X`) with an uncompressed alpha plane and ICC, EXIF and XMP
//! chunks in the order the specification gives.
//!
//! With feature `frames`, [`frames::read_frames`] decodes every frame through the `image` crate's
//! decoder with bounded memory.
//!
//! Input is treated as hostile: every read is bounds-checked, chunk sizes are checked against the
//! file, and the number of frames recorded is capped by the caller.

#[cfg(feature = "frames")]
pub mod frames;
#[cfg(test)]
mod tests;

/// Why a file could not be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The picture or one of its chunks does not fit the container.
    #[error("WebP: cannot write: {0}")]
    Encode(&'static str),
}

/// The largest canvas side of the extended format (24 bits, stored minus one).
pub const MAX_CANVAS: u32 = 1 << 24;
/// The largest side of a lossy (VP8) frame (14 bits).
pub const MAX_VP8_DIMENSION: u32 = 16383;

/// Whether `bytes` starts like a WebP file.
pub fn is_webp(bytes: &[u8]) -> bool {
    bytes.get(0..4) == Some(b"RIFF") && bytes.get(8..12) == Some(b"WEBP")
}

/// One frame (`ANMF` chunk) of an animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// Position of the frame on the canvas (always even).
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// How long the frame is shown, in milliseconds, as stored. Players show very small values
    /// longer; that is the caller's rule.
    pub duration_ms: u32,
    /// Whether the frame is alpha-blended onto the canvas (false: it replaces its rectangle).
    pub blend: bool,
    /// Whether the frame's rectangle is cleared to the background before the next frame.
    pub dispose_to_background: bool,
}

/// What the chunk headers of a WebP file say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// Canvas size.
    pub width: u32,
    pub height: u32,
    /// Whether the file says it has alpha (the `VP8X` alpha flag, or a lossless picture's alpha bit).
    pub alpha: bool,
    /// Whether the `VP8X` animation flag is set.
    pub animated: bool,
    /// The frames of an animation, in file order, up to the caller's limit. Empty for a still.
    pub frames: Vec<Frame>,
    /// The `ANIM` loop count: `Some(0)` loops forever. `None` without an `ANIM` chunk.
    pub loop_count: Option<u16>,
    /// Whether the walk reached the end of the RIFF data. When it did not (the file is cut short,
    /// or the frame limit was reached), more frames may exist than [`Info::frames`] holds.
    pub complete: bool,
}

impl Info {
    /// Whether the file is an animation of more than one frame; anything else is shown as a still.
    pub fn is_animation(&self) -> bool {
        self.animated && self.frames.len() > 1
    }
}

fn u24(b: &[u8], at: usize) -> Option<u32> {
    match b.get(at..at.checked_add(3)?)? {
        [a, b, c] => Some(u32::from(*a) | u32::from(*b) << 8 | u32::from(*c) << 16),
        _ => None,
    }
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// The size of a lossy picture from the start of its `VP8 ` chunk: a key frame tag, the start
/// code, then 14-bit width and height.
fn vp8_size(body: &[u8]) -> Option<(u32, u32)> {
    if body.first()? & 1 != 0 || body.get(3..6)? != [0x9D, 0x01, 0x2A] {
        return None;
    }
    let dim = |at: usize| Some(u32::from(u16::from_le_bytes(body.get(at..at + 2)?.try_into().ok()?) & 0x3FFF));
    Some((dim(6)?, dim(8)?))
}

/// The size and alpha bit of a lossless picture from the start of its `VP8L` chunk: the
/// signature byte, then 14-bit width and height (each minus one) and the alpha bit.
fn vp8l_size(body: &[u8]) -> Option<(u32, u32, bool)> {
    if *body.first()? != 0x2F {
        return None;
    }
    let bits = u32_at(body, 1)?;
    Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1, bits & (1 << 28) != 0))
}

/// Walks the chunks of a WebP file without decoding any pixels, recording at most `max_frames`
/// animation frames. `None` when `bytes` is not a WebP file or no chunk gives its size. A file
/// that breaks off later gives what was read, with [`Info::complete`] false.
pub fn scan(bytes: &[u8], max_frames: usize) -> Option<Info> {
    if !is_webp(bytes) {
        return None;
    }
    let declared_end = usize::try_from(u32_at(bytes, 4)?).ok()?.saturating_add(8);
    let end = declared_end.min(bytes.len());
    let mut at = 12usize;
    let (mut canvas, mut alpha, mut animated, mut frames, mut loop_count, mut complete) = (None, false, false, Vec::new(), None, false);
    loop {
        if at >= end {
            // The end of the data the RIFF header declares, unless the file stops before it.
            complete = declared_end <= bytes.len();
            break;
        }
        let Some(head) = at.checked_add(8).filter(|&e| e <= end).and_then(|e| bytes.get(at..e)) else { break };
        let Some(size) = u32_at(head, 4).and_then(|s| usize::try_from(s).ok()) else { break };
        let body_start = at + 8;
        // A chunk cut short by the end of the file still has its header fields read.
        let body_end = body_start.saturating_add(size);
        let body = bytes.get(body_start..body_end.min(end)).unwrap_or_default();
        match head.get(..4) {
            Some(b"VP8X") => {
                let Some(&flags) = body.first() else { break };
                let (Some(w), Some(h)) = (u24(body, 4), u24(body, 7)) else { break };
                canvas = Some((w + 1, h + 1));
                alpha = flags & 0x10 != 0;
                animated = flags & 0x02 != 0;
            }
            Some(b"ANIM") => loop_count = body.get(4..6).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes),
            Some(b"ANMF") => {
                if frames.len() >= max_frames {
                    break;
                }
                let (Some(x), Some(y), Some(w), Some(h), Some(duration_ms), Some(&flags)) =
                    (u24(body, 0), u24(body, 3), u24(body, 6), u24(body, 9), u24(body, 12), body.get(15))
                else {
                    break;
                };
                frames.push(Frame {
                    x: x * 2,
                    y: y * 2,
                    width: w + 1,
                    height: h + 1,
                    duration_ms,
                    blend: flags & 0x02 == 0,
                    dispose_to_background: flags & 0x01 != 0,
                });
            }
            // The simple forms have no canvas header: the picture's own size is the canvas.
            Some(b"VP8 ") if canvas.is_none() => canvas = vp8_size(body),
            Some(b"VP8L") if canvas.is_none() => {
                if let Some((w, h, a)) = vp8l_size(body) {
                    canvas = Some((w, h));
                    alpha = a;
                }
            }
            _ => {}
        }
        if body_end > end {
            break;
        }
        // Chunks are padded to an even length.
        at = body_end.saturating_add(size & 1);
    }
    let (width, height) = canvas?;
    Some(Info { width, height, alpha, animated, frames, loop_count, complete })
}

/// Appends a RIFF chunk: FourCC, little-endian size, payload, padded to an even length.
fn chunk(out: &mut Vec<u8>, fourcc: &[u8; 4], payload: &[u8]) -> Result<(), Error> {
    let size = u32::try_from(payload.len()).map_err(|_| Error::Encode("a chunk is larger than 4 GiB"))?;
    out.extend_from_slice(fourcc);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        out.push(0);
    }
    Ok(())
}

/// What goes around the picture of a still WebP. Everything `None` gives the simple form.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Extras<'a> {
    /// The alpha plane, one byte a pixel, top row first (`width × height` bytes). Stored uncompressed.
    pub alpha: Option<&'a [u8]>,
    /// An ICC profile.
    pub icc: Option<&'a [u8]>,
    /// EXIF, as a TIFF structure.
    pub exif: Option<&'a [u8]>,
    /// XMP, as its XML text.
    pub xmp: Option<&'a [u8]>,
}

/// The WebP container around one encoded VP8 key frame of `width` × `height`: the simple form
/// when `extras` is empty, else the extended form (`VP8X`) with the alpha plane and the metadata
/// chunks in the specified order (ICCP, ALPH, VP8, EXIF, XMP).
pub fn still(width: u32, height: u32, vp8_frame: &[u8], extras: &Extras) -> Result<Vec<u8>, Error> {
    if width == 0 || height == 0 || width > MAX_VP8_DIMENSION || height > MAX_VP8_DIMENSION {
        return Err(Error::Encode("a lossy picture is 1 to 16383 pixels a side"));
    }
    if extras.alpha.is_some_and(|a| a.len() as u64 != u64::from(width) * u64::from(height)) {
        return Err(Error::Encode("the alpha plane does not match the picture size"));
    }
    let mut body = b"WEBP".to_vec();
    let Extras { alpha, icc, exif, xmp } = *extras;
    if alpha.is_some() || icc.is_some() || exif.is_some() || xmp.is_some() {
        let flags = [(icc.is_some(), 0x20u8), (alpha.is_some(), 0x10), (exif.is_some(), 0x08), (xmp.is_some(), 0x04)]
            .iter()
            .filter(|(on, _)| *on)
            .fold(0, |f, (_, bit)| f | bit);
        let mut vp8x = vec![flags, 0, 0, 0];
        vp8x.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
        vp8x.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
        chunk(&mut body, b"VP8X", &vp8x)?;
        if let Some(icc) = icc {
            chunk(&mut body, b"ICCP", icc)?;
        }
        if let Some(a) = alpha {
            // Header byte: reserved, no pre-processing, no filtering, no compression.
            let mut alph = Vec::with_capacity(a.len() + 1);
            alph.push(0);
            alph.extend_from_slice(a);
            chunk(&mut body, b"ALPH", &alph)?;
        }
    }
    chunk(&mut body, b"VP8 ", vp8_frame)?;
    if let Some(exif) = exif {
        chunk(&mut body, b"EXIF", exif)?;
    }
    if let Some(xmp) = xmp {
        chunk(&mut body, b"XMP ", xmp)?;
    }
    let size = u32::try_from(body.len()).map_err(|_| Error::Encode("the file is larger than 4 GiB"))?;
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

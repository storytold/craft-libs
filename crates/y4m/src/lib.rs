#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! YUV4MPEG2 (`.y4m`) files: uncompressed planar video, as described by the format's public
//! documentation (the `yuv4mpeg(5)` manual page of mjpegtools, and the `C` colour space names
//! other tools added for high bit depths).
//!
//! The file starts with `YUV4MPEG2` and space-separated tagged parameters up to a newline: `W` and
//! `H` (size), `F` (frame rate, `n:d`), `A` (pixel aspect, `n:d`, `0:0` unknown), `I` (interlacing),
//! `C` (colour space; `420jpeg` when absent) and `X` (extensions; `XCOLORRANGE=FULL` is read). Each
//! frame is a `FRAME` line (with optional parameters of its own) followed by the Y, Cb and Cr planes
//! (and an alpha plane for `444alpha`); samples above 8 bits are 16-bit little-endian.
//!
//! [`Stream::open`] reads the stream header and finds the frames (their `FRAME` lines only)
//! through a read callback, so a large file is never loaded whole; [`Stream::frame`] then says
//! where one frame's planes are, and [`Header::planes`] splits them. Every number comes from the
//! file and is checked before use: the picture size, the line length and the number of frames
//! indexed are capped by [`Limits`].

use std::ops::Range;

#[cfg(test)]
mod tests;

const SIGNATURE: &[u8] = b"YUV4MPEG2";

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The stream header or the frames are missing or damaged.
    #[error("Y4M: {0}")]
    Malformed(&'static str),
    /// A colour space this reader does not cover.
    #[error("unsupported Y4M colour space {0}")]
    UnsupportedColourSpace(String),
    /// The file declares more than [`Limits`] allow.
    #[error("Y4M exceeds a limit: {0}")]
    LimitExceeded(&'static str),
}

/// Result of this crate's functions.
pub type Result<T> = std::result::Result<T, Error>;

/// Caps on what a file may ask for. `#[non_exhaustive]`: start from [`Limits::default`] and set
/// the fields you change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Most luma samples (width × height) a frame may have. Default 2^28.
    pub max_pixels: u64,
    /// Longest stream header or `FRAME` line read, in bytes. Default 64 KiB.
    pub max_line: usize,
    /// Most frames indexed; a longer file ends there. Each takes 8 bytes of index. Default 2^24.
    pub max_frames: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { max_pixels: 1 << 28, max_line: 64 << 10, max_frames: 1 << 24 }
    }
}

/// Whether `head` starts like a YUV4MPEG2 file.
pub fn sniff(head: &[u8]) -> bool {
    head.starts_with(SIGNATURE) && matches!(head.get(SIGNATURE.len()), Some(b' ' | b'\n'))
}

/// Chroma subsampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chroma {
    /// Half width, half height.
    C420,
    /// Half width.
    C422,
    /// Full size.
    C444,
}

impl Chroma {
    /// The horizontal and vertical subsampling as shifts.
    pub fn shifts(self) -> (u32, u32) {
        match self {
            Chroma::C420 => (1, 1),
            Chroma::C422 => (1, 0),
            Chroma::C444 => (0, 0),
        }
    }
}

/// The sample layout a `C` parameter names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// `None` for luma only (`mono`).
    pub chroma: Option<Chroma>,
    /// Bits per sample, 8 to 16. Above 8, samples are 16-bit little-endian.
    pub bits: u32,
    /// Whether an alpha plane follows the chroma planes (`444alpha`).
    pub alpha: bool,
}

impl Layout {
    /// The layout of a colour space name (`420jpeg`, `422p10`, `444alpha`, `mono16`, …), `None`
    /// for one this reader does not cover.
    pub fn parse(c: &str) -> Option<Layout> {
        let (base, bits) = match c.split_once('p') {
            Some((base, bits)) if !bits.is_empty() && bits.bytes().all(|b| b.is_ascii_digit()) => (base, bits.parse().ok()?),
            _ => (c, 8),
        };
        let (base, bits) = match base.strip_prefix("mono") {
            Some("") => ("mono", bits),
            Some(digits) if bits == 8 => ("mono", digits.parse().ok()?),
            _ => (base, bits),
        };
        if !(8..=16).contains(&bits) {
            return None;
        }
        let (chroma, alpha) = match base {
            "420" | "420jpeg" | "420paldv" | "420mpeg2" => (Some(Chroma::C420), false),
            "422" => (Some(Chroma::C422), false),
            "444" => (Some(Chroma::C444), false),
            "444alpha" if bits == 8 => (Some(Chroma::C444), true),
            "mono" => (None, false),
            _ => return None,
        };
        Some(Layout { chroma, bits, alpha })
    }

    /// Bytes of one stored sample: 1 at 8 bits, 2 above.
    pub fn bytes_per_sample(self) -> usize {
        if self.bits > 8 { 2 } else { 1 }
    }
}

/// The planes of one frame, as byte ranges within the frame's data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planes {
    pub y: Range<usize>,
    /// Empty for `mono`.
    pub cb: Range<usize>,
    /// Empty for `mono`.
    pub cr: Range<usize>,
    /// Empty without an alpha plane.
    pub alpha: Range<usize>,
}

/// The stream header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    /// Frame rate as numerator and denominator, both positive; `None` when the file names none.
    pub frame_rate: Option<(i64, i64)>,
    /// Pixel aspect ratio, both parts positive; `None` when absent or unknown (`0:0`).
    pub pixel_aspect: Option<(u32, u32)>,
    pub layout: Layout,
    /// Whether the file says `XCOLORRANGE=FULL`.
    pub full_range: bool,
}

fn parse_ratio(s: &str) -> Option<(i64, i64)> {
    let (n, d) = s.split_once(':')?;
    Some((n.parse().ok()?, d.parse().ok()?))
}

impl Header {
    /// Parses the stream header line (without its newline).
    pub fn parse(line: &[u8], limits: &Limits) -> Result<Header> {
        let header = String::from_utf8_lossy(line);
        let mut params = header.split(' ');
        if params.next() != Some("YUV4MPEG2") {
            return Err(Error::Malformed("not a YUV4MPEG2 file"));
        }
        let (mut width, mut height, mut frame_rate, mut pixel_aspect, mut colour, mut full_range) = (0u32, 0u32, None, None, "420jpeg", false);
        for p in params {
            // The tag is the first character, whatever its length in bytes; an unknown one is skipped.
            let Some(tag) = p.chars().next() else { continue };
            let v = p.get(tag.len_utf8()..).unwrap_or_default();
            match tag {
                'W' => width = v.parse().map_err(|_| Error::Malformed("invalid width"))?,
                'H' => height = v.parse().map_err(|_| Error::Malformed("invalid height"))?,
                'F' => match parse_ratio(v) {
                    Some((n, d)) if n > 0 && d > 0 => frame_rate = Some((n, d)),
                    _ => return Err(Error::Malformed("invalid frame rate")),
                },
                'A' => {
                    if let Some((n, d)) = parse_ratio(v).filter(|&(n, d)| n > 0 && d > 0)
                        && let (Ok(n), Ok(d)) = (u32::try_from(n), u32::try_from(d))
                    {
                        pixel_aspect = Some((n, d));
                    }
                }
                'C' => colour = v,
                'X' => full_range |= v.eq_ignore_ascii_case("COLORRANGE=FULL"),
                _ => {}
            }
        }
        if width == 0 || height == 0 {
            return Err(Error::Malformed("missing or zero size"));
        }
        if u64::from(width) * u64::from(height) > limits.max_pixels {
            return Err(Error::LimitExceeded("pixel count"));
        }
        let layout = Layout::parse(colour).ok_or_else(|| Error::UnsupportedColourSpace(colour.to_string()))?;
        Ok(Header { width, height, frame_rate, pixel_aspect, layout, full_range })
    }

    /// Sizes of one frame's planes in samples: luma, each chroma plane (0 for mono), alpha.
    pub fn plane_samples(&self) -> (usize, usize, usize) {
        plane_samples(self.width, self.height, self.layout)
    }

    /// Samples of a chroma plane of this size at `chroma` subsampling (what a caller needs to give
    /// a `mono` file neutral chroma).
    pub fn chroma_samples(&self, chroma: Chroma) -> usize {
        plane_samples(self.width, self.height, Layout { chroma: Some(chroma), ..self.layout }).1
    }

    /// Bytes of one frame's planes.
    pub fn frame_bytes(&self) -> usize {
        let (luma, chroma, alpha) = self.plane_samples();
        luma.saturating_add(chroma.saturating_mul(2)).saturating_add(alpha).saturating_mul(self.layout.bytes_per_sample())
    }

    /// Where each plane is within one frame's data.
    pub fn planes(&self) -> Planes {
        let (luma, chroma, alpha) = self.plane_samples();
        let b = self.layout.bytes_per_sample();
        let (y_end, cb_end, cr_end) = (luma * b, (luma + chroma) * b, (luma + 2 * chroma) * b);
        Planes { y: 0..y_end, cb: y_end..cb_end, cr: cb_end..cr_end, alpha: cr_end..cr_end + alpha * b }
    }
}

fn plane_samples(w: u32, h: u32, l: Layout) -> (usize, usize, usize) {
    let luma = (w as usize).saturating_mul(h as usize);
    let chroma = l.chroma.map_or(0, |c| {
        let (sx, sy) = c.shifts();
        (w.div_ceil(1 << sx) as usize).saturating_mul(h.div_ceil(1 << sy) as usize)
    });
    (luma, chroma, if l.alpha { luma } else { 0 })
}

/// The samples of a plane stored above 8 bits (16-bit little-endian), each clamped to the largest
/// value `bits` allows.
pub fn samples_u16(plane: &[u8], bits: u32) -> Vec<u16> {
    let max = u16::MAX >> (16 - bits.clamp(1, 16));
    plane.as_chunks::<2>().0.iter().map(|&s| u16::from_le_bytes(s).min(max)).collect()
}

/// An opened file: its header and where its frames are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub header: Header,
    /// File offset of each complete frame's planes, in order.
    pub frames: Vec<u64>,
    /// Bytes of one frame's planes ([`Header::frame_bytes`]).
    pub frame_bytes: usize,
}

/// The line starting at `pos` (without its newline) and the offset after it.
fn line_at(len: u64, read: &mut impl FnMut(u64, usize) -> Option<Vec<u8>>, pos: u64, max_line: usize) -> Option<(Vec<u8>, u64)> {
    let mut want = 128usize.min(max_line.max(1));
    loop {
        let take = len.saturating_sub(pos).min(want as u64) as usize;
        let mut buf = read(pos, take)?;
        buf.truncate(take);
        if let Some(end) = buf.iter().position(|&b| b == b'\n') {
            buf.truncate(end);
            return Some((buf, pos.checked_add(end as u64 + 1)?));
        }
        if take < want || want >= max_line {
            return None;
        }
        want = want.saturating_mul(8).min(max_line);
    }
}

impl Stream {
    /// Opens a file of `len` bytes through `read(offset, length)`, which returns that range of the
    /// file (`None` on failure). Only the stream header and the `FRAME` lines are read. The index
    /// ends at the first frame that is damaged or cut short, or at [`Limits::max_frames`]; a file
    /// without one complete frame is an error.
    pub fn open(len: u64, mut read: impl FnMut(u64, usize) -> Option<Vec<u8>>, limits: &Limits) -> Result<Stream> {
        let (line, mut pos) = line_at(len, &mut read, 0, limits.max_line).ok_or(Error::Malformed("no stream header"))?;
        let header = Header::parse(&line, limits)?;
        let frame_bytes = header.frame_bytes();
        let mut frames = Vec::new();
        while pos < len && frames.len() < limits.max_frames {
            let Some((line, data)) = line_at(len, &mut read, pos, limits.max_line) else { break };
            if !line.starts_with(b"FRAME") {
                break;
            }
            let Some(end) = data.checked_add(frame_bytes as u64).filter(|&e| e <= len) else { break };
            frames.push(data);
            pos = end;
        }
        if frames.is_empty() {
            return Err(Error::Malformed("no complete frames"));
        }
        Ok(Stream { header, frames, frame_bytes })
    }

    /// Opens a file held in memory.
    pub fn from_bytes(bytes: &[u8], limits: &Limits) -> Result<Stream> {
        Self::open(bytes.len() as u64, |pos, n| usize::try_from(pos).ok().and_then(|p| bytes.get(p..p.checked_add(n)?)).map(<[u8]>::to_vec), limits)
    }

    /// The file range of frame `index`'s planes, `None` past the last frame.
    pub fn frame(&self, index: usize) -> Option<Range<u64>> {
        let at = *self.frames.get(index)?;
        Some(at..at.checked_add(self.frame_bytes as u64)?)
    }
}

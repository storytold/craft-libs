//! Sierra VMD video decoder: the 8-bit paletted pictures of VMD (Video and Music Data) files, as
//! in Sierra's and Coktel Vision's 1990s games.
//!
//! Clean-room, from the public format description at <https://wiki.multimedia.cx/index.php/VMD>
//! ("Description of the Sierra Video and Music Data (VMD) Format", Mike Melanson and Vladimir
//! Gneushev). Where the description leaves a choice open, the decoder does what ffmpeg does with
//! the same bytes, observed from outside on files made by this crate's tests:
//!
//! - a frame's new palette replaces all 256 entries, whatever the two bytes before it say;
//! - a line of a method 1 or 3 frame must fill its width exactly.
//!
//! A [`Decoder`] is made from the 816-byte file header and holds one picture of palette indices.
//! Each video frame updates a rectangle of it: [`Decoder::decode`] takes the rectangle and the
//! palette flag from the frame's record in the file's table of contents (`craft_media::vmd`
//! reads both) and the frame data. The first frame of a file paints the whole picture; every
//! later frame depends on the ones before it, so decoding starts at the first frame.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let bytes: Vec<u8> = Vec::new();
//! use craft_media::vmd::{FrameKind, open};
//! use craft_video::vmd::{Decoder, Rect};
//!
//! let file = open(&bytes)?;
//! let mut decoder = Decoder::new(file.header_bytes())?;
//! for (i, frame) in file.frames.iter().enumerate() {
//!     let FrameKind::Video { new_palette, .. } = frame.kind else { continue };
//!     let data = file.read_frame(&bytes, i)?;
//!     match file.picture_rect(i) {
//!         Some(rect) => decoder.decode(Rect::from(rect), new_palette, &data)?,
//!         // a frame that paints nothing in the picture can still bring a palette
//!         None if new_palette => decoder.decode_palette(&data)?,
//!         None => {}
//!     }
//!     let rgb = decoder.rgb24(); // the picture after this block
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Not decoded: the 16-bit and 24-bit pictures and the Indeo 3 video of later Coktel Vision
//! files ([`Error::Unsupported`]).
//!
//! Input is treated as hostile: the picture is checked against the decode memory budget
//! ([`crate::budget`]), LZ output is bounded by the header's unpack buffer size and by a few
//! bytes a pixel of the picture, and every copy is bounds-checked. No `unsafe`, no panics.

#![warn(missing_docs)]

use crate::budget::{DECODE_MEMORY_BUDGET, PictureShape};

/// Bytes of the VMD file header a [`Decoder`] is made from.
pub const HEADER_LEN: usize = 816;

/// Errors from decoding VMD video.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The header or a frame is not valid.
    #[error("invalid VMD video: {0}")]
    Invalid(&'static str),
    /// Valid, but not something this decoder handles.
    #[error("unsupported VMD video: {0}")]
    Unsupported(&'static str),
    /// The picture does not fit the decode memory budget.
    #[error("a VMD picture of {0}x{1} exceeds the decode memory budget")]
    TooLarge(u16, u16),
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, Error>;

/// The part of the picture a frame updates. `right` and `bottom` are the last column and row
/// updated, not one past them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// First column.
    pub left: u16,
    /// First row.
    pub top: u16,
    /// Last column.
    pub right: u16,
    /// Last row.
    pub bottom: u16,
}

impl From<(u16, u16, u16, u16)> for Rect {
    /// From `(left, top, right, bottom)`, as `craft_media::vmd::VmdFile::picture_rect` returns
    /// it.
    fn from((left, top, right, bottom): (u16, u16, u16, u16)) -> Rect {
        Rect { left, top, right, bottom }
    }
}

/// A VMD video decoder: one picture of palette indices and the current palette.
#[derive(Clone, Debug)]
pub struct Decoder {
    width: usize,
    height: usize,
    indices: Vec<u8>,
    palette: [[u8; 3]; 256],
    /// The header's unpack buffer size: the most an LZ-packed frame may unpack to.
    unpack_limit: usize,
    /// LZ output of the current frame, kept between frames to save the allocation.
    unpacked: Vec<u8>,
}

impl Decoder {
    /// A decoder for the file whose header is `header` (its first 816 bytes; more are ignored).
    /// The picture starts as index 0 everywhere, with the header's palette.
    pub fn new(header: &[u8]) -> Result<Decoder> {
        let h = header.get(..HEADER_LEN).ok_or(Error::Invalid("the header is shorter than 816 bytes"))?;
        let u16_at = |at: usize| u16::from_le_bytes([h[at], h[at + 1]]);
        if usize::from(u16_at(0)) != HEADER_LEN - 2 {
            return Err(Error::Invalid("not a VMD header"));
        }
        if h[24..28].eq_ignore_ascii_case(b"iv32") {
            return Err(Error::Unsupported("Indeo 3 video"));
        }
        match u16_at(4) {
            5 => return Err(Error::Unsupported("24-bit pictures")),
            7 => return Err(Error::Unsupported("Indeo 3 video")),
            13 => return Err(Error::Unsupported("16-bit pictures")),
            _ => {}
        }
        let (width, height) = (u16_at(12), u16_at(14));
        if width == 0 || height == 0 {
            return Err(Error::Invalid("the picture has no pixels"));
        }
        // the indices, an RGBA copy of them for the caller, and the unpack buffer
        let shape = PictureShape { width: width.into(), height: height.into(), shift_x: 0, shift_y: 0, chroma_planes: 0, bytes_per_sample: 9 };
        if !shape.fits(1, DECODE_MEMORY_BUDGET) {
            return Err(Error::TooLarge(width, height));
        }
        let (width, height) = (usize::from(width), usize::from(height));
        let mut d = Decoder {
            width,
            height,
            indices: vec![0; width * height],
            palette: [[0; 3]; 256],
            unpack_limit: u32::from_le_bytes([h[800], h[801], h[802], h[803]]) as usize,
            unpacked: Vec::new(),
        };
        d.load_palette(&h[28..796]);
        Ok(d)
    }

    /// Picture width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Picture height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// The picture as palette indices: `width * height` bytes, rows from the top.
    pub fn indices(&self) -> &[u8] {
        &self.indices
    }

    /// The current palette as 8-bit R, G, B. The file stores 6-bit components; they are widened
    /// by repeating their top bits, so 63 becomes 255.
    pub fn palette(&self) -> &[[u8; 3]; 256] {
        &self.palette
    }

    /// The picture as packed 8-bit R, G, B: `width * height * 3` bytes, rows from the top.
    pub fn rgb24(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.indices.len() * 3);
        for &i in &self.indices {
            out.extend_from_slice(&self.palette[usize::from(i)]);
        }
        out
    }

    /// The picture as packed 8-bit R, G, B, A with opaque alpha: `width * height * 4` bytes.
    pub fn rgba8(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.indices.len() * 4);
        for &i in &self.indices {
            let [r, g, b] = self.palette[usize::from(i)];
            out.extend_from_slice(&[r, g, b, 255]);
        }
        out
    }

    /// Decode one video frame into the picture. `rect` is the part of the picture the frame
    /// updates and `new_palette` whether its data begins with a palette, both from the frame's
    /// record; `data` is the frame data. An empty frame changes nothing.
    ///
    /// On an error the picture holds whatever the frame had painted up to that point; later
    /// frames still decode against it.
    pub fn decode(&mut self, rect: Rect, new_palette: bool, data: &[u8]) -> Result<()> {
        let mut data = data;
        if new_palette && !data.is_empty() {
            self.decode_palette(data)?;
            data = &data[770..];
        }
        let Some((&method, body)) = data.split_first() else { return Ok(()) };
        if rect.right < rect.left || rect.bottom < rect.top || usize::from(rect.right) >= self.width || usize::from(rect.bottom) >= self.height {
            return Err(Error::Invalid("the frame lies outside the picture"));
        }
        if method & 0x80 == 0 {
            return self.render(rect, method, body);
        }
        let mut unpacked = std::mem::take(&mut self.unpacked);
        // a frame needs at most three bytes a pixel (more only with run-length codes that
        // paint nothing): unpacking stops past that, so that a frame's cost goes by the
        // picture's size and not by what the file says
        let useful = self.indices.len().saturating_mul(4).saturating_add(1024);
        let result = lz_unpack(body, self.unpack_limit, useful, &mut unpacked).and_then(|()| self.render(rect, method & 0x7F, &unpacked));
        self.unpacked = unpacked;
        result
    }

    /// Take the palette that the data of a frame with a new palette begins with, and paint
    /// nothing: for a video frame that has no rectangle in the picture (an empty one, as ffmpeg
    /// was seen to accept, or one outside the picture). [`Decoder::decode`] does this itself for
    /// the frames it is given. An empty frame changes nothing.
    pub fn decode_palette(&mut self, data: &[u8]) -> Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        // first index and count (ignored, see the module documentation), then 256 entries
        let chunk = data.get(..770).ok_or(Error::Invalid("the palette is cut short"))?;
        self.load_palette(&chunk[2..]);
        Ok(())
    }

    fn load_palette(&mut self, vga: &[u8]) {
        for (entry, rgb) in self.palette.iter_mut().zip(vga.as_chunks::<3>().0) {
            for (c, &v) in entry.iter_mut().zip(rgb) {
                let v = v & 0x3F;
                *c = v << 2 | v >> 4;
            }
        }
    }

    /// Paint `data` (the frame body, unpacked if it was LZ-packed) onto `rect`.
    fn render(&mut self, rect: Rect, method: u8, data: &[u8]) -> Result<()> {
        const SHORT: Error = Error::Invalid("the frame data ends early");
        let left = usize::from(rect.left);
        let width = usize::from(rect.right) - left + 1;
        let rows = usize::from(rect.top)..=usize::from(rect.bottom);
        let mut src = Reader(data);
        match method {
            // lines of runs: skip (keep the pixels of the previous picture) or copy
            1 | 3 => {
                for y in rows {
                    let line = &mut self.indices[y * self.width + left..][..width];
                    let mut x = 0;
                    while x < width {
                        let code = src.byte().ok_or(SHORT)?;
                        let n = usize::from(code & 0x7F) + 1;
                        let run = line.get_mut(x..x + n).ok_or(Error::Invalid("a run crosses the end of its line"))?;
                        if code & 0x80 != 0 {
                            if method == 3 && src.peek() == Some(0xFF) {
                                src.byte();
                                unpack_rle(&mut src, run)?;
                            } else {
                                run.copy_from_slice(src.take(n).ok_or(SHORT)?);
                            }
                        }
                        x += n;
                    }
                }
                Ok(())
            }
            // the pixels as they are
            2 => {
                for y in rows {
                    let line = &mut self.indices[y * self.width + left..][..width];
                    line.copy_from_slice(src.take(width).ok_or(SHORT)?);
                }
                Ok(())
            }
            _ => Err(Error::Unsupported("unknown rendering method")),
        }
    }
}

/// A cursor over frame data.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn peek(&self) -> Option<u8> {
        self.0.first().copied()
    }

    fn byte(&mut self) -> Option<u8> {
        let (&b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(b)
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }
}

/// Fill `run` from the run-length coding of method 3: a single byte if the run is odd, then
/// pairs of bytes, either copied or one pair repeated.
fn unpack_rle(src: &mut Reader, run: &mut [u8]) -> Result<()> {
    const SHORT: Error = Error::Invalid("the frame data ends early");
    const LONG: Error = Error::Invalid("run-length data overruns its run");
    let mut run = run;
    if run.len() % 2 == 1 {
        run[0] = src.byte().ok_or(SHORT)?;
        run = &mut run[1..];
    }
    while !run.is_empty() {
        let code = src.byte().ok_or(SHORT)?;
        let n = usize::from(code & 0x7F) * 2;
        let (head, rest) = run.split_at_mut_checked(n).ok_or(LONG)?;
        if code & 0x80 != 0 {
            head.copy_from_slice(src.take(n).ok_or(SHORT)?);
        } else {
            let pair = src.take(2).ok_or(SHORT)?;
            for p in head.as_chunks_mut::<2>().0 {
                p.copy_from_slice(pair);
            }
        }
        run = rest;
    }
    Ok(())
}

/// Unpack an LZ-packed frame body into `out`: the unpacked length, an optional marker that
/// selects long chains, then tag bytes whose bits (lowest first) each announce a literal byte
/// or a chain copied from a 4096-byte window of earlier output. Packed data that ends before
/// the length is reached leaves `out` short; the rendering then fails if it needs more. A length
/// above `limit` is an error; unpacking stops after `useful` bytes.
fn lz_unpack(src: &[u8], limit: usize, useful: usize, out: &mut Vec<u8>) -> Result<()> {
    out.clear();
    let mut src = Reader(src);
    let len = src.take(4).ok_or(Error::Invalid("the frame data ends early"))?;
    let mut left = u32::from_le_bytes([len[0], len[1], len[2], len[3]]) as usize;
    if left > limit {
        return Err(Error::Invalid("an LZ-packed frame is larger than the unpack buffer"));
    }
    left = left.min(useful);
    let mut window = Window { bytes: [0x20; 4096], at: 0xFEE };
    let mut long_chains = false;
    if src.0.starts_with(&[0x34, 0x12, 0x78, 0x56]) {
        src.take(4);
        window.at = 0x111;
        long_chains = true;
    }
    while left > 0 {
        let Some(tag) = src.byte() else { return Ok(()) };
        if tag == 0xFF && left > 8 {
            let Some(raw) = src.take(8) else { return Ok(()) };
            for &b in raw {
                window.emit(b, out);
            }
            left -= 8;
            continue;
        }
        for bit in 0..8 {
            if left == 0 {
                break;
            }
            if tag >> bit & 1 == 1 {
                let Some(b) = src.byte() else { return Ok(()) };
                window.emit(b, out);
                left -= 1;
                continue;
            }
            let Some(&[lo, hi]) = src.take(2).and_then(|p| p.first_chunk::<2>()) else { return Ok(()) };
            let from = usize::from(lo) | usize::from(hi & 0xF0) << 4;
            let mut n = usize::from(hi & 0x0F) + 3;
            if long_chains && n == 18 {
                let Some(more) = src.byte() else { return Ok(()) };
                n += usize::from(more);
            }
            let n = n.min(left);
            // byte by byte: a chain may read what it has just written
            for i in 0..n {
                window.emit(window.bytes[(from + i) & 0xFFF], out);
            }
            left -= n;
        }
    }
    Ok(())
}

/// The 4096 bytes of earlier output that LZ chains copy from, and where the next byte goes.
struct Window {
    bytes: [u8; 4096],
    at: usize,
}

impl Window {
    fn emit(&mut self, b: u8, out: &mut Vec<u8>) {
        out.push(b);
        self.bytes[self.at] = b;
        self.at = (self.at + 1) & 0xFFF;
    }
}

//! Uncompressed packed pictures: unpacking to and packing from planar samples, for the layouts
//! capture cards, QuickTime and the editing interchange formats use (feature `rawvideo`).
//!
//! | [`Layout`] | Samples | Packing |
//! |---|---|---|
//! | `Uyvy` | 8-bit 4:2:2 | `U Y0 V Y1` per two pixels (QuickTime `2vuy`, `UYVY`) |
//! | `Yuyv` | 8-bit 4:2:2 | `Y0 U Y1 V` (`YUY2`, `YUYV`) |
//! | `Yvyu` | 8-bit 4:2:2 | `Y0 V Y1 U` (`YVYU`) |
//! | `V210` | 10-bit 4:2:2 | six pixels in four little-endian 32-bit words (`Cb0 Y0 Cr0`, `Y1 Cb1 Y2`, `Cr1 Y3 Cb2`, `Y4 Cr2 Y5`, ten bits each, the low field first); rows padded to a multiple of 128 bytes |
//! | `R210` | 10-bit RGB | one big-endian 32-bit word per pixel, `R` in bits 29-20, `G` in 19-10, `B` in 9-0; rows padded to a multiple of 256 bytes |
//! | `R10k` | 10-bit RGB | one big-endian 32-bit word per pixel, `R` in bits 31-22, `G` in 21-12, `B` in 11-2; rows are not padded |
//!
//! [`decode`] returns a [`Frame`] of `u16` planes (8-bit samples are stored in the low byte);
//! [`encode`] packs a frame back, so `encode(decode(x))` reproduces a well-formed `x` byte for byte
//! (padding bytes are written as zero). The luma and chroma of 4:2:2 pictures are as stored, with no
//! range or matrix conversion. 4:2:2 layouts need an even width.
//!
//! Written from the vendors' and SMPTE's packing descriptions. ffmpeg is only run as a black-box
//! oracle in the tests.

/// What went wrong packing or unpacking.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The picture size cannot be used with the layout (zero, odd for 4:2:2, or too large).
    #[error("invalid picture size {width}x{height} for {layout:?}")]
    Size {
        /// The layout.
        layout: Layout,
        /// The width.
        width: u32,
        /// The height.
        height: u32,
    },
    /// The data is shorter than the picture needs.
    #[error("packed data is {got} bytes, {need} needed")]
    Short {
        /// Bytes given.
        got: usize,
        /// Bytes needed.
        need: usize,
    },
    /// The frame does not match the layout (kind, planes or a sample above the bit depth).
    #[error("frame does not fit the layout: {0}")]
    Frame(&'static str),
}

/// `Result` of this module.
pub type Result<T> = std::result::Result<T, Error>;

/// A packed layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layout {
    Uyvy,
    Yuyv,
    Yvyu,
    V210,
    R210,
    R10k,
}

/// What the planes of a [`Frame`] hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Planes Y (`width × height`), Cb and Cr (`width / 2 × height`).
    Yuv422,
    /// Planes R, G and B (`width × height`).
    Rgb,
}

/// The largest width or height accepted.
pub const MAX_DIMENSION: u32 = 1 << 15;

impl Layout {
    /// The planes a picture of this layout unpacks to.
    pub fn kind(self) -> Kind {
        match self {
            Layout::Uyvy | Layout::Yuyv | Layout::Yvyu | Layout::V210 => Kind::Yuv422,
            Layout::R210 | Layout::R10k => Kind::Rgb,
        }
    }

    /// Bits per sample: 8 or 10.
    pub fn bit_depth(self) -> u8 {
        match self {
            Layout::Uyvy | Layout::Yuyv | Layout::Yvyu => 8,
            _ => 10,
        }
    }

    /// Bytes of one packed row, or `None` if `width` cannot be used with the layout.
    pub fn stride(self, width: u32) -> Option<usize> {
        if width == 0 || width > MAX_DIMENSION || (self.kind() == Kind::Yuv422 && !width.is_multiple_of(2)) {
            return None;
        }
        let w = width as usize;
        Some(match self {
            Layout::Uyvy | Layout::Yuyv | Layout::Yvyu => w * 2,
            Layout::V210 => w.div_ceil(48) * 128,
            Layout::R210 => w.div_ceil(64) * 256,
            Layout::R10k => w * 4,
        })
    }

    /// Bytes of a whole packed picture.
    pub fn frame_bytes(self, width: u32, height: u32) -> Option<usize> {
        if height == 0 || height > MAX_DIMENSION {
            return None;
        }
        self.stride(width)?.checked_mul(height as usize)
    }
}

/// An unpacked picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub kind: Kind,
    /// 8 or 10.
    pub bit_depth: u8,
    /// Y, Cb, Cr or R, G, B, row by row.
    pub planes: [Vec<u16>; 3],
}

impl Frame {
    /// A black-filled frame (all zero samples).
    pub fn new(width: u32, height: u32, kind: Kind, bit_depth: u8) -> Result<Frame> {
        if width == 0 || width > MAX_DIMENSION || height == 0 || height > MAX_DIMENSION {
            return Err(Error::Frame("dimensions out of range"));
        }
        if kind == Kind::Yuv422 && !width.is_multiple_of(2) {
            return Err(Error::Frame("odd width for 4:2:2"));
        }
        if bit_depth != 8 && bit_depth != 10 {
            return Err(Error::Frame("unsupported bit depth"));
        }
        let (w, h) = (width as usize, height as usize);
        let luma_len = w.checked_mul(h).ok_or(Error::Frame("allocation overflow"))?;
        let chroma_w = if kind == Kind::Yuv422 { w / 2 } else { w };
        let chroma_len = chroma_w.checked_mul(h).ok_or(Error::Frame("allocation overflow"))?;
        let total_bytes =
            luma_len.checked_mul(2).and_then(|l| l.checked_add(chroma_len.checked_mul(4)?)).ok_or(Error::Frame("allocation overflow"))?;
        if total_bytes as u64 > crate::budget::DECODE_MEMORY_BUDGET {
            return Err(Error::Frame("frame exceeds memory budget"));
        }
        Ok(Frame { width, height, kind, bit_depth, planes: [vec![0; luma_len], vec![0; chroma_len], vec![0; chroma_len]] })
    }
}

/// `p[i] = v`, if `i` is in range.
fn put(p: &mut [u16], i: usize, v: u16) {
    if let Some(s) = p.get_mut(i) {
        *s = v;
    }
}

fn size_error(layout: Layout, width: u32, height: u32) -> Error {
    Error::Size { layout, width, height }
}

/// Unpack a picture. `data` may be longer than one picture; only the first is read.
pub fn decode(layout: Layout, width: u32, height: u32, data: &[u8]) -> Result<Frame> {
    let stride = layout.stride(width).ok_or_else(|| size_error(layout, width, height))?;
    let need = layout.frame_bytes(width, height).ok_or_else(|| size_error(layout, width, height))?;
    if data.len() < need {
        return Err(Error::Short { got: data.len(), need });
    }
    let mut f = Frame::new(width, height, layout.kind(), layout.bit_depth())?;
    let w = width as usize;
    let cw = if f.kind == Kind::Yuv422 { w / 2 } else { w };
    let [p0, p1, p2] = &mut f.planes;
    for (y, row) in data.chunks(stride).take(height as usize).enumerate() {
        let (r0, r1, r2) = (y * w, y * cw, y * cw);
        match layout {
            Layout::Uyvy | Layout::Yuyv | Layout::Yvyu => {
                let (iy, iu, iv) = match layout {
                    Layout::Uyvy => (1, 0, 2),
                    Layout::Yuyv => (0, 1, 3),
                    _ => (0, 3, 1),
                };
                for (x, q) in row.as_chunks::<4>().0.iter().take(cw).enumerate() {
                    let at = |i: usize| u16::from(q.get(i).copied().unwrap_or(0));
                    // the two lumas sit two bytes apart: first at `iy`, second at `iy + 2`
                    put(p0, r0 + 2 * x, at(iy));
                    put(p0, r0 + 2 * x + 1, at(iy + 2));
                    put(p1, r1 + x, at(iu));
                    put(p2, r2 + x, at(iv));
                }
            }
            Layout::V210 => {
                for (g, block) in row.as_chunks::<16>().0.iter().enumerate() {
                    let word = |i: usize| -> [u16; 3] {
                        let b = block.get(4 * i..4 * i + 4).unwrap_or(&[0; 4]);
                        let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                        [(v & 0x3FF) as u16, ((v >> 10) & 0x3FF) as u16, ((v >> 20) & 0x3FF) as u16]
                    };
                    let (w0, w1, w2, w3) = (word(0), word(1), word(2), word(3));
                    // Cb0 Y0 Cr0 | Y1 Cb1 Y2 | Cr1 Y3 Cb2 | Y4 Cr2 Y5
                    let ys = [w0[1], w1[0], w1[2], w2[1], w3[0], w3[2]];
                    let cbs = [w0[0], w1[1], w2[2]];
                    let crs = [w0[2], w2[0], w3[1]];
                    for (k, v) in ys.into_iter().enumerate() {
                        if g * 6 + k < w {
                            put(p0, r0 + g * 6 + k, v);
                        }
                    }
                    for k in 0..3 {
                        if g * 3 + k < cw {
                            put(p1, r1 + g * 3 + k, cbs.get(k).copied().unwrap_or(0));
                            put(p2, r2 + g * 3 + k, crs.get(k).copied().unwrap_or(0));
                        }
                    }
                }
            }
            Layout::R210 | Layout::R10k => {
                let shift = if layout == Layout::R210 { 20 } else { 22 };
                for (x, q) in row.as_chunks::<4>().0.iter().take(w).enumerate() {
                    let v = u32::from_be_bytes(*q);
                    let field = |s: u32| ((v >> s) & 0x3FF) as u16;
                    put(p0, r0 + x, field(shift));
                    put(p1, r1 + x, field(shift - 10));
                    put(p2, r2 + x, field(shift - 20));
                }
            }
        }
    }
    Ok(f)
}

/// Pack a frame. The frame's kind and bit depth must be the layout's, its planes the right size
/// and every sample within the bit depth.
pub fn encode(layout: Layout, frame: &Frame) -> Result<Vec<u8>> {
    let (width, height) = (frame.width, frame.height);
    let stride = layout.stride(width).ok_or_else(|| size_error(layout, width, height))?;
    let total = layout.frame_bytes(width, height).ok_or_else(|| size_error(layout, width, height))?;
    if frame.kind != layout.kind() || frame.bit_depth != layout.bit_depth() {
        return Err(Error::Frame("the frame's kind or bit depth is not the layout's"));
    }
    let (w, h) = (width as usize, height as usize);
    let cw = if frame.kind == Kind::Yuv422 { w / 2 } else { w };
    let sizes = [w * h, cw * h, cw * h];
    if frame.planes.iter().zip(sizes).any(|(p, n)| p.len() != n) {
        return Err(Error::Frame("a plane has the wrong size"));
    }
    let max = (1u16 << frame.bit_depth) - 1;
    if frame.planes.iter().flatten().any(|&v| v > max) {
        return Err(Error::Frame("a sample is above the bit depth"));
    }
    let mut out = vec![0u8; total];
    let [p0, p1, p2] = &frame.planes;
    let get = |p: &Vec<u16>, i: usize| p.get(i).copied().unwrap_or(0);
    for (y, row) in out.chunks_mut(stride).enumerate() {
        let (r0, r1, r2) = (y * w, y * cw, y * cw);
        match layout {
            Layout::Uyvy | Layout::Yuyv | Layout::Yvyu => {
                let (iy, iu, iv) = match layout {
                    Layout::Uyvy => (1, 0, 2),
                    Layout::Yuyv => (0, 1, 3),
                    _ => (0, 3, 1),
                };
                for (x, q) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    for (at, v) in [(iy, get(p0, r0 + 2 * x)), (iy + 2, get(p0, r0 + 2 * x + 1)), (iu, get(p1, r1 + x)), (iv, get(p2, r2 + x))] {
                        if let Some(b) = q.get_mut(at) {
                            *b = v as u8;
                        }
                    }
                }
            }
            Layout::V210 => {
                for (g, block) in row.as_chunks_mut::<16>().0.iter_mut().enumerate() {
                    let y = |k: usize| if g * 6 + k < w { u32::from(get(p0, r0 + g * 6 + k)) } else { 0 };
                    let cb = |k: usize| if g * 3 + k < cw { u32::from(get(p1, r1 + g * 3 + k)) } else { 0 };
                    let cr = |k: usize| if g * 3 + k < cw { u32::from(get(p2, r2 + g * 3 + k)) } else { 0 };
                    let words = [
                        cb(0) | (y(0) << 10) | (cr(0) << 20),
                        y(1) | (cb(1) << 10) | (y(2) << 20),
                        cr(1) | (y(3) << 10) | (cb(2) << 20),
                        y(4) | (cr(2) << 10) | (y(5) << 20),
                    ];
                    for (i, v) in words.into_iter().enumerate() {
                        if let Some(b) = block.get_mut(4 * i..4 * i + 4) {
                            b.copy_from_slice(&v.to_le_bytes());
                        }
                    }
                }
            }
            Layout::R210 | Layout::R10k => {
                let shift = if layout == Layout::R210 { 20 } else { 22 };
                for (x, q) in row.as_chunks_mut::<4>().0.iter_mut().take(w).enumerate() {
                    let v = (u32::from(get(p0, r0 + x)) << shift)
                        | (u32::from(get(p1, r1 + x)) << (shift - 10))
                        | (u32::from(get(p2, r2 + x)) << (shift - 20));
                    *q = v.to_be_bytes();
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strides() {
        assert_eq!(Layout::Uyvy.stride(1920), Some(3840));
        assert_eq!(Layout::V210.stride(1920), Some(5120));
        assert_eq!(Layout::V210.stride(50), Some(256));
        assert_eq!(Layout::R210.stride(1920), Some(7680));
        assert_eq!(Layout::R10k.stride(5), Some(20));
        assert_eq!(Layout::Uyvy.stride(3), None);
        assert_eq!(Layout::R10k.stride(0), None);
        assert_eq!(Layout::R10k.stride(MAX_DIMENSION + 1), None);
        assert_eq!(Layout::V210.frame_bytes(50, 0), None);
    }
}

//! Where each plane sits in a read-back buffer and in a tightly packed frame.

use crate::spec::{Error, Format};

/// One plane: its size in texels, the bytes of one texel, and where its rows start in a
/// read-back buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaneLayout {
    pub width: u32,
    pub height: u32,
    /// Bytes per texel: 1 or 2 for one sample, 2 or 4 for an interleaved Cb and Cr pair.
    pub texel_bytes: u32,
    /// The plane's first byte in the read-back buffer.
    pub offset: u64,
    /// Bytes from one row to the next in the read-back buffer, a multiple of
    /// `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`.
    pub bytes_per_row: u32,
}

impl PlaneLayout {
    /// Bytes of one row in a packed frame, or `u32::MAX` when the fields give more.
    pub fn row_bytes(&self) -> u32 {
        self.width.saturating_mul(self.texel_bytes)
    }
}

/// The planes of one frame of `format` at `width × height`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameLayout {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub planes: Vec<PlaneLayout>,
    /// Bytes a read-back buffer needs from its offset.
    pub buffer_size: u64,
}

fn aligned(n: u32) -> Option<u32> {
    let a = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    n.div_ceil(a).checked_mul(a)
}

/// The planes of `format` at `width × height` (the first `count` of the array) and the read-back
/// buffer size.
fn planes_of(format: Format, width: u32, height: u32) -> Result<([PlaneLayout; 3], usize, u64), Error> {
    if width == 0 || height == 0 || width % 2 == 1 || height % 2 == 1 {
        return Err(Error::OutputSize { width, height });
    }
    let s = format.sample_bytes();
    let (cw, ch) = (width / 2, height / 2);
    let (sizes, count) = match format {
        Format::Nv12 | Format::P010 => ([(width, height, s), (cw, ch, 2 * s), (0, 0, 0)], 2),
        Format::I420 => ([(width, height, s), (cw, ch, s), (cw, ch, s)], 3),
    };
    let overflow = Error::Overflow { width, height };
    let mut offset = 0u64;
    let mut tight = 0u64;
    let mut planes = [PlaneLayout { width: 0, height: 0, texel_bytes: 0, offset: 0, bytes_per_row: 0 }; 3];
    for (plane, (w, h, texel_bytes)) in planes.iter_mut().zip(sizes).take(count) {
        let row = w.checked_mul(texel_bytes).ok_or(overflow.clone())?;
        let bytes_per_row = aligned(row).ok_or(overflow.clone())?;
        *plane = PlaneLayout { width: w, height: h, texel_bytes, offset, bytes_per_row };
        offset = offset.checked_add(u64::from(bytes_per_row) * u64::from(h)).ok_or(overflow.clone())?;
        tight = tight.checked_add(u64::from(row) * u64::from(h)).ok_or(overflow.clone())?;
    }
    // A packed frame must also fit in memory on this target.
    usize::try_from(tight).map_err(|_| overflow)?;
    Ok((planes, count, offset))
}

impl FrameLayout {
    /// The layout of `format` at `width × height`; both sides must be even and not zero.
    pub fn new(format: Format, width: u32, height: u32) -> Result<FrameLayout, Error> {
        let (planes, count, buffer_size) = planes_of(format, width, height)?;
        Ok(FrameLayout { format, width, height, planes: planes[..count].to_vec(), buffer_size })
    }

    /// Whether the fields are what [`FrameLayout::new`] gives for the format and size. It does not
    /// allocate, so a frame loop can call it.
    fn is_valid(&self) -> bool {
        matches!(planes_of(self.format, self.width, self.height),
            Ok((planes, count, size)) if planes[..count] == self.planes[..] && size == self.buffer_size)
    }

    /// Bytes of one tightly packed frame: the planes one after another, each row without padding.
    /// Fields changed after [`FrameLayout::new`] that give more than `usize::MAX` give
    /// `usize::MAX`.
    pub fn frame_bytes(&self) -> usize {
        self.planes
            .iter()
            .fold(0usize, |n, p| n.saturating_add((p.width as usize).saturating_mul(p.texel_bytes as usize).saturating_mul(p.height as usize)))
    }

    /// Copy the planes from `padded` (a mapped read-back buffer, from the offset it was copied to)
    /// into `frame` without the row padding. `frame` must be [`FrameLayout::frame_bytes`] long.
    /// A layout whose fields differ from what [`FrameLayout::new`] gives is [`Error::Layout`].
    pub fn unpack(&self, padded: &[u8], frame: &mut [u8]) -> Result<(), Error> {
        if !self.is_valid() {
            return Err(Error::Layout);
        }
        if (padded.len() as u64) < self.buffer_size {
            return Err(Error::BufferSize { needed: self.buffer_size, actual: padded.len() as u64 });
        }
        let tight = self.frame_bytes();
        if frame.len() != tight {
            return Err(Error::BufferSize { needed: tight as u64, actual: frame.len() as u64 });
        }
        let mut out = frame;
        for p in &self.planes {
            let (row, padded_row) = (p.row_bytes() as usize, p.bytes_per_row as usize);
            let (dst, rest) = out.split_at_mut(row * p.height as usize);
            let src = padded.get(p.offset as usize..).unwrap_or_default();
            for (d, s) in dst.chunks_exact_mut(row).zip(src.chunks(padded_row)) {
                // The layout and size checks above leave every row whole.
                if let Some(s) = s.get(..row) {
                    d.copy_from_slice(s);
                }
            }
            out = rest;
        }
        Ok(())
    }
}

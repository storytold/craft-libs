//! EXIF orientation transforms for decoded images. Byte-level EXIF/XMP inspection and edits are
//! shared with `craft-metadata`.

use crate::error::CodecError;
use crate::image::Image;
use std::borrow::Cow;

pub use craft_metadata::edit::{exif_orientation, upright_exif, upright_xmp};

/// TIFF payload of an EXIF block (a JPEG-style `Exif\0\0` prefix is skipped).
pub(crate) fn tiff_body(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"Exif\0\0").unwrap_or(bytes)
}

/// Source pixel index for output pixel `(0, y2)` and the step per output column, for
/// orientation `o` of a `w`×`h` source (values 2–8; see the module docs).
fn walk(o: u16, w: isize, h: isize, y2: isize) -> (isize, isize) {
    match o {
        2 => (y2 * w + w - 1, -1),
        3 => ((h - 1 - y2) * w + w - 1, -1),
        4 => ((h - 1 - y2) * w, 1),
        5 => (y2, w),
        6 => ((h - 1) * w + y2, -w),
        7 => ((h - 1) * w + (w - 1 - y2), -w),
        8 => (w - 1 - y2, w),
        _ => (y2 * w, 1),
    }
}

/// Output columns per block: a block's source pixels (for a 90° turn, `BLOCK`
/// rows × the band height) stay in cache while it is written.
const BLOCK: usize = 64;
/// Output rows per parallel band.
const BAND: usize = 64;

/// Writes the oriented pixels of one band of output rows, `N` bytes per pixel.
fn band<const N: usize>(src: &[[u8; N]], dst: &mut [[u8; N]], o: u16, (w, h): (usize, usize), ow: usize, y0: usize) {
    let (wi, hi) = (w as isize, h as isize);
    let rows = dst.len() / ow.max(1);
    for x0 in (0..ow).step_by(BLOCK) {
        let x1 = (x0 + BLOCK).min(ow);
        for r in 0..rows {
            let (base, step) = walk(o, wi, hi, (y0 + r) as isize);
            let Some(out) = dst.get_mut(r * ow + x0..r * ow + x1) else { return };
            let mut i = base + x0 as isize * step;
            for px in out {
                if let Some(s) = usize::try_from(i).ok().and_then(|i| src.get(i)) {
                    *px = *s;
                }
                i += step;
            }
        }
    }
}

/// Orients `src` (`N` bytes per pixel) into a new buffer, a band of rows per task.
fn orient_n<const N: usize>(src: &[u8], o: u16, (w, h): (usize, usize), ow: usize) -> Result<Vec<u8>, CodecError> {
    let mut out = alloc_zeroed(src.len())?;
    let (s, _) = src.as_chunks::<N>();
    let (d, _) = out.as_chunks_mut::<N>();
    let per = (BAND * ow).max(1);
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        d.par_chunks_mut(per).enumerate().for_each(|(b, chunk)| band(s, chunk, o, (w, h), ow, b * BAND));
    }
    #[cfg(target_arch = "wasm32")]
    for (b, chunk) in d.chunks_mut(per).enumerate() {
        band(s, chunk, o, (w, h), ow, b * BAND);
    }
    Ok(out)
}

/// A zeroed buffer of `n` bytes, or an error (not an abort) when memory runs out.
fn alloc_zeroed(n: usize) -> Result<Vec<u8>, CodecError> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).map_err(|_| CodecError::LimitExceeded(format!("cannot allocate {n} bytes to orient the image")))?;
    out.resize(n, 0);
    Ok(out)
}

/// Pixel-format-agnostic orientation: any byte size per pixel.
fn orient_bytes(src: &[u8], bpp: usize, o: u16, (w, h): (usize, usize), ow: usize) -> Result<Vec<u8>, CodecError> {
    match bpp {
        1 => orient_n::<1>(src, o, (w, h), ow),
        2 => orient_n::<2>(src, o, (w, h), ow),
        3 => orient_n::<3>(src, o, (w, h), ow),
        4 => orient_n::<4>(src, o, (w, h), ow),
        5 => orient_n::<5>(src, o, (w, h), ow),
        6 => orient_n::<6>(src, o, (w, h), ow),
        8 => orient_n::<8>(src, o, (w, h), ow),
        10 => orient_n::<10>(src, o, (w, h), ow),
        12 => orient_n::<12>(src, o, (w, h), ow),
        16 => orient_n::<16>(src, o, (w, h), ow),
        _ => orient_n::<20>(src, o, (w, h), ow),
    }
}

/// Writes the oriented output rows `y0..` that fit in `dst` (whole rows of `ow` pixels), `bpp`
/// bytes per pixel: one band of [`orient_bytes`], the same pixels it writes.
fn orient_band(src: &[u8], dst: &mut [u8], bpp: usize, o: u16, (w, h): (usize, usize), ow: usize, y0: usize) {
    macro_rules! go {
        ($n:literal) => {{
            let (s, _) = src.as_chunks::<$n>();
            let (d, _) = dst.as_chunks_mut::<$n>();
            band(s, d, o, (w, h), ow, y0)
        }};
    }
    match bpp {
        1 => go!(1),
        2 => go!(2),
        3 => go!(3),
        4 => go!(4),
        5 => go!(5),
        6 => go!(6),
        8 => go!(8),
        10 => go!(10),
        12 => go!(12),
        16 => go!(16),
        _ => go!(20),
    }
}

/// An image turned upright a band of rows at a time, for a caller that must not block for the
/// whole image (the web build has no threads): [`OrientSteps::step`] gives the [`Image`]
/// [`Image::oriented`] gives, once every band is written.
pub struct OrientSteps {
    src: Image,
    o: u16,
    bpp: usize,
    /// The oriented size.
    ow: usize,
    oh: usize,
    out: Vec<u8>,
    /// Output rows written so far.
    done: usize,
}

impl OrientSteps {
    /// Sets up turning `img` upright for orientation `o` (2–8; anything else leaves it as it is).
    /// Allocates the output, which the image is turned into beside the original.
    pub fn new(img: Image, o: u16) -> Result<Self, CodecError> {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let (ow, oh) = if o >= 5 { (h, w) } else { (w, h) };
        let (layout, sample) = (img.layout(), img.sample_type());
        let bpp = layout.channels() * sample.bytes();
        let turns = (2..=8).contains(&o);
        if turns && (!matches!(bpp, 1..=6 | 8 | 10 | 12 | 16 | 20) || w == 0 || h == 0) {
            return Err(CodecError::InvalidImage(format!("cannot orient {layout:?} {sample:?} {w}x{h}")));
        }
        // Reserve without touching: `step` grows the buffer a band at a time, so no slice
        // zero-fills the whole output.
        let mut out = Vec::new();
        if turns {
            let n = img.data().len();
            out.try_reserve_exact(n).map_err(|_| CodecError::LimitExceeded(format!("cannot allocate {n} bytes to orient the image")))?;
        }
        Ok(Self { src: img, o, bpp, ow, oh, out, done: 0 })
    }

    /// How much of the image is turned, `0.0..=1.0`.
    pub fn progress(&self) -> f32 {
        if self.oh == 0 { 1.0 } else { self.done as f32 / self.oh as f32 }
    }

    /// Writes bands of [`BAND`] output rows, at least one, then goes on while `more()` returns
    /// true; `Some` with the upright image once the last band is written.
    pub fn step(&mut self, more: &mut dyn FnMut() -> bool) -> Result<Option<Image>, CodecError> {
        if !(2..=8).contains(&self.o) {
            let placeholder = Image::from_raw(0, 0, self.src.layout(), self.src.sample_type(), Vec::new())?;
            return Ok(Some(std::mem::replace(&mut self.src, placeholder)));
        }
        let (w, h) = (self.src.width() as usize, self.src.height() as usize);
        while self.done < self.oh {
            let rows = BAND.min(self.oh - self.done);
            let row_bytes = self.ow.checked_mul(self.bpp).ok_or_else(|| CodecError::InvalidImage("image too large".into()))?;
            let at = self.done.saturating_mul(row_bytes);
            let end = at.saturating_add(rows.saturating_mul(row_bytes));
            if self.out.len() < end {
                self.out.resize(end, 0);
            }
            let dst = self.out.get_mut(at..end).ok_or_else(|| CodecError::InvalidImage("the oriented rows are out of range".into()))?;
            orient_band(self.src.data(), dst, self.bpp, self.o, (w, h), self.ow, self.done);
            self.done += rows;
            if self.done < self.oh && !more() {
                return Ok(None);
            }
        }
        let (layout, sample) = (self.src.layout(), self.src.sample_type());
        let placeholder = Image::from_raw(0, 0, layout, sample, Vec::new())?;
        let src = std::mem::replace(&mut self.src, placeholder);
        let data = std::mem::take(&mut self.out);
        src.oriented_with(self.o, data).map(Some)
    }
}

impl Image {
    /// The image turned upright for orientation `o` (1–8; anything else is
    /// treated as 1 and returns the image unchanged). Width and height swap
    /// for 5–8, as does the DPI, and the EXIF and XMP orientation are
    /// rewritten to 1 so the result is not rotated again on display or export.
    pub fn oriented(self, o: u16) -> Result<Image, CodecError> {
        if !(2..=8).contains(&o) {
            return Ok(self);
        }
        let (w, h) = (self.width() as usize, self.height() as usize);
        let ow = if o >= 5 { h } else { w };
        let (layout, sample) = (self.layout(), self.sample_type());
        let bpp = layout.channels() * sample.bytes();
        // Every layout/sample pair is 1–20 bytes per pixel and has a fixed-size path.
        if !matches!(bpp, 1..=6 | 8 | 10 | 12 | 16 | 20) || w == 0 || h == 0 {
            return Err(CodecError::InvalidImage(format!("cannot orient {layout:?} {sample:?} {w}x{h}")));
        }
        let data = orient_bytes(self.data(), bpp, o, (w, h), ow)?;
        self.oriented_with(o, data)
    }

    /// The upright image of orientation `o` (2–8) whose turned pixels are `data`: the size,
    /// metadata and extra samples of [`Image::oriented`].
    fn oriented_with(self, o: u16, data: Vec<u8>) -> Result<Image, CodecError> {
        let (w, h) = (self.width() as usize, self.height() as usize);
        let swap = o >= 5;
        let (ow, oh) = if swap { (h, w) } else { (w, h) };
        let (layout, sample) = (self.layout(), self.sample_type());
        let mut meta = self.meta.clone();
        // Extra samples (a TIFF's alpha and spot planes, one sample a pixel) turn with the image.
        // A plane of the wrong size is left as it is: importers drop it with a warning.
        let plane_len = w.saturating_mul(h).saturating_mul(sample.bytes());
        for e in meta.extra_samples.iter_mut().filter(|e| e.data.len() == plane_len) {
            e.data = orient_bytes(&e.data, sample.bytes(), o, (w, h), ow)?;
        }
        if let Some(e) = &mut meta.exif
            && let Cow::Owned(fixed) = upright_exif(e)
        {
            *e = fixed;
        }
        if let Some(x) = &mut meta.xmp
            && let Cow::Owned(fixed) = upright_xmp(x)
        {
            *x = fixed;
        }
        if swap {
            meta.dpi = meta.dpi.map(|(x, y)| (y, x));
        }
        let mut out = Image::from_raw(ow as u32, oh as u32, layout, sample, data)?.with_icc(self.icc).with_meta(meta);
        out.warnings = self.warnings;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_impossible_allocation_is_an_error_not_an_abort() {
        assert!(matches!(alloc_zeroed(usize::MAX), Err(CodecError::LimitExceeded(_))));
        assert!(matches!(orient_n::<1>(&[], 6, (0, 0), 0), Ok(v) if v.is_empty()));
        assert_eq!(alloc_zeroed(3).unwrap(), vec![0, 0, 0]);
    }
}

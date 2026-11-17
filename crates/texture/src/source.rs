//! Row access to a plane of linear scalar samples, and the errors of the operations that read it.

use std::sync::Arc;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

use crate::{Image, ImageError, Kind, Limits};

/// A plane of linear scalar samples that an operation reads one row at a time.
///
/// An operation keeps a few rows of samples and its output, never a full `f32` copy of the
/// source, so a tiled or encoded image can be read in place: an adapter decodes the samples of a
/// row from whatever storage it has. Samples are data values, such as heights or roughness, read
/// without any transfer curve.
pub trait ScalarSource: Sync {
    /// Width and height in samples.
    fn dimensions(&self) -> (u32, u32);

    /// Write the samples of row `y` from column `x0` on into `out`, one per element. Operations
    /// ask only for rows inside [`ScalarSource::dimensions`] and for `x0 + out.len()` no greater
    /// than the width.
    fn read_row(&self, y: u32, x0: u32, out: &mut [f32]) -> Result<(), SourceError>;

    /// Whether the sample at `(x, y)` holds data. [`crate::normal::try_height_to_normal`] reads
    /// the center sample in place of a neighbor that is not covered. Every sample is covered by
    /// default.
    fn covered(&self, _x: u32, _y: u32) -> bool {
        true
    }
}

/// Why a [`ScalarSource`] could not give a row.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SourceError(pub String);

/// Why an operation over [`ScalarSource`]s could not make its image.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum TextureOpError {
    /// The output size is outside the limits, or its memory could not be reserved.
    #[error(transparent)]
    Image(#[from] ImageError),
    /// Two sources, or a source and the requested size, differ in size.
    #[error("source size {found:?} differs from {expected:?}")]
    SizeMismatch { expected: (u32, u32), found: (u32, u32) },
    /// No source is given and no output size either.
    #[error("no source or size gives the output size")]
    NoSize,
    /// A parameter is NaN, infinite or outside its range.
    #[error("parameter {name} is {value}, which is not finite or is out of range")]
    BadParameter { name: &'static str, value: f64 },
    /// A source gave a NaN or infinite sample.
    #[error("sample at ({x}, {y}) is not finite")]
    NonFiniteSample { x: u32, y: u32 },
    /// Finite samples gave a NaN or infinite result, such as two occlusion values whose product
    /// overflows `f32`.
    #[error("result at ({x}, {y}) is not finite")]
    NonFiniteResult { x: u32, y: u32 },
    /// A source could not give a row.
    #[error("reading row {y}: {error}")]
    Source { y: u32, error: SourceError },
}

/// An [`Image`] read as a scalar plane: the gray value, or the Rec. 709 luminance of a color
/// image, as [`Image::get_gray`] gives it.
impl ScalarSource for Image {
    fn dimensions(&self) -> (u32, u32) {
        (self.w, self.h)
    }

    fn read_row(&self, y: u32, x0: u32, out: &mut [f32]) -> Result<(), SourceError> {
        let end = u64::from(x0) + out.len() as u64;
        if y >= self.h || end > u64::from(self.w) {
            return Err(SourceError(format!("row {y} from column {x0} for {} samples is outside the {}x{} image", out.len(), self.w, self.h)));
        }
        for (x, o) in (x0..).zip(out.iter_mut()) {
            *o = self.get_gray(i64::from(x), i64::from(y), false);
        }
        Ok(())
    }
}

/// Read row `y` of `source` in full into `out`, which holds one row, and reject a non-finite
/// sample.
pub(crate) fn read_checked<S: ScalarSource + ?Sized>(source: &S, y: u32, out: &mut [f32]) -> Result<(), TextureOpError> {
    source.read_row(y, 0, out).map_err(|error| TextureOpError::Source { y, error })?;
    match out.iter().position(|v| !v.is_finite()) {
        Some(x) => Err(TextureOpError::NonFiniteSample { x: x as u32, y }),
        None => Ok(()),
    }
}

/// A zeroed buffer of `len` floats, reserved fallibly for a `w × h` operation.
pub(crate) fn try_buffer(len: usize, w: u32, h: u32) -> Result<Vec<f32>, TextureOpError> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).map_err(|_| ImageError::OutOfMemory(w, h))?;
    v.resize(len, 0.0);
    Ok(v)
}

/// Rows in each band that a worker fills when rows are filled on several threads.
#[cfg(feature = "parallel")]
const BAND_ROWS: usize = 64;

/// A `w × h` RGBA image whose rows `fill` writes. `fill(y0, rows)` gets the first row index and
/// the floats of whole rows: every row at once without the `parallel` feature, bands of rows on
/// several threads with it. The size is checked against `limits` and the buffer reserved first.
pub(crate) fn try_color_image<F>(w: u32, h: u32, limits: &Limits, fill: F) -> Result<Image, TextureOpError>
where
    F: Fn(u32, &mut [f32]) -> Result<(), TextureOpError> + Sync,
{
    let len = limits.check(w, h, Kind::Color)?;
    let mut data = try_buffer(len, w, h)?;
    #[cfg(feature = "parallel")]
    {
        let band = (w as usize * Kind::Color.channels()).saturating_mul(BAND_ROWS);
        data.par_chunks_mut(band).enumerate().try_for_each(|(i, rows)| fill((i * BAND_ROWS) as u32, rows))?;
    }
    #[cfg(not(feature = "parallel"))]
    fill(0, &mut data)?;
    Ok(Image { w, h, kind: Kind::Color, integer_sampling: false, data: Arc::new(data) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_read_as_gray_rows() {
        let gray = Image::gray(3, 2, |x, y| (x + 10 * y) as f32);
        let mut row = [0.0; 2];
        assert_eq!(gray.read_row(1, 1, &mut row), Ok(()));
        assert_eq!(row, [11.0, 12.0]);
        assert!(gray.read_row(2, 0, &mut row).is_err());
        assert!(gray.read_row(0, 2, &mut row).is_err());
        let color = Image::filled(1, 1, Kind::Color, [1.0, 0.0, 0.0, 1.0]);
        let mut one = [0.0];
        assert_eq!(color.read_row(0, 0, &mut one), Ok(()));
        assert!((one[0] - 0.2126).abs() < 1e-6);
    }
}

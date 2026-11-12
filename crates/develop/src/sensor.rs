//! The develop half of a camera raw: from [`craft_raw::RawImage`]'s sensor samples to camera RGB, and the
//! camera's colour model.
//!
//! - [`Develop::normalized`] subtracts black, scales white to 1.0 and crops to the active area (applying DNG
//!   `OpcodeList1`/`OpcodeList2`); [`demosaic()`] turns CFA data into camera-RGB [`Rgb32f`];
//!   [`Develop::develop`] does all of it plus `OpcodeList3` and the default crop; [`Binning::develop_binned`]
//!   produces the same at 1/k of the size straight from the mosaic (previews, thumbnails;
//!   [`Binning::develop_binned_masked`] with the mask of blocks holding clipped samples, for [`highlight`]).
//! - [`Saturation::lift_clipped_samples`] raises samples stuck below the stated white level to it, and
//!   [`HotPixels::suppress_hot_pixels`] replaces single outlier photosites; both work on the mosaic.
//! - [`color`] implements the DNG colour model (dual-illuminant interpolation, forward matrices, white balance)
//!   and produces camera → linear Rec.2020 D65 matrices; [`profile`] applies a DNG's own profile look tables and
//!   tone curve, and [`gaintable`] its gain table map (Apple ProRAW's local tone mapping); [`opcodes`] applies
//!   the opcode lists.
//! - Non-DNG files carry no colour matrix: [`spectral`] has matrices fitted to measured spectral sensitivities
//!   for 52 models, [`kelvin_presets`] gives a Kelvin scale from a camera's white-balance presets, and otherwise
//!   [`color`] falls back to a documented neutral model. [`dcp`] reads and writes DCP camera profiles.
//!
//! The base is LightCraft's `lightcraft-raw` (the parts [`craft_raw`] left out, see its README). Methods that
//! LightCraft had on its own `RawImage` are extension traits here ([`Develop`], [`Binning`], [`Saturation`],
//! [`HotPixels`]); `use craft_develop::sensor::prelude::*` brings them in.

mod binned;
pub mod color;
pub mod dcp;
pub mod demosaic;
pub mod gaintable;
pub mod highlight;
mod hotpixel;
pub mod kelvin_builtin;
pub mod kelvin_presets;
pub mod opcodes;
pub mod profile;
mod saturation;
pub mod semantic;
pub mod spectral;
pub mod wb_estimate;

pub use crate::color::Mat3;
pub use crate::raster::Rgb32f;
pub use binned::Binning;
pub use craft_raw::{
    BlackLevel, Camera, Cfa, ColorData, Opcode, OpcodeLists, Orientation, RawData, RawError, RawFormat, RawImage, RawInfo, Rect, Result,
    stretched_height,
};
pub use demosaic::{Method, demosaic};
pub use hotpixel::{HOT_PIXEL_MAX_SHARE, HotPixels};
pub use saturation::Saturation;

/// The extension traits of this module, for `use craft_develop::sensor::prelude::*`.
pub mod prelude {
    pub use super::gaintable::GainTableMapExt;
    pub use super::profile::{HsvTableExt, ProfileLookExt, ToneCurveExt};
    pub use super::{Binning, Develop, HotPixels, Saturation};
}

/// Black-subtracted, white-normalised data covering the active area.
#[derive(Clone, Debug, PartialEq)]
pub struct Normalized {
    pub width: usize,
    pub height: usize,
    pub cpp: usize,
    /// Nominal range [0, 1] (values may exceed 1 above white or be negative from noise).
    pub data: Vec<f32>,
    /// CFA anchored at (0, 0) of this buffer.
    pub cfa: Option<Cfa>,
}

/// The raw → camera RGB path.
pub trait Develop {
    /// Black-subtract, white-scale and crop to the active area, applying `OpcodeList1` (on raw values) first and
    /// `OpcodeList2` (on normalised values) after. 1.0 = white level.
    fn normalized(&self) -> Result<Normalized>;

    /// Full "raw → camera RGB" path: normalise, demosaic with `method`, apply `OpcodeList3`, then the default crop.
    /// The result is camera RGB (not white balanced), white level = 1.0, not oriented.
    fn develop(&self, method: Method) -> Result<Rgb32f>;

    /// The last steps of [`Self::develop`] for camera RGB made some other way (a denoiser that demosaics too) from
    /// [`Self::normalized`]'s mosaic: apply `OpcodeList3`, then the default crop.
    fn finish_demosaiced(&self, rgb: Rgb32f) -> Rgb32f;

    /// The part of a `width × height` demosaiced picture (the active area) that [`Self::develop`] keeps: the default
    /// crop, or all of it when there is none.
    fn develop_crop(&self, width: usize, height: usize) -> Rect;
}

impl Develop for RawImage {
    fn normalized(&self) -> Result<Normalized> {
        self.validate()?;
        let (w, cpp) = (self.width, self.cpp);
        let a = self.active_area;
        // stage 1 (full image) opcodes need a mutable float copy only when present
        let stage1: Option<Vec<f32>> = if self.opcodes.list1.is_empty() {
            None
        } else {
            let mut v: Vec<f32> = (0..self.data.len()).map(|i| self.data.get(i)).collect();
            opcodes::apply_list(&self.opcodes.list1, &mut v, self.width, self.height, cpp, self.cfa.as_ref(), 65535.0);
            Some(v)
        };
        let src = |i: usize| match &stage1 {
            Some(v) => v[i],
            None => self.data.get(i),
        };
        let scale: Vec<f32> = (0..cpp)
            .map(|s| {
                let range = self.white_at(s) - self.black.mean();
                if range > 0.0 { 1.0 / range } else { 1.0 }
            })
            .collect();
        let mut out = vec![0f32; a.width * a.height * cpp];
        let row_len = a.width * cpp;
        use rayon::prelude::*;
        out.par_chunks_mut(row_len).enumerate().for_each(|(y, row)| {
            let base = ((a.y + y) * w + a.x) * cpp;
            for x in 0..a.width {
                for s in 0..cpp {
                    let v = src(base + x * cpp + s);
                    let b = self.black.at(x, y, s, cpp);
                    // a NaN or infinite sample (a float DNG's, or a black level a file got wrong)
                    // reads as black, so the demosaic and every stage after it see numbers
                    let n = (v - b) * scale[s];
                    row[x * cpp + s] = if n.is_finite() { n } else { 0.0 };
                }
            }
        });
        let cfa = self.cfa.as_ref().map(|c| c.shifted(a.x, a.y));
        opcodes::apply_list(&self.opcodes.list2, &mut out, a.width, a.height, cpp, cfa.as_ref(), 1.0);
        Ok(Normalized { width: a.width, height: a.height, cpp, data: out, cfa })
    }

    fn develop(&self, method: Method) -> Result<Rgb32f> {
        let n = self.normalized()?;
        // with no `OpcodeList3` to apply to the whole picture first, AHD demosaics just the default crop (the same
        // pixels, without demosaicing the margin and then copying the crop out of the full picture)
        if self.opcodes.list3.is_empty() && method == Method::Ahd {
            let c = self.develop_crop(n.width, n.height);
            if let Some(rgb) = demosaic::ahd_window(&n, c) {
                drop(n);
                return Ok(stretched(self, rgb));
            }
        }
        let rgb = demosaic(&n, method);
        drop(n);
        Ok(self.finish_demosaiced(rgb))
    }

    fn finish_demosaiced(&self, mut rgb: Rgb32f) -> Rgb32f {
        opcodes::apply_list3(&self.opcodes.list3, &mut rgb);
        let c = self.develop_crop(rgb.width, rgb.height);
        let rgb =
            if c.x == 0 && c.y == 0 && c.width == rgb.width && c.height == rgb.height { rgb } else { rgb.into_crop(c.x, c.y, c.width, c.height) };
        stretched(self, rgb)
    }

    fn develop_crop(&self, width: usize, height: usize) -> Rect {
        let c = self.crop.clipped(width, height);
        if c.width == 0 || c.height == 0 { Rect::new(0, 0, width, height) } else { c }
    }
}

/// The cropped camera RGB with its rows stretched for non-square pixels.
fn stretched(raw: &RawImage, rgb: Rgb32f) -> Rgb32f {
    let h = stretched_height(rgb.height, raw.pixel_aspect);
    if h == rgb.height {
        return rgb;
    }
    crate::raster::resample::resize(&rgb, rgb.width, h, crate::raster::resample::Filter::Mitchell)
}

/// What a decoder of LightCraft's did to the samples and tags that [`craft_raw`] leaves to the developer, in place:
/// a DNG's (or a DNG-layout CFA TIFF's) samples stuck below the stated white raised to it
/// ([`Saturation::lift_clipped_samples`]), and for an Epson ERF or Mamiya MEF, which records no white balance, the
/// as-shot white measured from the picture ([`wb_estimate::estimate`], with the file's `bytes` for its embedded
/// preview). Call it once, after [`craft_raw::decode`] and before developing.
pub fn prepare(raw: &mut RawImage, bytes: Option<&[u8]>) {
    if matches!(raw.format, RawFormat::Dng | RawFormat::CfaTiff) {
        raw.lift_clipped_samples();
    }
    if raw.wb_multipliers.is_none()
        && matches!(raw.format, RawFormat::Erf | RawFormat::Mef)
        && let Some(bytes) = bytes
    {
        raw.wb_multipliers = wb_estimate::estimate(raw, bytes);
    }
}

/// [`craft_raw::Orientation`] as [`craft_geom::Orientation`] (the same eight Exif cases).
pub fn orientation(o: Orientation) -> craft_geom::Orientation {
    craft_geom::Orientation::from_exif(o.to_exif())
}

/// A colour-tag matrix ([`craft_raw::Matrix3`]) as a [`Mat3`].
pub fn mat3(m: craft_raw::Matrix3) -> Mat3 {
    Mat3(m)
}

#[cfg(test)]
mod tests;

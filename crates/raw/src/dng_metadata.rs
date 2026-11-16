//! Bounded DNG processing and camera metadata, before application rendering.
//!
//! This is the [`crate::dng_lines`] companion on the same `craft_tiff::classic` IFDs. The full
//! reader's `dng::color_data` works on `craft_tiff::Tiff` and returns the values it will render
//! with (raw IFD first, implausible or singular values dropped, `BaselineExposureOffset` added).
//! This reader returns the tags as written, IFD 0 first, and fails on a value outside the file,
//! so a damaged file does not silently lose its white balance. For a well-formed file both read the
//! same matrices and neutral (see `tests/it/dng_metadata.rs`).
use crate::dng_lines::{self, Error, PHOTOMETRIC_CFA, Result};
use craft_tiff::classic::{Entry, Ifd, Tiff, tags};
type M3 = [[f64; 3]; 3];
type V3 = [f64; 3];
/// Camera-plane colour filter pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub rows: usize,
    pub cols: usize,
    pub colors: Vec<u8>,
}
/// Parsed image processing tags.
#[derive(Clone, Debug, Default)]
pub struct Params {
    pub active_area: Option<[usize; 4]>,
    pub linearization: Option<Vec<u16>>,
    pub black_repeat: (usize, usize),
    pub black: Vec<f64>,
    pub black_delta_h: Vec<f64>,
    pub black_delta_v: Vec<f64>,
    pub white: Vec<f64>,
    pub crop_origin: Option<(f64, f64)>,
    pub crop_size: Option<(f64, f64)>,
    pub cfa: Option<Pattern>,
}
/// Parsed camera colour calibration.
#[derive(Clone, Debug, Default)]
pub struct Calibration {
    pub color_matrix: [Option<M3>; 2],
    pub camera_calibration: [Option<M3>; 2],
    pub forward_matrix: [Option<M3>; 2],
    pub illuminant: [Option<u64>; 2],
    pub analog_balance: Option<V3>,
    pub as_shot_neutral: Option<V3>,
    pub as_shot_white_xy: Option<(f64, f64)>,
    pub baseline_exposure: f64,
}
/// SMPTE ST 12-1 time code of a CinemaDNG frame: the crate's [`crate::Timecode`], which the full
/// reader's `CinemaTags` uses too.
pub use crate::Timecode;

/// File-level capture fields, without image geometry or rendering policy.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptureInfo {
    pub dng_version: [u8; 4],
    pub frame_rate: Option<(i64, i64)>,
    pub timecode: Option<Timecode>,
    pub camera: Option<String>,
    pub reel: Option<String>,
}
/// Metadata reader with the file's main and raw IFDs.
pub struct Metadata<'a, 'i> {
    pub tiff: Tiff<'a>,
    pub main: &'i Ifd,
    pub raw: &'i Ifd,
}
impl Metadata<'_, '_> {
    /// A raw-image tag: from the raw IFD, else IFD 0.
    fn raw_tag(&self, tag: u16) -> Option<&Entry> {
        self.raw.get(tag).or_else(|| self.main.get(tag))
    }

    /// A camera-wide tag: from IFD 0, else the raw IFD.
    fn main_tag(&self, tag: u16) -> Option<&Entry> {
        self.main.get(tag).or_else(|| self.raw.get(tag))
    }

    /// A numeric tag's values; a value outside the file (or a zero denominator) is an error, so a
    /// damaged file does not silently lose its white balance.
    fn floats(&self, e: Option<&Entry>) -> Result<Option<Vec<f64>>> {
        Ok(e.map(|e| self.tiff.floats(e)).transpose()?)
    }

    /// A 3 × 3 matrix tag (`None` when absent or not 3 × 3, e.g. for other colour counts).
    fn matrix(&self, tag: u16) -> Result<Option<M3>> {
        let Some(v) = self.floats(self.main_tag(tag))? else { return Ok(None) };
        let Ok(v) = <[f64; 9]>::try_from(v.as_slice()) else { return Ok(None) };
        Ok(Some([[v[0], v[1], v[2]], [v[3], v[4], v[5]], [v[6], v[7], v[8]]]))
    }

    fn vec3(&self, tag: u16) -> Result<Option<V3>> {
        Ok(self.floats(self.main_tag(tag))?.and_then(|v| <[f64; 3]>::try_from(v.as_slice()).ok()))
    }

    /// The CFA pattern in camera planes (`None` for LinearRaw).
    pub fn cfa(&self) -> Result<Option<Pattern>> {
        let l = dng_lines::layout(&self.tiff, self.raw)?;
        if l.photometric != PHOTOMETRIC_CFA {
            return Ok(None);
        }
        let dims = self.raw_tag(tags::CFA_REPEAT_PATTERN_DIM).map(|e| self.tiff.uints(e)).transpose()?.unwrap_or_else(|| vec![2, 2]);
        let (rows, cols) = match dims[..] {
            [r, c, ..] => (r as usize, c as usize),
            _ => return Err(Error::Malformed("CFARepeatPatternDim")),
        };
        if !(1..=8).contains(&rows) || !(1..=8).contains(&cols) {
            return Err(Error::Unsupported(format!("{rows} × {cols} CFA pattern")));
        }
        let pattern = self.tiff.uints(self.raw_tag(tags::CFA_PATTERN).ok_or(Error::Malformed("no CFAPattern"))?)?;
        if pattern.len() < rows * cols {
            return Err(Error::Malformed("CFAPattern shorter than its repeat"));
        }
        let planes = match self.raw_tag(tags::CFA_PLANE_COLOR) {
            Some(e) => self.tiff.uints(e)?,
            None => vec![0, 1, 2],
        };
        let mut sorted = planes.clone();
        sorted.sort_unstable();
        if sorted != [0, 1, 2] {
            return Err(Error::Unsupported("CFA colours other than red, green and blue".into()));
        }
        if let Some(e) = self.raw_tag(tags::CFA_LAYOUT)
            && self.tiff.uint(e)? != 1
        {
            return Err(Error::Unsupported("a non-rectangular CFA layout".into()));
        }
        let colors = pattern[..rows * cols]
            .iter()
            .map(|&v| planes.iter().position(|&p| p == v).map(|i| i as u8).ok_or(Error::Malformed("CFAPattern colour not in CFAPlaneColor")))
            .collect::<Result<Vec<u8>>>()?;
        Ok(Some(Pattern { rows, cols, colors }))
    }

    pub fn params(&self) -> Result<Params> {
        let uint_pair = |tag| -> Result<Option<(usize, usize)>> {
            match self.raw_tag(tag) {
                Some(e) => match self.tiff.uints(e)?[..] {
                    [a, b, ..] => Ok(Some((a as usize, b as usize))),
                    _ => Err(Error::Malformed("short field")),
                },
                None => Ok(None),
            }
        };
        let active_area = match self.raw_tag(tags::ACTIVE_AREA) {
            Some(e) => match self.tiff.uints(e)?[..] {
                [t, l, b, r, ..] => Some([t, l, b, r].map(|v| usize::try_from(v).unwrap_or(usize::MAX))),
                _ => return Err(Error::Malformed("ActiveArea")),
            },
            None => None,
        };
        let linearization = match self.raw_tag(tags::LINEARIZATION_TABLE) {
            Some(e) => {
                // checked before `uints` allocates a u64 per entry
                if e.count > 65_536 {
                    return Err(Error::Malformed("LinearizationTable longer than 65536"));
                }
                let v = self.tiff.uints(e)?;
                Some(v.into_iter().map(|x| x.min(65_535) as u16).collect())
            }
            None => None,
        };
        let pair = |tag| -> Result<Option<(f64, f64)>> {
            match self.raw_tag(tag) {
                Some(e) => match self.tiff.floats(e)?[..] {
                    [a, b, ..] => Ok(Some((a, b))),
                    _ => Err(Error::Malformed("short field")),
                },
                None => Ok(None),
            }
        };
        let list = |tag| -> Result<Vec<f64>> { Ok(self.raw_tag(tag).map(|e| self.tiff.floats(e)).transpose()?.unwrap_or_default()) };
        Ok(Params {
            active_area,
            linearization,
            black_repeat: uint_pair(tags::BLACK_LEVEL_REPEAT_DIM)?.unwrap_or((1, 1)),
            black: list(tags::BLACK_LEVEL)?,
            black_delta_h: list(tags::BLACK_LEVEL_DELTA_H)?,
            black_delta_v: list(tags::BLACK_LEVEL_DELTA_V)?,
            white: list(tags::WHITE_LEVEL)?,
            crop_origin: pair(tags::DEFAULT_CROP_ORIGIN)?,
            crop_size: pair(tags::DEFAULT_CROP_SIZE)?,
            cfa: self.cfa()?,
        })
    }

    fn text(&self, tag: u16) -> Option<String> {
        self.main_tag(tag).and_then(|e| self.tiff.ascii(e).ok()).filter(|s| !s.is_empty())
    }
    /// Read file-level CinemaDNG and camera fields.
    pub fn capture_info(&self) -> CaptureInfo {
        let dng_version = self
            .main_tag(tags::DNG_VERSION)
            .and_then(|e| self.tiff.uints(e).ok())
            .and_then(|v| v.get(..4).map(|v| [0, 1, 2, 3].map(|i| v[i] as u8)))
            .unwrap_or_default();
        let frame_rate = self.main_tag(tags::FRAME_RATE).and_then(|e| {
            let b = self.tiff.value_bytes(e).ok()?;
            if !matches!(e.ty, craft_tiff::classic::Type::SRational | craft_tiff::classic::Type::Rational) {
                return None;
            }
            let n = i64::from(self.tiff.u32_of(b.get(0..4)?.try_into().ok()?) as i32);
            let d = i64::from(self.tiff.u32_of(b.get(4..8)?.try_into().ok()?) as i32);
            crate::dng::plausible_frame_rate(n, d).then_some((n, d))
        });
        let timecode = self.main_tag(tags::TIME_CODES).and_then(|e| self.tiff.value_bytes(e).ok()).and_then(Timecode::from_bytes);
        let camera = self.text(tags::UNIQUE_CAMERA_MODEL).or_else(|| {
            let mm = [self.text(tags::MAKE), self.text(tags::MODEL)].into_iter().flatten().collect::<Vec<_>>().join(" ");
            (!mm.is_empty()).then_some(mm)
        });
        CaptureInfo { dng_version, frame_rate, timecode, camera, reel: self.text(tags::REEL_NAME) }
    }

    /// The colour tags.
    pub fn calibration(&self) -> Result<Calibration> {
        let illum = |tag| self.main_tag(tag).map(|e| self.tiff.uint(e)).transpose();
        let white_xy = self.floats(self.main_tag(tags::AS_SHOT_WHITE_XY))?.and_then(|v| match v[..] {
            [x, y] => Some((x, y)),
            _ => None,
        });
        Ok(Calibration {
            color_matrix: [self.matrix(tags::COLOR_MATRIX_1)?, self.matrix(tags::COLOR_MATRIX_2)?],
            camera_calibration: [self.matrix(tags::CAMERA_CALIBRATION_1)?, self.matrix(tags::CAMERA_CALIBRATION_2)?],
            forward_matrix: [self.matrix(tags::FORWARD_MATRIX_1)?, self.matrix(tags::FORWARD_MATRIX_2)?],
            illuminant: [illum(tags::CALIBRATION_ILLUMINANT_1)?, illum(tags::CALIBRATION_ILLUMINANT_2)?],
            analog_balance: self.vec3(tags::ANALOG_BALANCE)?,
            as_shot_neutral: self.vec3(tags::AS_SHOT_NEUTRAL)?,
            as_shot_white_xy: white_xy,
            baseline_exposure: self.floats(self.main_tag(tags::BASELINE_EXPOSURE))?.and_then(|v| v.first().copied()).unwrap_or(0.0),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecode_is_the_crate_timecode() {
        let tc = Timecode { frames: 12, seconds: 34, minutes: 12, hours: 9, drop_frame: true };
        assert_eq!(Timecode::from_bytes(&[0x52, 0x34, 0x12, 0x09]), Some(tc));
        let shared: crate::Timecode = tc;
        assert_eq!(Timecode::from_bytes(&shared.to_bytes()), Some(tc));
    }

    #[test]
    fn calibration_defaults_and_rejects_bad_rational() -> Result<()> {
        let empty = Ifd::default();
        let tiff = Tiff { data: &[], big_endian: false };
        let metadata = Metadata { tiff, main: &empty, raw: &empty };
        let calibration = metadata.calibration()?;
        assert_eq!(calibration.baseline_exposure, 0.0);
        assert_eq!(calibration.color_matrix, [None, None]);

        // IFD with one BaselineExposure SRational pointing past the end of the file.
        let data = [b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 42, 198, 10, 0, 1, 0, 0, 0, 200, 0, 0, 0, 0, 0, 0, 0];
        let (tiff, first) = Tiff::new(&data)?;
        let (ifd, _) = tiff.read_ifd(first, 0)?;
        assert!(Metadata { tiff, main: &ifd, raw: &ifd }.calibration().is_err());
        Ok(())
    }
}

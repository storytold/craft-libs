//! DNG camera-profile look data carried by the file itself (Adobe DNG Specification 1.6/1.7,
//! chapter 6 and the `ProfileHueSatMap*`, `ProfileLookTable*`, `ProfileToneCurve` and
//! `ProfileGainTableMap*` tags), as data: this module checks and holds the tables, the develop
//! pipeline applies them.
//!
//! - A hue/saturation/value table holds, for a grid of (value, hue, saturation) inputs, a hue shift
//!   in degrees, a saturation scale and a value scale (value-major, hue-middle, saturation-minor).
//!   Hue samples are spread evenly around the circle with wrap-around, saturation and value samples
//!   evenly over 0..=1; a dimension with one division is constant.
//! - Tables are defined in HSV of linear ProPhoto RGB (D50). With encoding 1 ("sRGB") the value
//!   axis is indexed through the sRGB transfer curve (3D tables only).
//! - `ProfileHueSatMapData1/2` follow the colour matrices (interpolated by the same illuminant
//!   weight); `ProfileLookTableData` comes after exposure compensation and before any tone curve.
//! - `ProfileToneCurve`: (input, output) pairs in linear gamma from (0, 0) to (1, 1).
//! - `ProfileGainTableMap*`: see [`crate::gaintable`].
//!
//! The data is read from the user's own file at run time; nothing here ships profile data.

use crate::gaintable::GainTableMap;
use serde::{Deserialize, Serialize};

/// Upper bound on table entries (Adobe profiles use ≤ 90 × 30 × 16; guards header-driven sizes).
const MAX_ENTRIES: usize = 1 << 20;

/// A DNG hue/saturation/value mapping table (`ProfileHueSatMapData*` / `ProfileLookTableData`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HsvTable {
    pub hue_divisions: usize,
    pub sat_divisions: usize,
    pub val_divisions: usize,
    /// `(hue shift °, saturation scale, value scale)`, value-major, hue-middle, saturation-minor.
    pub data: Vec<[f32; 3]>,
    /// The value axis is indexed through the sRGB encoding curve (`…Encoding` = 1).
    pub srgb_value: bool,
}

impl HsvTable {
    /// A table from its `…Dims` (hue, saturation, value divisions), data floats and encoding.
    /// `None` when the dimensions are out of range, the data length doesn't match or a value is
    /// not finite.
    pub fn from_tags(dims: &[u64], data: &[f64], encoding: u64) -> Option<HsvTable> {
        let [h, s, v] = <[u64; 3]>::try_from(dims).ok()?;
        let (h, s, v) = (usize::try_from(h).ok()?, usize::try_from(s).ok()?, usize::try_from(v).ok()?);
        if h < 1 || s < 2 || v < 1 || h > 4096 || s > 4096 || v > 4096 {
            return None;
        }
        let n = h.checked_mul(s)?.checked_mul(v)?;
        if n > MAX_ENTRIES || data.len() != n * 3 || data.iter().any(|x| !x.is_finite()) {
            return None;
        }
        let data = data.as_chunks::<3>().0.iter().map(|c| [c[0] as f32, (c[1] as f32).max(0.0), (c[2] as f32).max(0.0)]).collect();
        Some(HsvTable { hue_divisions: h, sat_divisions: s, val_divisions: v, data, srgb_value: encoding == 1 && v > 1 })
    }
}

/// A DNG tone curve (`ProfileToneCurve`): linear-gamma (input, output) pairs, increasing input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToneCurve {
    pub points: Vec<[f32; 2]>,
}

impl ToneCurve {
    /// From the tag's flat float pairs. `None` unless there are 2..=65536 finite pairs within
    /// 0..=1 with strictly increasing inputs.
    pub fn from_tag(v: &[f64]) -> Option<ToneCurve> {
        if !v.len().is_multiple_of(2) || v.len() < 4 || v.len() > 131_072 {
            return None;
        }
        let points: Vec<[f32; 2]> = v.as_chunks::<2>().0.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
        let ok = points.iter().all(|p| p.iter().all(|x| x.is_finite() && (0.0..=1.0).contains(x))) && points.windows(2).all(|w| w[1][0] > w[0][0]);
        ok.then_some(ToneCurve { points })
    }
}

/// The DNG profile look tags of a file (all optional).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProfileLook {
    /// `ProfileHueSatMapData1/2` (same shape, `ProfileHueSatMapDims`).
    pub hue_sat_map: [Option<HsvTable>; 2],
    /// `ProfileLookTableData`.
    pub look_table: Option<HsvTable>,
    /// `ProfileToneCurve`.
    pub tone_curve: Option<ToneCurve>,
    /// `ProfileGainTableMap2`, else `ProfileGainTableMap`.
    #[serde(default)]
    pub gain_table_map: Option<GainTableMap>,
}

impl ProfileLook {
    pub fn is_empty(&self) -> bool {
        self.hue_sat_map.iter().all(Option::is_none) && self.look_table.is_none() && self.tone_curve.is_none() && self.gain_table_map.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tags_and_rejects_bad_shapes() {
        let data: Vec<f64> = (0..4).flat_map(|_| [0.0, 1.0, 1.0]).collect();
        assert!(HsvTable::from_tags(&[2, 2, 1], &data, 0).is_some());
        assert!(HsvTable::from_tags(&[2, 2], &data, 0).is_none());
        assert!(HsvTable::from_tags(&[2, 1, 2], &data, 0).is_none(), "saturation needs ≥ 2 divisions");
        assert!(HsvTable::from_tags(&[2, 2, 2], &data, 0).is_none(), "length mismatch");
        assert!(HsvTable::from_tags(&[u64::MAX, 2, 2], &data, 0).is_none());
        let mut nan = data.clone();
        nan[1] = f64::NAN;
        assert!(HsvTable::from_tags(&[2, 2, 1], &nan, 0).is_none());
        // the sRGB encoding only applies to 3D tables
        assert!(!HsvTable::from_tags(&[2, 2, 1], &data, 1).unwrap().srgb_value);
    }

    #[test]
    fn tone_curve_needs_increasing_pairs_within_the_unit_range() {
        assert!(ToneCurve::from_tag(&[0.0, 0.0]).is_none());
        assert!(ToneCurve::from_tag(&[0.0, 0.0, 0.5, 0.6, 0.4, 0.7, 1.0, 1.0]).is_none(), "inputs must increase");
        assert!(ToneCurve::from_tag(&[0.0, 0.0, 1.0, 1.5]).is_none());
        assert!(ToneCurve::from_tag(&[0.0, 0.0, 0.5, f64::NAN, 1.0, 1.0]).is_none());
        let c = ToneCurve::from_tag(&[0.0, 0.0, 0.25, 0.4, 0.5, 0.7, 1.0, 1.0]);
        assert_eq!(c.map(|c| c.points.len()), Some(4));
    }
}

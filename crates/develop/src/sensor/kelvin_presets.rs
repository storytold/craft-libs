//! A Kelvin scale from the camera's own white-balance presets, for raws whose camera has no colour matrices.
//!
//! Sony ARWs from the DSLR-A200 on store, in their encrypted `SR2SubIFD` (see `vendor::sr2`), the multipliers the
//! camera would apply for each of its white-balance presets, among them five at a stated colour temperature:
//! `WB_RGBLevels2500K` (`0x782d`), `3200K` (`0x782c`), `4500K` (`0x7824`), `6000K` (`0x782b`) and `8500K` (`0x782a`;
//! tag numbers from `exiftool -v3`, run as a black-box tool on the files). They are Sony's own reading of its sensor:
//! the as-shot multipliers placed on the curve through them give the temperature Sony's Kelvin dial names for that
//! white. That is all a Kelvin slider needs; the look stays the file's JPEG fit, and the picture is adapted from the
//! as-shot white with Bradford, as for a camera read through measured matrices (`crate::sensor::spectral`, issue storytold/lightcraft#730).
//!
//! Canon CR2s list theirs by name in the maker note's `ColorData`, each with Canon's temperature for it (see
//! `vendor::cr2`): five planckian ones (Daylight, Shade, Cloudy, Tungsten, Flash) make the curve the same way,
//! taken in Kelvin order ([`KelvinPresets::from_unordered`]). On the PowerShot G1 X Mark III's nine settings the
//! curve reads the as-shot levels within 5 % of the camera's own `ColorTempAsShot`.
//!
//! The model, in `(ln R/G, ln B/G)` of the multipliers ([`KelvinPresets::temp_tint`]):
//! - The presets (three or more) are joined piecewise linearly, with mired linear along each segment and the end segments
//!   extended. On every Sony body measured, mired is nearly linear in arc length along the whole curve (2500 K to
//!   8500 K within ±3 %), so the segments add no visible kinks.
//! - A white off the curve is placed by its perpendicular offset from the nearest segment. Tint is that offset scaled
//!   by the segment's own spacing of the Planckian locus in CIE 1960 (u, v), per unit arc length.
//! - The isotherms are not perpendicular to the curve in these coordinates: an offset also changes mired by
//!   [`ISOTHERM_TILT`] times the mired the same distance along the curve would.
//!
//! Fujifilm X-Trans III bodies (X-T20, X-T2, X-Pro2, X-E3, X-H1, X100F) store no such presets but the camera's own
//! neutral multipliers for Illuminant A and D65 (`0xf00c`, [`KelvinPresets::from_neutral_pair`], tag number from
//! a black-box `exiftool -v3`). The pair gives a two-point line in the same coordinates; mired is linear along it,
//! isotherms are perpendicular to it, and tint is scaled by the CIE locus spacing. No other camera's data is used.
//!
//! The two constants come from the 14 Sony bodies that have both measured matrices and the presets (one CC0
//! raw.pixls.us sample each for the presets, a grid of multipliers through `crate::sensor::spectral` for the matrices):
//! the tilt measured 0.59–0.84 (mean 0.74), the tint spacing 0.90–1.04 of the locus's (mean 0.98, taken as 1).
//! On 150 ILCE-7SM2 photos read both ways, the temperatures agree to ±4 mired around a constant offset (Sony's
//! Kelvin is about 8 % above the matrices', as storytold/lightcraft#730 found for the ILCE-7M3's dial) and the tint to −9 ± 4: the
//! matrices put Sony's presets slightly off the locus, the presets themselves are tint 0 here.
use crate::color::{Xy, cct};
use crate::sensor::ColorData;
use serde::{Deserialize, Serialize};

/// Mired change per unit offset from the preset curve, in units of the mired change per unit distance along it.
pub const ISOTHERM_TILT: f64 = 0.74;

/// The slider's range: an as-shot reading is kept inside it.
const MIN_K: f64 = 2000.0;
const MAX_K: f64 = 50000.0;
const MAX_TINT: f64 = 150.0;

/// Models whose white balance is edited in Kelvin through their presets, as (brand, Exif model without the brand),
/// each checked to carry them: the Sony bodies on 80–150 own photos each, or the public corpus samples for the
/// ILCE-3500 and ILCE-7S; the Canon PowerShot G1 X Mark III on a reporter's nine photos, one per white-balance
/// setting (issue emmertex/lightcraft-classic#157, a downstream fork). A camera with measured matrices (`crate::sensor::spectral`) is read through
/// those instead.
const MODELS: &[(&str, &str)] = &[
    ("SONY", "DSC-RX10"),
    ("SONY", "ILCA-77M2"),
    ("SONY", "ILCA-99M2"),
    ("SONY", "ILCE-3500"),
    ("SONY", "ILCE-7S"),
    ("SONY", "SLT-A33"),
    ("CANON", "PowerShot G1 X Mark III"),
];

/// Fujifilm's X-Trans III bodies, whose raws carry the camera's own neutral multipliers for Illuminant A and D65
/// (`vendor::raf`, tag `0xf00c`; see [`KelvinPresets::from_neutral_pair`]): checked on the files of each (X-T20,
/// X-T2, X-Pro2, X-E3, X-H1, X100F), the Exif model as Fujifilm writes it.
const FUJI_MODELS: &[&str] = &["X-T20", "X-T2", "X-Pro2", "X-E3", "X-H1", "X100F"];

/// Whether a raw of this Exif make and model is edited in Kelvin through its presets (when it has no colour
/// matrices of its own or measured: see [`crate::sensor::spectral::find`], which comes first), from the file's presets or
/// built-in ones ([`crate::sensor::kelvin_builtin`]).
pub fn covers(make: Option<&str>, model: &str) -> bool {
    if crate::sensor::kelvin_builtin::covers(make, model) {
        return true;
    }
    let model = model.trim();
    let brand = make.and_then(|m| m.split_whitespace().next());
    let bare_of = |b: &str| {
        model
            .get(..b.len())
            .zip(model.get(b.len()..))
            .filter(|(p, rest)| p.eq_ignore_ascii_case(b) && rest.starts_with(' '))
            .map_or(model, |(_, rest)| rest.trim_start())
    };
    let listed = MODELS.iter().any(|(b, m)| brand.is_none_or(|x| x.eq_ignore_ascii_case(b)) && bare_of(b).eq_ignore_ascii_case(m));
    // A DxO demosaiced DNG keeps the Fujifilm model but carries DxO as its make.
    let fuji = brand.is_none_or(|b| b.eq_ignore_ascii_case("FUJIFILM") || b.eq_ignore_ascii_case("DxO"))
        && FUJI_MODELS.iter().any(|m| bare_of("FUJIFILM").eq_ignore_ascii_case(m));
    listed || fuji
}

/// [`covers`] for make and model joined by a space, as the catalog keeps them (`"SONY ILCA-99M2"`).
pub fn covers_camera(camera: &str) -> bool {
    let words: Vec<&str> = camera.split_whitespace().collect();
    // the make can be several words ("NIKON CORPORATION"), so try every split, as `spectral::find_camera` does
    (1..words.len()).any(|i| covers(Some(&words[..i].join(" ")), &words[i..].join(" "))) || covers(None, camera)
}

/// Most points a curve keeps (Canon writes about seven at a stated temperature, Sony five).
const MAX_POINTS: usize = 32;

/// A camera's multipliers at stated temperatures (at least three), ascending, green = 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KelvinPresets {
    points: Vec<(f32, [f32; 3])>,
    /// The camera's neutral multipliers for Illuminant A and D65 when the presets come from such a pair
    /// ([`Self::from_neutral_pair`]); the points are then the line between them at five temperatures.
    #[serde(default)]
    pair: Option<[[f32; 3]; 2]>,
}

/// Illuminant A and D65 as chromaticities (CIE).
const A_XY: Xy = Xy::new(0.44757, 0.40745);
const D65_XY: Xy = Xy::new(0.31271, 0.32902);
const POINT_KELVIN: [f32; 5] = [2500.0, 3200.0, 4500.0, 6000.0, 8500.0];

fn usable(m: [f32; 3]) -> bool {
    m.iter().all(|v| *v > 0.0 && v.is_finite())
}

fn log_point(m: [f32; 3]) -> [f64; 2] {
    [(m[0] as f64 / m[1] as f64).ln(), (m[2] as f64 / m[1] as f64).ln()]
}

fn locus_uv(t: f64) -> (f64, f64) {
    cct::xy_to_uv(cct::temp_tint_to_xy(t, 0.0))
}

impl KelvinPresets {
    /// `points` as (Kelvin, multipliers), ascending in Kelvin; `None` unless they make a curve: three to
    /// [`MAX_POINTS`] of them, positive multipliers, red rising and blue falling from one preset to the next.
    pub fn new(points: impl IntoIterator<Item = (f32, [f32; 3])>) -> Option<KelvinPresets> {
        let points: Vec<_> = points.into_iter().take(MAX_POINTS + 1).collect();
        let ok = (3..=MAX_POINTS).contains(&points.len())
            && points.iter().all(|(k, m)| *k > 0.0 && k.is_finite() && usable(*m))
            && points.windows(2).all(|w| {
                let (a, b) = (log_point(w[0].1), log_point(w[1].1));
                w[0].0 < w[1].0 && b[0] > a[0] && b[1] < a[1]
            });
        ok.then(|| KelvinPresets { points: points.into_iter().map(|(k, m)| (k, [m[0] / m[1], 1.0, m[2] / m[1]])).collect(), pair: None })
    }

    /// A curve from presets in any order, as a camera lists them by name (Canon's Daylight, Shade, Cloudy, …):
    /// sorted by Kelvin, a repeated temperature kept once, unusable points and any that would turn the curve back
    /// (red not rising or blue not falling from the warmer neighbour kept) dropped; then [`Self::new`].
    pub fn from_unordered(points: impl IntoIterator<Item = (f32, [f32; 3])>) -> Option<KelvinPresets> {
        let mut pts: Vec<_> = points.into_iter().take(MAX_POINTS).filter(|(k, m)| *k > 0.0 && k.is_finite() && usable(*m)).collect();
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut kept: Vec<(f32, [f32; 3])> = Vec::with_capacity(pts.len());
        for p in pts {
            let fits = kept.last().is_none_or(|q| {
                let (a, b) = (log_point(q.1), log_point(p.1));
                q.0 < p.0 && b[0] > a[0] && b[1] < a[1]
            });
            if fits {
                kept.push(p);
            }
        }
        KelvinPresets::new(kept)
    }

    /// Presets from a camera's own neutral multipliers (red, green, blue, green = 1) for Illuminant A and D65, as
    /// Fujifilm stores them in `WB_GRBLevelsStandard`. Two whites fix the camera's response only on the line
    /// between them, and nothing but the colorimetry of the two lights is used to place a white off it: in
    /// `(ln R/G, ln B/G)` mired is linear along the line (extended past both ends), the isotherms are taken
    /// perpendicular to it (no tilt), and the tint is the Planckian locus's own CIE 1960 spacing per unit length
    /// plus the tint the two lights themselves have (A is on the locus, D65 is about +9.5 off it). No other camera's
    /// data is involved. `None` for unusable multipliers or two that make no curve.
    pub fn from_neutral_pair(a: [f32; 3], d65: [f32; 3]) -> Option<KelvinPresets> {
        if !usable(a) || !usable(d65) {
            return None;
        }
        let (a, d65) = ([a[0] / a[1], 1.0, a[2] / a[1]], [d65[0] / d65[1], 1.0, d65[2] / d65[1]]);
        let (pa, pd) = (log_point(a), log_point(d65));
        let (ma, md) = (1e6 / cct::xy_to_temp_tint(A_XY).0, 1e6 / cct::xy_to_temp_tint(D65_XY).0);
        let points = POINT_KELVIN.map(|k| {
            let f = (1e6 / k as f64 - ma) / (md - ma);
            let q = [pa[0] + f * (pd[0] - pa[0]), pa[1] + f * (pd[1] - pa[1])];
            (k, [q[0].exp() as f32, 1.0, q[1].exp() as f32])
        });
        let mut out = KelvinPresets::new(points)?;
        out.pair = Some([a, d65]);
        Some(out)
    }

    /// [`Self::temp_tint`] for a neutral pair (see [`Self::from_neutral_pair`]).
    fn pair_temp_tint(pair: [[f32; 3]; 2], q: [f64; 2]) -> Option<(f64, f64)> {
        let (pa, pd) = (log_point(pair[0]), log_point(pair[1]));
        let d = [pd[0] - pa[0], pd[1] - pa[1]];
        let len = d[0].hypot(d[1]);
        // only a deserialized pair can repeat a point
        if !(len > 0.0 && len.is_finite()) {
            return None;
        }
        let t = [d[0] / len, d[1] / len];
        let n = if t[0] - t[1] < 0.0 { [t[1], -t[0]] } else { [-t[1], t[0]] };
        let r = [q[0] - pa[0], q[1] - pa[1]];
        let along = (r[0] * t[0] + r[1] * t[1]) / len;
        let off = r[0] * n[0] + r[1] * n[1];
        let ((ta, tint_a), (td, tint_d)) = (cct::xy_to_temp_tint(A_XY), cct::xy_to_temp_tint(D65_XY));
        let ((u0, v0), (u1, v1)) = (locus_uv(ta), locus_uv(td));
        let tint_per_len = cct::TINT_SCALE * (u1 - u0).hypot(v1 - v0) / len;
        let mired = 1e6 / ta + along * (1e6 / td - 1e6 / ta);
        Some((1e6 / mired.clamp(1e6 / MAX_K, 1e6 / MIN_K), tint_a + along * (tint_d - tint_a) + tint_per_len * off))
    }

    /// Temperature (K) and tint of the white these multipliers balance (see the module docs); `None` for unusable
    /// multipliers.
    pub fn temp_tint(&self, wb_multipliers: [f32; 3]) -> Option<(f64, f64)> {
        if !usable(wb_multipliers) {
            return None;
        }
        let q = log_point(wb_multipliers);
        if let Some(pair) = self.pair {
            let (t, tint) = Self::pair_temp_tint(pair, q)?;
            return Some((t, tint.clamp(-MAX_TINT, MAX_TINT)));
        }
        let last = self.points.len().saturating_sub(2);
        let mut best: Option<(f64, f64, f64)> = None;
        for (i, w) in self.points.windows(2).enumerate() {
            let ((k0, m0), (k1, m1)) = (w[0], w[1]);
            let (p0, p1) = (log_point(m0), log_point(m1));
            let d = [p1[0] - p0[0], p1[1] - p0[1]];
            let len = d[0].hypot(d[1]);
            // only a deserialized curve can repeat a point
            if !(len > 0.0 && len.is_finite()) {
                continue;
            }
            let t = [d[0] / len, d[1] / len];
            // the normal towards higher red and blue: a green white
            let n = if t[0] - t[1] < 0.0 { [t[1], -t[0]] } else { [-t[1], t[0]] };
            let r = [q[0] - p0[0], q[1] - p0[1]];
            let along = (r[0] * t[0] + r[1] * t[1]) / len;
            let off = r[0] * n[0] + r[1] * n[1];
            let lo = if i == 0 { f64::NEG_INFINITY } else { 0.0 };
            let hi = if i == last { f64::INFINITY } else { 1.0 };
            let c = along.clamp(lo, hi);
            let dist = (r[0] - c * d[0]).hypot(r[1] - c * d[1]);
            let (mired0, mired1) = (1e6 / k0 as f64, 1e6 / k1 as f64);
            let mired_per_len = (mired1 - mired0) / len;
            let ((u0, v0), (u1, v1)) = (locus_uv(k0 as f64), locus_uv(k1 as f64));
            let tint_per_len = cct::TINT_SCALE * (u1 - u0).hypot(v1 - v0) / len;
            let mired = mired0 + along * (mired1 - mired0) + ISOTHERM_TILT * mired_per_len * off;
            if best.is_none_or(|b| dist < b.0) {
                best = Some((dist, mired, tint_per_len * off));
            }
        }
        let (_, mired, tint) = best?;
        let t = 1e6 / mired.clamp(1e6 / MAX_K, 1e6 / MIN_K);
        Some((t, tint.clamp(-MAX_TINT, MAX_TINT)))
    }

    /// The scale of the presets a file records ([`craft_raw::kelvin::KelvinPresets`]): its points, or the line
    /// through its Illuminant A / D65 pair placed at five temperatures ([`Self::from_neutral_pair`]).
    pub fn from_file(p: &craft_raw::kelvin::KelvinPresets) -> Option<KelvinPresets> {
        match p.neutral_pair() {
            Some([a, d65]) => KelvinPresets::from_neutral_pair(a, d65),
            None => KelvinPresets::new(p.points().iter().copied()),
        }
    }

    /// The as-shot white of a file through its presets: [`Self::temp_tint`] of its multipliers as a chromaticity.
    pub fn as_shot_white(&self, wb_multipliers: Option<[f32; 3]>) -> Option<Xy> {
        let (t, tint) = self.temp_tint(wb_multipliers?)?;
        Some(cct::temp_tint_to_xy(t, tint))
    }
}

/// The as-shot white of a file through its own presets ([`ColorData::kelvin_presets`]); `None` without presets or
/// usable multipliers.
pub fn as_shot_white(color: &ColorData, wb_multipliers: Option<[f32; 3]>) -> Option<Xy> {
    KelvinPresets::from_file(color.kelvin_presets.as_ref()?)?.as_shot_white(wb_multipliers)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ILCA-99M2's presets (every file of the body carries the same).
    fn a99m2() -> KelvinPresets {
        let g = |r: f32, b: f32| [r / 1024.0, 1.0, b / 1024.0];
        KelvinPresets::new([
            (2500.0, g(1336.0, 3672.0)),
            (3200.0, g(1720.0, 2620.0)),
            (4500.0, g(2256.0, 1880.0)),
            (6000.0, g(2680.0, 1544.0)),
            (8500.0, g(3136.0, 1300.0)),
        ])
        .unwrap()
    }

    #[test]
    fn a_preset_reads_as_its_own_temperature_with_no_tint() {
        let p = a99m2();
        for &(k, m) in &p.points {
            let (t, tint) = p.temp_tint(m).unwrap();
            assert!((t - k as f64).abs() < 0.5 && tint.abs() < 1e-6, "{k}: {t} {tint}");
            // the scale of the multipliers doesn't matter, only their ratios to green
            let (t2, tint2) = p.temp_tint(m.map(|v| v * 2.3)).unwrap();
            assert!((t2 - t).abs() < 1e-3 && (tint2 - tint).abs() < 1e-4, "{t2} {tint2}");
        }
    }

    #[test]
    fn between_presets_the_temperature_follows_the_curve() {
        let p = a99m2();
        let mid = |a: [f32; 3], b: [f32; 3]| [(a[0] * b[0]).sqrt(), 1.0, (a[2] * b[2]).sqrt()];
        let (t, tint) = p.temp_tint(mid(p.points[2].1, p.points[3].1)).unwrap();
        assert!((4500.0..6000.0).contains(&t) && tint.abs() < 1.0, "{t} {tint}");
        // beyond the ends: extended, then held to the slider's range
        let (hot, _) = p.temp_tint([3.5, 1.0, 1.15]).unwrap();
        let (cold, _) = p.temp_tint([1.15, 1.0, 4.0]).unwrap();
        let (coldest, _) = p.temp_tint([0.3, 1.0, 20.0]).unwrap();
        assert!((8500.0..MAX_K).contains(&hot) && (MIN_K..2500.0).contains(&cold) && coldest == MIN_K, "{hot} {cold} {coldest}");
    }

    #[test]
    fn less_green_in_the_multipliers_is_a_green_white_and_a_magenta_correction() {
        let p = a99m2();
        let day = [2680.0 / 1024.0, 1.0, 1544.0 / 1024.0];
        let (t0, _) = p.temp_tint(day).unwrap();
        // a green light: the camera raises red and blue against green
        let (t1, tint1) = p.temp_tint([day[0] * 1.08, 1.0, day[2] * 1.08]).unwrap();
        assert!(tint1 > 15.0 && tint1 < 40.0, "{tint1}");
        // the tilted isotherm: a higher temperature than the curve point it sits beside
        assert!(t1 > t0 + 100.0, "{t1} {t0}");
        let (_, magenta) = p.temp_tint([day[0] / 1.08, 1.0, day[2] / 1.08]).unwrap();
        assert!(magenta < -15.0, "{magenta}");
    }

    #[test]
    fn unusable_presets_or_multipliers_give_nothing() {
        let p = a99m2();
        assert_eq!(p.temp_tint([0.0, 1.0, 1.0]), None);
        assert_eq!(p.temp_tint([f32::NAN, 1.0, 1.0]), None);
        assert_eq!(p.as_shot_white(None), None);
        let mut pts = p.points.clone();
        pts.swap(1, 2);
        assert_eq!(KelvinPresets::new(pts), None, "out of order");
        let mut pts = p.points.clone();
        pts[4].1 = pts[3].1;
        assert_eq!(KelvinPresets::new(pts), None, "no step");
        let mut pts = p.points.clone();
        pts[0].1[0] = 0.0;
        assert_eq!(KelvinPresets::new(pts), None);
        assert_eq!(as_shot_white(&ColorData::default(), Some([2.0, 1.0, 1.5])), None);
        assert_eq!(KelvinPresets::new(p.points[..2].to_vec()), None, "too few");
        let with = ColorData { kelvin_presets: craft_raw::kelvin::KelvinPresets::new(p.points.clone()), ..Default::default() };
        assert!(as_shot_white(&with, Some([2.0, 1.0, 1.5])).is_some());
    }

    /// Canon lists its presets by name, not by temperature, with repeats and an off-curve one among them: the
    /// curve takes them in Kelvin order, each temperature once, and leaves out a point that would turn it back.
    #[test]
    fn a_curve_from_presets_in_any_order() {
        let g = |r: f32, b: f32| [r / 1024.0, 1.0, b / 1024.0];
        let p = KelvinPresets::from_unordered([
            (5651.0, g(1418.0, 1357.0)),
            (7505.0, g(1653.0, 1171.0)),
            (6594.0, g(1533.0, 1259.0)),
            (3174.0, g(963.0, 2016.0)),
            (5651.0, g(1418.0, 1357.0)),
            (8000.0, g(1400.0, 1400.0)),
            (8529.0, g(1807.0, 1080.0)),
            (f32::NAN, g(1000.0, 1000.0)),
        ])
        .unwrap();
        assert_eq!(p.points.len(), 5);
        assert!(p.points.windows(2).all(|w| w[0].0 < w[1].0));
        let (t, tint) = p.temp_tint(g(1653.0, 1171.0)).unwrap();
        assert!((t - 7505.0).abs() < 1.0 && tint.abs() < 0.1, "{t} {tint}");
        assert_eq!(KelvinPresets::from_unordered([(5000.0, g(1400.0, 1400.0)), (6000.0, g(1500.0, 1300.0))]), None);
        assert_eq!(KelvinPresets::from_unordered(std::iter::empty()), None);
        // a stored curve with a repeated point (only deserializing can make one) reads without dividing by zero
        let bad = KelvinPresets { points: vec![(5000.0, g(1400.0, 1400.0)); 3], pair: None };
        assert_eq!(bad.temp_tint(g(1400.0, 1400.0)), None);
        assert_eq!(KelvinPresets { points: vec![], pair: None }.temp_tint(g(1400.0, 1400.0)), None);
        // and so does a stored neutral pair with equal endpoints
        let same = KelvinPresets { points: vec![], pair: Some([[1.0, 1.0, 1.0]; 2]) };
        assert_eq!(same.temp_tint(g(1400.0, 1400.0)), None);
    }

    /// An X-T20's stored pair (`302 366 866 17 302 646 498 21`).
    fn xt20() -> (KelvinPresets, [f32; 3], [f32; 3]) {
        let (a, d) = ([366.0 / 302.0, 1.0, 866.0 / 302.0], [646.0 / 302.0, 1.0, 498.0 / 302.0]);
        (KelvinPresets::from_neutral_pair(a, d).unwrap(), a, d)
    }

    #[test]
    fn a_neutral_pair_reads_back_as_its_own_illuminants() {
        let (p, a, d) = xt20();
        let (ta, tint_a) = p.temp_tint(a).unwrap();
        let (td, tint_d) = p.temp_tint(d).unwrap();
        let (want_t, want_tint) = cct::xy_to_temp_tint(D65_XY);
        assert!((ta - cct::xy_to_temp_tint(A_XY).0).abs() < 2.0 && (tint_a - cct::xy_to_temp_tint(A_XY).1).abs() < 0.5, "{ta} {tint_a}");
        assert!((td - want_t).abs() < 2.0 && (tint_d - want_tint).abs() < 0.5, "{td} {tint_d}");
        // the points are the same reading at their temperatures, on the locus
        for (k, m) in p.points.clone() {
            let (t, tint) = p.temp_tint(m).unwrap();
            assert!((t - k as f64).abs() < 2.0 && tint.abs() < 12.0, "{k}: {t} {tint}");
        }
        // greener light than the daylight pair: positive tint, as for a matrix camera
        let (_, green) = p.temp_tint([d[0] * 1.06, 1.0, d[2] * 1.06]).unwrap();
        let (_, magenta) = p.temp_tint([d[0] / 1.06, 1.0, d[2] / 1.06]).unwrap();
        assert!(green > magenta + 15.0, "{green} {magenta}");
    }

    #[test]
    fn unusable_pairs_give_nothing() {
        assert_eq!(KelvinPresets::from_neutral_pair([0.0, 1.0, 1.0], [2.0, 1.0, 1.5]), None);
    }

    #[test]
    fn fujifilm_x_trans_iii_models_are_covered() {
        for c in ["FUJIFILM X-T20", "Fujifilm X-T2", "FUJIFILM X100F", "FUJIFILM X-H1", "FUJIFILM X-Pro2", "FUJIFILM X-E3"] {
            assert!(covers_camera(c), "{c}");
        }
        for c in ["FUJIFILM X-T3", "FUJIFILM X-T30", "SONY X-T20", "FUJIFILM GFX 100"] {
            assert!(!covers_camera(c), "{c}");
        }
    }

    #[test]
    fn dxo_dng_fujifilm_model_is_covered_for_kelvin() {
        // A DxO demosaiced DNG carries DxO as its make, not FUJIFILM, but it is still a Fujifilm model.
        assert!(covers(Some("DxO"), "X-T2"));
        assert!(covers(Some("DxO"), "FUJIFILM X-T2"));
        assert!(covers_camera("DxO X-T2"));
    }

    #[test]
    fn covered_models() {
        assert!(covers_camera("NIKON CORPORATION NIKON Z5_2") && covers_camera("NIKON Z5_2") && !covers_camera("NIKON CORPORATION NIKON Z6"));
        for c in [
            "SONY ILCA-99M2",
            "SONY DSC-RX10",
            "SONY ILCA-77M2",
            "SONY SLT-A33",
            "ILCA-99M2",
            "Sony ilca-99m2",
            "Canon Canon PowerShot G1 X Mark III",
            "Canon PowerShot G1 X Mark III",
        ] {
            assert!(covers_camera(c), "{c}");
        }
        for c in
            ["SONY ILCE-7M3", "SONY DSC-RX10M4", "Canon SLT-A33", "", "SONY", "Canon Canon PowerShot G1 X Mark II", "SONY PowerShot G1 X Mark III"]
        {
            assert!(!covers_camera(c), "{c}");
        }
        assert!(covers(Some("SONY"), "ILCA-99M2") && covers(None, "ILCA-99M2") && !covers(Some("NIKON CORPORATION"), "ILCA-99M2"));
    }
}

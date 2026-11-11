//! Judges separated plates against a scene's `expect` block.
//!
//! An area point is the median of the 5 × 5 pixels around it on each plate. A text point is the
//! median over the glyph's pixels: those in its region with at least half the darkest ink on
//! the glyph plate (Black unless the point names another). Plates a point doesn't list expect 0.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::scene::{Level, PROCESS, PointKind, SamplePoint, Scene, TEXT_PLATE};
use crate::tiff::Plate;

/// Below this much ink on the glyph plate a text region counts as empty.
const TEXT_MIN_INK: f32 = 20.0;

/// Outcome of judging one sample point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    #[allow(missing_docs)]
    Pass,
    #[allow(missing_docs)]
    Fail,
    /// Measured, not judged.
    Info,
    /// Not measured: the scene's text was not drawn (no font).
    Skipped,
}

/// The measurement and verdict for one [`SamplePoint`].
#[derive(Debug, Clone, Serialize)]
pub struct PointResult {
    #[allow(missing_docs)]
    pub at: [f32; 2],
    #[allow(missing_docs)]
    pub kind: PointKind,
    #[allow(missing_docs)]
    pub note: Option<String>,
    /// Expected ink per plate name.
    pub expected: BTreeMap<String, Level>,
    /// Allowed difference in ink percentage points.
    pub tol: f32,
    /// Ink percentage per plate, rounded to 0.1; every expected and every separated plate.
    pub measured: BTreeMap<String, f32>,
    #[allow(missing_docs)]
    pub verdict: Verdict,
    /// Plates outside tolerance.
    pub off: Vec<String>,
    /// How the value was found, or why it wasn't.
    pub how: Option<String>,
}

/// The result of judging every sample point of one scene against its separated plates.
#[derive(Debug, Clone, Serialize)]
pub struct Judgement {
    /// The plates the separation produced.
    pub plates: Vec<String>,
    /// Expected plates the separation lacks.
    pub missing_plates: Vec<String>,
    /// Plates the separation produced that the scene doesn't expect.
    pub extra_plates: Vec<String>,
    #[allow(missing_docs)]
    pub points: Vec<PointResult>,
}

impl Judgement {
    /// `true` when no expected plate is missing, no unexpected plate was produced and no point failed.
    pub fn passed(&self) -> bool {
        self.missing_plates.is_empty() && self.extra_plates.is_empty() && self.failed_points() == 0
    }

    /// Number of points with a [`Verdict::Fail`] verdict.
    pub fn failed_points(&self) -> usize {
        self.points.iter().filter(|p| p.verdict == Verdict::Fail).count()
    }

    /// Number of points that were judged: passed or failed, not info or skipped.
    pub fn judged_points(&self) -> usize {
        self.points.iter().filter(|p| matches!(p.verdict, Verdict::Pass | Verdict::Fail)).count()
    }
}

/// Judge `plates` (separated at `res` dpi) against `scene`. `text_drawn` is false when the
/// scene's text was left out, so text points are skipped rather than failed.
pub fn judge(scene: &Scene, plates: &BTreeMap<String, Plate>, res: f32, text_drawn: bool) -> Judgement {
    let expected = &scene.expect.plates;
    let mut names: Vec<String> = expected.iter().chain(plates.keys()).cloned().collect();
    sort_plates(&mut names);
    names.dedup();
    let points = scene.expect.points.iter().map(|point| judge_point(point, plates, &names, res, text_drawn)).collect();
    let mut separated: Vec<String> = plates.keys().cloned().collect();
    sort_plates(&mut separated);
    Judgement {
        missing_plates: expected.iter().filter(|p| !plates.contains_key(*p)).cloned().collect(),
        extra_plates: separated.iter().filter(|p| !expected.contains(p)).cloned().collect(),
        plates: separated,
        points,
    }
}

fn judge_point(point: &SamplePoint, plates: &BTreeMap<String, Plate>, names: &[String], res: f32, text_drawn: bool) -> PointResult {
    let mut result = PointResult {
        at: point.at,
        kind: point.kind,
        note: point.note.clone(),
        expected: point.inks.clone(),
        tol: point.tol,
        measured: BTreeMap::new(),
        verdict: Verdict::Fail,
        off: Vec::new(),
        how: None,
    };
    if point.kind == PointKind::Text && !text_drawn {
        result.verdict = Verdict::Skipped;
        result.how = Some("text not drawn (no font)".into());
        return result;
    }
    let pixels = match point_pixels(point, plates, res) {
        Ok((pixels, how)) => {
            result.how = how;
            pixels
        }
        Err(why) => {
            result.how = Some(why);
            Vec::new()
        }
    };
    for name in names {
        let value = match plates.get(name) {
            Some(plate) => median(&mut pixels.iter().filter_map(|&(x, y)| plate.ink(x, y)).collect::<Vec<_>>()).unwrap_or(0.0),
            None => 0.0,
        };
        result.measured.insert(name.clone(), (value * 10.0).round() / 10.0);
    }
    if pixels.is_empty() {
        return result;
    }
    for (name, value) in &result.measured {
        let ok = match point.inks.get(name).copied().unwrap_or(Level::Value(0.0)) {
            Level::Value(expected) => (value - expected).abs() <= point.tol,
            Level::Range([lo, hi]) => (lo..=hi).contains(value),
        };
        if !ok {
            result.off.push(name.clone());
        }
    }
    sort_plates(&mut result.off);
    result.verdict = if point.info {
        Verdict::Info
    } else if result.off.is_empty() {
        Verdict::Pass
    } else {
        Verdict::Fail
    };
    result
}

/// Pixels a point samples, and a note on how they were found.
type Sample = (Vec<(usize, usize)>, Option<String>);

/// The pixels a point samples; `Err` says why there are none.
fn point_pixels(point: &SamplePoint, plates: &BTreeMap<String, Plate>, res: f32) -> Result<Sample, String> {
    let (width, height) = plates.values().next().map(|p| (p.width(), p.height())).ok_or("no plates")?;
    let px = |v: f32| (v * res / 72.0).round();
    let inside = |x: f32, y: f32| x >= 0.0 && y >= 0.0 && (x as usize) < width && (y as usize) < height;
    match (point.kind, point.region) {
        (PointKind::Text, Some([x, y, w, h])) => {
            let glyph = point.plate.as_deref().unwrap_or(TEXT_PLATE);
            let plate = plates.get(glyph).ok_or_else(|| format!("no {glyph} plate to find the text on"))?;
            // Integer pixel ranges clipped to the plate: a huge or NaN region (`NaN.max(0.0)` is 0,
            // `as usize` saturates) would otherwise stall a float counter or loop for ages.
            let clip = |v: f32, max: usize| px(v).max(0.0).min(max as f32) as usize;
            let (x0, x1) = (clip(x, width), clip(x + w, width));
            let (y0, y1) = (clip(y, height), clip(y + h, height));
            let mut region = Vec::new();
            for cy in y0..y1 {
                for cx in x0..x1 {
                    region.push(((cx, cy), plate.ink(cx, cy).unwrap_or(0.0)));
                }
            }
            let darkest = region.iter().map(|(_, ink)| *ink).fold(0.0, f32::max);
            if darkest < TEXT_MIN_INK {
                return Err(format!("text not found on the {glyph} plate"));
            }
            let glyph_pixels: Vec<(usize, usize)> = region.into_iter().filter(|(_, ink)| *ink >= darkest * 0.5).map(|(at, _)| at).collect();
            let how = format!("{} glyph pixels on {glyph}", glyph_pixels.len());
            Ok((glyph_pixels, Some(how)))
        }
        _ => {
            let (cx, cy) = (px(point.at[0]), px(point.at[1]));
            let mut pixels = Vec::with_capacity(25);
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let (x, y) = (cx + dx as f32, cy + dy as f32);
                    if inside(x, y) {
                        pixels.push((x as usize, y as usize));
                    }
                }
            }
            if pixels.is_empty() { Err("outside the page".into()) } else { Ok((pixels, None)) }
        }
    }
}

/// The median; the mean of the middle two for an even count.
pub fn median(values: &mut [f32]) -> Option<f32> {
    values.sort_by(f32::total_cmp);
    let n = values.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => values.get(n / 2).copied(),
        _ => Some((values.get(n / 2 - 1)? + values.get(n / 2)?) / 2.0),
    }
}

/// Process plates first in press order, then spots by name.
pub fn sort_plates(names: &mut [String]) {
    names.sort_by_key(|name| (PROCESS.iter().position(|p| p == name).unwrap_or(PROCESS.len()), name.clone()));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    const W: usize = 60;
    const H: usize = 40;

    fn plate(paint: impl Fn(usize, usize) -> u8) -> Plate {
        let ink = (0..W * H).map(|i| paint(i % W, i / W)).collect();
        Plate::new(W, H, ink).unwrap()
    }

    fn scene(json: &str) -> Scene {
        serde_json::from_str(json).unwrap()
    }

    /// Cyan everywhere; Gold at 50% in the right half; a "glyph" of solid Black in
    /// x 10..20, y 10..30 with stray 30% black pixels around it.
    fn plates(with_gold: bool, extra: Option<&str>) -> BTreeMap<String, Plate> {
        let mut plates = BTreeMap::new();
        plates.insert("Cyan".to_owned(), plate(|_, _| 255));
        plates.insert("Magenta".to_owned(), plate(|_, _| 0));
        plates.insert("Yellow".to_owned(), plate(|_, _| 0));
        plates.insert(
            "Black".to_owned(),
            plate(|x, y| match (x, y) {
                (10..20, 10..30) => 255,
                (5..25, 5..35) if (x + y) % 3 == 0 => 77,
                _ => 0,
            }),
        );
        if with_gold {
            plates.insert("Gold".to_owned(), plate(|x, _| if x >= 30 { 128 } else { 0 }));
        }
        if let Some(name) = extra {
            plates.insert(name.to_owned(), plate(|_, _| 0));
        }
        plates
    }

    const SCENE: &str = r#"{
        "id": "T", "title": "t", "source": "unit test", "page": [60, 40],
        "inks": [{"name": "Gold", "cmyk": [0, 20, 60, 20]}],
        "objects": [],
        "expect": {
            "plates": ["Cyan", "Magenta", "Yellow", "Black", "Gold"],
            "points": [
                {"at": [45, 20], "inks": {"Cyan": 100, "Gold": 50}, "tol": 2},
                {"at": [15, 20], "kind": "text", "region": [5, 5, 20, 30], "inks": {"Cyan": 100, "Black": 100}, "tol": 2},
                {"at": [45, 20], "inks": {"Gold": [10, 20]}, "tol": 2, "info": true},
                {"at": [45, 20], "inks": {"Cyan": 100, "Gold": [40, 60]}, "tol": 2}
            ]
        }
    }"#;

    #[test]
    fn fails_wrong_ink_and_wrong_plates() {
        let mut knocked = plates(false, Some("Orange"));
        knocked.insert("Cyan".to_owned(), plate(|x, _| if x >= 30 { 0 } else { 255 }));
        let judgement = judge(&scene(SCENE), &knocked, 72.0, true);
        assert_eq!(judgement.missing_plates, ["Gold"]);
        assert_eq!(judgement.extra_plates, ["Orange"]);
        let first = &judgement.points[0];
        assert_eq!(first.verdict, Verdict::Fail);
        assert_eq!(first.off, ["Cyan", "Gold"]);
        assert_eq!(judgement.points[2].verdict, Verdict::Info, "info points are never judged");
        assert!(!judgement.passed());
        assert_eq!((judgement.failed_points(), judgement.judged_points()), (2, 3));
    }

    #[test]
    fn a_huge_text_region_samples_only_the_plate() {
        let region = r#"{"at": [15, 20], "kind": "text", "region": [-3e38, -3e38, 3.4e38, 3.4e38], "inks": {"Black": 100}, "tol": 2}"#;
        let huge = scene(&SCENE.replacen(r#"{"at": [45, 20], "inks": {"Cyan": 100, "Gold": 50}, "tol": 2}"#, region, 1));
        let judgement = judge(&huge, &plates(true, None), 72.0, true);
        assert_eq!(judgement.points[0].how.as_deref(), Some("200 glyph pixels on Black"), "{judgement:#?}");
    }
}

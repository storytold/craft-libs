//! Plate-test scenes flattened end to end, with complex regions rasterized as ink images: the
//! plates of a scene and of its flattened page match at the scene's sample points, and the
//! flattened page has no transparency left.
#![cfg(feature = "flatten")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use craft_plate_test::reference::{self, Context};
use craft_plate_test::{scene, structure};
use craft_prepress::flatten::{FlattenOptions, FlattenPageOptions, flatten_page};
use craft_prepress::{Options, Plates, plates};

fn separate(pdf: &[u8], dpi: f32) -> Plates {
    plates(pdf, 0, dpi, &Options::default()).unwrap()
}

/// Ink `name`'s tint at (`x`, `y`) in points, 0 when the page doesn't use the ink.
fn tint(p: &Plates, name: &str, x: f32, y: f32) -> u8 {
    let s = p.dpi / 72.0;
    p.inks.lookup(name).and_then(|id| p.tint(id, (x * s) as u32, (y * s) as u32)).unwrap_or(0)
}

/// The flattened page has no transparency left ([`structure::transparency`]).
fn assert_opaque(pdf: &[u8]) {
    let left = structure::transparency(&structure::summarize(pdf).unwrap());
    assert!(left.is_empty(), "transparency left: {left:?}");
}

fn font() -> Option<Arc<Vec<u8>>> {
    let dir = std::env::var_os(reference::FONT_DIR_ENV)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/craft-pdf-test-fonts")))?;
    std::fs::read(dir.join(reference::FONT_FILE)).ok().map(Arc::new)
}

/// Scene `id` written by the reference writer and flattened with complex regions at 72 dpi: the
/// plates match at every judged sample point within `tol` (0–255), or the flattened page meets the
/// scene's expectation there. The renderer's source page can differ from the scene where it
/// converts a mask's CMYK luminosity through the printing condition (S11's K100 `/BC` reads as
/// about 12% luminosity); the flattener uses the DeviceCMYK formula, as Acrobat does. Returns the worst
/// difference from the source.
fn scene_round_trip(id: &str, tol: u8) -> u8 {
    scene_planes(id, tol).0
}

/// [`scene_round_trip`], with the source's and the flattened page's plates.
fn scene_planes(id: &str, tol: u8) -> (u8, Plates, Plates) {
    let scenes = scene::load_dir(&scene::scenes_dir()).unwrap();
    let s = scenes.iter().find(|s| s.id == id).unwrap();
    let pdf = reference::write(s, &Context { scenes: &scenes, font: font() }).unwrap().pdf;
    let opts = FlattenPageOptions { flatten: FlattenOptions { raster_dpi: 72.0, ..FlattenOptions::default() }, ..FlattenPageOptions::default() };
    let start = Instant::now();
    let (flat, report) = flatten_page(&pdf, 0, &opts).unwrap();
    eprintln!("{id}: flatten_page {:?}, {:?}", start.elapsed(), report.raster);
    assert_opaque(&flat);
    let (a, b) = (separate(&pdf, 72.0), separate(&flat, 72.0));
    let mut worst = 0;
    for point in s.expect.points.iter().filter(|p| !p.info) {
        let [x, y] = point.at;
        for name in s.expect.plates.iter().map(String::as_str) {
            let (want, got) = (tint(&a, name, x, y), tint(&b, name, x, y));
            if want.abs_diff(got) <= tol {
                worst = worst.max(want.abs_diff(got));
                continue;
            }
            let meets = |v: u8| {
                let pct = f32::from(v) / 2.55;
                match point.inks.get(name).copied().unwrap_or(scene::Level::Value(0.0)) {
                    scene::Level::Value(e) => (pct - e).abs() <= point.tol,
                    scene::Level::Range([lo, hi]) => pct >= lo - point.tol && pct <= hi + point.tol,
                }
            };
            assert!(meets(got), "{id} {name} at ({x}, {y}): {got}, the source has {want}");
            eprintln!("{id} {name} at ({x}, {y}): {got} meets the scene, the source renders {want}");
        }
    }
    eprintln!("{id}: worst ink difference {worst}/255");
    (worst, a, b)
}

#[test]
fn s11_shadow_over_spot() {
    scene_round_trip("S11", 6);
}

#[test]
fn f02_hidden_art_leaves_no_trace() {
    // vectorcraft#2161: no pixel of the opaque card and box, anti-aliased as viewers draw them,
    // differs from the source: art hidden under them doesn't cut them into pieces whose edges
    // show.
    let (_, a, b) = scene_planes("F02", 3);
    let s = a.dpi / 72.0;
    let mut off = Vec::new();
    for (x0, x1) in [(22.0, 168.0), (182.0, 338.0)] {
        for y in (22.0 * s) as u32..(158.0 * s) as u32 {
            for x in (x0 * s) as u32..(x1 * s) as u32 {
                for name in ["Cyan", "Magenta", "Yellow", "Black"] {
                    let tint = |p: &Plates| p.inks.lookup(name).and_then(|id| p.tint(id, x, y)).unwrap_or(0);
                    if tint(&a) != tint(&b) {
                        off.push(format!("{name} {} vs {} at pixel ({x}, {y})", tint(&b), tint(&a)));
                    }
                }
            }
        }
    }
    assert!(off.is_empty(), "{} pixels differ, e.g. {:?}", off.len(), &off[..off.len().min(8)]);
}

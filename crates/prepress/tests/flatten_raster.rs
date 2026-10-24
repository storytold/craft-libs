//! Complex regions rasterized as ink images, and plate-test scenes flattened end to end: the
//! plates of a scene and of its flattened page match at the scene's sample points, and the
//! flattened page has no transparency left.
//!
//! D10 needs Inter from `CRAFT_PDF_TEST_FONT_DIR` or `~/.cache/craft-pdf-test-fonts`, and is
//! skipped when it is missing.
#![cfg(feature = "flatten")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use craft_pdf::write::backend::SerializeSettings;
use craft_pdf::write::backend::configure::PdfVersion;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::print::{self, PrintColor};
use craft_pdf::write::{Metadata, Writer, configuration};
use craft_plate_test::reference::{self, Context};
use craft_plate_test::{scene, structure};
use craft_prepress::flatten::kurbo::{Affine, BezPath, Point, Rect, Shape};
use craft_prepress::flatten::raster::Rasterizer;
use craft_prepress::flatten::write::{WriteError, page_pdf, write_flattened};
use craft_prepress::flatten::{
    BlendMode, ComplexReason, ComplexRegion, DisplayList, FillItem, FillRule, FlattenOptions, FlattenPageOptions, Flattened, GradientGeometry,
    GroupItem, Item, Paint, flatten, flatten_page,
};
use craft_prepress::{Alternate, CmsConversion, Fallback, Ink, InkColor, InkId, InkSet, Options, Overprint, Plates, Process, Spot, plates};

const C: InkId = InkId::process(Process::Cyan);
const K: InkId = InkId::process(Process::Black);

fn settings() -> SerializeSettings {
    // Reading the profile takes a quarter second in a debug build.
    static SETTINGS: OnceLock<SerializeSettings> = OnceLock::new();
    SETTINGS.get_or_init(|| print::settings(PrintColor::fogra39(craft_cms::builtin::COATED_FOGRA39_ICC)).unwrap()).clone()
}

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
fn s06_half_black_over_cyan() {
    scene_round_trip("S06", 3);
}

#[test]
fn s07_spot_multiply() {
    scene_round_trip("S07", 3);
}

#[test]
fn s08_non_separable_blend_over_a_spot() {
    scene_round_trip("S08", 3);
}

#[test]
fn s10_isolated_multiply() {
    scene_round_trip("S10", 3);
}

#[test]
fn s11_shadow_over_spot() {
    scene_round_trip("S11", 6);
}

#[test]
fn s14_duotone_gradients() {
    scene_round_trip("S14", 6);
}

#[test]
fn m1_luminosity_mask_rasterizes() {
    scene_round_trip("M1", 3);
}

#[test]
fn m2_mask_transfer_rasterizes() {
    scene_round_trip("M2", 3);
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

#[test]
fn d10_text_over_transparency_stays_live() {
    if font().is_none() {
        eprintln!("skipped: no test font");
        return;
    }
    scene_round_trip("D10", 3);
    let scenes = scene::load_dir(&scene::scenes_dir()).unwrap();
    let s = scenes.iter().find(|s| s.id == "D10").unwrap();
    let pdf = reference::write(s, &Context { scenes: &scenes, font: font() }).unwrap().pdf;
    let (flat, _) = flatten_page(&pdf, 0, &FlattenPageOptions::default()).unwrap();
    let doc = craft_pdf::open_slice(&flat, None).unwrap();
    let pages = doc.pages();
    let mut ops = Vec::new();
    let mut it = pages.first().unwrap().operations();
    while let Some(op) = it.next() {
        ops.push(String::from_utf8_lossy(op.operator).into_owned());
    }
    assert!(ops.iter().any(|op| op == "Tj" || op == "TJ"), "text drawn as text: {ops:?}");
}

fn spot_inks() -> (InkSet, InkId) {
    let mut inks = InkSet::new();
    let gold = inks.add(Ink::Spot(Spot { name: "Gold".into(), alternate: Alternate::Cmyk([0.0, 0.3, 1.0, 0.0]), solidity: None })).unwrap();
    (inks, gold)
}

fn solid(r: Rect, tints: &[(InkId, f32)]) -> Item {
    Item::Fill(FillItem {
        path: r.to_path(0.1),
        rule: FillRule::NonZero,
        paint: Paint::Solid(InkColor::Tints(tints.to_vec())),
        overprint: Overprint::default(),
    })
}

fn group(opacity: f32, blend: BlendMode, children: Vec<Item>) -> Item {
    Item::Group(GroupItem { opacity, blend, isolated: false, knockout: false, clip: None, mask: None, actual_text: None, children })
}

fn ramp(r: Rect, from: InkColor, to: InkColor) -> Item {
    Item::Fill(FillItem {
        path: r.to_path(0.1),
        rule: FillRule::NonZero,
        paint: Paint::Gradient {
            stops: vec![(0.0, from), (1.0, to)],
            geometry: GradientGeometry::Axial {
                start: Point::new(r.x0, 0.0),
                end: Point::new(r.x1, 0.0),
                extend: [true; 2],
                transform: Affine::IDENTITY,
            },
        },
        overprint: Overprint::default(),
    })
}

/// `flat` written with complex regions rasterized at `dpi`.
fn write(list: &DisplayList, flat: &Flattened, dpi: f32) -> Vec<u8> {
    write_at(list, flat, dpi, PdfVersion::default()).unwrap()
}

/// [`write`] for an output of PDF `version`.
fn write_at(list: &DisplayList, flat: &Flattened, dpi: f32, version: PdfVersion) -> Result<Vec<u8>, WriteError> {
    let cms = CmsConversion::fogra39();
    let mut raster = Rasterizer::new(list, settings(), Fallback::Fogra39, &cms, dpi);
    let output = SerializeSettings { configuration: configuration(version, None).unwrap(), ..settings() };
    let mut writer = Writer::new(output, Metadata::default());
    let size = PageSettings::from_wh(list.page.width() as f32, list.page.height() as f32).unwrap();
    let mut page_builder = writer.document_mut().start_page_with(size);
    let mut surface = page_builder.surface();
    let drawn = write_flattened(&mut surface, list, flat, &cms, version, &mut |r| raster.region(r));
    surface.finish();
    page_builder.finish();
    drawn?;
    Ok(writer.finish().unwrap())
}

/// A page of 2 pt stripes, one of `process` tints and one of each of `count` spot inks, under a
/// half-transparent ramp: a complex region that paints them all.
fn many_inks(count: usize, process: &[(InkId, f32)]) -> (DisplayList, Flattened) {
    let mut inks = InkSet::new();
    let stripe = |i: usize| Rect::new(2.0 * i as f64, 0.0, 2.0 * (i + 1) as f64, 10.0);
    let mut items = vec![solid(stripe(0), process)];
    for i in 0..count {
        let spot = Spot { name: format!("Spot {i}"), alternate: Alternate::Cmyk([0.0, 0.0, 0.0, 0.1]), solidity: None };
        items.push(solid(stripe(i + 1), &[(inks.add(Ink::Spot(spot)).unwrap(), 0.5)]));
    }
    let width = 2.0 * (count + 1) as f64;
    let ramp = ramp(Rect::new(0.0, 0.0, width, 10.0), InkColor::Tints(vec![(K, 0.0)]), InkColor::Tints(vec![(K, 1.0)]));
    items.push(group(0.5, BlendMode::Multiply, vec![ramp]));
    let list = DisplayList { page: Rect::new(0.0, 0.0, width, 10.0), inks, items };
    let flat = flatten(&list, &FlattenOptions::default()).unwrap();
    assert!(!flat.complex.is_empty());
    (list, flat)
}

#[test]
fn a_region_of_more_spot_inks_than_plates_separates_is_an_error() {
    let (list, flat) = many_inks(33, &[(K, 0.5)]);
    match write_at(&list, &flat, 72.0, PdfVersion::default()) {
        Err(WriteError::SpotInks { inks: 33, max: 32, unseparated }) => assert_eq!(unseparated.len(), 1),
        other => panic!("{:?}", other.map(|_| ())),
    }
}

#[test]
fn a_raster_region_of_nine_inks_needs_pdf_1_5() {
    let (list, flat) = many_inks(5, &[(C, 0.5), (InkId::process(Process::Magenta), 0.5), (InkId::process(Process::Yellow), 0.5), (K, 0.5)]);
    match write_at(&list, &flat, 72.0, PdfVersion::Pdf14) {
        Err(WriteError::Colorants { colorants: 9, max: 8 }) => {}
        other => panic!("{:?}", other.map(|_| ())),
    }
    let p = separate(&write_at(&list, &flat, 72.0, PdfVersion::Pdf17).unwrap(), 72.0);
    assert!(tint(&p, "Spot 4", 11.0, 5.0) > 0);
}

#[test]
fn gradient_under_multiply_over_a_spot_prints_the_same_rasterized() {
    // A K ramp multiplied over Gold and cyan, half transparent: complex regions over a spot and
    // a process backdrop, with a kept K fill above them.
    let (inks, gold) = spot_inks();
    let list = DisplayList {
        page: Rect::new(0.0, 0.0, 100.0, 100.0),
        inks,
        items: vec![
            solid(Rect::new(0.0, 0.0, 50.0, 100.0), &[(gold, 1.0)]),
            solid(Rect::new(50.0, 0.0, 100.0, 100.0), &[(C, 0.6)]),
            group(
                0.5,
                BlendMode::Multiply,
                vec![ramp(Rect::new(10.0, 20.0, 90.0, 80.0), InkColor::Tints(vec![(K, 0.0)]), InkColor::Tints(vec![(K, 1.0)]))],
            ),
            solid(Rect::new(40.0, 45.0, 60.0, 55.0), &[(K, 1.0)]),
        ],
    };
    let flat = flatten(&list, &FlattenOptions::default()).unwrap();
    assert_eq!(flat.complex.len(), 1, "touching complex regions merge: {:?}", flat.complex.iter().map(|c| &c.items).collect::<Vec<_>>());
    let start = Instant::now();
    let out = write(&list, &flat, 72.0);
    eprintln!("ramp: written in {:?}", start.elapsed());
    assert_opaque(&out);
    let (a, b) = (separate(&page_pdf(&list, settings(), &CmsConversion::fogra39()).unwrap(), 72.0), separate(&out, 72.0));
    let mut worst = 0;
    for (x, y) in [(5.0, 50.0), (20.0, 30.0), (30.0, 70.0), (45.0, 40.0), (50.0, 50.0), (55.0, 60.0), (70.0, 30.0), (85.0, 75.0), (95.0, 50.0)] {
        for name in ["Cyan", "Magenta", "Yellow", "Black", "Gold"] {
            let (want, got) = (tint(&a, name, x, y), tint(&b, name, x, y));
            assert!(want.abs_diff(got) <= 3, "{name} at ({x}, {y}): {got}, expected {want}");
            worst = worst.max(want.abs_diff(got));
        }
    }
    eprintln!("ramp: worst ink difference {worst}/255");
}

#[test]
fn abutting_complex_regions_leave_no_paper_at_their_edge() {
    // Two regions cut off the grid at x = 50.3, each its own image of the same 60% K fill.
    let list = DisplayList {
        page: Rect::new(0.0, 0.0, 100.0, 100.0),
        inks: InkSet::new(),
        items: vec![solid(Rect::new(10.0, 10.0, 90.0, 90.0), &[(K, 0.6)])],
    };
    let region = |r: Rect| ComplexRegion { path: r.to_path(0.1), items: vec![0], at: 0, whole: None };
    let flat = Flattened {
        items: Vec::new(),
        complex: vec![region(Rect::new(10.0, 10.0, 50.3, 90.0)), region(Rect::new(50.3, 10.0, 90.0, 90.0))],
        report: Default::default(),
    };
    let dpi = 36.0;
    let p = separate(&write(&list, &flat, dpi), 2.0 * dpi);
    let k = p.inks.lookup("Black").unwrap();
    let edge = (50.3 * p.dpi / 72.0) as u32;
    for y in (12.0 * p.dpi / 72.0) as u32..(88.0 * p.dpi / 72.0) as u32 {
        for x in edge.saturating_sub(1)..=edge + 1 {
            let v = p.tint(k, x, y).unwrap();
            assert!(v >= 153 / 2, "K {v} at pixel ({x}, {y})");
        }
    }
}

#[test]
fn a_degenerate_group_is_an_opaque_image_over_paper() {
    // The region's path is a box larger than the half-black disc in it, with cyan beside it.
    // Nothing else is under the box, so its image needs no soft mask: paper where the disc
    // doesn't paint, the disc's own color where it does.
    let disc = kurbo_circle(Point::new(50.0, 50.0), 20.0);
    let list = DisplayList {
        page: Rect::new(0.0, 0.0, 100.0, 100.0),
        inks: InkSet::new(),
        items: vec![
            solid(Rect::new(0.0, 0.0, 20.0, 100.0), &[(C, 1.0)]),
            group(
                0.5,
                BlendMode::Normal,
                vec![Item::Fill(FillItem {
                    path: disc,
                    rule: FillRule::NonZero,
                    paint: Paint::Solid(InkColor::Tints(vec![(K, 1.0)])),
                    overprint: Overprint::default(),
                })],
            ),
        ],
    };
    let flat = Flattened {
        items: vec![list.items[0].clone()],
        complex: vec![ComplexRegion {
            path: Rect::new(25.0, 25.0, 75.0, 75.0).to_path(0.1),
            items: vec![1],
            at: 1,
            whole: Some(ComplexReason::Degenerate),
        }],
        report: Default::default(),
    };
    let out = write(&list, &flat, 72.0);
    assert_opaque(&out);
    let p = separate(&out, 72.0);
    // A corner of the box, outside the disc: paper.
    assert!(tint(&p, "Cyan", 28.0, 28.0) <= 3 && tint(&p, "Black", 28.0, 28.0) <= 3);
    // The disc: K 50%.
    assert!(tint(&p, "Black", 50.0, 50.0).abs_diff(128) <= 3 && tint(&p, "Cyan", 50.0, 50.0) <= 3);
    // The cyan beside the box, untouched.
    assert!(tint(&p, "Cyan", 10.0, 50.0) >= 250);
}

fn kurbo_circle(c: Point, r: f64) -> BezPath {
    craft_prepress::flatten::kurbo::Circle::new(c, r).to_path(0.01)
}

#[test]
fn regions_thinner_than_a_point_rasterize() {
    // At 300 dpi the hairline region is a tile of 3 px, 0.72 pt; the strip's last tile is 3 px
    // wide.
    let list = DisplayList {
        page: Rect::new(0.0, 0.0, 400.0, 40.0),
        inks: InkSet::new(),
        items: vec![group(0.5, BlendMode::Normal, vec![solid(Rect::new(10.0, 10.0, 390.0, 30.0), &[(K, 1.0)])])],
    };
    let region = |r: Rect| ComplexRegion { path: r.to_path(0.1), items: vec![0], at: 0, whole: None };
    let flat = Flattened {
        items: Vec::new(),
        complex: vec![region(Rect::new(20.0, 20.0, 20.1, 25.0)), region(Rect::new(10.0, 10.0, 378.71, 15.0))],
        report: Default::default(),
    };
    let p = separate(&write(&list, &flat, 300.0), 300.0);
    assert!(tint(&p, "Black", 20.05, 22.0) > 0, "the hairline prints");
    assert!(tint(&p, "Black", 378.4, 12.0).abs_diff(128) <= 3, "the last tile prints");
}

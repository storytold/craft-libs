//! The flattener's writer: display lists written with krilla and separated again by `plates`.
//! Pages are 100 pt square and separated at 36 dpi; sample points are in points.
//!
//! Text cases with an embedded font need Inter from `CRAFT_PDF_TEST_FONT_DIR` or
//! `~/.cache/craft-pdf-test-fonts`, and are skipped when it is missing.
#![cfg(feature = "flatten")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use craft_pdf::interpret::font::Glyph;
use craft_pdf::interpret::hayro_cmap::BfString;
use craft_pdf::interpret::{
    BlendMode as HBlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint as HPaint, PathDrawMode,
    SoftMask, TransformExt, interpret_page,
};
use craft_pdf::syntax::object::{Dict, Name, Stream};
use craft_pdf::write::backend::SerializeSettings;
use craft_pdf::write::backend::configure::PdfVersion;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::print::{self, PrintColor};
use craft_pdf::write::{Metadata, Writer};
use craft_prepress::flatten::kurbo::{Affine, BezPath, Point, Rect, Shape};
use craft_prepress::flatten::write::{WriteError, page_pdf, write_flattened};
use craft_prepress::flatten::{
    BlendMode, ComplexRegion, DisplayList, FillItem, FillRule, FlattenOptions, GradientGeometry, GroupItem, ImageItem, ImageSpace, Item, Paint,
    PlacedGlyph, TextFont, TextItem, flatten,
};
use craft_prepress::{Alternate, CmsConversion, Ink, InkColor, InkId, InkSet, Options, Overprint, Plates, Process, Spot, plates};

const C: InkId = InkId::process(Process::Cyan);
const M: InkId = InkId::process(Process::Magenta);
const Y: InkId = InkId::process(Process::Yellow);
const K: InkId = InkId::process(Process::Black);
const OVERPRINT: Overprint = Overprint { fill: true, stroke: false, nonzero: false };

fn fill(r: Rect, tints: &[(InkId, f32)], overprint: Overprint) -> Item {
    Item::Fill(FillItem { path: r.to_path(0.1), rule: FillRule::NonZero, paint: Paint::Solid(InkColor::Tints(tints.to_vec())), overprint })
}

fn solid(r: Rect, tints: &[(InkId, f32)]) -> Item {
    fill(r, tints, Overprint::default())
}

fn group(opacity: f32, children: Vec<Item>) -> Item {
    Item::Group(GroupItem {
        opacity,
        blend: BlendMode::Normal,
        isolated: false,
        knockout: false,
        clip: None,
        mask: None,
        actual_text: None,
        children,
    })
}

fn page(inks: InkSet, items: Vec<Item>) -> DisplayList {
    DisplayList { page: Rect::new(0.0, 0.0, 100.0, 100.0), inks, items }
}

fn pdf(list: &DisplayList) -> Vec<u8> {
    // Reading the profile takes a quarter second in a debug build.
    static SETTINGS: OnceLock<SerializeSettings> = OnceLock::new();
    let settings = SETTINGS.get_or_init(|| print::settings(PrintColor::fogra39(craft_cms::builtin::COATED_FOGRA39_ICC)).unwrap());
    page_pdf(list, settings.clone(), &CmsConversion::fogra39()).unwrap()
}

/// `list` flattened, with nothing left for the raster step, written.
fn flat_pdf(list: &DisplayList) -> Vec<u8> {
    let flat = flatten(list, &FlattenOptions::default()).unwrap();
    assert!(flat.complex.is_empty());
    pdf(&DisplayList { items: flat.items, ..list.clone() })
}

fn separate(pdf: &[u8]) -> Plates {
    plates(pdf, 0, 36.0, &Options::default()).unwrap()
}

/// Ink `name`'s tint at (`x`, `y`) in points, 0 when the page doesn't use the ink.
fn tint(p: &Plates, name: &str, x: u32, y: u32) -> u8 {
    p.inks.lookup(name).and_then(|id| p.tint(id, x / 2, y / 2)).unwrap_or(0)
}

/// Each ink's tint at (`x`, `y`) is within 3 of `expected`.
fn assert_inks(p: &Plates, x: u32, y: u32, expected: &[(&str, u8)]) {
    for (name, want) in expected {
        let got = tint(p, name, x, y);
        assert!(got.abs_diff(*want) <= 3, "{name} at ({x}, {y}): {got}, expected {want}");
    }
}

#[test]
fn half_black_over_cyan_in_a_group_composites_the_same_flattened_or_not() {
    let list = page(
        InkSet::new(),
        vec![solid(Rect::new(0.0, 0.0, 60.0, 100.0), &[(C, 1.0)]), group(0.5, vec![solid(Rect::new(40.0, 0.0, 100.0, 100.0), &[(K, 1.0)])])],
    );
    for written in [pdf(&list), flat_pdf(&list)] {
        let p = separate(&written);
        assert_inks(&p, 50, 50, &[("Cyan", 128), ("Black", 128)]);
        assert_inks(&p, 20, 50, &[("Cyan", 255), ("Black", 0)]);
        assert_inks(&p, 80, 50, &[("Cyan", 0), ("Black", 128)]);
    }
}

#[test]
fn group_opacity_applies_to_the_group_not_to_each_child() {
    // Two overlapping K100 fills in a 50% group: K50 where they overlap, not K75.
    let list = page(
        InkSet::new(),
        vec![group(0.5, vec![solid(Rect::new(0.0, 0.0, 60.0, 100.0), &[(K, 1.0)]), solid(Rect::new(40.0, 0.0, 100.0, 100.0), &[(K, 1.0)])])],
    );
    for written in [pdf(&list), flat_pdf(&list)] {
        let p = separate(&written);
        assert_inks(&p, 50, 50, &[("Black", 128)]);
        assert_inks(&p, 20, 50, &[("Black", 128)]);
    }
}

#[test]
fn black_overprint_keeps_the_cyan_under_it() {
    let list = page(
        InkSet::new(),
        vec![solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(C, 1.0)]), fill(Rect::new(50.0, 0.0, 100.0, 100.0), &[(K, 1.0)], OVERPRINT)],
    );
    for written in [pdf(&list), flat_pdf(&list)] {
        let p = separate(&written);
        assert_inks(&p, 75, 50, &[("Cyan", 255), ("Black", 255)]);
        assert_inks(&p, 25, 50, &[("Cyan", 255), ("Black", 0)]);
    }
}

#[test]
fn overprinting_process_color_leaves_the_plates_of_inks_it_doesnt_list() {
    let over = |tints: &[(InkId, f32)], nonzero: bool| {
        let list = page(
            InkSet::new(),
            vec![
                solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(C, 1.0)]),
                fill(Rect::new(50.0, 0.0, 100.0, 100.0), tints, Overprint { nonzero, ..OVERPRINT }),
            ],
        );
        separate(&pdf(&list))
    };
    // Magenta and yellow only: cyan stays, black stays empty.
    assert_inks(&over(&[(M, 1.0), (Y, 0.5)], false), 75, 50, &[("Cyan", 255), ("Magenta", 255), ("Yellow", 128), ("Black", 0)]);
    // A listed cyan at 0 knocks cyan out under OPM 0 ...
    assert_inks(&over(&[(C, 0.0), (M, 1.0)], false), 75, 50, &[("Cyan", 0), ("Magenta", 255)]);
    // ... and leaves it under OPM 1.
    assert_inks(&over(&[(C, 0.0), (M, 1.0)], true), 75, 50, &[("Cyan", 255), ("Magenta", 255)]);
    // Nothing named paints nothing.
    assert_inks(&over(&[(C, 0.0)], true), 75, 50, &[("Cyan", 255)]);
}

fn gold() -> Spot {
    Spot { name: "Gold".into(), alternate: Alternate::Cmyk([0.0, 0.2, 0.8, 0.1]), solidity: None }
}

#[test]
fn spot_fills_print_on_their_plate_or_where_the_ink_manager_sends_them() {
    let mut inks = InkSet::new();
    let gold = inks.add(Ink::Spot(gold())).unwrap();
    let list = page(inks.clone(), vec![solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(gold, 0.8)])]);
    assert_inks(&separate(&pdf(&list)), 50, 50, &[("Gold", 204), ("Cyan", 0), ("Magenta", 0), ("Yellow", 0)]);

    let mut aliased = inks.clone();
    assert!(aliased.alias(gold, M));
    let p = separate(&pdf(&DisplayList { inks: aliased, ..list.clone() }));
    assert!(p.inks.lookup("Gold").is_none());
    assert_inks(&p, 50, 50, &[("Magenta", 204), ("Yellow", 0)]);

    let mut process = inks;
    assert!(process.set_to_process(gold, true));
    let p = separate(&pdf(&DisplayList { inks: process, ..list }));
    assert!(p.inks.lookup("Gold").is_none());
    assert_inks(&p, 50, 50, &[("Cyan", 0), ("Magenta", 41), ("Yellow", 163), ("Black", 20)]);
}

// ---- text ----------------------------------------------------------------------------------

const SAMPLE: &str = "Hamburg";

fn inter() -> Option<Vec<u8>> {
    let dir = std::env::var_os("CRAFT_PDF_TEST_FONT_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/craft-pdf-test-fonts")));
    let data = dir.as_ref().and_then(|d| std::fs::read(d.join("Inter-Regular.ttf")).ok());
    if data.is_none() {
        println!("note: Inter-Regular.ttf not found in {dir:?}; case skipped");
    }
    data
}

struct Outline(BezPath);

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), f64::from(y)));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.0.quad_to((f64::from(x1), f64::from(y1)), (f64::from(x), f64::from(y)));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.0.curve_to((f64::from(x1), f64::from(y1)), (f64::from(x2), f64::from(y2)), (f64::from(x), f64::from(y)));
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

/// `SAMPLE` in K100 at 18 pt on a baseline at (5, 60), with its outlines.
fn text_run(font: &[u8], embed: bool) -> TextItem {
    let face = ttf_parser::Face::parse(font, 0).unwrap();
    let upem = f64::from(face.units_per_em());
    let size = 18.0;
    let transform = Affine::new([1.0, 0.0, 0.0, -1.0, 5.0, 60.0]);
    let (mut x, mut glyphs, mut outlines) = (0.0, Vec::new(), BezPath::new());
    for (i, c) in SAMPLE.char_indices() {
        let id = face.glyph_index(c).unwrap();
        glyphs.push(PlacedGlyph { id: u32::from(id.0), origin: Point::new(x, 0.0), text: i..i + c.len_utf8() });
        let mut outline = Outline(BezPath::new());
        face.outline_glyph(id, &mut outline);
        outlines.extend((transform * Affine::translate((x, 0.0)) * Affine::scale(size / upem) * outline.0).iter());
        x += f64::from(face.glyph_hor_advance(id).unwrap()) * size / upem;
    }
    TextItem {
        font: embed.then(|| Arc::new(TextFont { data: Arc::new(font.to_vec()), index: 0, key: 1 })),
        size: size as f32,
        transform,
        glyphs,
        text: Some(SAMPLE.to_owned()),
        outlines,
        paint: Paint::Solid(InkColor::Tints(vec![(K, 1.0)])),
        overprint: Overprint::default(),
    }
}

#[derive(Default)]
struct Reader {
    unicode: String,
    actual_text: String,
}

impl<'a> Device<'a> for Reader {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: HBlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &HPaint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: HBlendMode) {}
    fn draw_glyph(&mut self, glyph: &Glyph<'a>, _: Affine, _: Affine, _: &HPaint<'a>, _: &GlyphDrawMode) {
        if let Glyph::Outline(o) = glyph {
            match o.as_unicode() {
                Some(BfString::Char(c)) => self.unicode.push(c),
                Some(BfString::String(s)) => self.unicode.push_str(&s),
                None => {}
            }
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
    fn actual_text(&mut self, text: &[u8]) {
        self.actual_text.push_str(&match text {
            [0xFE, 0xFF, rest @ ..] => String::from_utf16_lossy(&rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect::<Vec<_>>()),
            _ => text.iter().map(|&b| char::from(b)).collect(),
        });
    }
}

fn read_text(pdf: &[u8]) -> Reader {
    let doc = craft_pdf::syntax::Pdf::new(Arc::new(pdf.to_vec())).unwrap();
    let page = &doc.pages()[0];
    let cache = InterpreterCache::new();
    let bbox = Rect::new(0.0, 0.0, 100.0, 100.0);
    let mut context = Context::new(page.initial_transform(true).to_kurbo(), bbox, &cache, page.xref(), InterpreterSettings::default());
    let mut reader = Reader::default();
    interpret_page(page, &mut context, &mut reader);
    reader
}

#[test]
fn text_with_a_font_is_live_and_prints_like_its_outlines() {
    let Some(font) = inter() else { return };
    let live = pdf(&page(InkSet::new(), vec![Item::Text(text_run(&font, true))]));
    let outlined = pdf(&page(InkSet::new(), vec![Item::Text(text_run(&font, false))]));
    assert_eq!(read_text(&live).unicode, SAMPLE);
    let (a, b) = (separate(&live), separate(&outlined));
    let (a, b) = (a.plane(K).unwrap(), b.plane(K).unwrap());
    assert!(a.iter().map(|v| u64::from(*v)).sum::<u64>() > 255 * 50, "the text prints");
    let differ = a.iter().zip(b).filter(|(x, y)| x.abs_diff(**y) > 64).count();
    assert!(differ <= a.len() / 200, "{differ} pixels differ between glyphs and outlines");
}

#[test]
fn text_without_a_font_is_drawn_as_outlines_with_its_text() {
    let mut run = TextItem {
        font: None,
        size: 18.0,
        transform: Affine::new([1.0, 0.0, 0.0, -1.0, 5.0, 60.0]),
        glyphs: Vec::new(),
        text: Some(SAMPLE.to_owned()),
        outlines: Rect::new(5.0, 45.0, 80.0, 60.0).to_path(0.1),
        paint: Paint::Solid(InkColor::Tints(vec![(K, 1.0)])),
        overprint: Overprint::default(),
    };
    if let Some(font) = inter() {
        run = TextItem { font: None, ..text_run(&font, false) };
    }
    let written = pdf(&page(InkSet::new(), vec![Item::Text(run)]));
    let read = read_text(&written);
    assert_eq!(read.actual_text, SAMPLE);
    assert!(read.unicode.is_empty());
}

// ---- structure -----------------------------------------------------------------------------

/// Transparency left on a page: groups, soft masks, blend modes other than Normal and
/// constant opacity below 1, in its resources and those of the forms it uses.
fn transparency(pdf: &[u8]) -> Vec<String> {
    let doc = craft_pdf::syntax::Pdf::new(Arc::new(pdf.to_vec())).unwrap();
    let page = &doc.pages()[0];
    let mut found = Vec::new();
    if page.raw().get::<Dict<'_>>(b"Group".as_slice()).is_some() {
        found.push("page group".to_owned());
    }
    visit(&page.resources().x_objects, &page.resources().ext_g_states, &mut found, 0);
    found
}

fn visit(x_objects: &Dict<'_>, states: &Dict<'_>, found: &mut Vec<String>, depth: usize) {
    assert!(depth < 16);
    for key in states.keys() {
        let Some(state) = states.get::<Dict<'_>>(key.as_ref()) else { continue };
        if state.get::<Dict<'_>>(b"SMask".as_slice()).is_some() {
            found.push("soft mask".to_owned());
        }
        if let Some(bm) = state.get::<Name<'_>>(b"BM".as_slice())
            && bm.as_str() != "Normal"
        {
            found.push(format!("blend mode {}", bm.as_str()));
        }
        for alpha in ["ca", "CA"] {
            if state.get::<f32>(alpha.as_bytes()).is_some_and(|a| a < 1.0) {
                found.push(alpha.to_owned());
            }
        }
    }
    for key in x_objects.keys() {
        let Some(x) = x_objects.get::<Stream<'_>>(key.as_ref()) else { continue };
        let dict = x.dict();
        if dict.get::<Dict<'_>>(b"Group".as_slice()).is_some() {
            found.push("group".to_owned());
        }
        if let Some(res) = dict.get::<Dict<'_>>(b"Resources".as_slice()) {
            let sub = |key: &[u8]| res.get::<Dict<'_>>(key).unwrap_or_default();
            visit(&sub(b"XObject"), &sub(b"ExtGState"), found, depth + 1);
        }
    }
}

#[test]
fn a_flattened_page_has_no_transparency_left() {
    let multiply = Item::Group(GroupItem {
        blend: BlendMode::Multiply,
        isolated: true,
        ..match group(0.7, vec![solid(Rect::new(30.0, 30.0, 90.0, 90.0), &[(M, 1.0)]), solid(Rect::new(50.0, 10.0, 95.0, 60.0), &[(Y, 0.6)])]) {
            Item::Group(g) => g,
            _ => unreachable!(),
        }
    });
    let list = page(
        InkSet::new(),
        vec![
            solid(Rect::new(0.0, 0.0, 60.0, 100.0), &[(C, 1.0)]),
            group(0.5, vec![solid(Rect::new(40.0, 0.0, 100.0, 100.0), &[(K, 1.0)])]),
            multiply,
        ],
    );
    let unflattened = transparency(&pdf(&list));
    assert!(unflattened.iter().any(|t| t == "ca") && unflattened.iter().any(|t| t.starts_with("blend mode")), "{unflattened:?}");
    assert_eq!(transparency(&flat_pdf(&list)), Vec::<String>::new());
}

// ---- images, gradients and complex regions ------------------------------------------------

fn axial(from: InkColor, to: InkColor) -> Paint {
    Paint::Gradient {
        stops: vec![(0.0, from), (1.0, to)],
        geometry: GradientGeometry::Axial {
            start: Point::new(0.0, 0.0),
            end: Point::new(100.0, 0.0),
            extend: [true, true],
            transform: Affine::IDENTITY,
        },
    }
}

#[test]
fn an_overprinting_cyan_gradient_keeps_the_yellow_under_it() {
    let ramp = Item::Fill(FillItem {
        path: Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1),
        rule: FillRule::NonZero,
        paint: axial(InkColor::Tints(vec![(C, 0.0)]), InkColor::Tints(vec![(C, 1.0)])),
        overprint: OVERPRINT,
    });
    let p = separate(&pdf(&page(InkSet::new(), vec![solid(Rect::new(0.0, 0.0, 100.0, 100.0), &[(Y, 1.0)]), ramp])));
    assert_inks(&p, 26, 50, &[("Cyan", 66), ("Yellow", 255)]);
    assert_inks(&p, 74, 50, &[("Cyan", 189), ("Yellow", 255)]);
}

#[test]
fn ink_images_print_through_the_ink_manager() {
    let mut inks = InkSet::new();
    let gold = inks.add(Ink::Spot(gold())).unwrap();
    assert!(inks.alias(gold, C));
    let image = Item::Image(ImageItem {
        space: ImageSpace::Inks(vec![C, gold]),
        size: (2, 1),
        samples: Arc::from(&[128, 128, 0, 255][..]),
        transform: Affine::new([100.0, 0.0, 0.0, 100.0, 0.0, 0.0]),
        smask: None,
        overprint: Overprint::default(),
        jpeg: None,
    });
    let p = separate(&pdf(&page(inks, vec![image])));
    // Two half tints on one plate overlap as halftones: 1 − 0.5 × 0.5.
    assert_inks(&p, 25, 50, &[("Cyan", 191)]);
    assert_inks(&p, 75, 50, &[("Cyan", 255)]);
    assert!(p.inks.lookup("Gold").is_none());
}

#[test]
fn complex_regions_are_drawn_where_the_flattened_items_put_them() {
    let ramp = Item::Fill(FillItem {
        path: Rect::new(40.0, 0.0, 100.0, 100.0).to_path(0.1),
        rule: FillRule::NonZero,
        paint: axial(InkColor::Tints(vec![(K, 0.2)]), InkColor::Tints(vec![(K, 1.0)])),
        overprint: Overprint::default(),
    });
    let list = page(InkSet::new(), vec![solid(Rect::new(0.0, 0.0, 60.0, 100.0), &[(C, 1.0)]), group(0.5, vec![ramp])]);
    let flat = flatten(&list, &FlattenOptions::default()).unwrap();
    assert!(!flat.complex.is_empty());
    // A stand-in for the raster step: magenta over each region's bounds.
    let mut raster = |region: &ComplexRegion| -> Result<Option<ImageItem>, WriteError> {
        let b = region.path.bounding_box();
        Ok(Some(ImageItem {
            space: ImageSpace::Inks(vec![M]),
            size: (1, 1),
            samples: Arc::from(&[255][..]),
            transform: Affine::new([b.width(), 0.0, 0.0, b.height(), b.x0, b.y0]),
            smask: None,
            overprint: Overprint::default(),
            jpeg: None,
        }))
    };
    let settings = print::settings(PrintColor::fogra39(craft_cms::builtin::COATED_FOGRA39_ICC)).unwrap();
    let mut writer = Writer::new(settings, Metadata::default());
    let mut page_builder = writer.document_mut().start_page_with(PageSettings::from_wh(100.0, 100.0).unwrap());
    let mut surface = page_builder.surface();
    write_flattened(&mut surface, &list, &flat, &CmsConversion::fogra39(), PdfVersion::default(), &mut raster).unwrap();
    surface.finish();
    page_builder.finish();
    let p = separate(&writer.finish().unwrap());
    assert_inks(&p, 20, 50, &[("Cyan", 255), ("Magenta", 0)]);
    assert_inks(&p, 50, 50, &[("Cyan", 0), ("Magenta", 255)]);
    assert_inks(&p, 80, 50, &[("Cyan", 0), ("Magenta", 255), ("Black", 0)]);
}

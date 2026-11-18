//! The PDF-in adapter on plate-test scenes and hand-written pages: display list structure, ink
//! colors, overprint and group attributes, and plates of opaque items written back.
#![cfg(feature = "flatten")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use craft_pdf::syntax::object::{Name, Object};
use craft_pdf::write::backend::blend::Overprint as KOverprint;
use craft_pdf::write::backend::geom::PathBuilder;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::backend::paint::{Fill, FillRule as KFillRule};
use craft_pdf::write::{Metadata, Writer};
use craft_plate_test::reference::{self, Context};
use craft_plate_test::scene;
use craft_prepress::flatten::kurbo::{BezPath, PathEl, Point, Shape};
use craft_prepress::flatten::pdf::{ReadOptions, ReadReport, read_page};
use craft_prepress::flatten::write::{WriteError, page_pdf};
use craft_prepress::flatten::{
    BlendMode, DisplayList, FillItem, FillRule, GradientGeometry, GroupItem, ImageItem, ImageSpace, Item, MaskKind, Paint,
};
use craft_prepress::flatten::{FlattenPageOptions, Mesh, flatten_page};
use craft_prepress::{CmsConversion, InkColor, InkId, Options, Overprint, Process, plates, to_pdf_color};

fn scene_pdf(id: &str) -> Vec<u8> {
    let scenes = scene::load_dir(&scene::scenes_dir()).unwrap();
    let s = scenes.iter().find(|s| s.id == id).unwrap();
    reference::write(s, &Context { scenes: &scenes, font: None }).unwrap().pdf
}

fn read(pdf: &[u8]) -> (DisplayList, ReadReport) {
    read_page(pdf, 0, &ReadOptions::default()).unwrap()
}

/// Every non-group item with the groups it is in, outermost first.
fn leaves(items: &[Item]) -> Vec<(Vec<&GroupItem>, &Item)> {
    fn walk<'a>(items: &'a [Item], path: &mut Vec<&'a GroupItem>, out: &mut Vec<(Vec<&'a GroupItem>, &'a Item)>) {
        for item in items {
            match item {
                Item::Group(g) => {
                    path.push(g);
                    walk(&g.children, path, out);
                    path.pop();
                }
                other => out.push((path.clone(), other)),
            }
        }
    }
    let mut out = Vec::new();
    walk(items, &mut Vec::new(), &mut out);
    out
}

fn fill_items(list: &DisplayList) -> Vec<(Vec<&GroupItem>, &FillItem)> {
    leaves(&list.items)
        .into_iter()
        .filter_map(|(groups, item)| match item {
            Item::Fill(f) => Some((groups, f)),
            _ => None,
        })
        .collect()
}

fn ink(p: Process) -> InkId {
    InkId::process(p)
}

fn cmyk_color(c: f32, m: f32, y: f32, k: f32) -> InkColor {
    InkColor::Tints(vec![(ink(Process::Cyan), c), (ink(Process::Magenta), m), (ink(Process::Yellow), y), (ink(Process::Black), k)])
}

fn cmyk(c: f32, m: f32, y: f32, k: f32) -> Paint {
    Paint::Solid(cmyk_color(c, m, y, k))
}

fn op(fill: bool, nonzero: bool) -> Overprint {
    Overprint { fill, stroke: fill, nonzero }
}

#[test]
fn black_overprints_cyan() {
    let (list, report) = read(&scene_pdf("S01"));
    let fills = fill_items(&list);
    assert_eq!(fills.len(), 2, "two rects (text is skipped without a font): {fills:?}");
    assert_eq!(fills[0].1.paint, cmyk(1.0, 0.0, 0.0, 0.0));
    assert_eq!(fills[0].1.overprint, op(false, false));
    assert_eq!(fills[1].1.paint, cmyk(0.0, 0.0, 0.0, 1.0));
    assert_eq!(fills[1].1.overprint, op(true, true));
    assert_eq!(fills[1].1.path.bounding_box().size().width.round(), 100.0);
    assert_eq!((report.images, report.gradients), (0, 0));
}

#[test]
fn spot_inks_join_the_set() {
    let (list, _) = read(&scene_pdf("S04"));
    let gold = list.inks.lookup("Gold").unwrap();
    let fills = fill_items(&list);
    let Paint::Solid(InkColor::Tints(tints)) = &fills[1].1.paint else { panic!() };
    assert!(matches!(tints[..], [(id, t)] if id == gold && (t - 0.5).abs() < 0.01), "{tints:?}");
    assert!(fills[1].1.overprint.fill);
    let alternate = list.inks.spots().find(|(id, _)| *id == gold).unwrap().1.alternate;
    assert_eq!(alternate, craft_prepress::Alternate::Cmyk([0.0, 0.2, 0.6, 0.2]));

    // An overprinting stroke carries `OP` in `fill`.
    let (list, _) = read(&scene_pdf("D06"));
    let gold = list.inks.lookup("Gold").unwrap();
    let stroke = fill_items(&list).into_iter().find(|(_, f)| f.paint == Paint::Solid(InkColor::Tints(vec![(gold, 1.0)]))).unwrap().1;
    assert_eq!(stroke.overprint, op(true, false));
    // The 10 pt stroke outlined around a 110 × 60 rect.
    assert_eq!(stroke.path.bounding_box().size().width.round(), 120.0);
}

#[test]
fn registration_is_all_inks() {
    let (list, _) = read(&scene_pdf("S13"));
    let all: Vec<_> = fill_items(&list).into_iter().filter(|(_, f)| f.paint == Paint::Solid(InkColor::All(1.0))).collect();
    assert_eq!(all.len(), 2, "the registration fill and stroke");
}

#[test]
fn knockout_and_isolated_groups() {
    let (list, _) = read(&scene_pdf("S09"));
    let fills = fill_items(&list);
    let ellipses: Vec<_> = fills.iter().filter(|(groups, _)| groups.iter().any(|g| g.knockout)).collect();
    assert_eq!(ellipses.len(), 2);
    for (groups, _) in ellipses {
        // The writer puts each ellipse in an opacity group of its own, inside the knockout group.
        let at = groups.iter().position(|g| g.knockout).unwrap();
        assert!(groups[at + 1..].iter().any(|g| g.opacity == 0.5));
    }

    let (list, _) = read(&scene_pdf("S10"));
    let fills = fill_items(&list);
    let magenta = fills.iter().find(|(_, f)| f.paint == cmyk(0.0, 1.0, 0.0, 0.0)).unwrap();
    assert!(magenta.0.iter().any(|g| g.isolated));
    assert_eq!(magenta.0.last().unwrap().blend, BlendMode::Multiply);
    let yellow = fills.iter().find(|(_, f)| f.paint == cmyk(0.0, 0.0, 1.0, 0.0)).unwrap();
    assert!(yellow.0.iter().any(|g| g.isolated));
    assert!(yellow.0.iter().all(|g| g.blend == BlendMode::Normal));
}

#[test]
fn soft_masks_keep_backdrop_and_transfer() {
    let (list, _) = read(&scene_pdf("M1"));
    let (groups, _) = fill_items(&list).into_iter().find(|(_, f)| f.paint == cmyk(0.0, 1.0, 0.0, 0.0)).unwrap();
    let mask = groups.iter().find_map(|g| g.mask.as_deref()).unwrap();
    assert_eq!(mask.kind, MaskKind::Luminosity);
    assert!(mask.group.isolated);
    assert_eq!(leaves(&mask.group.children).len(), 1, "the mask's rect");
    // The scene's backdrop is white (no black ink).
    assert_eq!(mask.backdrop, Some(cmyk_color(0.0, 0.0, 0.0, 0.0)));

    let (list, _) = read(&scene_pdf("M2"));
    let (groups, _) = fill_items(&list).into_iter().find(|(_, f)| f.paint == cmyk(0.0, 1.0, 0.0, 0.0)).unwrap();
    let mask = groups.iter().find_map(|g| g.mask.as_deref()).unwrap();
    let transfer = mask.transfer.as_deref().unwrap();
    assert_eq!((transfer.len(), transfer[0], transfer[255]), (256, 255, 0));
}

#[test]
fn shadow_over_spot_counts_images() {
    let (list, report) = read(&scene_pdf("S11"));
    assert_eq!(report.images, 3, "the split image and two feather images in masks");
    let shadows: Vec<_> = fill_items(&list).into_iter().filter(|(_, f)| f.paint == cmyk(0.0, 0.0, 0.0, 1.0)).collect();
    assert_eq!(shadows.len(), 2);
    for (groups, _) in shadows {
        let masked = groups.iter().find(|g| g.mask.is_some()).unwrap();
        assert_eq!(masked.blend, BlendMode::Multiply);
        let mask = masked.mask.as_deref().unwrap();
        assert_eq!(mask.kind, MaskKind::Luminosity);
        assert_eq!(mask.backdrop, Some(cmyk_color(0.0, 0.0, 0.0, 1.0)));
        // The mask's content is the feather image, alpha and all.
        let feathers: Vec<_> =
            leaves(&mask.group.children).into_iter().filter_map(|(_, i)| if let Item::Image(im) = i { Some(im) } else { None }).collect();
        assert_eq!(feathers.len(), 1, "{:?}", mask.group.children);
        assert!(groups.iter().any(|g| g.opacity == 0.75));
    }
}

/// A one-page PDF from raw content and resources, objects numbered from 5.
fn raw_pdf(content: &str, resources: &str, objects: &[String]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    out.extend_from_slice(b"1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n");
    out.extend_from_slice(b"2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n");
    out.extend_from_slice(
        format!("3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 120] /Contents 4 0 R /Resources << {resources} >> >> endobj\n").as_bytes(),
    );
    out.extend_from_slice(format!("4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n", content.len()).as_bytes());
    for (i, body) in objects.iter().enumerate() {
        out.extend_from_slice(format!("{} 0 obj {body} endobj\n", i + 5).as_bytes());
    }
    out.extend_from_slice(b"trailer << /Root 1 0 R >>\n%%EOF");
    out
}

fn stream(dict: &str, data: &str) -> String {
    format!("<< {dict} /Length {} >> stream\n{data}\nendstream", data.len())
}

#[test]
fn text_clips_strokes_and_process_color() {
    let to_unicode = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T3 def /CMapType 2 def \
        1 begincodespacerange <00> <FF> endcodespacerange 2 beginbfchar <41> <0041> <42> <0042> endbfchar \
        endcmap CMapName currentdict /CMap defineresource pop end end";
    let objects = [
        // 5: a Type 3 font with a shape glyph (A) and a colored one (B).
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] \
         /CharProcs << /sq 6 0 R /cg 7 0 R >> /Encoding << /Type /Encoding /Differences [65 /sq /cg] >> \
         /FirstChar 65 /LastChar 66 /Widths [1000 1000] /ToUnicode 8 0 R >>"
            .to_string(),
        stream("", "1000 0 0 0 1000 1000 d1 0 0 1000 1000 re f"),
        stream("", "1000 0 d0 0 1 0 0 k 0 0 1000 1000 re f"),
        stream("", to_unicode),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let content = "BT /F1 20 Tf 10 90 Td (AAB) Tj ET \
        BT /F2 24 Tf 100 90 Td (Hi) Tj ET \
        q 0 0 100 50 re 25 10 50 30 re W* n 0 0 0 1 k 0 0 100 50 re f Q \
        0.5 G 4 w [6 3] 0 d 110 10 m 290 10 l S \
        1 0 0 rg 200 30 10 10 re f";
    let pdf = raw_pdf(content, "/Font << /F1 5 0 R /F2 9 0 R >>", &objects);
    let (list, report) = read(&pdf);
    let items = leaves(&list.items);

    // The two shape glyphs form one run; the colored glyph is a group with its text.
    let Item::Text(t3) = items[0].1 else { panic!("{:?}", items[0]) };
    assert_eq!((t3.glyphs.len(), t3.text.as_deref(), t3.font.is_none()), (2, Some("AA"), true));
    assert_eq!(t3.size.round(), 20.0);
    assert_eq!(t3.glyphs[1].origin.x.round(), 20.0, "origins in text space");
    assert_eq!(t3.paint, cmyk(0.0, 0.0, 0.0, 1.0), "DeviceGray 0 is solid black only");
    let colored = items[1].0.last().unwrap();
    assert_eq!(colored.actual_text.as_deref(), Some("B"));
    assert!(matches!(items[1].1, Item::Fill(f) if f.paint == cmyk(0.0, 1.0, 0.0, 0.0)));

    let Item::Text(hi) = items[2].1 else { panic!("{:?}", items[2]) };
    assert_eq!(hi.text.as_deref(), Some("Hi"));
    assert_eq!(hi.glyphs[1].text, 1..2);
    assert!(!hi.outlines.is_empty());
    // Text space is y up: the run's transform flips y into page space.
    assert!(hi.transform.as_coeffs()[3] < 0.0);

    // The even-odd clip, as a nonzero path with the hole kept.
    let (groups, _) = &items[3];
    let clip = groups.last().unwrap().clip.as_ref().unwrap();
    assert_eq!(clip.winding(Point::new(50.0, 95.0)), 0, "the hole");
    assert_ne!(clip.winding(Point::new(10.0, 115.0)), 0);

    // The dashed stroke, outlined: 20 dashes of 6 pt in 180 pt, 4 pt wide.
    let Item::Fill(dashes) = items[4].1 else { panic!() };
    assert_eq!(dashes.paint, cmyk(0.0, 0.0, 0.0, 0.5));
    assert_eq!(dashes.path.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count(), 20);
    assert_eq!(dashes.path.bounding_box().height().round(), 4.0);

    // RGB converts through FOGRA39: red is mostly magenta and yellow.
    let Item::Fill(red) = items[5].1 else { panic!() };
    let Paint::Solid(InkColor::Tints(tints)) = &red.paint else { panic!() };
    assert_eq!(tints.len(), 4);
    assert!(tints[1].1 > 0.8 && tints[2].1 > 0.8 && tints[0].1 < 0.2, "{tints:?}");
    assert!(matches!(report.conversion, plates::Conversion::Fallback { .. }));
    assert_eq!(items.len(), 6);
}

fn images(list: &DisplayList) -> Vec<(Vec<&GroupItem>, &ImageItem)> {
    leaves(&list.items)
        .into_iter()
        .filter_map(|(groups, item)| match item {
            Item::Image(im) => Some((groups, im)),
            _ => None,
        })
        .collect()
}

/// The ink tints of `im` at page point `p` (nearest sample).
fn tints_at(im: &ImageItem, p: Point) -> Vec<(InkId, f32)> {
    let u = im.transform.inverse() * p;
    let (w, h) = (im.size.0 as usize, im.size.1 as usize);
    let (x, y) = (((u.x * w as f64) as usize).min(w - 1), ((u.y * h as f64) as usize).min(h - 1));
    let ids: Vec<InkId> = match &im.space {
        ImageSpace::Gray => vec![ink(Process::Black)],
        ImageSpace::Cmyk => Process::ALL.iter().map(|p| ink(*p)).collect(),
        ImageSpace::Inks(ids) => ids.clone(),
        ImageSpace::Rgb => panic!("RGB image"),
    };
    let n = if im.space == ImageSpace::Gray { 1 } else { ids.len() };
    let s = &im.samples[(y * w + x) * n..(y * w + x + 1) * n];
    if im.space == ImageSpace::Gray {
        return vec![(ids[0], 1.0 - f32::from(s[0]) / 255.0)];
    }
    ids.into_iter().zip(s).map(|(id, v)| (id, f32::from(*v) / 255.0)).collect()
}

fn tint(tints: &[(InkId, f32)], id: InkId) -> f32 {
    tints.iter().find(|t| t.0 == id).map_or(0.0, |t| t.1)
}

fn hex_image(dict: &str, hex: &str) -> String {
    stream(&format!("/Type /XObject /Subtype /Image /Filter /ASCIIHexDecode {dict}"), &format!("{hex}>"))
}

const GOLD: &str = "[/Separation /Gold /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0 0.2 0.6 0.2] /N 1 >>]";

#[test]
fn cmyk_image_keeps_its_samples() {
    let objects = [hex_image("/Width 2 /Height 1 /ColorSpace /DeviceCMYK /BitsPerComponent 8", "FF000000 00804020")];
    let pdf = raw_pdf("q 100 0 0 50 10 10 cm /Im1 Do Q", "/XObject << /Im1 5 0 R >>", &objects);
    let (list, report) = read(&pdf);
    let ims = images(&list);
    assert_eq!(ims.len(), 1);
    let im = ims[0].1;
    assert_eq!((&im.space, im.size, &im.samples[..]), (&ImageSpace::Cmyk, (2, 1), &[255, 0, 0, 0, 0, 128, 64, 32][..]));
    assert_eq!(im.transform * Point::ORIGIN, Point::new(10.0, 60.0), "(0, 0) is the top left sample");
    assert_eq!(im.transform * Point::new(1.0, 1.0), Point::new(110.0, 110.0));
    assert!(im.smask.is_none());
    assert_eq!((report.images, report.converted_images), (1, 0));
}

#[test]
fn cmyk_image_overprints_without_nonzero_mode() {
    let objects = [hex_image("/Width 2 /Height 1 /ColorSpace /DeviceCMYK /BitsPerComponent 8", "FF000000 00804020")];
    let pdf = raw_pdf("/GS1 gs q 100 0 0 50 10 10 cm /Im1 Do Q", "/XObject << /Im1 5 0 R >> /ExtGState << /GS1 << /op true /OPM 1 >> >>", &objects);
    let (list, _) = read(&pdf);
    assert_eq!(images(&list)[0].1.overprint, op(true, false), "nonzero overprint mode is not for images");
}

#[test]
fn separation_image_is_its_ink_with_decode_applied() {
    let objects = [hex_image(&format!("/Width 2 /Height 1 /ColorSpace {GOLD} /BitsPerComponent 8 /Decode [1 0]"), "FF40")];
    let pdf = raw_pdf("/GS1 gs q 100 0 0 50 10 10 cm /Im1 Do Q", "/XObject << /Im1 5 0 R >> /ExtGState << /GS1 << /op true /OPM 1 >> >>", &objects);
    let (list, _) = read(&pdf);
    let gold = list.inks.lookup("Gold").unwrap();
    let im = images(&list)[0].1;
    assert_eq!(im.space, ImageSpace::Inks(vec![gold]));
    assert_eq!(&im.samples[..], &[0, 0xBF]);
    assert_eq!(im.overprint, op(true, false), "nonzero overprint mode is for DeviceCMYK only");
}

#[test]
fn rgb_image_converts_to_cmyk() {
    let objects = [hex_image("/Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8", "FF0000")];
    let pdf = raw_pdf("q 10 0 0 10 10 10 cm /Im1 Do Q", "/XObject << /Im1 5 0 R >>", &objects);
    let (list, report) = read(&pdf);
    let im = images(&list)[0].1;
    assert_eq!(im.space, ImageSpace::Cmyk);
    assert!(im.samples[1] > 200 && im.samples[2] > 200 && im.samples[0] < 50, "{:?}", im.samples);
    assert_eq!(report.converted_images, 1);
}

#[test]
fn image_smask_is_resampled_to_the_image() {
    let objects = [
        hex_image("/Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8 /SMask 6 0 R", "00FF8040"),
        hex_image("/Width 1 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8", "FF00"),
    ];
    let pdf = raw_pdf("q 20 0 0 20 10 10 cm /Im1 Do Q", "/XObject << /Im1 5 0 R >>", &objects);
    let (list, report) = read(&pdf);
    let im = images(&list)[0].1;
    assert_eq!((&im.space, &im.samples[..]), (&ImageSpace::Gray, &[0, 255, 128, 64][..]));
    assert_eq!(im.smask.as_deref(), Some(&[255, 255, 0, 0][..]));
    assert_eq!(report.resampled_masks, 1);
}

#[test]
fn spot_stencil_is_a_traced_fill() {
    // Sample 0 paints: the left half of each row.
    let objects = [hex_image("/Width 8 /Height 2 /ImageMask true /BitsPerComponent 1", "0F0F")];
    let pdf = raw_pdf("/CS1 cs 1 scn q 80 0 0 20 10 10 cm /Im1 Do Q", &format!("/XObject << /Im1 5 0 R >> /ColorSpace << /CS1 {GOLD} >>"), &objects);
    let (list, report) = read(&pdf);
    let gold = list.inks.lookup("Gold").unwrap();
    let fills = fill_items(&list);
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].1.paint, Paint::Solid(InkColor::Tints(vec![(gold, 1.0)])));
    let bbox = fills[0].1.path.bounding_box();
    assert_eq!((bbox.x0, bbox.x1, bbox.y0, bbox.y1), (10.0, 50.0, 90.0, 110.0));
    assert_eq!((report.stencils, images(&list).len()), (1, 0));
}

/// The gradient's ink values at offset `s`, interpolated between stops.
fn stop_tints(stops: &[(f32, InkColor)], s: f32) -> Vec<(InkId, f32)> {
    let k = stops.iter().position(|st| st.0 >= s).unwrap();
    let (lo, hi) = (&stops[k.saturating_sub(1)], &stops[k]);
    let u = if hi.0 > lo.0 { (s - lo.0) / (hi.0 - lo.0) } else { 1.0 };
    let (InkColor::Tints(a), InkColor::Tints(b)) = (&lo.1, &hi.1) else { panic!() };
    a.iter().zip(b).map(|(x, y)| (x.0, x.1 + (y.1 - x.1) * u)).collect()
}

#[test]
fn axial_spot_to_paper_shading_is_a_gradient() {
    // Gold from 100% to paper, quadratic: 1 - s².
    let objects = [format!(
        "<< /ShadingType 2 /ColorSpace {GOLD} /Coords [0 0 100 0] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 2 >> >>"
    )];
    let pdf = raw_pdf("q 0 0 100 100 re W n /Sh1 sh Q", "/Shading << /Sh1 5 0 R >>", &objects);
    let (list, report) = read(&pdf);
    let gold = list.inks.lookup("Gold").unwrap();
    let fills = fill_items(&list);
    assert_eq!(fills.len(), 1);
    let Paint::Gradient { stops, geometry } = &fills[0].1.paint else { panic!("{:?}", fills[0].1.paint) };
    for s in [0.25_f32, 0.5, 0.75] {
        let t = stop_tints(stops, s);
        assert!((tint(&t, gold) - (1.0 - s * s)).abs() < 1.0 / 255.0, "at {s}: {t:?}");
    }
    assert!(stops.len() < 64, "{} stops", stops.len());
    let GradientGeometry::Axial { start, end, extend, .. } = geometry else { panic!() };
    assert_eq!((*start, *end, *extend), (Point::new(0.0, 0.0), Point::new(100.0, 0.0), [true, true]));
    assert_eq!(report.gradients, 1);
}

#[test]
fn radial_process_shading_is_clipped_to_its_extent() {
    let objects = ["<< /ShadingType 3 /ColorSpace /DeviceCMYK /Coords [50 50 0 50 50 40] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0 0] /C1 [0 1 0 0] /N 1 >> >>".to_string()];
    let pdf = raw_pdf("/Sh1 sh", "/Shading << /Sh1 5 0 R >>", &objects);
    let (list, _) = read(&pdf);
    let fills = fill_items(&list);
    assert_eq!(fills.len(), 1);
    let (groups, fill) = &fills[0];
    let Paint::Gradient { stops, geometry } = &fill.paint else { panic!() };
    assert!(matches!(geometry, GradientGeometry::Radial { extend: [true, true], end_radius, .. } if *end_radius == 40.0));
    let mid = stop_tints(stops, 0.5);
    assert!((tint(&mid, ink(Process::Cyan)) - 0.5).abs() < 0.01 && (tint(&mid, ink(Process::Magenta)) - 0.5).abs() < 0.01);
    assert_eq!(mid.len(), 4);
    // Without Extend it paints within the outer circle only (page space: y flipped).
    let extent = groups.iter().rev().find_map(|g| g.clip.as_ref()).unwrap();
    assert_ne!(extent.winding(Point::new(50.0, 70.0)), 0);
    assert_ne!(extent.winding(Point::new(89.5, 70.0)), 0);
    assert_eq!(extent.winding(Point::new(91.0, 70.0)), 0);
}

/// A square Coons patch over 0..100 with corners `colors` (hex, in patch order), as a Type 6
/// shading in `space`.
fn coons_square(space: &str, decode: &str, colors: &str) -> String {
    let points = "00 00 00 55 00 AA 00 FF 55 FF AA FF FF FF FF AA FF 55 FF 00 AA 00 55 00";
    hex_image_free(
        &format!("/ShadingType 6 /ColorSpace {space} /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 100 0 100 {decode}]"),
        &format!("00 {points} {colors}"),
    )
}

fn meshes(list: &DisplayList) -> Vec<&Mesh> {
    fill_items(list)
        .into_iter()
        .filter_map(|(_, f)| match &f.paint {
            Paint::Mesh(m) => Some(m.as_ref()),
            _ => None,
        })
        .collect()
}

/// `pdf` read, flattened and written again.
fn flattened(pdf: &[u8]) -> Vec<u8> {
    flatten_page(pdf, 0, &FlattenPageOptions::default()).unwrap().0
}

/// The type, color space family and colorants of each shading in `pdf`, and whether `pdf`
/// holds an image.
fn shadings(pdf: &[u8]) -> (Vec<(i32, String, Vec<String>)>, bool) {
    let doc = craft_pdf::syntax::Pdf::new(Arc::new(pdf.to_vec())).unwrap();
    let (mut found, mut image) = (Vec::new(), false);
    let name = |o: Option<&Object<'_>>| match o {
        Some(Object::Name(n)) => n.as_str().to_owned(),
        _ => String::new(),
    };
    for object in doc.objects() {
        let Object::Stream(stream) = object else { continue };
        let dict = stream.dict();
        image |= dict.get::<Name<'_>>(b"Subtype".as_slice()).is_some_and(|n| n.as_str() == "Image");
        let Some(kind) = dict.get::<i32>(b"ShadingType".as_slice()) else { continue };
        let (family, names) = match dict.get::<Object<'_>>(b"ColorSpace".as_slice()) {
            Some(Object::Array(a)) => {
                let items: Vec<Object<'_>> = a.iter::<Object<'_>>().collect();
                let names = match items.get(1) {
                    Some(Object::Array(names)) => names.iter::<Name<'_>>().map(|n| n.as_str().to_owned()).collect(),
                    other => vec![name(other)],
                };
                (name(items.first()), names)
            }
            other => (name(other.as_ref()), Vec::new()),
        };
        found.push((kind, family, names));
    }
    (found, image)
}

#[test]
fn cmyk_coons_mesh_is_written_as_a_type_6_shading() {
    let objects = [coons_square("/DeviceCMYK", "0 1 0 1 0 1 0 1", "FF000000 00FF0000 0000FF00 000000FF")];
    let pdf = raw_pdf("q 1 0 0 1 10 10 cm /Sh1 sh Q", "/Shading << /Sh1 5 0 R >>", &objects);
    let (list, report) = read(&pdf);
    assert!(images(&list).is_empty());
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (1, 0));
    let found = meshes(&list);
    assert_eq!(found.len(), 1);
    // DeviceCMYK is its inks, so the patch isn't split.
    let [patch] = found[0].patches.as_slice() else { panic!("{:?}", found[0]) };
    // Page space runs down from the top of the 120 pt page.
    assert_eq!((patch.points[0], patch.points[6]), (Point::new(10.0, 110.0), Point::new(110.0, 10.0)));
    let expect = [cmyk_color(1.0, 0.0, 0.0, 0.0), cmyk_color(0.0, 1.0, 0.0, 0.0), cmyk_color(0.0, 0.0, 1.0, 0.0), cmyk_color(0.0, 0.0, 0.0, 1.0)];
    assert_eq!(patch.colors, expect);

    let (found, image) = shadings(&flattened(&pdf));
    assert_eq!(found, vec![(6, "DeviceCMYK".to_owned(), vec![])]);
    assert!(!image);
}

#[test]
fn tensor_and_lattice_meshes_are_vectors_too() {
    // A type 7 patch on the square whose p11 is pulled to (0x90, 0x20): stream order is the
    // boundary, then p11, p12, p22, p21.
    let boundary = "00 00 00 55 00 AA 00 FF 55 FF AA FF FF FF FF AA FF 55 FF 00 AA 00 55 00";
    let tensor = hex_image_free(
        "/ShadingType 7 /ColorSpace /DeviceCMYK /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 100 0 100 0 1 0 1 0 1 0 1]",
        &format!("00 {boundary} 90 20 55 AA AA AA AA 55 FF000000 00FF0000 0000FF00 000000FF"),
    );
    let pdf = raw_pdf("q 1 0 0 1 10 10 cm /Sh1 sh Q", "/Shading << /Sh1 5 0 R >>", &[tensor]);
    let (list, report) = read(&pdf);
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (1, 0));
    // Split for its shape, not its colors.
    assert!(meshes(&list)[0].patches.len() > 1);
    assert!(!separates_like_the_source(&pdf), "no image");

    // A type 5 lattice, two vertices a row: two triangles. Moved off the pixel grid: the plate
    // renderer, sampling without anti-aliasing, misses pixel centers that sit exactly on the
    // triangles' shared edge where it meets their collapsed sides.
    let lattice = hex_image_free(
        "/ShadingType 5 /VerticesPerRow 2 /ColorSpace /DeviceCMYK /BitsPerCoordinate 8 /BitsPerComponent 8 /Decode [0 100 0 100 0 1 0 1 0 1 0 1]",
        "00 00 FF000000  FF 00 00FF0000  00 FF 0000FF00  FF FF 000000FF",
    );
    let pdf = raw_pdf("q 1 0 0 1 10.25 10 cm /Sh1 sh Q", "/Shading << /Sh1 5 0 R >>", &[lattice]);
    let (list, report) = read(&pdf);
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (1, 0));
    assert_eq!(meshes(&list)[0].patches.len(), 2);
    assert!(!separates_like_the_source(&pdf), "no image");
}

#[test]
fn device_n_triangle_mesh_keeps_its_inks() {
    let objects = [
        hex_image_free(
            "/ShadingType 4 /ColorSpace [/DeviceN [/Gold /Magenta] /DeviceCMYK 6 0 R] /BitsPerCoordinate 8 /BitsPerComponent 8 \
             /BitsPerFlag 8 /Decode [0 100 0 100 0 1 0 1]",
            "00 00 00 FF 00  00 FF 00 00 FF  00 80 FF 80 80",
        ),
        stream("/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1 0 1]", "{ pop pop 0 0 0 1 }"),
    ];
    let pdf = raw_pdf("q 1 0 0 1 10 10 cm /Sh1 sh Q", "/Shading << /Sh1 5 0 R >>", &objects);
    let (list, report) = read(&pdf);
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (1, 0));
    let gold = list.inks.lookup("Gold").unwrap();
    let [patch] = meshes(&list)[0].patches.as_slice() else { panic!() };
    // The triangle's third vertex is the patch's whole fourth side.
    let c = Point::new(10.0 + 128.0 / 255.0 * 100.0, 10.0);
    for p in &patch.points[6..10] {
        assert!(p.distance(c) < 1e-6, "{:?}", patch.points);
    }
    let tints = |k: usize| {
        let InkColor::Tints(t) = &patch.colors[k] else { panic!() };
        (tint(t, gold), tint(t, ink(Process::Magenta)))
    };
    assert_eq!((tints(0), tints(1)), ((1.0, 0.0), (0.0, 1.0)));
    assert_eq!(tints(2), tints(3));

    let (found, image) = shadings(&flattened(&pdf));
    assert_eq!(found, vec![(6, "DeviceN".to_owned(), vec!["Magenta".to_owned(), "Gold".to_owned()])]);
    assert!(!image);
}

#[test]
fn rgb_mesh_is_split_where_its_conversion_bends_and_separates_like_the_source() {
    // Saturated corners bend the conversion past what the pieces resolve; see
    // `meshes_that_cant_be_vectors_stay_images`.
    let mesh = coons_square("/DeviceRGB", "0 1 0 1 0 1", "CC6666 66CC66 6666CC 999999");
    let pdf = raw_pdf("q 1 0 0 1 10 10 cm /Sh1 sh Q", "/Shading << /Sh1 5 0 R >>", &[mesh]);
    let (list, report) = read(&pdf);
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (1, 0));
    let pieces = meshes(&list)[0].patches.len();
    assert!(pieces > 1, "RGB to CMYK isn't linear");

    assert!(!separates_like_the_source(&pdf), "no image");

    let uniform = coons_square("/DeviceRGB", "0 1 0 1 0 1", "336699 336699 336699 336699");
    let (list, _) = read(&raw_pdf("/Sh1 sh", "/Shading << /Sh1 5 0 R >>", &[uniform]));
    assert_eq!(meshes(&list)[0].patches.len(), 1, "a uniform mesh isn't split");
}

/// Every process plate of `pdf` and of `pdf` flattened, within a level or three, over 10..110;
/// whether the flattened page holds an image.
fn separates_like_the_source(pdf: &[u8]) -> bool {
    let flat = flattened(pdf);
    let (source, written) = (plates(pdf, 0, 72.0, &Options::default()).unwrap(), plates(&flat, 0, 72.0, &Options::default()).unwrap());
    for y in (15..110).step_by(10) {
        for x in (15..110).step_by(10) {
            for p in Process::ALL {
                let (a, b) = (source.tint(ink(p), x, y).unwrap(), written.tint(ink(p), x, y).unwrap());
                assert!(a.abs_diff(b) <= 3, "{p:?} at ({x}, {y}): {a} written as {b}");
            }
        }
    }
    shadings(&flat).1
}

#[test]
fn meshes_keep_their_overprint_and_sit_under_transparency() {
    let cyan_magenta = coons_square("[/DeviceN [/Cyan /Magenta] /DeviceCMYK 6 0 R]", "0 1 0 1", "FF00 00FF FFFF 0000");
    let tint = stream("/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1 0 1]", "{ 0 0 }");
    let gs = "/ExtGState << /OP << /OP true /op true >> /Half << /ca 0.5 >> >>";
    // Overprinted, the mesh leaves the black under it.
    let pdf = raw_pdf(
        "0 0 0 1 k 0 0 300 120 re f /OP gs q 1 0 0 1 10 10 cm /Sh1 sh Q",
        &format!("/Shading << /Sh1 5 0 R >> {gs}"),
        &[cyan_magenta.clone(), tint.clone()],
    );
    let (_, report) = read(&pdf);
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (1, 0));
    assert!(!separates_like_the_source(&pdf), "no image");
    // Under a half-transparent yellow square, whose region is flattened to an image.
    let pdf = raw_pdf(
        "q 1 0 0 1 10 10 cm /Sh1 sh Q /Half gs 0 0 1 0 k 30 30 50 50 re f",
        &format!("/Shading << /Sh1 5 0 R >> {gs}"),
        &[cyan_magenta, tint],
    );
    assert!(separates_like_the_source(&pdf), "the transparent region is an image");
}

#[test]
fn meshes_that_cant_be_vectors_stay_images() {
    // A step in the function: no split brings the blend within tolerance.
    let step = "/Function << /FunctionType 3 /Domain [0 1] /Bounds [0.5] /Encode [0 1 0 1] /Functions [\
                << /FunctionType 2 /Domain [0 1] /C0 [1 0 0 0] /C1 [1 0 0 0] /N 1 >> \
                << /FunctionType 2 /Domain [0 1] /C0 [0 1 0 0] /C1 [0 1 0 0] /N 1 >> ] >>";
    let objects = [hex_image_free(
        &format!("/ShadingType 4 /ColorSpace /DeviceCMYK {step} /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 100 0 100 0 1]"),
        "00 00 00 00  00 FF 00 FF  00 80 FF FF",
    )];
    let (list, report) = read(&raw_pdf("/Sh1 sh", "/Shading << /Sh1 5 0 R >>", &objects));
    assert_eq!((report.vector_meshes, report.rasterized_shadings, images(&list).len()), (0, 1, 1));

    // A conversion that still bends at the smallest piece.
    let mesh = coons_square("/DeviceRGB", "0 1 0 1 0 1", "FF0000 00FF00 0000FF FFFFFF");
    let (list, report) = read(&raw_pdf("/Sh1 sh", "/Shading << /Sh1 5 0 R >>", &[mesh]));
    assert_eq!((report.vector_meshes, report.rasterized_shadings, images(&list).len()), (0, 1, 1));

    // In a tiling cell.
    let objects = [
        stream(
            "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 100 100] /XStep 200 /YStep 200 /Resources << /Shading << /Sh1 6 0 R >> >>",
            "/Sh1 sh",
        ),
        coons_square("/DeviceCMYK", "0 1 0 1 0 1 0 1", "FF000000 00FF0000 0000FF00 000000FF"),
    ];
    let (list, report) = read(&raw_pdf("/Pattern cs /P1 scn 0 0 100 100 re f", "/Pattern << /P1 5 0 R >>", &objects));
    assert_eq!((report.tiling_cells, report.vector_meshes, report.rasterized_shadings, images(&list).len()), (1, 0, 1, 1));

    // Function-based.
    let objects = [
        stream("/ShadingType 1 /ColorSpace /DeviceGray /Domain [0 1 0 1] /Matrix [100 0 0 100 0 0] /Function 6 0 R", ""),
        stream("/FunctionType 4 /Domain [0 1 0 1] /Range [0 1]", "{ pop }"),
    ];
    let (list, report) = read(&raw_pdf("/Sh1 sh", "/Shading << /Sh1 5 0 R >>", &objects));
    assert_eq!((report.vector_meshes, report.rasterized_shadings, images(&list).len()), (0, 1, 1));
}

#[test]
fn a_narrow_band_in_a_mesh_function_is_kept() {
    // Cyan only for t in 0.125..0.375, a band no corner, edge midpoint or center of the
    // patch samples.
    let constant = |c: &str| format!("<< /FunctionType 2 /Domain [0 1] /C0 [{c}] /C1 [{c}] /N 1 >>");
    let (zero, cyan) = (constant("0 0 0 0"), constant("1 0 0 0"));
    let band = format!("/Function << /FunctionType 3 /Domain [0 1] /Bounds [0.125 0.375] /Encode [0 1 0 1 0 1] /Functions [{zero} {cyan} {zero}] >>");
    let mesh = coons_square(&format!("/DeviceCMYK {band}"), "0 1", "00 FF FF 00");
    let pdf = raw_pdf("q 1 0 0 1 10 10 cm /Sh1 sh Q", "/Shading << /Sh1 5 0 R >>", &[mesh]);
    let source = plates(&pdf, 0, 72.0, &Options::default()).unwrap();
    // t = 0.25, 85 pt up from the page's top edge.
    assert_eq!(source.tint(ink(Process::Cyan), 60, 85), Some(255));
    separates_like_the_source(&pdf);
    // The band's edges are jumps, which no piece fits, so the shading is read as an image.
    let (_, report) = read(&pdf);
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (0, 1));
    // So is a band between any two points a fixed grid over t would sample.
    let narrow = band.replace("0.125 0.375", "0.126 0.127");
    let mesh = coons_square(&format!("/DeviceCMYK {narrow}"), "0 1", "00 FF FF 00");
    let (_, report) = read(&raw_pdf("/Sh1 sh", "/Shading << /Sh1 5 0 R >>", &[mesh]));
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (0, 1));

    // A linear function stays one patch.
    let ramp = "/Function << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [1 0 0 0] /N 1 >>";
    let mesh = coons_square(&format!("/DeviceCMYK {ramp}"), "0 1", "00 FF FF 00");
    let (list, report) = read(&raw_pdf("/Sh1 sh", "/Shading << /Sh1 5 0 R >>", &[mesh]));
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (1, 0));
    assert_eq!(meshes(&list)[0].patches.len(), 1);
}

#[test]
fn mesh_functions_are_vectors_only_where_their_pieces_are_known() {
    let read_mesh = |function: &str, objects: &[String]| {
        let mut all = vec![coons_square(&format!("/DeviceCMYK /Function {function}"), "0 1", "00 FF FF 00")];
        all.extend_from_slice(objects);
        let pdf = raw_pdf("q 1 0 0 1 10 10 cm /Sh1 sh Q", "/Shading << /Sh1 5 0 R >>", &all);
        let (list, report) = read(&pdf);
        let pieces = meshes(&list).first().map(|m| m.patches.len());
        (pdf, report.vector_meshes, report.rasterized_shadings, pieces)
    };
    // Sampled, cyan rising to t = 0.5 and falling back: split at the middle sample.
    let sampled = hex_image_free("/FunctionType 0 /Domain [0 1] /Range [0 1 0 1 0 1 0 1] /Size [3] /BitsPerSample 8", "00000000 FF000000 00000000");
    let (pdf, vectors, images, pieces) = read_mesh("6 0 R", &[sampled]);
    assert_eq!((vectors, images, pieces), (1, 0, Some(4)));
    assert!(!separates_like_the_source(&pdf), "no image");
    // Stitched linear pieces with a bend at their bound.
    let rise = |c0: &str, c1: &str| format!("<< /FunctionType 2 /Domain [0 1] /C0 [{c0} 0 0 0] /C1 [{c1} 0 0 0] /N 1 >>");
    let stitched =
        format!("<< /FunctionType 3 /Domain [0 1] /Bounds [0.5] /Encode [0 1 0 1] /Functions [{} {}] >>", rise("0", "0.8"), rise("0.8", "1"));
    let (pdf, vectors, images, pieces) = read_mesh(&stitched, &[]);
    assert_eq!((vectors, images, pieces), (1, 0, Some(4)));
    assert!(!separates_like_the_source(&pdf), "no image");
    // PostScript, linear or not, has no known pieces.
    let calculator = stream("/FunctionType 4 /Domain [0 1] /Range [0 1 0 1 0 1 0 1]", "{ 0 0 0 }");
    let (_, vectors, images, _) = read_mesh("6 0 R", &[calculator]);
    assert_eq!((vectors, images), (0, 1));
}

#[test]
fn a_mesh_function_clamped_by_its_space_is_split_where_it_clamps() {
    // Gray -0.5 to 1.5 over t: clamped to 0 below t = 0.25 and to 1 above t = 0.75, the two
    // bends cancelling at t = 0.5, the only t the unsplit patch samples between its corners.
    let ramp = "/Function << /FunctionType 2 /Domain [0 1] /C0 [-0.5] /C1 [1.5] /N 1 >>";
    let mesh = coons_square(&format!("/DeviceGray {ramp}"), "0 1", "00 FF FF 00");
    let pdf = raw_pdf("q 1 0 0 1 10 10 cm /Sh1 sh Q", "/Shading << /Sh1 5 0 R >>", &[mesh]);
    let (list, report) = read(&pdf);
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (1, 0));
    assert!(meshes(&list)[0].patches.len() > 1);
    assert!(!separates_like_the_source(&pdf), "no image");

    // The same ramp inside the space's range stays one patch.
    let ramp = "/Function << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>";
    let mesh = coons_square(&format!("/DeviceGray {ramp}"), "0 1", "00 FF FF 00");
    let (list, report) = read(&raw_pdf("/Sh1 sh", "/Shading << /Sh1 5 0 R >>", &[mesh]));
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (1, 0));
    assert_eq!(meshes(&list)[0].patches.len(), 1);
}

#[test]
fn spots_aliased_onto_one_ink_combine_across_the_mesh() {
    // Gold and Silver both run 0 to 1 along v; on one plate they combine as overlapping
    // halftones, 1 - (1 - v)^2, which a single patch's bilinear blend doesn't follow.
    let spots = coons_square("[/DeviceN [/Gold /Silver] /DeviceCMYK 6 0 R]", "0 1 0 1", "0000 FFFF FFFF 0000");
    let tint = stream("/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1 0 1]", "{ pop pop 0 0 0 1 }");
    let pdf = raw_pdf("q 1 0 0 1 10 10 cm /Sh1 sh Q", "/Shading << /Sh1 5 0 R >>", &[spots, tint]);
    let (mut list, report) = read(&pdf);
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (1, 0));
    assert_eq!(meshes(&list)[0].patches.len(), 1, "the spots are the mesh's inks");
    let (gold, silver) = (list.inks.lookup("Gold").unwrap(), list.inks.lookup("Silver").unwrap());
    assert!(list.inks.alias(gold, ink(Process::Cyan)) && list.inks.alias(silver, ink(Process::Cyan)));
    let out = page_pdf(&list, Default::default(), &CmsConversion::fogra39()).unwrap();
    assert!(!shadings(&out).1, "still a mesh, not an image");
    let written = plates(&out, 0, 72.0, &Options::default()).unwrap();
    for y in (15..110).step_by(10) {
        let v = (110.0 - y as f32) / 100.0;
        let want = (255.0 * (1.0 - (1.0 - v) * (1.0 - v))).round() as i32;
        let got = i32::from(written.tint(ink(Process::Cyan), 60, y).unwrap());
        assert!((got - want).abs() <= 3, "v = {v}: {got}, not {want}");
    }

    // The writer has no image fallback: a mesh it can't split within the patch limit, or
    // with an All corner, is an error, each with its own message.
    fn each_mesh(items: &mut [Item], edit: &dyn Fn(&mut Mesh)) -> usize {
        let mut n = 0;
        for item in items {
            match item {
                Item::Group(g) => n += each_mesh(&mut g.children, edit),
                Item::Fill(f) => {
                    if let Paint::Mesh(m) = &mut f.paint {
                        edit(Arc::make_mut(m));
                        n += 1;
                    }
                }
                _ => {}
            }
        }
        n
    }
    let edited = |list: &DisplayList, edit: &dyn Fn(&mut Mesh)| {
        let mut list = list.clone();
        assert_eq!(each_mesh(&mut list.items, edit), 1);
        page_pdf(&list, Default::default(), &CmsConversion::fogra39())
    };
    let error = |r: Result<Vec<u8>, WriteError>| match r {
        Err(WriteError::Unsupported(what)) => what,
        other => panic!("{:?}", other.map(|_| ())),
    };
    let many = edited(&list, &|m| m.patches = vec![m.patches[0].clone(); 65536]);
    assert_eq!(error(many), "a mesh whose inks combine past the patch limit");
    let all = edited(&list, &|m| m.patches[0].colors[0] = InkColor::All(1.0));
    assert_eq!(error(all), "a mesh corner of the All or None colorant");
}

fn hex_image_free(dict: &str, hex: &str) -> String {
    stream(&format!("{dict} /Filter /ASCIIHexDecode"), &format!("{hex}>"))
}

#[test]
fn tiling_pattern_expands_into_cells() {
    let objects = [
        stream("/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 20 /YStep 20 /Resources << >>", "1 0 0 0 k 0 0 10 10 re f"),
        stream(
            "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 0.25 0.25] /XStep 0.5 /YStep 0.5 /Resources << >>",
            "0 1 0 0 k 0 0 0.25 0.25 re f",
        ),
    ];
    let pdf = raw_pdf("/Pattern cs /P1 scn 0 0 90 30 re f /P2 scn 100 0 90 30 re f", "/Pattern << /P1 5 0 R /P2 6 0 R >>", &objects);
    let (list, report) = read(&pdf);
    // Cells start every 20 pt from 0: five across 90 pt, two up 30 pt.
    assert_eq!(report.tiling_cells, 10);
    let cyan = fill_items(&list).into_iter().filter(|(_, f)| f.paint == cmyk(1.0, 0.0, 0.0, 0.0)).count();
    assert_eq!(cyan, 10);
    // 180 × 60 cells are past the cap: an image, a quarter magenta on average.
    assert_eq!(report.rasterized_tilings, 1);
    let im = images(&list)[0].1;
    let a = im.smask.as_ref().unwrap();
    let mean = a.iter().map(|v| f64::from(*v)).sum::<f64>() / a.len() as f64 / 255.0;
    assert!((mean - 0.25).abs() < 0.05, "{mean}");
}

#[test]
fn tiling_cells_past_the_item_budget_rasterize() {
    // Each of the 99 × 99 outer cells holds an expanded 15 × 15 inner pattern: 2.2 million
    // items in all.
    let objects = [
        stream(
            "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 1 1] /XStep 1 /YStep 1 /Resources << /Pattern << /P2 6 0 R >> >>",
            "/Pattern cs /P2 scn 0 0 1 1 re f",
        ),
        stream(
            "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 0.02 0.02] /XStep 0.07 /YStep 0.07 /Resources << >>",
            "1 0 0 0 k 0 0 0.02 0.02 re f",
        ),
    ];
    let pdf = raw_pdf("/Pattern cs /P1 scn 0.5 0.5 98 98 re f", "/Pattern << /P1 5 0 R >>", &objects);
    let (list, report) = read(&pdf);
    assert_eq!(report.tiling_cells, 225, "only the inner pattern expands");
    assert_eq!(report.rasterized_tilings, 1);
    assert_eq!(images(&list).len(), 1);
}

#[test]
fn deeply_overlapping_tiling_cells_paint_nothing() {
    // Every point lies in 6 × 10^14 cells.
    let objects = [stream(
        "/PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 100000 100000] /XStep 0.004 /YStep 0.004 /Resources << >>",
        "1 0 0 0 k 0 0 1 1 re f",
    )];
    let pdf = raw_pdf("/Pattern cs /P1 scn 0 0 90 30 re f", "/Pattern << /P1 5 0 R >>", &objects);
    let (list, report) = read(&pdf);
    assert_eq!((report.tiling_cells, report.rasterized_tilings, report.dropped_tilings), (0, 0, 1));
    assert!(images(&list).is_empty());
}

#[test]
fn dashes_past_the_budget_stroke_solid() {
    // A billion dashes.
    let pdf = raw_pdf("[0.000001] 0 d 0 0 m 1000 0 l S", "", &[]);
    let (list, _) = read(&pdf);
    let fills = fill_items(&list);
    assert_eq!(fills.len(), 1);
    assert!(fills[0].1.path.elements().len() < 20, "{:?}", fills[0].1.path);
}

#[test]
fn plate_test_gradients_and_meshes_read() {
    let (list, report) = read(&scene_pdf("S14"));
    let gold = list.inks.lookup("Gold").unwrap();
    assert_eq!((report.gradients, report.unreadable_images), (2, 0));
    let duotone = images(&list)[0].1;
    let at = tints_at(duotone, Point::new(100.0, 60.0));
    assert!((tint(&at, ink(Process::Black)) - 0.25).abs() < 0.1 && (tint(&at, gold) - 0.15).abs() < 0.1, "{at:?}");
    for (_, f) in fill_items(&list) {
        assert!(matches!(f.paint, Paint::Gradient { .. }), "{:?}", f.paint);
    }

    let pdf = scene_pdf("S17");
    let (list, report) = read(&pdf);
    assert_eq!((report.vector_meshes, report.rasterized_shadings), (1, 0));
    assert!(images(&list).is_empty());
    let (found, image) = shadings(&flattened(&pdf));
    assert_eq!(found, vec![(6, "DeviceN".to_owned(), vec!["Cyan".to_owned(), "Magenta".to_owned(), "Gold".to_owned()])]);
    assert!(!image);
}

fn write_fills(list: &DisplayList) -> Vec<u8> {
    let cms = CmsConversion::fogra39();
    let mut writer = Writer::new(Default::default(), Metadata::default());
    let size = list.page.size();
    let mut page = writer.document_mut().start_page_with(PageSettings::from_wh(size.width as f32, size.height as f32).unwrap());
    let mut surface = page.surface();
    for (groups, item) in leaves(&list.items) {
        assert!(groups.iter().all(|g| g.opacity == 1.0 && g.mask.is_none() && g.clip.is_none()), "opaque scenes only");
        let Item::Fill(f) = item else { panic!("{item:?}") };
        let Paint::Solid(color) = &f.paint else { panic!() };
        let o = f.overprint;
        surface.push_overprint(KOverprint { fill: o.fill, stroke: o.stroke, nonzero: o.nonzero });
        let rule = if f.rule == FillRule::EvenOdd { KFillRule::EvenOdd } else { KFillRule::NonZero };
        surface.set_fill(Some(Fill { paint: to_pdf_color(color, &list.inks, &cms).unwrap().into(), rule, ..Default::default() }));
        surface.draw_path(&krilla_path(&f.path));
        surface.pop();
    }
    surface.finish();
    page.finish();
    writer.finish().unwrap()
}

fn krilla_path(path: &BezPath) -> craft_pdf::write::backend::geom::Path {
    let mut b = PathBuilder::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => b.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => b.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(p1, p) => b.quad_to(p1.x as f32, p1.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(p1, p2, p) => b.cubic_to(p1.x as f32, p1.y as f32, p2.x as f32, p2.y as f32, p.x as f32, p.y as f32),
            PathEl::ClosePath => b.close(),
        }
    }
    b.finish().unwrap()
}

#[test]
fn opaque_items_separate_as_the_source() {
    for id in ["S01", "S04", "D06", "S13"] {
        let source = scene_pdf(id);
        let (list, _) = read(&source);
        let copy = write_fills(&list);
        let (a, b) = (plates(&source, 0, 36.0, &Options::default()).unwrap(), plates(&copy, 0, 36.0, &Options::default()).unwrap());
        assert_eq!(a.planes.len(), b.planes.len(), "{id}: plates");
        for (k, (pa, pb)) in a.planes.iter().zip(&b.planes).enumerate() {
            let differ = pa.iter().zip(pb).filter(|(x, y)| x.abs_diff(**y) > 8).count();
            assert!(differ * 100 < pa.len(), "{id}: plate {k} differs in {differ} of {} pixels", pa.len());
        }
    }
}

#[test]
fn print_flagged_annotations_left_out_are_counted() {
    let ap = stream("/Type /XObject /Subtype /Form /BBox [0 0 10 10]", "0 0 0 1 k 0 0 10 10 re f");
    let annot = |flags: u32| format!("<< /Type /Annot /Subtype /Stamp /Rect [10 10 20 20] /F {flags} /AP << /N 5 0 R >> >>");
    // Print; Print and Hidden; no flags (screen only); Print without an appearance.
    let objects = [ap, annot(4), annot(4 | 2), annot(0), "<< /Type /Annot /Subtype /Text /Rect [0 0 5 5] /F 4 >>".to_string()];
    // `raw_pdf` puts its argument inside the page's /Resources; this closes that dictionary to
    // add /Annots to the page.
    let pdf = raw_pdf("", ">> /Annots [6 0 R 7 0 R 8 0 R 9 0 R] /PieceInfo <<", &objects);
    let (list, report) = read_page(&pdf, 0, &ReadOptions::default()).unwrap();
    assert!(list.items.is_empty());
    assert_eq!(report.dropped_annotations, 1);
    let (list, report) = read_page(&pdf, 0, &ReadOptions { annotations: true, ..ReadOptions::default() }).unwrap();
    assert!(!list.items.is_empty());
    assert_eq!(report.dropped_annotations, 0);
}

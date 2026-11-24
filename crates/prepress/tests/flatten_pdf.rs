//! The PDF-in adapter on plate-test scenes and hand-written pages: display list structure, ink
//! colors, overprint and group attributes, and plates of opaque items written back.
#![cfg(feature = "flatten")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_pdf::write::backend::blend::Overprint as KOverprint;
use craft_pdf::write::backend::geom::PathBuilder;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::backend::paint::{Fill, FillRule as KFillRule};
use craft_pdf::write::{Metadata, Writer};
use craft_plate_test::reference::{self, Context};
use craft_plate_test::scene;
use craft_prepress::flatten::kurbo::{BezPath, PathEl, Point, Shape};
use craft_prepress::flatten::pdf::{ReadOptions, ReadReport, read_page};
use craft_prepress::flatten::{
    BlendMode, DisplayList, FillItem, FillRule, GradientGeometry, GroupItem, ImageItem, ImageSpace, Item, MaskKind, Paint,
};
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

#[test]
fn coons_mesh_shading_rasterizes() {
    // One square patch over 0..100 with C, M, Y and K corners.
    let points = "00 00 00 55 00 AA 00 FF 55 FF AA FF FF FF FF AA FF 55 FF 00 AA 00 55 00";
    let colors = "FF000000 00FF0000 0000FF00 000000FF";
    let objects = [hex_image_free(
        "/ShadingType 6 /ColorSpace /DeviceCMYK /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 /Decode [0 100 0 100 0 1 0 1 0 1 0 1]",
        &format!("00 {points} {colors}"),
    )];
    let pdf = raw_pdf("q 1 0 0 1 10 10 cm /Sh1 sh Q", "/Shading << /Sh1 5 0 R >>", &objects);
    let (list, report) = read(&pdf);
    let ims = images(&list);
    assert_eq!((ims.len(), report.rasterized_shadings), (1, 1));
    let im = ims[0].1;
    assert_eq!(im.space, ImageSpace::Cmyk);
    let corner = tints_at(im, Point::new(11.0, 109.0));
    assert!(tint(&corner, ink(Process::Cyan)) > 0.95, "{corner:?}");
    let center = tints_at(im, Point::new(60.0, 60.0));
    for p in Process::ALL {
        assert!((tint(&center, ink(p)) - 0.25).abs() < 0.03, "{center:?}");
    }
    assert_eq!(im.smask.as_ref().unwrap()[0], 0, "outside the patch");
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

    let (list, report) = read(&scene_pdf("S17"));
    let gold = list.inks.lookup("Gold").unwrap();
    assert_eq!(report.rasterized_shadings, 1);
    let mesh = images(&list)[0].1;
    let expect = [
        ((24.0, 24.0), vec![(gold, 0.95)]),
        ((216.0, 24.0), vec![(ink(Process::Cyan), 0.95)]),
        ((120.0, 80.0), vec![(gold, 0.375), (ink(Process::Cyan), 0.25), (ink(Process::Magenta), 0.25)]),
    ];
    for ((x, y), inks) in expect {
        let at = tints_at(mesh, Point::new(x, y));
        for (id, v) in inks {
            assert!((tint(&at, id) - v).abs() < 0.1, "({x}, {y}): {at:?}");
        }
    }
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

//! The PDF-in adapter on plate-test scenes: opaque items read and written back separate as the
//! source does.
#![cfg(feature = "flatten")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_cms::{Builtin, Intent, TransformOptions, builtin, cached};
use craft_pdf::write::backend::blend::Overprint as KOverprint;
use craft_pdf::write::backend::geom::PathBuilder;
use craft_pdf::write::backend::page::PageSettings;
use craft_pdf::write::backend::paint::{Fill, FillRule as KFillRule};
use craft_pdf::write::{Metadata, Writer};
use craft_plate_test::reference::{self, Context};
use craft_plate_test::scene;
use craft_prepress::flatten::kurbo::{BezPath, PathEl, Point};
use craft_prepress::flatten::pdf::{ReadOptions, ReadReport, read_page};
use craft_prepress::flatten::{DisplayList, FillItem, FillRule, GroupItem, ImageItem, ImageSpace, Item, Paint};
use craft_prepress::{CmsConversion, InkColor, InkId, Options, Process, plates, to_pdf_color};

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

fn hex_image(dict: &str, hex: &str) -> String {
    stream(&format!("/Type /XObject /Subtype /Image /Filter /ASCIIHexDecode {dict}"), &format!("{hex}>"))
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

fn stop_tints(stops: &[(f32, InkColor)], s: f32) -> Vec<(InkId, f32)> {
    let k = stops.iter().position(|st| st.0 >= s).unwrap();
    let (lo, hi) = (&stops[k.saturating_sub(1)], &stops[k]);
    let u = if hi.0 > lo.0 { (s - lo.0) / (hi.0 - lo.0) } else { 1.0 };
    let (InkColor::Tints(a), InkColor::Tints(b)) = (&lo.1, &hi.1) else { panic!() };
    a.iter().zip(b).map(|(x, y)| (x.0, x.1 + (y.1 - x.1) * u)).collect()
}

/// sRGB `rgb` in FOGRA39, the default fallback, with `intent`, converted by craft-cms itself.
fn fogra39_with(intent: Intent, rgb: [f32; 3]) -> [f32; 4] {
    let opts = TransformOptions { intent, bpc: false, precise_float: false };
    let t = cached(builtin::srgb(), Builtin::CoatedFogra39.profile(), opts).unwrap();
    let mut cmyk = [0.0; 4];
    t.convert_f32(&rgb, 3, &mut cmyk, 4, false).unwrap();
    cmyk.map(|v| v.clamp(0.0, 1.0))
}

fn fill_tints(list: &DisplayList) -> Vec<[f32; 4]> {
    fill_items(list)
        .into_iter()
        .map(|(_, f)| {
            let Paint::Solid(InkColor::Tints(tints)) = &f.paint else { panic!("{:?}", f.paint) };
            Process::ALL.map(|p| tint(tints, ink(p)))
        })
        .collect()
}

fn near(a: [f32; 4], b: [f32; 4], tolerance: f32) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() <= tolerance)
}

// Coated FOGRA39 separates sRGB yellow differently under each of the three intents tested.
const YELLOW: [f32; 3] = [1.0, 1.0, 0.0];

#[test]
fn fills_convert_with_the_rendering_intent_current_when_painted() {
    let perceptual = fogra39_with(Intent::Perceptual, YELLOW);
    let relative = fogra39_with(Intent::RelativeColorimetric, YELLOW);
    let saturation = fogra39_with(Intent::Saturation, YELLOW);
    assert!(
        !near(perceptual, relative, 0.02) && !near(saturation, relative, 0.02) && !near(saturation, perceptual, 0.02),
        "the intents must differ for this test: {perceptual:?} {relative:?} {saturation:?}"
    );
    let content = "1 1 0 rg 0 0 10 10 re f \
        /Perceptual ri 20 0 10 10 re f \
        q /Saturation ri 40 0 10 10 re f Q \
        60 0 10 10 re f \
        /GS1 gs 80 0 10 10 re f \
        /Unknown ri 100 0 10 10 re f \
        /GS2 gs 120 0 10 10 re f";
    let resources = "/ExtGState << /GS1 << /RI /RelativeColorimetric >> /GS2 << /RI /Perceptual >> >>";
    let (list, _) = read(&raw_pdf(content, resources, &[]));
    let got = fill_tints(&list);
    let want = [relative, perceptual, saturation, perceptual, relative, relative, perceptual];
    assert_eq!(got.len(), want.len());
    for (i, (got, want)) in got.iter().zip(want).enumerate() {
        assert!(near(*got, want, 1e-4), "fill {i}: {got:?}, want {want:?}");
    }
}

#[test]
fn an_image_converts_with_its_own_intent() {
    let perceptual = fogra39_with(Intent::Perceptual, YELLOW);
    let relative = fogra39_with(Intent::RelativeColorimetric, YELLOW);
    let objects = [
        hex_image("/Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Intent /Perceptual", "FFFF00"),
        hex_image("/Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8", "FFFF00"),
    ];
    let content = "q 10 0 0 10 0 0 cm /Im1 Do Q q 10 0 0 10 20 0 cm /Im2 Do Q \
        /Perceptual ri q 10 0 0 10 40 0 cm /Im2 Do Q \
        /RelativeColorimetric ri 1 1 0 rg 60 0 10 10 re f";
    let (list, _) = read(&raw_pdf(content, "/XObject << /Im1 5 0 R /Im2 6 0 R >>", &objects));
    let samples: Vec<[f32; 4]> = images(&list)
        .iter()
        .map(|(_, im)| {
            let t = tints_at(im, Point::new(im.transform.as_coeffs()[4] + 1.0, 5.0));
            Process::ALL.map(|p| tint(&t, ink(p)))
        })
        .collect();
    let want = [perceptual, relative, perceptual];
    assert_eq!(samples.len(), want.len());
    for (i, (got, want)) in samples.iter().zip(want).enumerate() {
        assert!(near(*got, want, 1.0 / 255.0 + 1e-4), "image {i}: {got:?}, want {want:?}");
    }
    // The image's `/Intent` is its own: the next fill converts with the graphics state's.
    assert!(near(fill_tints(&list)[0], relative, 1e-4));
}

#[test]
fn a_shading_converts_with_the_rendering_intent() {
    let perceptual = fogra39_with(Intent::Perceptual, YELLOW);
    let objects = [
        "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 1 0] /C1 [1 1 0] /N 1 >> >>"
            .to_string(),
    ];
    let pdf = raw_pdf("/Perceptual ri q 0 0 100 100 re W n /Sh1 sh Q", "/Shading << /Sh1 5 0 R >>", &objects);
    let (list, _) = read(&pdf);
    let fills = fill_items(&list);
    let Paint::Gradient { stops, .. } = &fills[0].1.paint else { panic!("{:?}", fills[0].1.paint) };
    let t = stop_tints(stops, 0.5);
    let got = Process::ALL.map(|p| tint(&t, ink(p)));
    assert!(near(got, perceptual, 1e-4), "{got:?}, want {perceptual:?}");
}

/// An ICCBased CMYK space whose perceptual, colorimetric and saturation A2B tables are `a2b`,
/// built from craft-cms's synthetic coated CMYK profile.
fn icc_cmyk_object(a2b: [Option<&craft_cms::profile::Lut>; 3]) -> String {
    let mut profile = Builtin::CoatedCmyk.profile().clone();
    profile.a2b = a2b.map(|lut| lut.cloned());
    let bytes = profile.with_encoded_bytes().to_bytes();
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    stream("/N 4 /Filter /ASCIIHexDecode", &format!("{hex}>"))
}

#[test]
fn an_icc_source_converts_with_the_rendering_intent() {
    let coated = Builtin::CoatedCmyk.profile().a2b[1].clone().unwrap();
    let fogra = Builtin::CoatedFogra39.profile().a2b[1].clone().unwrap();
    // Profile 1 has different perceptual and colorimetric tables and no saturation table;
    // profiles 2 and 3 have only its perceptual and only its colorimetric one.
    let objects = [icc_cmyk_object([Some(&fogra), Some(&coated), None]), icc_cmyk_object([Some(&fogra); 3]), icc_cmyk_object([Some(&coated); 3])];
    let fills = |intent: &str| {
        let content = format!(
            "/{intent} ri /C1 cs 0.2 0.4 0.6 0.1 sc 0 0 10 10 re f \
             /C2 cs 0.2 0.4 0.6 0.1 sc 20 0 10 10 re f /C3 cs 0.2 0.4 0.6 0.1 sc 40 0 10 10 re f"
        );
        let resources = "/ColorSpace << /C1 [/ICCBased 5 0 R] /C2 [/ICCBased 6 0 R] /C3 [/ICCBased 7 0 R] >>";
        let got = fill_tints(&read(&raw_pdf(&content, resources, &objects)).0);
        assert_eq!(got.len(), 3);
        (got[0], got[1], got[2])
    };
    let (mixed, perceptual, colorimetric) = fills("Perceptual");
    assert!(!near(perceptual, colorimetric, 0.02), "the tables must differ for this test: {perceptual:?} {colorimetric:?}");
    assert!(near(mixed, perceptual, 1e-4), "perceptual: {mixed:?}, want {perceptual:?}");
    let (mixed, _, colorimetric) = fills("RelativeColorimetric");
    assert!(near(mixed, colorimetric, 1e-4), "relative colorimetric: {mixed:?}, want {colorimetric:?}");
    // Without a saturation table the perceptual one converts (ICC.1 clause 8.10.2).
    let (mixed, perceptual, _) = fills("Saturation");
    assert!(near(mixed, perceptual, 1e-4), "saturation: {mixed:?}, want {perceptual:?}");
}

#[test]
fn a_mask_backdrop_converts_with_the_mask_intent() {
    let perceptual = fogra39_with(Intent::Perceptual, YELLOW);
    let relative = fogra39_with(Intent::RelativeColorimetric, YELLOW);
    let objects = [
        hex_image("/Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Intent /Perceptual", "FFFF00"),
        hex_image("/Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8", "FFFF00"),
        stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 300 120] /Group << /S /Transparency /CS /DeviceRGB >> /Resources << >>",
            "1 g 0 0 300 120 re f",
        ),
    ];
    // The masked image follows one with its own `/Intent`; the mask's backdrop converts as the
    // mask's own content starts, relative colorimetric.
    let content = "q 10 0 0 10 0 0 cm /Im1 Do Q q /GS1 gs 10 0 0 10 20 0 cm /Im2 Do Q";
    let resources = "/XObject << /Im1 5 0 R /Im2 6 0 R >> /ExtGState << /GS1 << /SMask << /S /Luminosity /G 7 0 R /BC [1 1 0] >> >> >>";
    let (list, _) = read(&raw_pdf(content, resources, &objects));
    let masked = images(&list);
    let mask = masked.iter().find_map(|(groups, _)| groups.iter().find_map(|g| g.mask.as_deref())).unwrap();
    let Some(InkColor::Tints(tints)) = &mask.backdrop else { panic!("{:?}", mask.backdrop) };
    let got = Process::ALL.map(|p| tint(tints, ink(p)));
    assert!(!near(perceptual, relative, 0.02), "the intents must differ for this test");
    assert!(near(got, relative, 1e-4), "{got:?}, want {relative:?}");
}

#[test]
fn type3_glyphs_and_uncolored_patterns_convert_with_their_intent() {
    let perceptual = fogra39_with(Intent::Perceptual, YELLOW);
    let relative = fogra39_with(Intent::RelativeColorimetric, YELLOW);
    let saturation = fogra39_with(Intent::Saturation, YELLOW);
    let objects = [
        // 5: a Type 3 font with two shape glyphs, the second setting its own intent.
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] /FontMatrix [0.001 0 0 0.001 0 0] \
         /CharProcs << /sq 6 0 R /ri 7 0 R >> /Encoding << /Type /Encoding /Differences [65 /sq /ri] >> \
         /FirstChar 65 /LastChar 66 /Widths [1000 1000] >>"
            .to_string(),
        stream("", "1000 0 0 0 1000 1000 d1 0 0 1000 1000 re f"),
        stream("", "1000 0 0 0 1000 1000 d1 /Saturation ri 0 0 1000 1000 re f"),
        // 8, 9: uncolored tiling patterns, the second setting its own intent.
        stream("/PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 20 /YStep 20 /Resources << >>", "0 0 10 10 re f"),
        stream("/PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 20 /YStep 20 /Resources << >>", "/Saturation ri 0 0 10 10 re f"),
    ];
    let content = "/Perceptual ri 1 1 0 rg BT /F1 10 Tf 0 100 Td (A) Tj ET BT /F1 10 Tf 20 100 Td (B) Tj ET \
        /CS1 cs 1 1 0 /P1 scn 0 0 10 10 re f 1 1 0 /P2 scn 40 0 10 10 re f";
    let resources = "/Font << /F1 5 0 R >> /Pattern << /P1 8 0 R /P2 9 0 R >> /ColorSpace << /CS1 [/Pattern /DeviceRGB] >>";
    let (list, _) = read(&raw_pdf(content, resources, &objects));
    let paints: Vec<[f32; 4]> = leaves(&list.items)
        .into_iter()
        .map(|(_, item)| {
            let paint = match item {
                Item::Fill(f) => &f.paint,
                Item::Text(t) => &t.paint,
                other => panic!("{other:?}"),
            };
            let Paint::Solid(InkColor::Tints(tints)) = paint else { panic!("{paint:?}") };
            Process::ALL.map(|p| tint(tints, ink(p)))
        })
        .collect();
    // A glyph converts with the intent of the text show, or its procedure's own; a pattern cell
    // starts from the initial graphics state (ISO 32000-2 §8.7.3.1), so relative colorimetric.
    let want = [perceptual, saturation, relative, saturation];
    assert_eq!(paints.len(), want.len(), "{paints:?}");
    for (i, (got, want)) in paints.iter().zip(want).enumerate() {
        assert!(near(*got, want, 1e-4), "paint {i}: {got:?}, want {want:?}");
    }
}

/// An ICCBased CMYK space: craft-cms's synthetic coated CMYK profile with media white `white`.
fn icc_cmyk_white_object(white: [f64; 3]) -> String {
    let mut profile = Builtin::CoatedCmyk.profile().clone();
    profile.white_point = white;
    let bytes = profile.with_encoded_bytes().to_bytes();
    let hex: String = bytes.iter().map(|b| format!("{b:02X}")).collect();
    stream("/N 4 /Filter /ASCIIHexDecode", &format!("{hex}>"))
}

#[test]
fn absolute_colorimetric_scales_by_the_source_media_white() {
    const D50: [f64; 3] = [0.9642, 1.0, 0.8249];
    const PAPER: [f64; 3] = [0.84, 0.87, 0.66];
    let objects = [icc_cmyk_white_object(D50), icc_cmyk_white_object(PAPER)];
    let fills = |intent: &str| {
        let content = format!("/{intent} ri /C1 cs 0.2 0.4 0.6 0.1 sc 0 0 10 10 re f /C2 cs 0.2 0.4 0.6 0.1 sc 20 0 10 10 re f");
        let resources = "/ColorSpace << /C1 [/ICCBased 5 0 R] /C2 [/ICCBased 6 0 R] >>";
        let got = fill_tints(&read(&raw_pdf(&content, resources, &objects)).0);
        assert_eq!(got.len(), 2);
        (got[0], got[1])
    };
    let (d50, paper) = fills("RelativeColorimetric");
    assert!(near(d50, paper, 1e-4), "relative colorimetric ignores the media white: {d50:?} {paper:?}");
    let (d50, paper) = fills("AbsoluteColorimetric");
    assert!(!near(d50, paper, 0.02), "absolute colorimetric keeps the media white: {d50:?} {paper:?}");
    // As craft-cms converts the whole way, absolute colorimetric.
    let mut source = Builtin::CoatedCmyk.profile().clone();
    source.white_point = PAPER;
    let opts = TransformOptions { intent: Intent::AbsoluteColorimetric, bpc: false, precise_float: false };
    let t = cached(&source, Builtin::CoatedFogra39.profile(), opts).unwrap();
    let mut want = [0.0; 4];
    t.convert_f32(&[0.2, 0.4, 0.6, 0.1], 4, &mut want, 4, false).unwrap();
    let want = want.map(|v| v.clamp(0.0, 1.0));
    assert!(near(paper, want, 0.02), "{paper:?}, want {want:?}");
}

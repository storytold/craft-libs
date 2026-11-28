//! Page background fills (Design › Page Color › Fill Effects): a solid colour, a gradient, a
//! preset pattern or a picture with transparency, read from and written to `w:background`'s VML
//! fill (ECMA-376 Part 4 §2.2.1, VML §6.1.2.5), plus hostile fills.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;

use craft_wordmodel::Document;
use craft_wordmodel::PagePicture;
use craft_wordmodel::fill::{Gradient, GradientFlip, GradientKind, PathShape};
use craft_wordmodel::pattern::PagePattern;
use craft_wordmodel::props::Rgb;
use craft_wordmodel::wordart::GradStop;

const W_NS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office""#;
const ROOT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

fn rels(list: &[(&str, &str, &str)]) -> String {
    let mut s = String::from(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    for (id, ty, target) in list {
        s.push_str(&format!(
            r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{ty}" Target="{target}"/>"#
        ));
    }
    s.push_str("</Relationships>");
    s
}

/// A package whose document holds `before_body` (the `w:background`) before an empty paragraph.
fn package(before_body: &str, doc_rels: &[(&str, &str, &str)], extra: &[(&str, &[u8])]) -> Vec<u8> {
    let document =
        format!(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W_NS}>{before_body}<w:body><w:p/></w:body></w:document>"#);
    let r = rels(doc_rels);
    let mut entries: Vec<(&str, &[u8])> =
        vec![("_rels/.rels", ROOT_RELS.as_bytes()), ("word/document.xml", document.as_bytes()), ("word/_rels/document.xml.rels", r.as_bytes())];
    for (path, bytes) in extra {
        entries.push((path, bytes));
    }
    crate::fixtures::zip(&entries)
}

fn read_markup(before_body: &str) -> Document {
    craft_docx::read(&package(before_body, &[], &[])).expect("read")
}

fn rt(doc: &Document) -> Document {
    let bytes = craft_docx::write(doc).expect("write");
    craft_docx::read(&bytes).expect("read")
}

fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("zip");
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut entry = z.by_index(i).expect("entry");
        let mut data = Vec::new();
        entry.read_to_end(&mut data).expect("bytes");
        out.insert(entry.name().expect("part name").to_string(), data);
    }
    out
}

fn document_xml(bytes: &[u8]) -> String {
    String::from_utf8(unzip(bytes)["word/document.xml"].clone()).expect("utf-8")
}

fn gradient(stops: Vec<GradStop>, kind: GradientKind) -> Gradient {
    Gradient { stops, kind, tile: None, rot_with_shape: false, flip: GradientFlip::None }
}

fn stop(pos: f32, color: Rgb, transparency: f32) -> GradStop {
    GradStop { pos, color, transparency }
}

/// A tiny valid PNG.
fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 150, 100, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).expect("png");
    out
}

#[test]
fn solid_page_background_round_trips() {
    let mut d = Document::new();
    d.settings.page_color = Some(Rgb(0x12, 0x34, 0x56));
    let bytes = craft_docx::write(&d).expect("write");
    let xml = document_xml(&bytes);
    assert!(xml.contains(r#"<w:background w:color="123456">"#), "{xml}");
    assert!(!xml.contains("<v:background"), "a solid page has no VML fill: {xml}");
    let r = craft_docx::read(&bytes).expect("read");
    assert_eq!(r.settings.page_color, Some(Rgb(0x12, 0x34, 0x56)));
    assert!(r.settings.page_gradient.is_none() && r.settings.page_pattern.is_none() && r.settings.page_picture.is_none());
}

/// A multi-stop gradient keeps its stops (the ends as `color`/`color2`, the rest in `colors`),
/// its angle and its ends' transparency; the page colour older readers show is written but not
/// read back as an explicit page colour.
#[test]
fn gradient_page_background_round_trips() {
    let mut d = Document::new();
    d.settings.page_gradient = Some(gradient(
        vec![stop(0.0, Rgb(0x10, 0x00, 0x30), 20.0), stop(50.0, Rgb(0x80, 0x20, 0x40), 20.0), stop(100.0, Rgb(0xFA, 0xFB, 0xFC), 40.0)],
        GradientKind::Linear { angle: 135.0, scaled: false },
    ));
    let bytes = craft_docx::write(&d).expect("write");
    let xml = document_xml(&bytes);
    let fill = xml.split("<v:fill ").nth(1).and_then(|f| f.split("/>").next()).unwrap_or_default();
    assert!(fill.contains(r#"type="gradient""#), "{fill}");
    assert!(fill.contains(r##"color="#100030""##) && fill.contains(r##"color2="#FAFBFC""##), "{fill}");
    assert_eq!(fill.matches("f #").count(), 3, "every stop is a `colors` pair: {fill}");
    assert!(fill.contains(r#"angle="315""#), "VML's angle is counter-clockwise from vertical: {fill}");
    assert!(fill.contains(r#"opacity="0.8""#) && fill.contains(r#"o:opacity2="0.6""#), "{fill}");
    // The page colour Word's older readers show is the first stop, not an explicit page colour.
    assert!(xml.contains(r#"w:color="100030""#), "{xml}");

    let r = rt(&d);
    let g = r.settings.page_gradient.as_ref().expect("gradient kept");
    assert_eq!(
        g.stops.iter().map(|s| (s.pos, s.color)).collect::<Vec<_>>(),
        [(0.0, Rgb(0x10, 0x00, 0x30)), (50.0, Rgb(0x80, 0x20, 0x40)), (100.0, Rgb(0xFA, 0xFB, 0xFC))]
    );
    let transparency = |i: usize| g.stops.get(i).map_or(f32::NAN, |s| s.transparency);
    assert!([(0, 20.0), (1, 20.0), (2, 40.0)].iter().all(|&(i, want)| (transparency(i) - want).abs() < 0.01), "{:?}", g.stops);
    assert!(matches!(g.kind, GradientKind::Linear { angle, .. } if (angle - 135.0).abs() < 0.01), "{:?}", g.kind);
    assert_eq!(r.settings.page_color, None, "the fallback colour belongs to the gradient");
    assert!(r.settings.page_pattern.is_none() && r.settings.page_picture.is_none());
    assert!(rt(&r).settings.page_gradient.is_some());
}

/// A radial gradient keeps its focus rectangle.
#[test]
fn radial_gradient_page_background_round_trips() {
    let mut d = Document::new();
    d.settings.page_gradient = Some(gradient(
        vec![stop(0.0, Rgb::WHITE, 0.0), stop(100.0, Rgb::BLACK, 0.0)],
        GradientKind::Path { shape: PathShape::Circle, focus: [25.0, 75.0, 75.0, 25.0] },
    ));
    let xml = document_xml(&craft_docx::write(&d).expect("write"));
    assert!(xml.contains(r#"type="gradientRadial""#), "{xml}");
    assert!(xml.contains(r#"focusposition="0.25,0.75""#), "{xml}");
    let r = rt(&d);
    let Some(GradientKind::Path { focus, .. }) = r.settings.page_gradient.as_ref().map(|g| g.kind) else {
        panic!("radial kept: {:?}", r.settings.page_gradient)
    };
    assert_eq!(focus, [25.0, 75.0, 75.0, 25.0]);
}

/// A pattern is written as Word writes one: an 8 × 8 one-bit BMP of the preset, its set bits the
/// `fillcolor`, the rest `color2`, the preset named by its `o:title` label.
#[test]
fn pattern_page_background_round_trips() {
    let mut d = Document::new();
    d.settings.page_pattern = Some(PagePattern {
        preset: "dkUpDiag".into(),
        fore: Rgb(0x33, 0x66, 0x99),
        back: Rgb(0xEE, 0xE8, 0xD8),
        transparency: 35.0,
        media: String::new(),
    });
    let bytes = craft_docx::write(&d).expect("write");
    let parts = unzip(&bytes);
    let xml = String::from_utf8(parts["word/document.xml"].clone()).expect("utf-8");
    let fill = xml.split("<v:fill ").nth(1).and_then(|f| f.split("/>").next()).unwrap_or_default();
    assert!(fill.contains(r#"type="pattern""#), "{fill}");
    assert!(fill.contains(r#"o:title="Dark upward diagonal""#), "{fill}");
    assert!(fill.contains(r##"color2="#EEE8D8""##) && fill.contains(r#"opacity="0.65""#), "{fill}");
    assert!(xml.contains(r##"fillcolor="#336699""##), "the foreground inks the tile's set bits: {xml}");
    // The tile picture is a related 8 x 8 one-bit BMP, tiled by Word at the preset's size.
    let tile = parts.get("word/media/page-pattern.bmp").expect("pattern tile part");
    assert_eq!(tile.len(), 94, "8 x 8 one-bit BMP with a two-colour palette");
    assert_eq!(&tile[..2], b"BM");
    assert_eq!(u32::from_le_bytes([tile[18], tile[19], tile[20], tile[21]]), 8);
    assert_eq!(u32::from_le_bytes([tile[22], tile[23], tile[24], tile[25]]), 8);
    assert_eq!(u16::from_le_bytes([tile[28], tile[29]]), 1, "one bit a pixel");
    // The tile's rows bottom-up, each byte with the leftmost pixel in its high bit: `dkUpDiag`'s
    // rows are 0x33, 0x99, 0xcc, 0x66 from the top, so 0x66, 0x33, 0x99, 0xcc from the bottom,
    // bit-reversed into Word's pixel order.
    assert_eq!(PagePattern::bits("dkUpDiag"), Some([0x33, 0x99, 0xcc, 0x66, 0x33, 0x99, 0xcc, 0x66]));
    let rows: Vec<u8> = (0..8).map(|row| tile[62 + row * 4]).collect();
    assert_eq!(rows, [0x66, 0x33, 0x99, 0xcc, 0x66, 0x33, 0x99, 0xcc], "the tile's 8 rows");
    let types = String::from_utf8(parts["[Content_Types].xml"].clone()).expect("utf-8");
    assert!(types.contains(r#"Extension="bmp""#) && types.contains("image/bmp"), "{types}");
    let rid = fill.split(r#"r:id=""#).nth(1).and_then(|r| r.split('"').next()).expect("the fill relates its tile");
    let rels = String::from_utf8(parts["word/_rels/document.xml.rels"].clone()).expect("utf-8");
    assert!(rels.contains(&format!(r#"Id="{rid}""#)) && rels.contains(r#"Target="media/page-pattern.bmp""#), "{rels}");

    let r = rt(&d);
    let p = r.settings.page_pattern.as_ref().expect("pattern kept");
    assert_eq!((p.preset.as_str(), p.fore, p.back), ("dkUpDiag", Rgb(0x33, 0x66, 0x99), Rgb(0xEE, 0xE8, 0xD8)));
    assert!((p.transparency - 35.0).abs() < 0.01, "{p:?}");
    assert!(p.media.is_empty(), "a known preset needs no kept tile");
    assert_eq!(r.settings.page_color, None, "the pattern's back colour is its fallback, not the page's");
}

/// A picture page background keeps its transparency (`v:fill/@opacity`, the complement).
#[test]
fn picture_page_background_keeps_transparency() {
    let mut d = Document::new();
    d.settings.page_color = Some(Rgb(0xFF, 0xFF, 0xCC));
    let bytes = png();
    d.media.insert("paper.png".into(), Arc::new(bytes.clone()));
    d.settings.page_picture = Some(PagePicture { media: "paper.png".into(), fill_type: "tile".into(), title: "Paper".into(), transparency: 40.0 });
    let written = craft_docx::write(&d).expect("write");
    let xml = document_xml(&written);
    let fill = xml.split("<v:fill ").nth(1).and_then(|f| f.split("/>").next()).unwrap_or_default();
    assert!(fill.contains(r#"type="tile""#) && fill.contains(r#"opacity="0.6""#), "{fill}");
    let r = craft_docx::read(&written).expect("read");
    let p = r.settings.page_picture.as_ref().expect("picture kept");
    assert!((p.transparency - 40.0).abs() < 0.01, "{p:?}");
    assert_eq!(r.media.get(&p.media).map(|b| b.as_slice()), Some(bytes.as_slice()));
    assert_eq!(r.settings.page_color, Some(Rgb(0xFF, 0xFF, 0xCC)));
}

/// A pattern whose label this build doesn't know keeps the file's own tile picture and label, so a
/// save writes the fill back as it found it, with unknown children of the fill ignored.
#[test]
fn unknown_pattern_tile_survives_a_round_trip() {
    let tile: Vec<u8> = {
        let mut b = vec![0u8; 94];
        b[..2].copy_from_slice(b"BM");
        b[18..22].copy_from_slice(&8u32.to_le_bytes());
        b[22..26].copy_from_slice(&8u32.to_le_bytes());
        b[28..30].copy_from_slice(&1u16.to_le_bytes());
        b[62..].copy_from_slice(&[
            0x5a, 0, 0, 0, 0xa5, 0, 0, 0, 0x5a, 0, 0, 0, 0xa5, 0, 0, 0, 0x5a, 0, 0, 0, 0xa5, 0, 0, 0, 0x5a, 0, 0, 0, 0xa5, 0, 0, 0,
        ]);
        b
    };
    let markup = r##"<w:background w:color="102030"><v:background fillcolor="#102030"><v:fill type="pattern" o:title="Fancy weave" r:id="rIdBg" color2="#FFFFFF" o:detectmouseclick="t"><o:fill v:ext="view" type="gradientCenter"/></v:fill></v:background></w:background>"##;
    let d = craft_docx::read(&package(markup, &[("rIdBg", "image", "media/tile.bmp")], &[("word/media/tile.bmp", &tile)])).expect("read");
    let p = d.settings.page_pattern.as_ref().expect("unknown pattern kept as a pattern");
    assert_eq!((p.preset.as_str(), p.fore, p.back), ("Fancy weave", Rgb(0x10, 0x20, 0x30), Rgb::WHITE));
    assert_eq!(d.media.get(&p.media).map(|b| b.as_slice()), Some(tile.as_slice()));

    let bytes = craft_docx::write(&d).expect("write");
    let parts = unzip(&bytes);
    let xml = String::from_utf8(parts["word/document.xml"].clone()).expect("utf-8");
    let fill = xml.split("<v:fill ").nth(1).and_then(|f| f.split("/>").next()).unwrap_or_default();
    assert!(fill.contains(r#"type="pattern""#) && fill.contains(r#"o:title="Fancy weave""#), "{fill}");
    let kept = parts.iter().find(|(name, b)| name.starts_with("word/media/") && b.as_slice() == tile.as_slice());
    assert!(kept.is_some(), "the file's tile picture is written back: {:?}", parts.keys().collect::<Vec<_>>());
    let again = craft_docx::read(&bytes).expect("read");
    assert_eq!(again.settings.page_pattern.as_ref().map(|p| p.preset.as_str()), Some("Fancy weave"));
}

/// An unknown attribute of `v:fill` (Office's `o:` extension) and an unknown child (`o:fill`) are
/// kept on the document and written back, while an attribute in a namespace a document doesn't
/// declare is dropped.
#[test]
fn unknown_fill_attributes_and_children_survive_a_round_trip() {
    let markup = r##"<w:background w:color="FFFFFF"><v:background fillcolor="#FFFFFF"><v:fill type="gradient" color="#FF0000" color2="#0000FF" angle="0" o:detectmouseclick="t" xmlns:x="urn:x" x:y="1"><o:fill v:ext="view" type="gradientCenter"/><v:extrusion on="f"/></v:fill></v:background></w:background>"##;
    let d = read_markup(markup);
    let g = d.settings.page_gradient.as_ref().expect("gradient read past the unknown children");
    assert_eq!(g.stops.iter().map(|s| s.color).collect::<Vec<_>>(), [Rgb(255, 0, 0), Rgb(0, 0, 255)]);
    assert!(matches!(g.kind, GradientKind::Linear { angle, .. } if (angle - 90.0).abs() < 0.01), "{:?}", g.kind);
    let extra = &d.settings.page_fill_extra;
    assert!(extra.attrs.contains(&("o:detectmouseclick".to_string(), "t".to_string())), "{:?}", extra.attrs);
    assert!(extra.attrs.iter().all(|(name, _)| name != "x:y"), "a namespace the document doesn't declare is not written: {:?}", extra.attrs);
    assert_eq!(extra.children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["o:fill", "v:extrusion"]);

    // Saved, the gradient is unchanged and the Office extensions are still there.
    let bytes = craft_docx::write(&d).expect("write");
    let xml = document_xml(&bytes);
    assert!(xml.contains(r#"o:detectmouseclick="t""#), "{xml}");
    assert!(xml.contains(r#"<o:fill v:ext="view" type="gradientCenter"/>"#), "{xml}");
    assert!(xml.contains(r#"<v:extrusion on="f"/>"#), "{xml}");
    assert!(!xml.contains("x:y"), "{xml}");
    let r = craft_docx::read(&bytes).expect("read");
    assert_eq!(
        r.settings.page_gradient.as_ref().map(|g| g.stops.iter().map(|s| s.color).collect::<Vec<_>>()),
        Some(vec![Rgb(255, 0, 0), Rgb(0, 0, 255)])
    );
    assert!(r.settings.page_fill_extra.attrs.contains(&("o:detectmouseclick".to_string(), "t".to_string())));
}

/// VML's `angle` runs counter-clockwise from a top-to-bottom blend, so an explicit 0 is the
/// model's 90° and saving gives VML's angle back.
#[test]
fn vml_gradient_angle_is_read_and_written_back() {
    let base = r#"<w:background><v:background><v:fill type="gradient" color="FF0000" color2="0000FF" angle="ANGLE"/></v:background></w:background>"#;
    let angle = |a: &str| {
        read_markup(&base.replace("ANGLE", a)).settings.page_gradient.as_ref().map(|g| match g.kind {
            GradientKind::Linear { angle, .. } => angle,
            _ => f32::NAN,
        })
    };
    assert_eq!(angle("0"), Some(90.0));
    assert_eq!(angle("315"), Some(135.0));
    assert_eq!(angle("not a number"), Some(90.0), "an unreadable angle is VML's default");
    let d = read_markup(&base.replace("ANGLE", "0"));
    let xml = document_xml(&craft_docx::write(&d).expect("write"));
    assert!(xml.contains(r#"angle="0""#), "the blend is still top to bottom: {xml}");
}

/// `focus` bends a two-colour gradient: 50 % puts the second colour in the middle and the first
/// at both ends, which the model keeps as three stops and writes back in `colors`.
#[test]
fn vml_gradient_focus_becomes_a_three_stop_gradient() {
    let markup = r#"<w:background><v:background><v:fill type="gradient" color="FF0000" color2="0000FF" focus="50%"/></v:background></w:background>"#;
    let d = read_markup(markup);
    let g = d.settings.page_gradient.as_ref().expect("gradient");
    assert_eq!(
        g.stops.iter().map(|s| (s.pos, s.color)).collect::<Vec<_>>(),
        [(0.0, Rgb(255, 0, 0)), (50.0, Rgb(0, 0, 255)), (100.0, Rgb(255, 0, 0))]
    );
    let r = rt(&d);
    assert_eq!(r.settings.page_gradient.as_ref().map(|g| g.stops.len()), Some(3));
}

/// VML `opacity` in each form Word and other writers give it.
#[test]
fn vml_opacity_forms_are_read() {
    let transparency = |opacity: &str| {
        let markup = format!(
            r##"<w:background><v:background fillcolor="#000000"><v:fill type="pattern" o:title="20%" color2="#FFFFFF" opacity="{opacity}"/></v:background></w:background>"##
        );
        read_markup(&markup).settings.page_pattern.expect("pattern read").transparency
    };
    for form in ["0.75", "75%", "49152f", "75"] {
        assert!((transparency(form) - 25.0).abs() < 0.01, "{form} is three quarters opaque");
    }
    assert_eq!(transparency("0"), 100.0);
    assert_eq!(transparency("1"), 0.0);
    assert_eq!(transparency("not a number"), 0.0);
}

/// Word writes a one-colour gradient's other colour as Office's `fill darken(n)` / `fill
/// lighten(n)` formula against the first; it is read as that shade and saved as a hex colour.
#[test]
fn office_shade_formulas_are_read_as_colours() {
    let base = Rgb(0x9B, 0xBB, 0x59);
    let shade = |formula: &str| {
        let markup = format!(
            r##"<w:background><v:background fillcolor="#9BBB59"><v:fill type="gradient" color2="{formula}" method="linear sigma" focus="100%"/></v:background></w:background>"##
        );
        let d = read_markup(&markup);
        let g = d.settings.page_gradient.expect("gradient");
        assert_eq!(g.stops.first().map(|s| s.color), Some(base), "{formula}");
        *g.stops.last().map(|s| &s.color).expect("second stop")
    };
    let dark = shade("fill darken(118)");
    let light = shade("fill lighten(118)");
    assert!(dark.luma() < base.luma(), "darkened: {dark:?}");
    assert!(light.luma() > base.luma(), "lightened: {light:?}");
    assert_ne!(dark, Rgb::WHITE, "not the missing-colour default");
    // Saved, the formula is a concrete colour.
    let d = read_markup(
        r##"<w:background><v:background fillcolor="#9BBB59"><v:fill type="gradient" color2="fill darken(118)"/></v:background></w:background>"##,
    );
    assert!(document_xml(&craft_docx::write(&d).expect("write")).contains(&format!("#{}", dark.hex())));
}

/// A pattern is recognised by its label (`o:title`) as well as by its name, and an unlabelled one
/// with no readable tile falls back to the page colour rather than inventing a pattern.
#[test]
fn patterns_are_named_by_label_or_name() {
    for label in ["Light downward diagonal", "ltDnDiag", "light DOWNWARD diagonal"] {
        let markup =
            format!(r#"<w:background><v:background><v:fill type="pattern" o:title="{label}" color2="FFFFFF"/></v:background></w:background>"#);
        assert_eq!(read_markup(&markup).settings.page_pattern.as_ref().map(|p| p.preset.as_str()), Some("ltDnDiag"), "{label}");
    }
    let bare = read_markup(r#"<w:background w:color="102030"><v:background><v:fill type="pattern" color2="FFFFFF"/></v:background></w:background>"#);
    assert!(bare.settings.page_pattern.is_none());
    assert_eq!(bare.settings.page_color, Some(Rgb(0x10, 0x20, 0x30)));
    // Every preset is named by its own name and label (see the wordmodel unit test).
    assert_eq!(PagePattern::preset_named("pct5"), Some("pct5"));
    assert_eq!(PagePattern::preset_named("5%"), Some("pct5"));
}

/// A solid `v:background/@fillcolor` with no `w:color` is the page colour.
#[test]
fn a_vml_fill_color_stands_in_for_the_page_color() {
    let d = read_markup(r##"<w:background><v:background fillcolor="#336699"/></w:background>"##);
    assert_eq!(d.settings.page_color, Some(Rgb(0x33, 0x66, 0x99)));
}

/// Hostile fills: a huge `colors` list, an oversized label, a non-finite angle and a giant
/// focusposition are bounded, and a truncated package is an error rather than a panic.
#[test]
fn hostile_page_fills_are_bounded() {
    let colors: Vec<String> = (0..5_000).map(|i| format!("{}f #00FF00", i % 101)).collect();
    let markup = format!(
        r#"<w:background><v:background><v:fill type="gradient" color="FF0000" color2="0000FF" angle="not-a-number" colors="{}" focusposition="nan,1e99"/></v:background></w:background>"#,
        colors.join(";")
    );
    let d = read_markup(&markup);
    let g = d.settings.page_gradient.as_ref().expect("gradient");
    assert!(g.stops.len() <= craft_wordmodel::wordart::MAX_STOPS);
    assert!(g.stops.iter().all(|s| (0.0..=100.0).contains(&s.pos)));
    assert!(matches!(g.kind, GradientKind::Linear { angle, .. } if angle.is_finite()), "{:?}", g.kind);

    // Many unknown attributes and children are capped.
    let attrs: String = (0..200).map(|i| format!(r#" o:k{i}="v""#)).collect();
    let kids: String = (0..200).map(|i| format!(r#"<o:x{i}/>"#)).collect();
    let markup = format!(
        r##"<w:background><v:background><v:fill type="gradient" color="FF0000" color2="0000FF"{attrs}>{kids}</v:fill></v:background></w:background>"##
    );
    let d = read_markup(&markup);
    assert!(d.settings.page_fill_extra.attrs.len() <= craft_wordmodel::MAX_FILL_EXTRA_ATTRS);
    assert!(d.settings.page_fill_extra.children.len() <= craft_wordmodel::MAX_FILL_EXTRA_CHILDREN);
    assert!(craft_docx::write(&d).is_ok());

    let label = "x".repeat(10_000);
    let markup = format!(
        r##"<w:background><v:background fillcolor="#000000"><v:fill type="pattern" o:title="{label}" r:id="rIdBg" color2="#FFFFFF" opacity="1e300"/></v:background></w:background>"##
    );
    let tile = vec![0u8; 94];
    let d = craft_docx::read(&package(&markup, &[("rIdBg", "image", "media/t.bmp")], &[("word/media/t.bmp", &tile)])).expect("read");
    let p = d.settings.page_pattern.as_ref().expect("unknown pattern kept");
    assert_eq!(p.preset.chars().count(), 64);
    assert!((0.0..=100.0).contains(&p.transparency));
    assert!(craft_docx::write(&d).is_ok());

    // A truncated package is an error, not a panic.
    let good = craft_docx::write(&d).expect("write");
    for cut in (0..good.len()).step_by(97) {
        assert!(craft_docx::read(&good[..cut]).is_err(), "a package cut at {cut} is an error, not a panic");
    }
}

/// A radial gradient's `focusposition` that is not numbers at all still reads as a gradient.
#[test]
fn unreadable_focusposition_keeps_the_radial_gradient() {
    let markup = r#"<w:background><v:background><v:fill type="gradientRadial" color="FF0000" color2="0000FF" focusposition="x,y"/></v:background></w:background>"#;
    let d = read_markup(markup);
    let Some(g) = d.settings.page_gradient.as_ref() else { panic!("gradient kept") };
    assert_eq!(g.kind, GradientKind::Path { shape: PathShape::Circle, focus: [50.0, 50.0, 50.0, 50.0] });
}

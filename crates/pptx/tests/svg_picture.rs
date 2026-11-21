//! SVG pictures: PowerPoint keeps the vector in `asvg:svgBlip` next to a raster fallback in
//! `a:blip/@r:embed`. A file may carry only the vector, and a writer given a rasterizer writes the
//! fallback a reader without SVG support needs beside every vector.

mod common;

use std::sync::Arc;

use common::{bytes_of, deck, export, import, part, rewrite};
use craft_pptx::style::PictureFill;
use craft_pptx::{Fill, MediaId, MediaItem, Plain, Presentation, Shape, ShapeId, ShapeKind, Xfrm};

const SVG: &[u8] =
    b"<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24' viewBox='0 0 24 24'><path d='M0 0 L24 0 L24 24 Z' fill='#ff0000'/></svg>";

/// A 1 by 1 PNG: the raster fallback a writer produces for an SVG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

const SLIDE: &str = "ppt/slides/slide1.xml";
const RELS: &str = "ppt/slides/_rels/slide1.xml.rels";
const IMAGE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";

fn add_media(p: &mut Presentation, name: &str, content_type: &str, data: &[u8]) -> MediaId {
    let id = MediaId(p.alloc_id().unwrap());
    p.media.push(MediaItem { id, name: name.into(), content_type: content_type.into(), data: Arc::new(data.to_vec()), link: None });
    id
}

fn add_picture(p: &mut Presentation, media: MediaId) {
    let slide = Arc::make_mut(p.slides.first_mut().expect("the deck's slide"));
    slide.shapes.push(Shape {
        id: ShapeId(100),
        name: "Picture".into(),
        xfrm: Some(Xfrm::new(0.0, 0.0, 2_400_000.0, 2_400_000.0)),
        kind: ShapeKind::Picture { fill: PictureFill { media, ..Default::default() } },
        ..Default::default()
    });
}

/// A deck with one slide whose picture is the SVG.
fn svg_deck() -> Presentation {
    let mut p = deck();
    let svg = add_media(&mut p, "a.svg", "image/svg+xml", SVG);
    add_picture(&mut p, svg);
    p
}

fn picture(p: &Presentation) -> PictureFill {
    p.slides
        .first()
        .expect("slide")
        .shapes
        .iter()
        .find_map(|s| match &s.kind {
            ShapeKind::Picture { fill } => Some(fill.clone()),
            _ => match &s.fill {
                Some(Fill::Picture(fill)) => Some(fill.clone()),
                _ => None,
            },
        })
        .expect("a picture")
}

fn target(rels: &str, rid: &str) -> String {
    let at = rels.find(&format!("Id=\"{rid}\"")).unwrap_or_else(|| panic!("no {rid} in {rels}"));
    let rest = &rels[at..];
    let start = rest.find("Target=\"").expect("Target") + "Target=\"".len();
    let rest = &rest[start..];
    rest[..rest.find('"').expect("end")].to_string()
}

/// The `r:embed` of the first `<a:blip`, and the one inside its `asvg:svgBlip` when it has one.
fn blip_rels(slide: &str) -> (String, Option<String>) {
    let embed = |from: &str| {
        let start = from.find("r:embed=\"").expect("r:embed") + "r:embed=\"".len();
        let rest = &from[start..];
        rest[..rest.find('"').expect("end")].to_string()
    };
    let at = slide.find("<a:blip").expect("a blip");
    let rid = embed(&slide[at..]);
    let svg = slide.find("asvg:svgBlip").map(|svg_at| embed(&slide[svg_at..]));
    (rid, svg)
}

#[test]
fn a_vector_picture_is_written_with_a_png_fallback_beside_it() {
    let bytes = craft_pptx::export_with(&svg_deck(), &Plain, &|_m| Some(PNG.to_vec())).expect("export");
    let slide = part(&bytes, SLIDE);
    let rels = part(&bytes, RELS);
    let (embed, svg_blip) = blip_rels(&slide);

    // `r:embed` is the raster, the vector rides in the extension, and both parts are in the package.
    let raster = target(&rels, &embed);
    assert!(raster.ends_with(".png"), "r:embed is the raster fallback: {raster}");
    assert_eq!(bytes_of(&bytes, &format!("ppt/{}", raster.trim_start_matches("../"))), PNG);

    let vector = target(&rels, svg_blip.as_deref().expect("the vector extension"));
    assert!(vector.ends_with(".svg"), "asvg:svgBlip is the vector: {vector}");
    assert_eq!(bytes_of(&bytes, &format!("ppt/{}", vector.trim_start_matches("../"))), SVG);

    // Read back, both are there: the app picks which one it draws.
    let fill = picture(&import(&bytes));
    let vector_id = fill.svg.expect("the vector is reported");
    assert_eq!(import(&bytes).media.iter().find(|m| m.id == vector_id).expect("media").data.as_slice(), SVG);
    assert_eq!(import(&bytes).media.iter().find(|m| m.id == fill.media).expect("media").content_type, "image/png");
}

#[test]
fn an_item_without_a_content_type_is_still_a_vector() {
    // The name says what the part is, as the package layer reads it: an item that carries no
    // content type still gets its PNG fallback.
    let mut p = deck();
    let svg = add_media(&mut p, "logo.svg", "", SVG);
    add_picture(&mut p, svg);
    let bytes = craft_pptx::export_with(&p, &Plain, &|_m| Some(PNG.to_vec())).expect("export");
    let (embed, svg_blip) = blip_rels(&part(&bytes, SLIDE));
    assert!(target(&part(&bytes, RELS), &embed).ends_with(".png"), "a raster fallback is written");
    assert!(svg_blip.is_some(), "the vector rides in the extension");
}

const OTHER: &[u8] = b"<svg xmlns='http://www.w3.org/2000/svg' width='4' height='4'><rect width='4' height='4' fill='#00ff00'/></svg>";

#[test]
fn a_fill_names_its_own_vector_over_the_one_the_media_implies() {
    // A fill whose media is one SVG and whose `svg` field names another writes the field's part.
    let mut p = deck();
    let a = add_media(&mut p, "a.svg", "image/svg+xml", SVG);
    let b = add_media(&mut p, "b.svg", "image/svg+xml", OTHER);
    add_picture(&mut p, a);
    if let ShapeKind::Picture { fill } = &mut Arc::make_mut(p.slides.first_mut().expect("slide")).shapes[0].kind {
        fill.svg = Some(b);
    }
    let bytes = craft_pptx::export_with(&p, &Plain, &|_m| Some(PNG.to_vec())).expect("export");
    let rels = part(&bytes, RELS);
    let (_, svg_blip) = blip_rels(&part(&bytes, SLIDE));
    let vector = target(&rels, svg_blip.as_deref().expect("the vector extension"));
    let part_name = format!("ppt/{}", vector.trim_start_matches("../"));
    assert_eq!(bytes_of(&bytes, &part_name), OTHER, "the fill's own vector, not the media's");
}

#[test]
fn without_a_rasterizer_the_vector_stays_in_the_blip() {
    let bytes = export(&svg_deck());
    let slide = part(&bytes, SLIDE);
    assert!(!slide.contains("asvg:svgBlip"), "no fallback without a rasterizer");
    let (embed, _) = blip_rels(&slide);
    assert!(target(&part(&bytes, RELS), &embed).ends_with(".svg"), "the vector itself is r:embed");
}

#[test]
fn a_rasterizer_that_declines_is_asked_once_per_picture_source() {
    // Two pictures share one SVG: a rasterizer that gives up is not called again for the second.
    let mut p = deck();
    let svg = add_media(&mut p, "a.svg", "image/svg+xml", SVG);
    add_picture(&mut p, svg);
    let slide = Arc::make_mut(p.slides.first_mut().expect("slide"));
    let mut second = slide.shapes[0].clone();
    second.id = ShapeId(101);
    slide.shapes.push(second);

    let calls = std::cell::Cell::new(0u32);
    craft_pptx::export_with(&p, &Plain, &|_m| {
        calls.set(calls.get() + 1);
        None
    })
    .expect("export");
    assert_eq!(calls.get(), 1, "the declined draw is remembered");
}

#[test]
fn a_rasterizer_that_declines_leaves_the_vector_alone() {
    let bytes = craft_pptx::export_as_with(&svg_deck(), &Plain, "potx", &|_m| None).expect("export");
    assert!(!part(&bytes, SLIDE).contains("asvg:svgBlip"));
}

#[test]
fn a_picture_with_only_a_vector_is_still_a_picture() {
    // PowerPoint saves a picture with no raster at all: `a:blip` names only the extension.
    let mut p = deck();
    let png = add_media(&mut p, "a.png", "image/png", PNG);
    add_picture(&mut p, png);
    let bytes = export(&p);
    let slide = part(&bytes, SLIDE);
    let at = slide.find("<a:blip").expect("a blip");
    let end = slide[at..].find("/>").map(|i| at + i).expect("self-closing blip");
    let vector_only = format!(
        "{}<a:blip><a:extLst><a:ext uri=\"{{96DAC541-7B7A-43D3-8B79-37D633B846F1}}\">\
         <asvg:svgBlip xmlns:asvg=\"http://schemas.microsoft.com/office/drawing/2016/SVG/main\" r:embed=\"rIdSvg\"/>\
         </a:ext></a:extLst></a:blip>{}",
        &slide[..at],
        &slide[end + 2..]
    );
    let rels = part(&bytes, RELS).replacen(
        "</Relationships>",
        &format!("<Relationship Id=\"rIdSvg\" Type=\"{IMAGE_REL}\" Target=\"../media/a.svg\"/></Relationships>"),
        1,
    );
    let package = rewrite(
        &bytes,
        |name, data| match name {
            SLIDE => Some(vector_only.clone().into_bytes()),
            RELS => Some(rels.clone().into_bytes()),
            "ppt/media/image1.png" => None,
            _ => Some(data.to_vec()),
        },
        &[("ppt/media/a.svg", &String::from_utf8_lossy(SVG))],
    );

    let back = import(&package);
    let fill = picture(&back);
    let media = back.media.iter().find(|m| m.id == fill.media).expect("the picture survived");
    assert_eq!(media.content_type, "image/svg+xml");
    assert_eq!(media.data.as_slice(), SVG);
    assert_eq!(fill.svg, Some(fill.media));
}

//! Round-trip tests for shared SVG syntax across paths, clips, styles, text, and units.

use craft_svg::doc_style::{
    Cap, FillRule, Join, PaintRef, Style, cascade, length_strict, parse_dash_array, parse_font_size, parse_opacity, parse_viewbox,
    symbol_percent_basis, viewbox_transform_with,
};
use craft_svg::path_data::{QuadSegment, parse_path_with_quads};
use craft_svg::preflight::{Limits, parse_text};
use craft_svg::simple_css::{StyleSheet, parse as parse_css, quote_css_string};
use craft_svg::transform_compat::{css_transform_to_svg, parse_transform};
use craft_svg::write::{Attribute, Element, Scene, SvgWriter, path_data};
use kurbo::{BezPath, Point};

#[test]
fn roundtrip_paths_through_writer_and_parser() {
    let mut path = BezPath::new();
    path.move_to((10.0, 20.0));
    path.line_to((40.0, 20.0));
    path.curve_to((50.0, 20.0), (50.0, 40.0), (40.0, 40.0));
    path.close_path();

    let d = path_data(&path);
    let mut w = SvgWriter::new();
    w.root().attr("viewBox", "0 0 100 100").open();
    w.tag("path").attr("d", &d).empty();
    w.close();
    let svg = w.finish();

    let Ok(doc) = parse_text(&svg, &Limits::default()) else {
        unreachable!("valid generated SVG must parse");
    };
    let Some(path_node) = doc.descendants().find(|n| n.has_tag_name("path")) else {
        unreachable!("path element must exist");
    };
    let Some(d_attr) = path_node.attribute("d") else {
        unreachable!("d attribute must exist");
    };
    let Ok(subpaths) = parse_path_with_quads(d_attr) else {
        unreachable!("written path data must parse back");
    };
    assert_eq!(subpaths.len(), 1);
    assert!(subpaths[0].closed);
    assert_eq!(subpaths[0].start.x, 10.0);
    assert_eq!(subpaths[0].start.y, 20.0);
    assert!(matches!(subpaths[0].segs.first(), Some(QuadSegment::Line(_))));
}

#[test]
fn roundtrip_clips_and_styles() {
    let mut w = SvgWriter::new();
    w.root().attr("width", "120mm").attr("height", "80mm").attr("viewBox", "0 0 120 80").open();
    w.tag("defs").open();
    w.tag("style").open();
    w.text(&format!(".clipped {{ fill: #ff0000; stroke: #0000ff; stroke-width: 2mm; font-family: {}; }}", quote_css_string("Noto Sans", '\'')));
    w.close();
    w.tag("clipPath").attr("id", "c1").open();
    w.tag("rect").attr("x", "5").attr("y", "5").attr("width", "50").attr("height", "50").empty();
    w.close();
    w.close();
    w.tag("g")
        .attr("class", "clipped")
        .attr("clip-path", "url(#c1)")
        .attr("style", "stroke-linecap:round;stroke-linejoin:bevel;stroke-dasharray:4 2")
        .open();
    w.tag("rect").attr("x", "0").attr("y", "0").attr("width", "60").attr("height", "60").empty();
    w.close();
    w.close();
    let svg = w.finish();

    let Ok(doc) = parse_text(&svg, &Limits::default()) else {
        unreachable!("valid SVG with clip and style must parse");
    };
    let mut sheet = StyleSheet::default();
    for style_node in doc.descendants().filter(|n| n.has_tag_name("style")) {
        if let Some(css) = style_node.text() {
            assert!(parse_css(css, &mut sheet).is_ok());
        }
    }
    sheet.index();

    let Some(group) = doc.descendants().find(|n| n.has_tag_name("g")) else {
        unreachable!("group element must exist");
    };
    assert_eq!(group.attribute("clip-path"), Some("url(#c1)"));
    let mut st = Style::default();
    let (_opacity, display) = cascade(group, &sheet, &mut st, 120.0, 80.0, |_| true);
    assert!(display);
    assert_eq!(st.fill, Some(PaintRef::Color([1.0, 0.0, 0.0, 1.0])));
    assert_eq!(st.stroke, Some(PaintRef::Color([0.0, 0.0, 1.0, 1.0])));
    assert_eq!(st.cap, Cap::Round);
    assert_eq!(st.join, Join::Bevel);
    assert_eq!(st.dash, Some(vec![4.0, 2.0]));
    assert_eq!(st.fill_rule, FillRule::NonZero);
}

#[test]
fn roundtrip_text_and_units() {
    let scene = Scene {
        width_mm: 100.0,
        height_mm: 50.0,
        view_box: [0.0, 0.0, 100.0, 50.0],
        elements: vec![Element::Text {
            attributes: vec![Attribute::text("font-size", "12pt"), Attribute::text("opacity", "50%")],
            x: 10.0,
            y: 25.0,
            text: "Hello <SVG> & \"units\"".to_string(),
        }],
    };
    let svg = scene.write();
    let Ok(doc) = parse_text(&svg, &Limits::default()) else {
        unreachable!("written Scene must parse");
    };
    let root = doc.root_element();
    assert_eq!(root.attribute("width"), Some("100.000mm"));
    assert_eq!(length_strict("100.000mm", 100.0), Some(100.0 * 96.0 / 25.4));
    let Some(vb) = root.attribute("viewBox").and_then(parse_viewbox) else {
        unreachable!("viewBox must parse");
    };
    let xf = viewbox_transform_with(vb, 200.0, 100.0, "xMidYMid meet");
    assert_eq!(xf * Point::new(10.0, 25.0), Point::new(20.0, 50.0));

    let Some(text_el) = doc.descendants().find(|n| n.has_tag_name("text")) else {
        unreachable!("text element must exist");
    };
    assert_eq!(text_el.text(), Some("Hello <SVG> & \"units\""));
    assert_eq!(text_el.attribute("font-size").and_then(|v| parse_font_size(v, 16.0, 100.0)), Some(16.0));
    assert_eq!(text_el.attribute("opacity").and_then(parse_opacity), Some(0.5));
    assert_eq!(parse_dash_array("4, 2", 100.0), Some(vec![4.0, 2.0]));
    assert_eq!(symbol_percent_basis("width", 200.0, 100.0), Some(200.0));
    assert_eq!(css_transform_to_svg("translate(10px, 20px)").map(|t| parse_transform(&t)), Some(kurbo::Affine::translate((10.0, 20.0))));
}

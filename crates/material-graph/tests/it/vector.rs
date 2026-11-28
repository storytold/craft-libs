// SPDX-License-Identifier: MIT OR Apache-2.0
use craft_material_graph::font::{Font, Ttf};
use craft_material_graph::graph::Graph;
use craft_material_graph::vector::*;

fn alpha_sum(t: &craft_material_graph::tex::Tex) -> f32 {
    t.px.iter().map(|p| p[3]).sum()
}

#[test]
fn svg_shapes_and_transforms() {
    let rect = r##"<svg viewBox="0 0 10 10"><rect x="2" y="2" width="6" height="6" fill="#ff0000"/></svg>"##;
    let t = render_svg(rect, 100, 100, 1, 1.0, 0.0, 0.0).unwrap();
    // 6x6 of 10x10 = 36% of the image, red, anti-aliased edges
    assert!((alpha_sum(&t) - 3600.0).abs() < 40.0, "area {}", alpha_sum(&t));
    assert_eq!(t.px[50 * 100 + 50], [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(t.px[0][3], 0.0);
    // circle area
    let circ = r#"<svg viewBox="0 0 100 100"><circle cx="50" cy="50" r="40"/></svg>"#;
    let t = render_svg(circ, 200, 200, 1, 1.0, 0.0, 0.0).unwrap();
    let want = std::f32::consts::PI * 80.0 * 80.0;
    assert!((alpha_sum(&t) - want).abs() / want < 0.01, "circle area {} vs {want}", alpha_sum(&t));
    // a group transform moves the shape; stroke only draws an outline
    let moved = r#"<svg viewBox="0 0 10 10"><g transform="translate(5 0)"><rect width="5" height="10"/></g></svg>"#;
    let t = render_svg(moved, 10, 10, 1, 1.0, 0.0, 0.0).unwrap();
    assert!(t.px[5][3] > 0.99 && t.px[4][3] < 0.01);
    let stroked = r#"<svg viewBox="0 0 10 10"><rect x="1" y="1" width="8" height="8" fill="none" stroke="blue" stroke-width="1"/></svg>"#;
    let t = render_svg(stroked, 100, 100, 1, 1.0, 0.0, 0.0).unwrap();
    assert!(t.px[50 * 100 + 50][3] < 0.01, "inside stays empty");
    assert!(t.px[50 * 100 + 10][3] > 0.9, "stroke on the border");
    // paths: relative commands, arcs, close
    let path = r#"<svg viewBox="0 0 10 10"><path d="M1 1 h8 v8 h-8 z M3 3 h4 v4 h-4 z" fill-rule="evenodd"/></svg>"#;
    let t = render_svg(path, 100, 100, 1, 1.0, 0.0, 0.0).unwrap();
    assert!(t.px[50 * 100 + 50][3] < 0.01, "even-odd hole");
    assert!(t.px[20 * 100 + 20][3] > 0.99);
    let arc = r#"<svg viewBox="0 0 10 10"><path d="M1 5 A4 4 0 0 1 9 5 A4 4 0 0 1 1 5 Z"/></svg>"#;
    let t = render_svg(arc, 100, 100, 1, 1.0, 0.0, 0.0).unwrap();
    let want = std::f32::consts::PI * 40.0 * 40.0;
    assert!((alpha_sum(&t) - want).abs() / want < 0.02, "arc circle {}", alpha_sum(&t));
    assert!(render_svg("<html/>", 8, 8, 0, 1.0, 0.0, 0.0).is_err());
    // garbage path data never panics
    let _ = parse_path("M 1 2 L 3 q a b c 1e");
}

#[test]
fn svg_and_text_nodes() {
    let svg = r##"<svg viewBox="0 0 10 10"><rect width="10" height="10" fill="#00ff00"/></svg>"##;
    let b64 = |s: &str| {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let d = s.as_bytes();
        let mut o = String::new();
        for c in d.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                if i <= c.len() {
                    o.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
                } else {
                    o.push('=');
                }
            }
        }
        o
    };
    let text =
        format!("size 32\nresource a.svg {}\nnode 1 svg file=\"a.svg\" fit=1\nnode 2 text text=\"HI\" size=0.5\noutput a 1\noutput b 2\n", b64(svg));
    let g = Graph::parse(&text).unwrap();
    let out = g.evaluate(Some(32)).unwrap();
    assert_eq!(out["a"].px[16 * 32 + 16], [0.0, 1.0, 0.0, 1.0]);
    let lit = out["b"].px.iter().filter(|p| p[0] > 0.5).count();
    assert!(lit > 20 && lit < 600, "text pixels {lit}");
    // the text is centred horizontally: lit pixels on both halves
    let left = out["b"].px.iter().enumerate().filter(|(i, p)| i % 32 < 16 && p[0] > 0.5).count();
    assert!(left > 5 && lit - left > 5);
    // missing files are errors
    assert!(Graph::parse("size 8\nnode 1 svg file=\"nope.svg\"\noutput a 1\n").unwrap().evaluate(Some(8)).is_err());
}

#[test]
fn builtin_font_layout() {
    let f = Font::Builtin;
    let polys = f.layout("Ag\nb", 20.0, 1, 0.0, 1.0, &Xf::translate(50.0, 50.0));
    assert!(!polys.is_empty());
    let m = fill(&polys, 100, 100, false);
    let lit = m.a.iter().filter(|a| **a > 0.5).count();
    assert!(lit > 50);
    assert!(Ttf::parse(b"not a font").is_err());
    assert!(Ttf::parse(&[0u8; 3]).is_err());
}

#[test]
fn truetype_system_font_if_present() {
    for path in [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            let f = Font::load(&bytes).unwrap();
            let polys = f.layout("Wo", 40.0, 1, 0.0, 1.0, &Xf::translate(50.0, 50.0));
            let m = fill(&polys, 100, 100, false);
            let lit = m.a.iter().filter(|a| **a > 0.5).count();
            assert!(lit > 100 && lit < 5000, "{path}: lit {lit}");
            return;
        }
    }
}

#[test]
fn text_y_counts_up_with_y_up() {
    // the mean row of the lit pixels: y=0.25 moves the text down, with y_up=1 up
    let row = |extra: &str| {
        let g = Graph::parse(&format!("size 32\nnode 1 text text=\"HI\" size=0.3 y=0.25{extra}\noutput o 1\n")).unwrap();
        let out = g.evaluate(Some(32)).unwrap();
        let lit: Vec<usize> = out["o"].px.iter().enumerate().filter(|(_, p)| p[0] > 0.5).map(|(i, _)| i / 32).collect();
        assert!(!lit.is_empty(), "no text pixels{extra}");
        lit.iter().sum::<usize>() as f32 / lit.len() as f32
    };
    let (down, up) = (row(""), row(" y_up=1"));
    assert!(down > 20.0 && up < 12.0, "rows {down} {up}");
}

//! Writing `.cmx` files: what [`write_cmx`] writes reads back through [`read_cmx`] as the same
//! drawing, and what it can't hold is refused or reported.

use craft_cdr::model::{
    Cap, Color, Drawing, Flags, Geometry, Join, Layer, LayerKind, Matrix, Object, Outline, Page, Path, Pt, Segment, Shape, SubPath,
};
use craft_cdr::{Error, Limits, MAX_CURVE_POINTS, Written, is_cmx, read_cmx};
use proptest::prelude::*;

/// [`craft_cdr::write_cmx`] within the default limits.
fn write_cmx(d: &Drawing) -> Result<Written, Error> {
    craft_cdr::write_cmx(d, Limits::default())
}

fn layer(name: &str, objects: Vec<Object>) -> Layer {
    Layer { name: name.into(), kind: LayerKind::Drawing, flags: Flags::default(), objects, guides: Vec::new() }
}

fn drawing(pages: Vec<Page>) -> Drawing {
    Drawing { version: 0, page_size: None, pages, master: Vec::new(), guides: Vec::new(), warnings: Vec::new() }
}

fn square(x: f64, y: f64, side: f64) -> Geometry {
    let p = |x, y| Pt { x, y };
    Geometry::Path(Path {
        subpaths: vec![SubPath {
            start: p(x, y),
            segments: vec![Segment::Line(p(x + side, y)), Segment::Line(p(x + side, y + side)), Segment::Line(p(x, y + side))],
            closed: true,
        }],
    })
}

fn outline(color: Color) -> Outline {
    Outline { width: 2.0, color, cap: Cap::Round, join: Join::Bevel, dashes: Vec::new(), behind_fill: false, scale_with: false }
}

fn shape(o: &Object) -> &Shape {
    match o {
        Object::Shape(s) => s,
        other => panic!("not a shape: {other:?}"),
    }
}

/// Every point (anchors and controls) of a geometry that is a path.
fn points(g: &Geometry) -> Vec<Pt> {
    let Geometry::Path(p) = g else { panic!("not a path: {g:?}") };
    let mut out = Vec::new();
    for sp in &p.subpaths {
        out.push(sp.start);
        for s in &sp.segments {
            match s {
                Segment::Line(a) => out.push(*a),
                Segment::Cubic(a, b, c) => out.extend([*a, *b, *c]),
            }
        }
    }
    out
}

fn bounds(g: &Geometry) -> (f64, f64, f64, f64) {
    points(g).iter().fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |b, p| (b.0.min(p.x), b.1.min(p.y), b.2.max(p.x), b.3.max(p.y)))
}

/// Within a coordinate unit of 0.1 µm (0.000283 pt), in points.
fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.0003
}

fn near_color(a: &Color, b: &Color) -> bool {
    let close = |x: &[f32], y: &[f32], tol: f32| x.len() == y.len() && x.iter().zip(y).all(|(p, q)| (p - q).abs() <= tol);
    match (a, b) {
        (Color::Rgb(x), Color::Rgb(y)) => close(x, y, 1.0 / 255.0),
        (Color::Cmyk(x), Color::Cmyk(y)) => close(x, y, 0.005),
        (Color::Gray(x), Color::Gray(y)) => (x - y).abs() <= 1.0 / 255.0,
        (Color::Registration, Color::Registration) => true,
        (Color::Spot { name: n, tint: t, look: l }, Color::Spot { name: m, tint: u, look: k }) => {
            n == m && (t - u).abs() <= 0.005 && close(&l.unwrap_or_default(), &k.unwrap_or_default(), 2.0 / 255.0)
        }
        _ => false,
    }
}

fn round_trip(d: &Drawing) -> Drawing {
    let w = write_cmx(d).unwrap();
    assert!(is_cmx(&w.bytes));
    let back = read_cmx(&w.bytes, Limits::default()).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    back
}

#[test]
fn a_drawing_reads_back_as_written() {
    let red = Color::Rgb([1.0, 0.2, 0.0]);
    let cyan = Color::Cmyk([0.6, 0.0, 0.13, 0.05]);
    let spot = Color::Spot { name: "Ink 300".into(), tint: 0.4, look: Some([0.0, 0.4, 0.8]) };
    let dashed =
        Outline { dashes: vec![3.0, 1.0], behind_fill: true, scale_with: true, cap: Cap::Square, join: Join::Miter, ..outline(Color::Gray(0.75)) };
    let back_layer = layer(
        "Art",
        vec![
            Object::Shape(Box::new(Shape::new(square(-50.0, -50.0, 100.0), Matrix::IDENTITY, Some(red.clone()), Some(outline(Color::Registration))))),
            Object::Group {
                objects: vec![
                    Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 10.0), Matrix::IDENTITY, Some(cyan.clone()), Some(dashed.clone())))),
                    Object::Shape(Box::new(Shape::new(square(20.0, 0.0, 10.0), Matrix::IDENTITY, Some(spot.clone()), None))),
                ],
                flags: Flags::default(),
            },
        ],
    );
    let front_layer =
        layer("Über", vec![Object::Shape(Box::new(Shape::new(square(5.0, 5.0, 1.0), Matrix::IDENTITY, None, Some(outline(cyan.clone())))))]);
    let d = drawing(vec![
        Page { size: Some((595.0, 842.0)), layers: vec![front_layer, back_layer] },
        Page {
            size: Some((200.0, 100.0)),
            layers: vec![layer(
                "Second",
                vec![Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.3)), None)))],
            )],
        },
    ]);
    let back = round_trip(&d);
    assert_eq!(back.pages.len(), 2);
    let (w, h) = back.pages[0].size.unwrap();
    assert!(near(w, 595.0) && near(h, 842.0));
    let names: Vec<&str> = back.pages[0].layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Über", "Art"], "layers keep their order and names");
    let art = &back.pages[0].layers[1].objects;
    let s = shape(&art[0]);
    assert!(near_color(s.fill.as_ref().unwrap(), &red));
    assert_eq!(s.outline.as_ref().unwrap().color, Color::Registration);
    let (x0, y0, x1, y1) = bounds(&s.geometry);
    assert!(near(x0, -50.0) && near(y0, -50.0) && near(x1, 50.0) && near(y1, 50.0));
    let Object::Group { objects, .. } = &art[1] else { panic!("not a group: {:?}", art[1]) };
    let o = shape(&objects[0]).outline.clone().unwrap();
    assert!(near_color(shape(&objects[0]).fill.as_ref().unwrap(), &cyan));
    assert!(near(o.width, 2.0) && near_color(&o.color, &Color::Gray(0.75)));
    assert_eq!((o.cap, o.join, o.dashes, o.behind_fill, o.scale_with), (Cap::Square, Join::Miter, vec![3.0, 1.0], true, true));
    assert!(near_color(shape(&objects[1]).fill.as_ref().unwrap(), &spot), "{:?}", shape(&objects[1]).fill);
    assert!(shape(&objects[1]).outline.is_none());
    let front = shape(&back.pages[0].layers[0].objects[0]);
    assert!(front.fill.is_none() && front.outline.as_ref().is_some_and(|o| o.cap == Cap::Round && o.join == Join::Bevel));
    let (w, h) = back.pages[1].size.unwrap();
    assert!(near(w, 200.0) && near(h, 100.0));
}

#[test]
fn rectangles_and_ellipses_are_written_as_curves_in_their_place() {
    let m = Matrix([1.0, 0.0, 0.0, 1.0, 10.0, -20.0]);
    let rect = Shape::new(Geometry::Rect { w: 40.0, h: -30.0, radii: [5.0, 0.0, 50.0, 2.0] }, m, Some(Color::Gray(1.0)), None);
    let ellipse = Shape::new(Geometry::Ellipse { w: 20.0, h: 10.0 }, m, Some(Color::Gray(1.0)), None);
    let d = drawing(vec![Page { size: None, layers: vec![layer("", vec![Object::Shape(Box::new(rect)), Object::Shape(Box::new(ellipse))])] }]);
    let back = round_trip(&d);
    let objects = &back.pages[0].layers[0].objects;
    let (x0, y0, x1, y1) = bounds(&shape(&objects[0]).geometry);
    assert!(near(x0, 10.0) && near(y0, -50.0) && near(x1, 50.0) && near(y1, -20.0), "{:?}", (x0, y0, x1, y1));
    let Geometry::Path(p) = &shape(&objects[1]).geometry else { panic!() };
    assert!(p.subpaths[0].closed && p.subpaths[0].segments.iter().all(|s| matches!(s, Segment::Cubic(..))));
    let (x0, y0, x1, y1) = bounds(&shape(&objects[1]).geometry);
    assert!(near(x0, 10.0) && near(x1, 30.0) && near(y0, -20.0) && near(y1, -10.0));
}

#[test]
fn a_path_past_the_points_a_curve_holds_is_written_as_a_group_of_curves() {
    let n = MAX_CURVE_POINTS * 2 + 10;
    let segments = (1..n).map(|i| Segment::Line(Pt { x: i as f64 * 0.01, y: (i % 2) as f64 })).collect();
    let long = Shape::new(
        Geometry::Path(Path { subpaths: vec![SubPath { start: Pt { x: 0.0, y: 0.0 }, segments, closed: false }] }),
        Matrix::IDENTITY,
        None,
        Some(outline(Color::Gray(1.0))),
    );
    let d = drawing(vec![Page { size: None, layers: vec![layer("", vec![Object::Shape(Box::new(long))])] }]);
    let w = write_cmx(&d).unwrap();
    assert!(w.split_paths && w.cut_subpaths, "an outline gets caps where the pieces meet");
    let back = read_cmx(&w.bytes, Limits::default()).unwrap();
    let Object::Group { objects, .. } = &back.pages[0].layers[0].objects[0] else { panic!("not a group") };
    assert!(objects.len() >= 3);
    // The pieces meet end to start, so every point is kept, the joins twice.
    let total: usize = objects.iter().map(|o| points(&shape(o).geometry).len()).sum();
    assert_eq!(total, n + objects.len() - 1);
    // A filled shape cut into pieces is reported.
    let Object::Shape(s) = &d.pages[0].layers[0].objects[0] else { panic!() };
    let filled = Shape { fill: Some(Color::Gray(0.5)), ..(**s).clone() };
    let d = drawing(vec![Page { size: None, layers: vec![layer("", vec![Object::Shape(Box::new(filled))])] }]);
    assert!(write_cmx(&d).unwrap().cut_subpaths);
}

#[test]
fn names_the_reader_shortens_are_reported() {
    // A name as long as the reader keeps whole reads back whole.
    let kept = "k".repeat(256);
    let plain = Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, None, None)));
    let d = drawing(vec![Page { size: None, layers: vec![layer(&kept, vec![plain])] }]);
    let w = write_cmx(&d).unwrap();
    assert!(!w.long_names);
    assert_eq!(read_cmx(&w.bytes, Limits::default()).unwrap().pages[0].layers[0].name, kept);
    let long = "n".repeat(257);
    let spot = |name: &str| Color::Spot { name: name.into(), tint: 1.0, look: None };
    let s = Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(spot(&long)), None);
    let short = drawing(vec![Page { size: None, layers: vec![layer("L", vec![Object::Shape(Box::new(s.clone()))])] }]);
    assert!(write_cmx(&short).unwrap().long_names, "a long spot name");
    let named = drawing(vec![Page { size: None, layers: vec![layer(&long, vec![Object::Shape(Box::new(Shape { fill: None, ..s.clone() }))])] }]);
    assert!(write_cmx(&named).unwrap().long_names, "a long layer name");
    // Two long spot names that differ only past what the reader keeps still read back apart.
    let other = format!("{}m", "n".repeat(256));
    let t = Shape { fill: Some(spot(&other)), ..s.clone() };
    let both = drawing(vec![Page { size: None, layers: vec![layer("L", vec![Object::Shape(Box::new(s)), Object::Shape(Box::new(t))])] }]);
    let back = read_cmx(&write_cmx(&both).unwrap().bytes, Limits::default()).unwrap();
    let names: Vec<String> = back.pages[0].layers[0]
        .objects
        .iter()
        .filter_map(|o| match shape(o).fill.as_ref() {
            Some(Color::Spot { name, .. }) => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert!(names.len() == 2 && names[0] != names[1], "{names:?}");
}

#[test]
fn a_spot_color_at_no_tint_is_reported() {
    let spot = |tint| Color::Spot { name: "Ink".into(), tint, look: Some([0.0, 0.0, 1.0]) };
    let file = |c| {
        let s = Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(c), None);
        drawing(vec![Page { size: None, layers: vec![layer("L", vec![Object::Shape(Box::new(s))])] }])
    };
    assert!(write_cmx(&file(spot(0.0))).unwrap().zero_tints);
    assert!(!write_cmx(&file(spot(0.05))).unwrap().zero_tints);
}

#[test]
fn coordinates_past_the_32_bit_range_are_refused() {
    let far = Shape::new(square(1e6, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(1.0)), None);
    let d = drawing(vec![Page { size: None, layers: vec![layer("", vec![Object::Shape(Box::new(far))])] }]);
    assert_eq!(write_cmx(&d), Err(Error::Limit("coordinate range")));
}

#[test]
fn dash_lengths_are_rounded_to_whole_outline_widths() {
    let o = Outline { dashes: vec![2.4, 0.2], ..outline(Color::Gray(1.0)) };
    let s = Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, None, Some(o));
    let d = drawing(vec![Page { size: None, layers: vec![layer("", vec![Object::Shape(Box::new(s))])] }]);
    let w = write_cmx(&d).unwrap();
    assert!(w.rounded_dashes);
    let back = read_cmx(&w.bytes, Limits::default()).unwrap();
    assert_eq!(shape(&back.pages[0].layers[0].objects[0]).outline.as_ref().unwrap().dashes, vec![2.0, 0.0]);
}

fn one_shape(s: Shape) -> Drawing {
    drawing(vec![Page { size: None, layers: vec![layer("L", vec![Object::Shape(Box::new(s))])] }])
}

#[test]
fn a_page_without_its_own_size_takes_the_drawings() {
    let mut d = one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None));
    d.page_size = Some((300.0, 200.0));
    let (w, h) = round_trip(&d).pages[0].size.unwrap();
    assert!(near(w, 300.0) && near(h, 200.0), "{w} {h}");
}

#[test]
fn pages_are_numbered_in_their_order() {
    let page = |side| Page {
        size: Some((100.0, 100.0)),
        layers: vec![layer("L", vec![Object::Shape(Box::new(Shape::new(square(0.0, 0.0, side), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)))])],
    };
    let bytes = write_cmx(&drawing(vec![page(1.0), page(2.0)])).unwrap().bytes;
    // Each page chunk starts with its begin-page instruction: a u16 size, the code 9, then field 1
    // (a tag and a u16 length) whose first u16 is the page number.
    let numbers: Vec<u16> = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == b"page")
        .map(|(i, _)| {
            let body = &bytes[i + 8..];
            assert_eq!(i16::from_le_bytes([body[2], body[3]]), 9);
            u16::from_le_bytes([body[7], body[8]])
        })
        .collect();
    assert_eq!(numbers, [1, 2]);
}

/// Field 1 of each entry of the table chunk `id`: a u16 count, then a field list per entry (a tag,
/// a u16 length counting the 3 header bytes, the data; 0xff after the last field).
fn table_entries(bytes: &[u8], id: &[u8; 4]) -> Vec<Vec<u8>> {
    let at = bytes.windows(4).position(|w| w == id).unwrap_or_else(|| panic!("no {id:?} chunk")) + 8;
    // The chunk's body runs for the size after its id.
    let end = at + u32::from_le_bytes(bytes[at - 4..at].try_into().unwrap()) as usize;
    assert!(end <= bytes.len(), "{id:?} chunk ends at {end}, past the file");
    let count = usize::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]));
    let mut p = at + 2;
    let mut out = Vec::new();
    for _ in 0..count {
        let mut first = None;
        loop {
            assert!(p < end, "entry runs past the {id:?} chunk at {p}");
            if bytes[p] == 0xff {
                break;
            }
            assert!(p + 3 <= end, "field header at {p} runs past the {id:?} chunk");
            let len = usize::from(u16::from_le_bytes([bytes[p + 1], bytes[p + 2]]));
            // A length shorter than its header, or past the chunk, would stop the walk.
            assert!(len >= 3 && p + len <= end, "field at {p} has length {len}");
            if bytes[p] == 1 {
                first = Some(bytes[p + 3..p + len].to_vec());
            }
            p += len;
        }
        p += 1;
        out.push(first.expect("field 1"));
    }
    // The size counts the u16 count and the entries; the pad byte comes after it.
    assert_eq!(p, end, "{id:?} chunk has bytes left after its entries");
    out
}

#[test]
fn every_outline_names_a_screen_and_a_dash_pattern_the_file_holds() {
    let solid = Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, None, Some(outline(Color::Gray(1.0))));
    let dashed = Shape::new(square(2.0, 0.0, 1.0), Matrix::IDENTITY, None, Some(Outline { dashes: vec![3.0, 1.0], ..outline(Color::Gray(1.0)) }));
    let d = drawing(vec![Page { size: None, layers: vec![layer("L", vec![Object::Shape(Box::new(solid)), Object::Shape(Box::new(dashed))])] }]);
    let bytes = write_cmx(&d).unwrap().bytes;
    let (screens, dashes) = (table_entries(&bytes, b"rscr"), table_entries(&bytes, b"rdot"));
    let (line_types, outlines) = (table_entries(&bytes, b"rott"), table_entries(&bytes, b"rotl"));
    assert_eq!(outlines.len(), 2);
    let mut patterns: Vec<(bool, Vec<u16>)> = outlines
        .iter()
        .map(|o| {
            let entry = |i: usize| usize::from(u16::from_le_bytes([o[2 * i], o[2 * i + 1]]));
            assert!((1..=screens.len()).contains(&entry(1)), "the screen entry exists");
            let line = entry(0);
            assert!((1..=line_types.len()).contains(&line), "the line type entry exists");
            let dash = entry(5);
            assert!((1..=dashes.len()).contains(&dash), "the dash entry exists");
            let pattern = &dashes[dash - 1];
            let count = usize::from(u16::from_le_bytes([pattern[0], pattern[1]]));
            assert_eq!(pattern.len(), 2 + 2 * count);
            let lengths = pattern[2..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            // The outline's own line type says whether it is dashed.
            (line_types[line - 1][0] & 4 != 0, lengths)
        })
        .collect();
    patterns.sort();
    // The solid outline names an empty pattern; the dashed one names its two lengths.
    assert_eq!(patterns, vec![(false, vec![]), (true, vec![3, 1])]);
}

#[test]
fn an_outline_that_scales_with_its_shape_keeps_its_drawn_width() {
    let m = Matrix([3.0, 0.0, 0.0, 3.0, 0.0, 0.0]);
    let width = |scale_with| {
        let o = Outline { scale_with, dashes: vec![2.0, 1.0], ..outline(Color::Gray(1.0)) };
        let back = round_trip(&one_shape(Shape::new(square(0.0, 0.0, 1.0), m, None, Some(o))));
        let o = shape(&back.pages[0].layers[0].objects[0]).outline.clone().unwrap();
        assert_eq!(o.dashes, vec![2.0, 1.0], "dashes stay in outline widths");
        o.width
    };
    assert!(near(width(true), 6.0) && near(width(false), 2.0));
}

#[test]
fn a_dash_pattern_of_zero_lengths_stays_solid() {
    let o = Outline { dashes: vec![0.0, 0.0], ..outline(Color::Gray(1.0)) };
    let w = write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, None, Some(o)))).unwrap();
    assert!(!w.rounded_dashes);
    let back = read_cmx(&w.bytes, Limits::default()).unwrap();
    assert!(shape(&back.pages[0].layers[0].objects[0]).outline.as_ref().unwrap().dashes.is_empty());
}

#[test]
fn a_spot_color_at_a_low_tint_keeps_its_ink() {
    let black = Color::Spot { name: "Ink".into(), tint: 0.0051, look: Some([0.0, 0.0, 0.0]) };
    let back = round_trip(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(black), None)));
    let Some(Color::Spot { look: Some(look), .. }) = shape(&back.pages[0].layers[0].objects[0]).fill.clone() else { panic!("not a spot color") };
    assert!(look.iter().all(|v| *v < 0.1), "{look:?}");
}

#[test]
fn hidden_art_is_left_out_and_what_the_file_cant_hold_is_reported() {
    let plain = || Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None);
    assert!(!write_cmx(&one_shape(plain())).unwrap().left_out);
    let hidden = Shape { flags: Flags { hidden: true, locked: false }, ..plain() };
    let mut hidden_layer = layer("Hidden", vec![Object::Shape(Box::new(plain()))]);
    hidden_layer.flags.hidden = true;
    let d = drawing(vec![Page {
        size: None,
        layers: vec![hidden_layer, layer("Shown", vec![Object::Shape(Box::new(hidden)), Object::Shape(Box::new(plain()))])],
    }]);
    let w = write_cmx(&d).unwrap();
    assert!(w.left_out);
    let back = read_cmx(&w.bytes, Limits::default()).unwrap();
    assert_eq!(back.pages[0].layers.len(), 1);
    assert_eq!(back.pages[0].layers[0].objects.len(), 1);
    assert!(write_cmx(&one_shape(Shape { opacity: 0.5, ..plain() })).unwrap().left_out, "transparency");
    let mut master = one_shape(plain());
    master.master = vec![layer("Master", vec![Object::Shape(Box::new(plain()))])];
    assert!(write_cmx(&master).unwrap().left_out, "a master layer");
}

#[test]
fn a_clipping_containers_outline_is_drawn_over_what_it_clips() {
    let container = Shape::new(square(0.0, 0.0, 10.0), Matrix::IDENTITY, Some(Color::Gray(0.2)), Some(outline(Color::Gray(1.0))));
    let child = Shape::new(square(2.0, 2.0, 5.0), Matrix::IDENTITY, Some(Color::Gray(0.8)), None);
    let clip = Object::Clip { container: Box::new(container), objects: vec![Object::Shape(Box::new(child))] };
    let d = drawing(vec![Page { size: None, layers: vec![layer("L", vec![clip])] }]);
    let w = write_cmx(&d).unwrap();
    assert!(w.left_out, "the clip itself is not written");
    let back = read_cmx(&w.bytes, Limits::default()).unwrap();
    let Object::Group { objects, .. } = &back.pages[0].layers[0].objects[0] else { panic!("not a group") };
    // Front to back: the container's outline, the clipped shape, the container's fill.
    let kinds: Vec<(bool, bool)> = objects.iter().map(|o| (shape(o).fill.is_some(), shape(o).outline.is_some())).collect();
    assert_eq!(kinds, [(false, true), (true, false), (true, false)]);
    assert!(near_color(shape(&objects[2]).fill.as_ref().unwrap(), &Color::Gray(0.2)));
}

#[test]
fn a_hidden_clipping_container_hides_what_it_clips() {
    let mut container = Shape::new(square(0.0, 0.0, 10.0), Matrix::IDENTITY, Some(Color::Gray(0.2)), None);
    container.flags.hidden = true;
    let child = Shape::new(square(2.0, 2.0, 5.0), Matrix::IDENTITY, Some(Color::Gray(0.8)), None);
    let clip = Object::Clip { container: Box::new(container), objects: vec![Object::Shape(Box::new(child))] };
    let shown = Object::Shape(Box::new(Shape::new(square(20.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)));
    let w = write_cmx(&drawing(vec![Page { size: None, layers: vec![layer("L", vec![clip, shown])] }])).unwrap();
    assert!(w.left_out);
    let back = read_cmx(&w.bytes, Limits::default()).unwrap();
    assert_eq!(back.pages[0].layers[0].objects.len(), 1);
    assert!(matches!(back.pages[0].layers[0].objects[0], Object::Shape(_)));
}

#[test]
fn guide_and_grid_layers_are_left_out() {
    let art = || vec![Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)))];
    let kinds = [LayerKind::Guides, LayerKind::Grid];
    let mut layers: Vec<Layer> = kinds.iter().map(|k| Layer { kind: *k, ..layer("Construction", art()) }).collect();
    layers.push(layer("Art", art()));
    let w = write_cmx(&drawing(vec![Page { size: None, layers }])).unwrap();
    assert!(w.left_out);
    let back = read_cmx(&w.bytes, Limits::default()).unwrap();
    let names: Vec<&str> = back.pages[0].layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Art"]);
}

#[test]
fn locked_art_is_reported() {
    let mut s = Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None);
    s.flags.locked = true;
    assert!(write_cmx(&one_shape(s)).unwrap().left_out);
}

#[test]
fn nonzero_fills_that_may_fill_differently_are_reported() {
    let p = |x, y| Pt { x, y };
    // Two squares turning the same way, one inside the other: nonzero fills the inner one,
    // even-odd leaves it a hole.
    let ring = |cw: bool, x: f64, side: f64| {
        let corners = [p(x, x), p(x + side, x), p(x + side, x + side), p(x, x + side)];
        let pts: Vec<Pt> = if cw { corners.into_iter().rev().collect() } else { corners.to_vec() };
        SubPath { start: pts[0], segments: pts[1..].iter().map(|q| Segment::Line(*q)).collect(), closed: true }
    };
    let filled = |inner_cw: bool, nonzero: bool| {
        let geometry = Geometry::Path(Path { subpaths: vec![ring(false, 0.0, 10.0), ring(inner_cw, 2.0, 5.0)] });
        let s = Shape { nonzero, ..Shape::new(geometry, Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
        write_cmx(&one_shape(s)).unwrap().nonzero_fills
    };
    assert!(filled(false, true), "same turn, nested");
    assert!(!filled(true, true), "a counter turning the other way fills the same");
    assert!(!filled(false, false), "an even-odd fill is written as it is");
}

#[test]
fn a_subpath_crossing_itself_with_one_net_turn_is_reported() {
    let p = |x, y| Pt { x, y };
    // One net turn, but the inside around (1.25, 2.5) is wound twice.
    let pts = [p(4.0, 1.0), p(3.0, 1.0), p(0.0, 4.0), p(0.0, 1.0), p(4.0, 4.0), p(0.0, 3.0)];
    let sub = SubPath { start: pts[0], segments: pts[1..].iter().map(|q| Segment::Line(*q)).collect(), closed: true };
    let s = Shape { nonzero: true, ..Shape::new(Geometry::Path(Path { subpaths: vec![sub] }), Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(write_cmx(&one_shape(s)).unwrap().nonzero_fills);
    // A plain square crosses nothing.
    let plain = Shape { nonzero: true, ..Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(!write_cmx(&one_shape(plain)).unwrap().nonzero_fills);
}

#[test]
fn a_curve_crossing_where_its_control_points_dont_is_reported() {
    let p = |x, y| Pt { x, y };
    // M0 0 C0 10 10 10 10 0 L12 16 -4 11 -4 -1 12 -6 2 9 Z winds twice around (2.5, 7.5).
    let mut segments = vec![Segment::Cubic(p(0.0, 10.0), p(10.0, 10.0), p(10.0, 0.0))];
    segments.extend([p(12.0, 16.0), p(-4.0, 11.0), p(-4.0, -1.0), p(12.0, -6.0), p(2.0, 9.0)].map(Segment::Line));
    let geometry = Geometry::Path(Path { subpaths: vec![SubPath { start: p(0.0, 0.0), segments, closed: true }] });
    let s = Shape { nonzero: true, ..Shape::new(geometry, Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(write_cmx(&one_shape(s)).unwrap().nonzero_fills);
}

#[test]
fn a_closing_line_back_to_the_start_is_no_crossing() {
    let p = |x, y| Pt { x, y };
    // Clockwise, with the closing line written out.
    let segments = [p(0.0, 10.0), p(10.0, 10.0), p(10.0, 0.0), p(0.0, 0.0)].map(Segment::Line).to_vec();
    let geometry = Geometry::Path(Path { subpaths: vec![SubPath { start: p(0.0, 0.0), segments, closed: true }] });
    let s = Shape { nonzero: true, ..Shape::new(geometry, Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(!write_cmx(&one_shape(s)).unwrap().nonzero_fills);
}

#[test]
fn an_ink_takes_its_look_from_its_strongest_tint_and_from_master_layers() {
    let ink = |tint, look| Color::Spot { name: "Ink".into(), tint, look };
    let filled = |c| Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(c), None)));
    let red = Some([1.0, 0.0, 0.0]);
    // The 0% use, whose look is white, is met first.
    let d = drawing(vec![Page {
        size: None,
        layers: vec![layer("L", vec![filled(ink(1.0, None)), filled(ink(1.0, red)), filled(ink(0.0, Some([1.0; 3])))])],
    }]);
    let back = round_trip(&d);
    assert!(near_color(shape(&back.pages[0].layers[0].objects[0]).fill.as_ref().unwrap(), &ink(1.0, red)));
    let mut d = drawing(vec![Page { size: None, layers: vec![layer("L", vec![filled(ink(1.0, None))])] }]);
    d.master = vec![layer("Master", vec![filled(ink(1.0, red))])];
    let back = read_cmx(&write_cmx(&d).unwrap().bytes, Limits::default()).unwrap();
    assert!(near_color(shape(&back.pages[0].layers[0].objects[0]).fill.as_ref().unwrap(), &ink(1.0, red)));
}

#[test]
fn an_empty_layer_and_a_name_ending_in_nul_are_reported() {
    let plain = || Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)));
    let empty = drawing(vec![Page { size: None, layers: vec![layer("Empty", Vec::new()), layer("L", vec![plain()])] }]);
    assert!(write_cmx(&empty).unwrap().left_out);
    let padded = drawing(vec![Page { size: None, layers: vec![layer("L\0", vec![plain()])] }]);
    assert!(write_cmx(&padded).unwrap().long_names);
}

#[test]
fn a_subpath_going_over_its_edges_again_is_reported() {
    let p = |x, y| Pt { x, y };
    // The square right of the start twice counterclockwise, then the square left of it once
    // clockwise: one net turn, and the right square wound twice.
    let pts = [(1, 0), (1, 1), (0, 1), (0, 0), (1, 0), (1, 1), (0, 1), (0, 0), (-1, 0), (-1, 1), (0, 1)].map(|(x, y)| p(f64::from(x), f64::from(y)));
    let sub = SubPath { start: p(0.0, 0.0), segments: pts.map(Segment::Line).to_vec(), closed: true };
    let s = Shape { nonzero: true, ..Shape::new(Geometry::Path(Path { subpaths: vec![sub] }), Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(write_cmx(&one_shape(s)).unwrap().nonzero_fills);
}

#[test]
fn a_shape_with_no_segments_is_reported() {
    let empty = Shape::new(Geometry::Path(Path::default()), Matrix::IDENTITY, Some(Color::Gray(0.5)), None);
    let w = write_cmx(&one_shape(empty)).unwrap();
    assert!(w.left_out);
}

#[test]
fn a_page_side_that_rounds_to_nothing_is_refused() {
    let mut d = one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None));
    d.pages[0].size = Some((0.000_01, 720.0));
    assert_eq!(write_cmx(&d), Err(Error::Limit("page size")));
}

#[test]
fn layer_names_the_reader_cuts_or_drops_are_reported() {
    let plain = || Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)));
    let named = |name: &str| write_cmx(&drawing(vec![Page { size: None, layers: vec![layer(name, vec![plain()])] }])).unwrap().long_names;
    assert!(named("Art\0backup"), "cut at the NUL");
    assert!(named("   "), "blank");
    assert!(!named("Art"));
}

#[test]
fn an_unfilled_clipping_containers_outline_keeps_its_place() {
    // Front to back, whether the outline comes first: over the clipped shape, as an outline with
    // no fill to be behind is drawn over it too.
    for behind_fill in [false, true] {
        let o = Outline { behind_fill, ..outline(Color::Gray(1.0)) };
        let container = Shape::new(square(0.0, 0.0, 10.0), Matrix::IDENTITY, None, Some(o));
        let child = Shape::new(square(2.0, 2.0, 5.0), Matrix::IDENTITY, Some(Color::Gray(0.8)), None);
        let clip = Object::Clip { container: Box::new(container), objects: vec![Object::Shape(Box::new(child))] };
        let back = round_trip(&drawing(vec![Page { size: None, layers: vec![layer("L", vec![clip])] }]));
        let Object::Group { objects, .. } = &back.pages[0].layers[0].objects[0] else { panic!("not a group") };
        let outlined: Vec<bool> = objects.iter().map(|o| shape(o).outline.is_some()).collect();
        assert_eq!(outlined, vec![true, false], "behind_fill {behind_fill}");
    }
}

#[test]
fn a_filled_clipping_containers_outline_goes_over_unless_behind_the_fill() {
    // Front to back, as (fill, outline) of each curve: the outline alone over the clipped shape
    // and the fill alone behind it, or the whole container behind it when the outline is behind
    // the fill.
    for behind_fill in [false, true] {
        let o = Outline { behind_fill, ..outline(Color::Gray(1.0)) };
        let container = Shape::new(square(0.0, 0.0, 10.0), Matrix::IDENTITY, Some(Color::Gray(0.1)), Some(o));
        let child = Shape::new(square(2.0, 2.0, 5.0), Matrix::IDENTITY, Some(Color::Gray(0.8)), None);
        let clip = Object::Clip { container: Box::new(container), objects: vec![Object::Shape(Box::new(child))] };
        let back = round_trip(&drawing(vec![Page { size: None, layers: vec![layer("L", vec![clip])] }]));
        let Object::Group { objects, .. } = &back.pages[0].layers[0].objects[0] else { panic!("not a group") };
        let parts: Vec<(bool, bool)> = objects.iter().map(|o| (shape(o).fill.is_some(), shape(o).outline.is_some())).collect();
        let want = if behind_fill { vec![(true, false), (true, true)] } else { vec![(false, true), (true, false), (true, false)] };
        assert_eq!(parts, want, "behind_fill {behind_fill}");
    }
}

#[test]
fn a_page_size_of_nothing_is_refused() {
    for size in [(0.0, 720.0), (-10.0, 720.0), (f64::NAN, 720.0)] {
        let mut d = one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None));
        d.pages[0].size = Some(size);
        assert_eq!(write_cmx(&d), Err(Error::Limit("page size")), "{size:?}");
    }
}

#[test]
fn an_ink_with_no_known_look_is_reported() {
    let ink = |look| Color::Spot { name: "Ink".into(), tint: 1.0, look };
    let written = |c| write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(c), None))).unwrap().unknown_looks;
    assert!(written(ink(None)));
    assert!(!written(ink(Some([1.0, 0.0, 0.0]))));
}

#[test]
fn loops_meeting_at_one_point_are_reported() {
    let p = |x, y| Pt { x, y };
    let o = p(0.0, 0.0);
    // Three triangles from the origin; the second, inside the first, is wound twice.
    let pts = [p(5.0, 0.0), p(2.0, 5.0), o, p(3.0, 1.0), p(2.0, 3.0), o, p(-5.0, 1.0), p(-2.0, 4.0)];
    let sub = SubPath { start: o, segments: pts.map(Segment::Line).to_vec(), closed: true };
    let s = Shape { nonzero: true, ..Shape::new(Geometry::Path(Path { subpaths: vec![sub] }), Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(write_cmx(&one_shape(s)).unwrap().nonzero_fills);
}

#[test]
fn a_very_thin_outline_stays_thinner_than_a_hairline() {
    let o = Outline { width: 0.000_1, ..outline(Color::Gray(1.0)) };
    let back = round_trip(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, None, Some(o))));
    let width = shape(&back.pages[0].layers[0].objects[0]).outline.as_ref().unwrap().width;
    assert!(width > 0.0 && width < 0.01, "{width}");
}

#[test]
fn nested_contours_far_from_the_origin_are_still_compared() {
    let p = |x, y| Pt { x, y };
    let ring = |x: f64, side: f64| SubPath {
        start: p(x, x),
        segments: [p(x + side, x), p(x + side, x + side), p(x, x + side)].map(Segment::Line).to_vec(),
        closed: true,
    };
    let far = 1.0e8;
    let geometry = Geometry::Path(Path { subpaths: vec![ring(far, 10.0), ring(far + 2.0, 1.0)] });
    let back_home = Matrix([1.0, 0.0, 0.0, 1.0, -far, -far]);
    let s = Shape { nonzero: true, ..Shape::new(geometry, back_home, Some(Color::Gray(0.5)), None) };
    assert!(write_cmx(&one_shape(s)).unwrap().nonzero_fills);
}

#[test]
fn a_page_without_a_size_still_writes_its_four_edges() {
    let bytes = write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None))).unwrap().bytes;
    let at = bytes.windows(4).position(|w| w == b"page").expect("a page chunk") + 8;
    // The begin-page instruction: a u16 size, the code 9, then field 1: a tag and its length.
    assert_eq!(i16::from_le_bytes([bytes[at + 2], bytes[at + 3]]), 9);
    let len = usize::from(u16::from_le_bytes([bytes[at + 5], bytes[at + 6]]));
    assert_eq!(len - 3, 2 + 4 + 16, "the page number, the flags and four edges");
}

#[test]
fn spot_colors_with_no_name_share_no_look() {
    let unnamed = |look| Color::Spot { name: String::new(), tint: 1.0, look };
    let shapes = [unnamed(None), unnamed(Some([1.0, 0.0, 0.0]))]
        .map(|c| Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(c), None))));
    let w = write_cmx(&drawing(vec![Page { size: None, layers: vec![layer("L", shapes.to_vec())] }])).unwrap();
    assert!(w.unnamed_spots && w.unknown_looks);
    let back = read_cmx(&w.bytes, Limits::default()).unwrap();
    let fill = shape(&back.pages[0].layers[0].objects[0]).fill.clone().unwrap();
    assert!(near_color(&fill, &Color::Rgb([0.0, 0.0, 0.0])), "{fill:?}");
}

#[test]
fn an_ink_used_with_different_looks_is_reported() {
    let ink = |tint, look| Color::Spot { name: "Ink".into(), tint, look: Some(look) };
    let written = |second| {
        let shapes =
            [ink(1.0, [1.0, 0.0, 0.0]), second].map(|c| Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(c), None))));
        write_cmx(&drawing(vec![Page { size: None, layers: vec![layer("L", shapes.to_vec())] }])).unwrap().merged_looks
    };
    assert!(!written(ink(0.5, [1.0, 0.0, 0.0])), "one look");
    assert!(written(ink(0.5, [0.0, 1.0, 0.0])), "a second look");
    // At 1 % both looks store the same color, and only the tint changes it.
    let gray = |tint, v| Color::Spot { name: "Gray".into(), tint, look: Some([v; 3]) };
    let shapes =
        [gray(1.0, 0.5), gray(0.01, 0.501)].map(|c| Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(c), None))));
    let w = write_cmx(&drawing(vec![Page { size: None, layers: vec![layer("L", shapes.to_vec())] }])).unwrap();
    assert!(w.zero_tints && !w.merged_looks);
}

#[test]
fn a_cubic_looping_between_its_samples_is_reported() {
    let p = |x, y| Pt { x, y };
    // (s² − 1, s³ − s) for s from −2 to 30 crosses itself at s = −1 and 1.
    let mut segments = vec![Segment::Cubic(p(-119.0 / 3.0, 334.0 / 3.0), p(259.0, -5458.0 / 3.0), p(899.0, 26970.0))];
    segments.extend([p(-10.0, 26970.0), p(-10.0, -6.0)].map(Segment::Line));
    let geometry = Geometry::Path(Path { subpaths: vec![SubPath { start: p(3.0, -6.0), segments, closed: true }] });
    let s = Shape { nonzero: true, ..Shape::new(geometry, Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(write_cmx(&one_shape(s)).unwrap().nonzero_fills);
}

#[test]
fn a_page_gives_the_edges_of_its_art() {
    // A small square off the center of a large page.
    let d = drawing(vec![Page {
        size: Some((600.0, 800.0)),
        layers: vec![layer(
            "L",
            vec![Object::Shape(Box::new(Shape::new(square(100.0, 50.0, 10.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)))],
        )],
    }]);
    let bytes = write_cmx(&d).unwrap().bytes;
    let at = bytes.windows(4).position(|w| w == b"page").expect("a page chunk") + 8;
    // Field 1 of the begin-page instruction: the page number, the flags, then the left, top, right
    // and bottom edges as i32 in 0.1 µm.
    let edge = |i: usize| f64::from(i32::from_le_bytes(bytes[at + 13 + 4 * i..at + 17 + 4 * i].try_into().unwrap())) / (0.0254 / 72.0 / 1e-7);
    let (l, t, r, b) = (edge(0), edge(1), edge(2), edge(3));
    assert!(near(l, 100.0) && near(t, 60.0) && near(r, 110.0) && near(b, 50.0), "{l} {t} {r} {b}");
}

#[test]
fn an_outline_width_that_is_not_a_width_is_refused() {
    for width in [f64::NAN, -1.0] {
        let o = Outline { width, ..outline(Color::Gray(1.0)) };
        assert_eq!(write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, None, Some(o)))), Err(Error::Limit("outline width")));
    }
}

#[test]
fn a_dash_rounded_away_is_reported() {
    let o = Outline { dashes: vec![0.000_000_5, 1.0], ..outline(Color::Gray(1.0)) };
    assert!(write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, None, Some(o)))).unwrap().rounded_dashes);
}

#[test]
fn a_cubic_closing_on_its_start_is_no_crossing() {
    let p = |x, y| Pt { x, y };
    let segments = vec![Segment::Cubic(p(1.0, 0.0), p(0.0, 1.0), p(0.0, 0.0))];
    let geometry = Geometry::Path(Path { subpaths: vec![SubPath { start: p(0.0, 0.0), segments, closed: true }] });
    let s = Shape { nonzero: true, ..Shape::new(geometry, Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(!write_cmx(&one_shape(s)).unwrap().nonzero_fills);
}

#[test]
fn a_negative_width_is_refused_whatever_its_scale() {
    let o = Outline { width: -1.0, scale_with: true, ..outline(Color::Gray(1.0)) };
    let flat = Matrix([1.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    assert_eq!(write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), flat, None, Some(o)))), Err(Error::Limit("outline width")));
}

#[test]
fn a_page_as_wide_as_the_size_field_holds_is_written() {
    // A width that rounds to u32::MAX coordinate units of 0.1 µm, and whose half rounds past what
    // an i32 holds.
    let widest = (f64::from(u32::MAX) + 0.4) * 1e-7 / 0.0254 * 72.0;
    let art = Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None);
    for layers in [vec![layer("L", vec![Object::Shape(Box::new(art))])], Vec::new()] {
        let d = drawing(vec![Page { size: Some((widest, 100.0)), layers }]);
        let back = read_cmx(&write_cmx(&d).unwrap().bytes, Limits::default()).unwrap();
        let (w, _) = back.pages[0].size.unwrap();
        assert!((w - widest).abs() < 0.01, "{w}");
    }
}

#[test]
fn a_page_gives_the_edges_its_curves_reach() {
    let p = |x, y| Pt { x, y };
    // An arch whose control points reach 100 pt and whose curve reaches 75 pt.
    let arch = SubPath { start: p(0.0, 0.0), segments: vec![Segment::Cubic(p(0.0, 100.0), p(100.0, 100.0), p(100.0, 0.0))], closed: false };
    let s = Shape::new(Geometry::Path(Path { subpaths: vec![arch] }), Matrix::IDENTITY, None, Some(outline(Color::Gray(1.0))));
    let d = drawing(vec![Page { size: Some((600.0, 800.0)), layers: vec![layer("L", vec![Object::Shape(Box::new(s))])] }]);
    let bytes = write_cmx(&d).unwrap().bytes;
    let at = bytes.windows(4).position(|w| w == b"page").expect("a page chunk") + 8;
    let top = f64::from(i32::from_le_bytes(bytes[at + 17..at + 21].try_into().unwrap())) / (0.0254 / 72.0 / 1e-7);
    assert!(near(top, 75.0), "{top}");
}

#[test]
fn dashes_too_short_to_store_keep_their_gaps_of_nothing() {
    let o = Outline { dashes: vec![0.4, 0.0], ..outline(Color::Gray(1.0)) };
    let back = round_trip(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, None, Some(o))));
    assert_eq!(shape(&back.pages[0].layers[0].objects[0]).outline.as_ref().unwrap().dashes, vec![1.0, 0.0]);
}

#[test]
fn a_rounded_corner_beside_square_ones_keeps_its_radius() {
    let rect = Shape::new(Geometry::Rect { w: 100.0, h: 100.0, radii: [75.0, 0.0, 0.0, 0.0] }, Matrix::IDENTITY, Some(Color::Gray(0.5)), None);
    let back = round_trip(&one_shape(rect));
    // The corner at (100, 0) is entered from (100 − 75, 0) and left toward (100, 75).
    let pts = points(&shape(&back.pages[0].layers[0].objects[0]).geometry);
    assert!(pts.iter().any(|q| near(q.x, 25.0) && near(q.y, 0.0)), "{pts:?}");
    assert!(pts.iter().any(|q| near(q.x, 100.0) && near(q.y, 75.0)), "{pts:?}");
}

#[test]
fn many_contours_apart_fill_the_same_either_way() {
    let p = |x, y| Pt { x, y };
    let triangle = |i: u32| {
        let x = f64::from(i) * 3.0;
        SubPath { start: p(x, 0.0), segments: vec![Segment::Line(p(x + 1.0, 0.0)), Segment::Line(p(x, 1.0))], closed: true }
    };
    let geometry = Geometry::Path(Path { subpaths: (0..1001).map(triangle).collect() });
    let s = Shape { nonzero: true, ..Shape::new(geometry, Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(!write_cmx(&one_shape(s)).unwrap().nonzero_fills);
}

#[test]
fn a_page_gives_the_top_of_a_nearly_flat_cubic() {
    let p = |x, y| Pt { x, y };
    // The end one coordinate unit above the start keeps the y derivative quadratic.
    let unit = 1e-7 / 0.0254 * 72.0;
    let arch = SubPath { start: p(0.0, 0.0), segments: vec![Segment::Cubic(p(0.0, 10000.0), p(1.0, 10000.0), p(1.0, unit))], closed: false };
    let s = Shape::new(Geometry::Path(Path { subpaths: vec![arch] }), Matrix::IDENTITY, None, Some(outline(Color::Gray(1.0))));
    let d = drawing(vec![Page { size: Some((600.0, 800.0)), layers: vec![layer("L", vec![Object::Shape(Box::new(s))])] }]);
    let bytes = write_cmx(&d).unwrap().bytes;
    let at = bytes.windows(4).position(|w| w == b"page").expect("a page chunk") + 8;
    let top = f64::from(i32::from_le_bytes(bytes[at + 17..at + 21].try_into().unwrap())) / (0.0254 / 72.0 / 1e-7);
    assert!(near(top, 7500.0), "{top}");
}

#[test]
fn a_rounded_rectangle_scaled_unevenly_keeps_round_corners() {
    // 100 pt square, 10 pt corners, stretched 4 times across: 400 by 100 pt with 20 pt corners.
    let rect =
        Shape::new(Geometry::Rect { w: 100.0, h: 100.0, radii: [10.0; 4] }, Matrix([4.0, 0.0, 0.0, 1.0, 0.0, 0.0]), Some(Color::Gray(0.5)), None);
    let back = round_trip(&one_shape(rect));
    let pts = points(&shape(&back.pages[0].layers[0].objects[0]).geometry);
    // The corner at (400, 0) is entered 20 pt along the bottom edge and left 20 pt up the side.
    assert!(pts.iter().any(|q| near(q.x, 380.0) && near(q.y, 0.0)), "{pts:?}");
    assert!(pts.iter().any(|q| near(q.x, 400.0) && near(q.y, 20.0)), "{pts:?}");
}

#[test]
fn a_dash_length_off_a_whole_width_by_a_little_is_reported() {
    let o = Outline { dashes: vec![1.000_000_5, 1.0], ..outline(Color::Gray(1.0)) };
    assert!(write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, None, Some(o)))).unwrap().rounded_dashes);
}

#[test]
fn only_overlapping_corners_shrink() {
    let rect = Shape::new(Geometry::Rect { w: 100.0, h: 100.0, radii: [80.0, 80.0, 10.0, 0.0] }, Matrix::IDENTITY, Some(Color::Gray(0.5)), None);
    let back = round_trip(&one_shape(rect));
    let pts = points(&shape(&back.pages[0].layers[0].objects[0]).geometry);
    // The corner at (0, 100) keeps its 10 pt: it is left toward (0, 90).
    assert!(pts.iter().any(|q| near(q.x, 0.0) && near(q.y, 90.0)), "{pts:?}");
    // The two 80 pt corners on the right edge meet at its middle.
    assert!(pts.iter().any(|q| near(q.x, 100.0) && near(q.y, 50.0)), "{pts:?}");
}

#[test]
fn a_closed_contour_that_fits_one_curve_stays_one_curve() {
    let p = |x, y| Pt { x, y };
    // MAX_CURVE_POINTS anchors, the last away from the start: the closing bit closes it.
    let n = MAX_CURVE_POINTS;
    let segments = (1..n).map(|i| Segment::Line(p(i as f64 * 0.01, if i % 2 == 0 { 0.0 } else { 1.0 }))).collect();
    let s = Shape::new(
        Geometry::Path(Path { subpaths: vec![SubPath { start: p(0.0, 0.0), segments, closed: true }] }),
        Matrix::IDENTITY,
        None,
        Some(outline(Color::Gray(1.0))),
    );
    let w = write_cmx(&one_shape(s)).unwrap();
    assert!(!w.split_paths && !w.cut_subpaths);
    let back = read_cmx(&w.bytes, Limits::default()).unwrap();
    let Geometry::Path(path) = &shape(&back.pages[0].layers[0].objects[0]).geometry else { panic!() };
    assert!(path.subpaths.len() == 1 && path.subpaths[0].closed);
}

#[test]
fn a_page_gives_the_edges_of_the_curve_as_written() {
    let p = |x, y| Pt { x, y };
    // Controls 0.6 units up round to 1 unit, so the written curve peaks at 0.75 units.
    let unit = 1e-7 / 0.0254 * 72.0;
    let arch = SubPath { start: p(0.0, 0.0), segments: vec![Segment::Cubic(p(0.0, 0.6 * unit), p(1.0, 0.6 * unit), p(1.0, 0.0))], closed: false };
    let s = Shape::new(Geometry::Path(Path { subpaths: vec![arch] }), Matrix::IDENTITY, None, Some(outline(Color::Gray(1.0))));
    let d = drawing(vec![Page { size: Some((600.0, 800.0)), layers: vec![layer("L", vec![Object::Shape(Box::new(s))])] }]);
    let bytes = write_cmx(&d).unwrap().bytes;
    let at = bytes.windows(4).position(|w| w == b"page").expect("a page chunk") + 8;
    let top = i32::from_le_bytes(bytes[at + 17..at + 21].try_into().unwrap());
    assert!(top >= 1, "{top}");
}

#[test]
fn a_blank_layer_name_opens_with_no_name() {
    let plain = Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)));
    let bytes = write_cmx(&drawing(vec![Page { size: None, layers: vec![layer("\u{2003}", vec![plain])] }])).unwrap().bytes;
    let back = read_cmx(&bytes, Limits::default()).unwrap();
    assert!(back.pages[0].layers[0].name.is_empty(), "{:?}", back.pages[0].layers[0].name);
}

#[test]
fn a_larger_corner_reaches_where_its_smaller_neighbor_begins() {
    let rect = Shape::new(Geometry::Rect { w: 100.0, h: 100.0, radii: [80.0, 40.0, 0.0, 0.0] }, Matrix::IDENTITY, Some(Color::Gray(0.5)), None);
    let back = round_trip(&one_shape(rect));
    let pts = points(&shape(&back.pages[0].layers[0].objects[0]).geometry);
    // On the right edge the 80 pt corner is held to 60 pt and the 40 pt one keeps its radius.
    assert!(pts.iter().any(|q| near(q.x, 100.0) && near(q.y, 60.0)), "{pts:?}");
    assert!(pts.iter().any(|q| near(q.x, 60.0) && near(q.y, 100.0)), "{pts:?}");
}

#[test]
fn a_long_blank_layer_name_opens_with_no_name() {
    let plain = Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)));
    let bytes = write_cmx(&drawing(vec![Page { size: None, layers: vec![layer(&" ".repeat(257), vec![plain])] }])).unwrap().bytes;
    let back = read_cmx(&bytes, Limits::default()).unwrap();
    assert!(back.pages[0].layers[0].name.is_empty(), "{:?}", back.pages[0].layers[0].name);
}

#[test]
fn a_spot_color_with_no_name_at_no_tint_is_reported() {
    let unnamed = Color::Spot { name: String::new(), tint: 0.0, look: Some([0.0, 0.0, 1.0]) };
    let w = write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(unnamed), None))).unwrap();
    assert!(w.unnamed_spots && w.zero_tints);
}

#[test]
fn a_drawing_with_no_pages_keeps_its_page_size() {
    let mut d = drawing(Vec::new());
    d.page_size = Some((300.0, 200.0));
    let back = read_cmx(&write_cmx(&d).unwrap().bytes, Limits::default()).unwrap();
    let (w, h) = back.pages[0].size.unwrap();
    assert!(near(w, 300.0) && near(h, 200.0));
}

#[test]
fn layers_give_their_page_and_their_number() {
    let plain = || Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)));
    let page = || Page { size: None, layers: vec![layer("A", vec![plain()]), layer("B", vec![plain()])] };
    let bytes = write_cmx(&drawing(vec![page(), page()])).unwrap().bytes;
    // Each begin-layer instruction: a u16 size, the code 11, field 1 (a tag and a u16 length), then
    // the page number and the layer number.
    let mut numbers = Vec::new();
    let mut at = 0;
    while let Some(i) = bytes[at..].windows(4).position(|w| w == b"page") {
        let start = at + i + 8;
        let end = start + u32::from_le_bytes(bytes[at + i + 4..start].try_into().unwrap()) as usize;
        let mut p = start;
        while p < end {
            let size = usize::from(u16::from_le_bytes([bytes[p], bytes[p + 1]]));
            assert!(size >= 4 && p + size <= end, "an instruction of {size} bytes at {p}");
            if i16::from_le_bytes([bytes[p + 2], bytes[p + 3]]) == 11 {
                numbers.push((u16::from_le_bytes([bytes[p + 7], bytes[p + 8]]), u16::from_le_bytes([bytes[p + 9], bytes[p + 10]])));
            }
            p += size;
        }
        at = end;
    }
    assert_eq!(numbers, [(1, 1), (1, 2), (2, 1), (2, 2)]);
}

#[test]
fn a_uniform_fill_names_the_screen_the_file_holds() {
    let bytes = write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None))).unwrap().bytes;
    // The fill's field 1: a tag, a length of 7, the color entry 1 and the screen entry 1.
    assert!(bytes.windows(7).any(|w| w == [1, 7, 0, 1, 0, 1, 0]));
    assert_eq!(table_entries(&bytes, b"rscr").len(), 1);
}

#[test]
fn an_unpainted_clipping_container_is_not_written() {
    let container = Shape::new(square(0.0, 0.0, 10.0), Matrix::IDENTITY, None, None);
    let child = Shape::new(square(2.0, 2.0, 5.0), Matrix::IDENTITY, Some(Color::Gray(0.8)), None);
    let clip = Object::Clip { container: Box::new(container), objects: vec![Object::Shape(Box::new(child))] };
    let back = round_trip(&drawing(vec![Page { size: None, layers: vec![layer("L", vec![clip])] }]));
    let Object::Group { objects, .. } = &back.pages[0].layers[0].objects[0] else { panic!("not a group") };
    assert_eq!(objects.len(), 1, "only the clipped shape");
}

#[test]
fn groups_nested_deeper_than_the_reader_reads_are_refused() {
    let nest = |depth: usize| {
        let mut o = Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)));
        for _ in 0..depth {
            o = Object::Group { objects: vec![o], flags: Flags::default() };
        }
        drawing(vec![Page { size: None, layers: vec![layer("L", vec![o])] }])
    };
    let deepest = Limits::default().depth;
    assert!(round_trip(&nest(deepest)).pages[0].layers.len() == 1, "as deep as the reader reads");
    assert_eq!(write_cmx(&nest(deepest + 1)), Err(Error::Limit("nesting depth")));
}

#[test]
fn a_file_the_reader_would_not_take_whole_within_the_limits_is_refused() {
    let dashed = Outline { dashes: vec![2.0, 1.0, 3.0], ..outline(Color::Gray(0.0)) };
    let art = |x: f64| Object::Shape(Box::new(Shape::new(square(x, 0.0, 10.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), Some(dashed.clone()))));
    let group = Object::Group { objects: vec![art(0.0), art(20.0)], flags: Flags::default() };
    let d = drawing(vec![Page { size: None, layers: vec![layer("L", vec![group, art(40.0)])] }]);
    let whole = write_cmx(&d).unwrap().bytes;
    let fields: [fn(&mut Limits) -> &mut usize; 5] =
        [|l| &mut l.chunks, |l| &mut l.objects, |l| &mut l.coordinates, |l| &mut l.dashes, |l| &mut l.depth];
    for (i, field) in fields.into_iter().enumerate() {
        let (mut written, mut refused) = (false, false);
        for v in 0..=80 {
            let mut limits = Limits::default();
            *field(&mut limits) = v;
            // What the reader takes from the file written within the default limits.
            let opens_whole = read_cmx(&whole, limits).is_ok_and(|back| back.warnings.is_empty());
            match craft_cdr::write_cmx(&d, limits) {
                Ok(w) => {
                    written = true;
                    assert!(opens_whole, "field {i} at {v}: written but not read whole");
                    assert_eq!(w.bytes, whole, "field {i} at {v}");
                }
                Err(e) => {
                    refused = true;
                    assert!(matches!(e, Error::Limit(_)), "field {i} at {v}: {e:?}");
                    assert!(!opens_whole, "field {i} at {v}: refused but read whole");
                }
            }
        }
        assert!(written && refused, "field {i}: both outcomes within 0 to 80");
    }
    let input = |n: usize| craft_cdr::write_cmx(&d, Limits { input: n, ..Limits::default() });
    assert!(input(whole.len()).is_ok());
    assert_eq!(input(whole.len() - 1), Err(Error::Limit("input size")));
    // A pattern with no positive length is solid, and the reader takes no dashes for it.
    let solid = Outline { dashes: vec![f64::NAN, 0.0], ..outline(Color::Gray(0.0)) };
    let one = one_shape(Shape::new(square(0.0, 0.0, 10.0), Matrix::IDENTITY, None, Some(solid)));
    let w = craft_cdr::write_cmx(&one, Limits { dashes: 0, ..Limits::default() }).unwrap();
    assert!(w.rounded_dashes, "the length that is not a number");
    assert!(read_cmx(&w.bytes, Limits { dashes: 0, ..Limits::default() }).unwrap().warnings.is_empty());
}

#[test]
fn a_negative_radius_leaves_its_corner_square() {
    let rect = Shape::new(Geometry::Rect { w: 100.0, h: 100.0, radii: [-20.0, 0.0, 0.0, 0.0] }, Matrix::IDENTITY, Some(Color::Gray(0.5)), None);
    let back = round_trip(&one_shape(rect));
    let pts = points(&shape(&back.pages[0].layers[0].objects[0]).geometry);
    assert!(pts.iter().any(|q| near(q.x, 100.0) && near(q.y, 0.0)), "{pts:?}");
}

#[test]
fn every_use_of_an_ink_takes_the_look_of_its_strongest_tint() {
    let ink = |tint, look| Color::Spot { name: "Ink".into(), tint, look: Some(look) };
    let filled = |c, hidden| {
        let mut s = Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(c), None);
        s.flags.hidden = hidden;
        Object::Shape(Box::new(s))
    };
    let d = drawing(vec![Page {
        size: None,
        layers: vec![layer("L", vec![filled(ink(0.5, [0.0, 1.0, 0.0]), false), filled(ink(1.0, [1.0, 0.0, 0.0]), true)])],
    }]);
    let back = read_cmx(&write_cmx(&d).unwrap().bytes, Limits::default()).unwrap();
    assert!(near_color(shape(&back.pages[0].layers[0].objects[0]).fill.as_ref().unwrap(), &ink(0.5, [1.0, 0.0, 0.0])));
}

#[test]
fn a_name_that_is_only_nuls_as_written_is_reported() {
    let name = format!("{}Ink", "\0".repeat(4096));
    let c = Color::Spot { name, tint: 1.0, look: Some([1.0, 0.0, 0.0]) };
    let w = write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(c), None))).unwrap();
    assert!(w.unnamed_spots && w.long_names);
}

#[test]
fn a_tint_too_low_to_keep_the_look_is_reported() {
    let pale = Color::Spot { name: "Pale".into(), tint: 0.01, look: Some([1.0, 0.85, 0.85]) };
    assert!(write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(pale), None))).unwrap().zero_tints);
}

#[test]
fn a_shape_split_into_curves_paints_its_fill_before_its_outline() {
    let p = |x, y| Pt { x, y };
    // A long polyline, then a square: two curves.
    let segments = (1..MAX_CURVE_POINTS).map(|i| Segment::Line(p(i as f64 * 0.01, 5.0))).collect();
    let long = SubPath { start: p(0.0, 0.0), segments, closed: false };
    let Geometry::Path(sq) = square(2.0, 2.0, 5.0) else { panic!() };
    for behind_fill in [false, true] {
        let geometry = Geometry::Path(Path { subpaths: vec![long.clone(), sq.subpaths[0].clone()] });
        let o = Outline { behind_fill, ..outline(Color::Gray(1.0)) };
        let back = round_trip(&one_shape(Shape::new(geometry, Matrix::IDENTITY, Some(Color::Gray(0.2)), Some(o))));
        let Object::Group { objects, .. } = &back.pages[0].layers[0].objects[0] else { panic!("not a group") };
        // Each curve carries the fill or the outline, the outlines all on one side of the fills.
        let outlines: Vec<bool> = objects.iter().map(|o| shape(o).outline.is_some()).collect();
        assert!(objects.iter().all(|o| shape(o).fill.is_some() != shape(o).outline.is_some()), "{outlines:?}");
        assert!(outlines.windows(2).all(|w| w[0] >= w[1]) || outlines.windows(2).all(|w| w[0] <= w[1]), "{outlines:?}");
        // Front to back: the outline is in front unless it is drawn behind the fill.
        assert_eq!(outlines[0], !behind_fill, "{outlines:?}");
    }
}

#[test]
fn a_dash_pattern_past_one_table_entry_is_refused() {
    let file = |n: usize| {
        let o = Outline { dashes: vec![1.0; n], ..outline(Color::Gray(1.0)) };
        write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, None, Some(o))))
    };
    assert!(file(32_765).is_ok());
    assert_eq!(file(32_766), Err(Error::Limit("table entry size")));
}

#[test]
fn a_contour_whose_area_sums_to_nothing_counts_as_either_turn() {
    let p = |x, y| Pt { x, y };
    let tri = |pts: [Pt; 3]| SubPath { start: pts[0], segments: vec![Segment::Line(pts[1]), Segment::Line(pts[2])], closed: true };
    let (a, b, c) = (p(0.0, 0.0), p(100_000_000.0, 99_999_999.0), p(100_000_001.0, 100_000_000.0));
    let geometry = Geometry::Path(Path { subpaths: vec![tri([a, b, c]), tri([b, c, a])] });
    let m = Matrix([16.0, -299_999_997.0, -16.0, 300_000_000.0, 0.0, 0.0]);
    let s = Shape { nonzero: true, ..Shape::new(geometry, m, Some(Color::Gray(0.5)), None) };
    assert!(write_cmx(&one_shape(s)).unwrap().nonzero_fills);
}

#[test]
fn a_cubic_looping_from_its_start_is_reported() {
    let p = |x, y| Pt { x, y };
    let mut segments = vec![Segment::Cubic(p(-10.0, 0.0), p(150.0, -10.0), p(480.0, 480.0))];
    segments.extend([p(-1.0, 480.0), p(-1.0, -1.0)].map(Segment::Line));
    let geometry = Geometry::Path(Path { subpaths: vec![SubPath { start: p(0.0, 0.0), segments, closed: true }] });
    let s = Shape { nonzero: true, ..Shape::new(geometry, Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(write_cmx(&one_shape(s)).unwrap().nonzero_fills);
}

#[test]
fn a_cubic_crossing_an_edge_between_its_flattened_points_is_reported() {
    let p = |x, y| Pt { x, y };
    // The arch (16 t, 9 t (1 − t)) is flattened at whole x; at x = 7.5 its line is at y = 2.2324
    // and the arch at y = 2.2412. A notch down to `tip` stays clear of the line, and crosses the
    // arch when the tip is below it.
    let filled = |tip: f64| {
        let mut segments = vec![Segment::Cubic(p(16.0 / 3.0, 3.0), p(32.0 / 3.0, 3.0), p(16.0, 0.0))];
        segments.extend([p(16.0, 6.0), p(7.5, tip), p(0.0, 6.0)].map(Segment::Line));
        let geometry = Geometry::Path(Path { subpaths: vec![SubPath { start: p(0.0, 0.0), segments, closed: true }] });
        let s = Shape { nonzero: true, ..Shape::new(geometry, Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
        write_cmx(&one_shape(s)).unwrap().nonzero_fills
    };
    assert!(filled(2.237));
    assert!(!filled(2.3));
}

#[test]
fn a_cubic_crossing_the_next_edge_near_their_shared_point_is_reported() {
    let p = |x: f64, y: f64| Pt { x: x * 1000.0, y: y * 1000.0 };
    // The arch (16 t, 9 t (1 − t)) ends at (16, 0) heading back at a slope of 9/16, and the line
    // from there to (0, 8.8) leaves at a slope of 0.55: the arch starts above the line and crosses
    // it. The arch's last flattened piece, at a slope of 0.527, stays below the line.
    let segments = vec![Segment::Cubic(p(16.0 / 3.0, 3.0), p(32.0 / 3.0, 3.0), p(16.0, 0.0)), Segment::Line(p(0.0, 8.8))];
    let arch = SubPath { start: p(0.0, 0.0), segments, closed: true };
    // A clockwise square apart from it.
    let square = SubPath { start: p(20.0, 0.0), segments: [p(20.0, 1.0), p(21.0, 1.0), p(21.0, 0.0)].map(Segment::Line).to_vec(), closed: true };
    let geometry = Geometry::Path(Path { subpaths: vec![arch, square] });
    let s = Shape { nonzero: true, ..Shape::new(geometry, Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(write_cmx(&one_shape(s)).unwrap().nonzero_fills);
    // A square whose bottom edge is a straight cubic that eases in and out, so its first and last
    // flattened pieces are short beside the lines next to them, crosses nothing.
    let eased = vec![Segment::Cubic(p(0.0, 0.0), p(10.0, 0.0), p(10.0, 0.0)), Segment::Line(p(10.0, 10.0)), Segment::Line(p(0.0, 10.0))];
    let geometry = Geometry::Path(Path { subpaths: vec![SubPath { start: p(0.0, 0.0), segments: eased, closed: true }] });
    let s = Shape { nonzero: true, ..Shape::new(geometry, Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(!write_cmx(&one_shape(s)).unwrap().nonzero_fills);
    // A crescent: two arcs that leave each tip along the same tangent and part at once.
    let arcs = vec![Segment::Cubic(p(2.0, 4.0), p(8.0, 4.0), p(10.0, 0.0)), Segment::Cubic(p(9.0, 2.0), p(1.0, 2.0), p(0.0, 0.0))];
    let geometry = Geometry::Path(Path { subpaths: vec![SubPath { start: p(0.0, 0.0), segments: arcs, closed: true }] });
    let s = Shape { nonzero: true, ..Shape::new(geometry, Matrix::IDENTITY, Some(Color::Gray(0.5)), None) };
    assert!(!write_cmx(&one_shape(s)).unwrap().nonzero_fills);
}

#[test]
fn a_blank_layer_name_before_a_nul_opens_with_no_name() {
    let plain = Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)));
    let bytes = write_cmx(&drawing(vec![Page { size: None, layers: vec![layer("\u{2003}\0", vec![plain])] }])).unwrap().bytes;
    let back = read_cmx(&bytes, Limits::default()).unwrap();
    assert!(back.pages[0].layers[0].name.is_empty(), "{:?}", back.pages[0].layers[0].name);
}

#[test]
fn marks_too_short_to_store_stay_visible() {
    let o = Outline { dashes: vec![0.4, 2.0], ..outline(Color::Gray(1.0)) };
    let back = round_trip(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, None, Some(o))));
    assert_eq!(shape(&back.pages[0].layers[0].objects[0]).outline.as_ref().unwrap().dashes, vec![1.0, 2.0]);
}

#[test]
fn a_spot_color_without_a_look_takes_the_look_of_its_ink() {
    let ink = |look| Color::Spot { name: "Ink".into(), tint: 1.0, look };
    let shapes =
        [ink(None), ink(Some([1.0, 0.0, 0.0]))].map(|c| Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(c), None))));
    let back = round_trip(&drawing(vec![Page { size: None, layers: vec![layer("L", shapes.to_vec())] }]));
    for o in &back.pages[0].layers[0].objects {
        assert!(near_color(shape(o).fill.as_ref().unwrap(), &ink(Some([1.0, 0.0, 0.0]))), "{:?}", shape(o).fill);
    }
}

#[test]
fn art_on_a_desktop_layer_is_reported() {
    let art = vec![Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)))];
    let desktop = Layer { kind: LayerKind::Desktop, ..layer("Desktop", art) };
    let w = write_cmx(&drawing(vec![Page { size: None, layers: vec![desktop] }])).unwrap();
    assert!(w.left_out);
    assert_eq!(read_cmx(&w.bytes, Limits::default()).unwrap().pages[0].layers[0].objects.len(), 1, "the art is kept");
}

#[test]
fn dashes_too_short_to_store_are_reported() {
    let o = Outline { dashes: vec![0.0, 0.000_000_5], ..outline(Color::Gray(1.0)) };
    assert!(write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, None, Some(o)))).unwrap().rounded_dashes);
}

#[test]
fn a_spot_color_with_no_name_is_reported() {
    let unnamed = Color::Spot { name: String::new(), tint: 0.5, look: Some([0.0, 0.0, 1.0]) };
    let w = write_cmx(&one_shape(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(unnamed), None))).unwrap();
    assert!(w.unnamed_spots && !w.zero_tints);
    let back = read_cmx(&w.bytes, Limits::default()).unwrap();
    let fill = shape(&back.pages[0].layers[0].objects[0]).fill.clone().unwrap();
    assert!(near_color(&fill, &Color::Rgb([0.5, 0.5, 1.0])), "{fill:?}");
}

#[test]
fn a_layer_name_keeps_its_windows_1252_characters_in_the_8_bit_name() {
    let plain = Object::Shape(Box::new(Shape::new(square(0.0, 0.0, 1.0), Matrix::IDENTITY, Some(Color::Gray(0.5)), None)));
    let bytes = write_cmx(&drawing(vec![Page { size: None, layers: vec![layer("€ ‰ 中", vec![plain])] }])).unwrap().bytes;
    // The 8-bit name: a u16 count, then the bytes.
    let narrow = [5u8, 0, 0x80, b' ', 0x89, b' ', b'?'];
    assert!(bytes.windows(narrow.len()).any(|w| w == narrow), "the euro and per-mille signs have bytes; the last character has none");
}

#[test]
fn more_layers_than_the_layer_number_holds_are_refused() {
    let layers = (0..=usize::from(u16::MAX)).map(|_| layer("", Vec::new())).collect();
    assert_eq!(write_cmx(&drawing(vec![Page { size: None, layers }])), Err(Error::Limit("layers")));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Paths of lines and curves read back with every point within a coordinate unit.
    #[test]
    fn paths_read_back_point_for_point(
        pts in prop::collection::vec((-1.0e5f64..1.0e5, -1.0e5f64..1.0e5), 4..60),
        closed in any::<bool>(),
        curves in any::<bool>(),
    ) {
        let p = |(x, y): (f64, f64)| Pt { x, y };
        let start = p(pts[0]);
        let segments: Vec<Segment> = if curves {
            pts[1..].as_chunks::<3>().0.iter().map(|[a, b, c]| Segment::Cubic(p(*a), p(*b), p(*c))).collect()
        } else {
            pts[1..].iter().map(|q| Segment::Line(p(*q))).collect()
        };
        prop_assume!(!segments.is_empty());
        let geometry = Geometry::Path(Path { subpaths: vec![SubPath { start, segments, closed }] });
        let s = Shape::new(geometry.clone(), Matrix::IDENTITY, Some(Color::Rgb([0.5, 0.5, 0.5])), None);
        let d = drawing(vec![Page { size: Some((612.0, 792.0)), layers: vec![layer("L", vec![Object::Shape(Box::new(s))])] }]);
        let back = round_trip(&d);
        let read = &shape(&back.pages[0].layers[0].objects[0]).geometry;
        let Geometry::Path(path) = read else { panic!() };
        prop_assert_eq!(path.subpaths[0].closed, closed);
        let (want, got) = (points(&geometry), points(read));
        prop_assert_eq!(got.len(), want.len());
        for (a, b) in want.iter().zip(&got) {
            prop_assert!(near(a.x, b.x) && near(a.y, b.y), "{:?} {:?}", a, b);
        }
    }
}

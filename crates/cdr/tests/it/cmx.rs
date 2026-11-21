//! Reading synthetic `.cmx` files built from the layouts in `PROVENANCE.md` (R53 to R63), and
//! hostile ones.

use craft_cdr::model::{Cap, Color, Geometry, Join, Object, Shape};
use craft_cdr::synth_cmx::*;
use craft_cdr::{Error, Limits, Warning, is_cdr, is_cmx, read_cmx};
use proptest::prelude::*;

fn shape(o: &Object) -> &Shape {
    match o {
        Object::Shape(s) => s,
        other => panic!("not a shape: {other:?}"),
    }
}

/// The (x, y) bounds of a shape's path.
fn bounds(s: &Shape) -> (f64, f64, f64, f64) {
    let Geometry::Path(p) = &s.geometry else { panic!("not a path") };
    let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for sp in &p.subpaths {
        for pt in std::iter::once(sp.start).chain(sp.segments.iter().map(|s| match s {
            craft_cdr::model::Segment::Line(p) | craft_cdr::model::Segment::Cubic(_, _, p) => *p,
        })) {
            b = (b.0.min(pt.x), b.1.min(pt.y), b.2.max(pt.x), b.3.max(pt.y));
        }
    }
    b
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// The file with `children` after a 32-bit header and the sample tables.
fn file(pages: &[Vec<u8>]) -> Vec<u8> {
    let mut children = vec![cont(b'4', 1e-7)];
    children.extend(pages.iter().cloned());
    children.extend(tables());
    riff(&children)
}

#[test]
fn a_cmx_file_is_told_from_a_cdr_drawing() {
    let f = sample();
    assert!(is_cmx(&f) && !is_cdr(&f));
    let mut rifx = f.clone();
    rifx[..4].copy_from_slice(b"RIFX");
    assert!(is_cmx(&rifx));
    assert!(!is_cmx(b"RIFF\0\0\0\0CDR9"));
    assert_eq!(read_cmx(b"RIFF\0\0\0\0CDR9", Limits::default()), Err(Error::NotCmx));
}

#[test]
fn pages_layers_groups_and_curves() {
    let d = read_cmx(&sample(), Limits::default()).unwrap();
    assert_eq!(d.version, 0);
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert_eq!(d.pages.len(), 1);
    let page = &d.pages[0];
    // 2,100,000 units of 0.1 µm are 210 mm.
    let (w, h) = page.size.unwrap();
    assert!(near(w, 210.0 / 25.4 * 72.0) && near(h, 297.0 / 25.4 * 72.0), "{w} {h}");
    // The layer drawn last is in front: the model lists layers front to back.
    let names: Vec<&str> = page.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Top", "Art"]);
    let art = &page.layers[1].objects;
    assert_eq!(art.len(), 2);
    // The group, drawn after the red square, is in front of it.
    let Object::Group { objects: kids, .. } = &art[0] else { panic!("{:?}", art[0]) };
    assert_eq!(kids.len(), 2);
    let red = shape(&art[1]);
    assert_eq!(red.fill, Some(Color::Rgb([1.0, 0.0, 0.0])));
    // 1,000,000 units are 100 mm; the square's corner is 50 mm left of and below the center.
    let mm = 72.0 / 25.4;
    let b = bounds(red);
    assert!(near(b.0, -50.0 * mm) && near(b.1, -50.0 * mm) && near(b.2, 50.0 * mm) && near(b.3, 50.0 * mm), "{b:?}");
    let Geometry::Path(p) = &red.geometry else { panic!() };
    assert!(p.subpaths.len() == 1 && p.subpaths[0].closed && p.subpaths[0].segments.len() == 4);
    let o = red.outline.as_ref().unwrap();
    assert!(near(o.width, 0.5 * mm));
    assert_eq!((o.color.clone(), o.cap, o.join), (Color::Cmyk([0.0, 0.0, 0.0, 1.0]), Cap::Round, Join::Round));
    assert!(o.dashes.is_empty() && !o.behind_fill && !o.scale_with);
    // Line type 1 draws no outline.
    let blue = shape(&kids[0]);
    let black = shape(&kids[1]);
    assert!(black.outline.is_none());
    assert_eq!(black.fill, Some(Color::Cmyk([0.0, 0.0, 0.0, 1.0])));
    // The color record's values are blue, green and red.
    assert_eq!(blue.fill, Some(Color::Rgb([0.0, 0.0, 1.0])));
    let o = blue.outline.as_ref().unwrap();
    assert!(near(o.width, 1.0 * mm));
    assert_eq!((o.dashes.clone(), o.behind_fill, o.cap, o.join), (vec![3.0, 1.0], true, Cap::Square, Join::Miter));
    assert_eq!(page.layers[0].name, "Top");
}

#[test]
fn layer_names_and_layers_without_objects() {
    let p = page(&[
        begin_page(None),
        begin_layer("Ebene 1", Some("Слой 1")),
        square(&attrs(Some(1), None, false), 0, 0, 1000),
        end_layer(),
        begin_layer("Gitter", None),
        end_layer(),
        begin_layer("Caf\u{e9}", None),
        square(&attrs(Some(1), None, false), 0, 0, 1000),
        end_layer(),
        end_page(),
    ]);
    let d = read_cmx(&file(&[p]), Limits::default()).unwrap();
    let page = &d.pages[0];
    assert_eq!(page.size, None);
    // The empty layer is left out, and the UTF-16 name wins over the 8-bit one.
    let names: Vec<&str> = page.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Caf\u{c3}\u{a9}", "Слой 1"]);
}

#[test]
fn objects_before_any_layer_get_one() {
    let p = page(&[begin_page(None), square(&attrs(Some(1), None, false), 0, 0, 1000)]);
    let d = read_cmx(&file(&[p]), Limits::default()).unwrap();
    assert_eq!(d.pages[0].layers.len(), 1);
    // The layer is left unnamed, as a `.cdr` drawing's is; the importer names it by its place.
    assert_eq!(d.pages[0].layers[0].name, "");
}

#[test]
fn a_page_without_a_size_takes_the_drawings_bounds() {
    // A square of 100 mm whose bounds lie off the file's origin, above and to the right.
    let p = page(&[
        begin_page_in_bounds(1_000_000, 3_000_000, 2_000_000, 2_000_000),
        begin_layer("L", None),
        square(&attrs(Some(1), None, false), 1_000_000, 2_000_000, 1_000_000),
        end_layer(),
        end_page(),
    ]);
    let d = read_cmx(&file(&[p]), Limits::default()).unwrap();
    let page = &d.pages[0];
    let side = 100.0 / 25.4 * 72.0;
    let (w, h) = page.size.unwrap();
    assert!(near(w, side) && near(h, side), "{w} x {h}");
    // The page is centered on the bounds, so the square fills it.
    let (x0, y0, x1, y1) = bounds(shape(&page.layers[0].objects[0]));
    assert!(near(x0, -side / 2.0) && near(x1, side / 2.0) && near(y0, -side / 2.0) && near(y1, side / 2.0), "{x0} {y0} {x1} {y1}");
}

#[test]
fn curves_and_open_groups() {
    let c =
        curve(&attrs(None, Some(1), false), &[(0, 0), (100, 0), (100, 100), (0, 100), (200, 200), (300, 200)], &[0x00, 0xc0, 0xc0, 0x84, 0x00, 0x44]);
    // A group the layer leaves open closes with it.
    let p = page(&[begin_page(None), begin_layer("L", None), begin_group(), c, end_layer()]);
    let d = read_cmx(&file(&[p]), Limits::default()).unwrap();
    let objects = &d.pages[0].layers[0].objects;
    let Object::Group { objects: kids, .. } = &objects[0] else { panic!("{objects:?}") };
    let s = shape(&kids[0]);
    assert!(s.fill.is_none());
    let Geometry::Path(p) = &s.geometry else { panic!() };
    assert_eq!(p.subpaths.len(), 2);
    assert!(matches!(p.subpaths[0].segments[..], [craft_cdr::model::Segment::Cubic(..)]));
    assert!(!p.subpaths[1].closed);
}

#[test]
fn a_curve_without_its_points_is_reported() {
    let p = page(&[begin_page(None), begin_layer("L", None), instr(67, &fields(&[field(1, &attrs(Some(1), None, false))])), end_layer()]);
    let d = read_cmx(&file(&[p]), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
}

#[test]
fn a_layer_name_too_short_to_read_in_utf_16_reads_in_8_bit() {
    let mut d = vec![1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0];
    d.extend(b"Art");
    let layer = instr(11, &fields(&[field(1, &d), field(4, &[3, 0])]));
    let p = page(&[begin_page(None), layer, square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let d = read_cmx(&file(&[p]), Limits::default()).unwrap();
    assert_eq!(d.pages[0].layers[0].name, "Art");
}

#[test]
fn what_is_not_read_is_reported() {
    let other = |code: i16| instr(code, &fields(&[field(1, &[0; 8])]));
    let p = page(&[
        begin_page(None),
        begin_layer("L", None),
        other(66),
        other(68),
        other(20),
        other(88),
        square(&attrs_fill_type(2), 0, 0, 1000),
        square(&attrs(Some(1), None, true), 0, 0, 1000),
        end_layer(),
    ]);
    let d = read_cmx(&file(&[p]), Limits::default()).unwrap();
    for w in [Warning::UnsupportedObjects, Warning::UnsupportedFills, Warning::Lenses] {
        assert!(d.warnings.contains(&w), "{w:?}: {:?}", d.warnings);
    }
    // A fill of another type leaves the curve unfilled; the curve is still read.
    let objects = &d.pages[0].layers[0].objects;
    assert_eq!(objects.len(), 2);
    assert!(shape(&objects[1]).fill.is_none());
}

#[test]
fn colors_of_other_models_and_missing_entries() {
    let mut children = vec![cont(b'4', 1e-7)];
    children.push(page(&[
        begin_page(None),
        begin_layer("L", None),
        square(&attrs(Some(1), None, false), 0, 0, 1000),
        square(&attrs(Some(9), None, false), 0, 0, 1000),
        square(&attrs(Some(2), None, false), 0, 0, 1000),
    ]));
    children.push(table(b"rclr", &[color(7, &[1, 2, 3, 4]), color(3, &[255, 0, 0, 255])]));
    let d = read_cmx(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::UnknownColors) && d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    let objects = &d.pages[0].layers[0].objects;
    assert_eq!(shape(&objects[0]).fill, Some(Color::Cmyk([1.0, 0.0, 0.0, 1.0])));
    assert_eq!(shape(&objects[1]).fill, Some(Color::Rgb([0.0; 3])));
}

#[test]
fn gray_cmy_and_rgb_colors_of_the_older_fields() {
    let mut children = vec![cont(b'4', 1e-7)];
    children.push(page(&[
        begin_page(None),
        begin_layer("L", None),
        square(&attrs(Some(1), None, false), 0, 0, 1000),
        square(&attrs(Some(2), None, false), 0, 0, 1000),
        square(&attrs(Some(3), None, false), 0, 0, 1000),
    ]));
    children.push(table(b"rclr", &[color(9, &[51]), color(4, &[255, 0, 51]), color(5, &[255, 0, 0])]));
    let d = read_cmx(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    let objects = &d.pages[0].layers[0].objects;
    // The model holds objects front to back: the RGB square, the CMY one, then the gray one.
    assert_eq!(shape(&objects[0]).fill, Some(Color::Rgb([1.0, 0.0, 0.0])));
    match (shape(&objects[1]).fill.clone(), shape(&objects[2]).fill.clone()) {
        (Some(Color::Cmyk([c, m, y, k])), Some(Color::Gray(g))) => {
            assert!((c - 1.0).abs() < 1e-6 && m == 0.0 && (y - 0.2).abs() < 1e-6 && k == 0.0, "{c} {m} {y} {k}");
            assert!((g - 0.8).abs() < 1e-6, "{g}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn outline_arrowheads_are_reported() {
    let mut children = vec![cont(b'4', 1e-7)];
    children.push(page(&[begin_page(None), begin_layer("L", None), square(&attrs(None, Some(1), false), 0, 0, 1000)]));
    children.extend(tables().into_iter().filter(|t| !t.starts_with(b"rota")));
    children.push(arrowheads(&[(0, 2)]));
    let d = read_cmx(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.warnings, vec![Warning::Arrowheads]);
}

#[test]
fn a_long_instruction_and_a_jump_over_image_bytes() {
    // The jump's target counts from the start of the file: header, `cont`, then the page body.
    let before = riff(&[cont(b'4', 1e-7), page(&[])]).len() - 8;
    let head = [begin_page(None), begin_layer("L", None)].concat();
    let jump_len = 4 + 1 + 3 + 4;
    let image = vec![0xEEu8; 37];
    let target = (before + 8 + head.len() + jump_len + image.len()) as u32;
    let jump = instr(111, &fields(&[field(1, &target.to_le_bytes())]));
    assert_eq!(jump.len(), jump_len);
    let body = [
        head,
        jump,
        image,
        long_instr(67, &curve(&attrs(Some(1), None, false), &[(0, 0), (10, 0), (10, 10)], &[0x0c, 0x44, 0x48])[4..]),
        end_layer(),
    ]
    .concat();
    let mut children = vec![cont(b'4', 1e-7), chunk(b"page", &body)];
    children.extend(tables());
    let d = read_cmx(&riff(&children), Limits::default()).unwrap();
    // The images jumped over are not read, and the warning says so.
    assert_eq!(d.warnings, [Warning::UnsupportedObjects]);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn bytes_after_a_chunk_that_runs_short_are_passed_over() {
    let stray = vec![0x10, 0x00, 0x16, 0x00, 0x01, 0x0b, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0xff];
    let mut children = vec![cont(b'4', 1e-7), stray, list(b"pgls", &[page(&[begin_page(None), square(&attrs(Some(1), None, false), 0, 0, 1000)])])];
    children.extend(tables());
    let d = read_cmx(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn other_byte_orders_and_coordinate_sizes_are_refused() {
    let mut f = sample();
    f[..4].copy_from_slice(b"RIFX");
    assert_eq!(read_cmx(&f, Limits::default()), Err(Error::Unsupported("big-endian CMX files")));
    let mut children = vec![cont(b'2', 1e-3)];
    children.extend(tables());
    assert_eq!(read_cmx(&riff(&children), Limits::default()), Err(Error::Unsupported("CMX files with 16-bit coordinates")));
    assert_eq!(read_cmx(&riff(&tables()), Limits::default()), Err(Error::Damaged("CMX header")));
    assert_eq!(read_cmx(&riff(&[cont(b'9', 1e-7)]), Limits::default()), Err(Error::Damaged("CMX header")));
}

#[test]
fn a_header_without_a_usable_unit_reads_0_1_micrometers() {
    for meters in [0.0, -1.0, f64::NAN, 2.0] {
        let mut children = vec![cont(b'4', meters), page(&[begin_page(Some((2_100_000, 2_970_000)))])];
        children.extend(tables());
        let d = read_cmx(&riff(&children), Limits::default()).unwrap();
        let (w, _) = d.pages[0].size.unwrap();
        assert!(near(w, 210.0 / 25.4 * 72.0), "{meters}: {w}");
    }
}

#[test]
fn groups_past_the_depth_limit_are_left_out() {
    let mut ins = vec![begin_page(None), begin_layer("L", None)];
    for _ in 0..6 {
        ins.push(begin_group());
    }
    ins.push(square(&attrs(Some(1), None, false), 0, 0, 1000));
    for _ in 0..6 {
        ins.push(end_group());
    }
    ins.push(square(&attrs(Some(1), None, false), 0, 0, 1000));
    let d = read_cmx(&file(&[page(&ins)]), Limits { depth: 3, ..Limits::default() }).unwrap();
    assert!(d.warnings.contains(&Warning::TooDeep));
    // The square after the groups is still read, beside the three groups kept.
    let objects = &d.pages[0].layers[0].objects;
    assert_eq!(objects.len(), 2);
    let mut depth = 0;
    let mut o = &objects[1];
    while let Object::Group { objects: kids, .. } = o {
        depth += 1;
        match kids.first() {
            Some(k) => o = k,
            None => break,
        }
    }
    assert_eq!(depth, 3);
}

#[test]
fn counts_past_the_limits_are_bounded() {
    let ins: Vec<Vec<u8>> = std::iter::once(begin_page(None)).chain((0..50).map(|_| square(&attrs(Some(1), None, false), 0, 0, 1000))).collect();
    let f = file(&[page(&ins)]);
    let d = read_cmx(&f, Limits { objects: 10, ..Limits::default() }).unwrap();
    assert_eq!(d.pages[0].layers[0].objects.len(), 10);
    assert!(d.warnings.contains(&Warning::TooDeep));
    let d = read_cmx(&f, Limits { coordinates: 12, ..Limits::default() }).unwrap();
    assert_eq!(d.pages[0].layers[0].objects.len(), 2);
    assert_eq!(read_cmx(&f, Limits { chunks: 20, ..Limits::default() }), Err(Error::Limit("chunk count")));
    assert_eq!(read_cmx(&f, Limits { input: 10, ..Limits::default() }), Err(Error::Limit("input size")));
    let dashed = |n: u16| {
        let mut children = vec![cont(b'4', 1e-7)];
        children.push(page(&[begin_page(None), square(&attrs(None, Some(3), false), 0, 0, 1000), square(&attrs(None, Some(3), false), 0, 0, 1000)]));
        children.extend(tables());
        read_cmx(&riff(&children), Limits { dashes: usize::from(n), ..Limits::default() }).unwrap()
    };
    let d = dashed(3);
    let o: Vec<usize> = d.pages[0].layers[0].objects.iter().map(|o| shape(o).outline.as_ref().unwrap().dashes.len()).collect();
    assert_eq!(o, [0, 2]);
    assert!(d.warnings.contains(&Warning::TooDeep));
}

#[test]
fn a_curve_cut_short_keeps_the_points_it_holds() {
    let mut c = curve(&attrs(Some(1), None, false), &[(0, 0), (10, 0), (10, 10)], &[0x0c, 0x44, 0x48]);
    // Claim 9 points where 3 are stored: the field's own length still covers only 3.
    let at = c.windows(2).position(|w| w == [3, 0]).unwrap();
    c[at] = 9;
    let p = page(&[begin_page(None), c]);
    let d = read_cmx(&file(&[p]), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged));
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn a_curve_that_lost_type_bytes_keeps_its_first_points() {
    // Three whole points and only two of their type bytes.
    let mut g = 3u16.to_le_bytes().to_vec();
    for (x, y) in [(0i32, 0i32), (1_000_000, 0), (1_000_000, 1_000_000)] {
        g.extend(x.to_le_bytes());
        g.extend(y.to_le_bytes());
    }
    g.extend([0x0c, 0x44]);
    let c = instr(67, &fields(&[field(1, &attrs(None, Some(1), false)), field(2, &g)]));
    let d = read_cmx(&file(&[page(&[begin_page(None), c])]), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    let Geometry::Path(p) = &shape(&d.pages[0].layers[0].objects[0]).geometry else { panic!() };
    let side = 100.0 / 25.4 * 72.0;
    assert!(near(p.subpaths[0].start.x, 0.0) && near(p.subpaths[0].start.y, 0.0));
    match p.subpaths[0].segments[..] {
        [craft_cdr::model::Segment::Line(e)] => assert!(near(e.x, side) && near(e.y, 0.0), "{e:?}"),
        ref other => panic!("{other:?}"),
    }
}

#[test]
fn a_file_cut_short_of_its_declared_size_is_reported() {
    let mut f = file(&[]);
    let declared = (f.len() as u32 - 8 + 100).to_le_bytes();
    f[4..8].copy_from_slice(&declared);
    let d = read_cmx(&f, Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
}

#[test]
fn a_page_that_ends_inside_an_instruction_is_reported() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer(), vec![1, 0]]);
    let d = read_cmx(&file(&[p]), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn a_chunk_64_bytes_past_stray_bytes_is_found() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut children = vec![cont(b'4', 1e-7), [vec![1u8; 64], p].concat()];
    children.extend(tables());
    let d = read_cmx(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn stray_letters_with_a_length_past_the_file_are_passed_over() {
    let mut stray = b"abcd".to_vec();
    stray.extend(0xFFFF_FF00u32.to_le_bytes());
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut children = vec![cont(b'4', 1e-7), [stray, p].concat()];
    children.extend(tables());
    let d = read_cmx(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn a_dash_pattern_of_zero_lengths_draws_a_solid_line() {
    let mut children = vec![cont(b'4', 1e-7)];
    children.push(page(&[begin_page(None), begin_layer("L", None), square(&attrs(None, Some(1), false), 0, 0, 1000)]));
    children.extend(tables().into_iter().filter(|t| !t.starts_with(b"rdot") && !t.starts_with(b"rotl")));
    children.push(table(b"rdot", &[dashes(&[0, 0])]));
    children.push(table(b"rotl", &[outline([3, 1, 2, 1, 1, 1])]));
    let d = read_cmx(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    let o = shape(&d.pages[0].layers[0].objects[0]).outline.clone().unwrap();
    assert!(o.dashes.is_empty(), "{:?}", o.dashes);
}

#[test]
fn a_curve_field_that_lost_points_is_left_out() {
    // Three points claimed, two whole points stored and the third cut inside its y: the type
    // bytes can't be found, so no node types are read from coordinates.
    let mut g = 3u16.to_le_bytes().to_vec();
    for (x, y) in [(0i32, 0i32), (1_000_000, 0)] {
        g.extend(x.to_le_bytes());
        g.extend(y.to_le_bytes());
    }
    g.extend(1_000_000i32.to_le_bytes());
    g.extend([0x40, 0x42]);
    let c = instr(67, &fields(&[field(1, &attrs(Some(1), None, false)), field(2, &g)]));
    let d = read_cmx(&file(&[page(&[begin_page(None), begin_layer("L", None), c])]), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    assert!(d.pages[0].layers.is_empty(), "{:?}", d.pages[0].layers);
}

#[test]
fn a_chunk_whose_id_has_a_digit_is_found_after_stray_bytes() {
    // Stray bytes, a chunk the reader passes over whose id ends in a digit, then the page.
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = vec![1u8; 6];
    b.extend(chunk(b"inf1", &[0; 70]));
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_cmx(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn padding_and_ids_with_impossible_lengths_are_not_taken_for_chunks() {
    // After stray bytes, four spaces with a small length, or letters with a length past the list,
    // come before the page: neither is taken for a chunk, so the page is found.
    for filler in [b"    \x08\x00\x00\x00".to_vec(), b"junk\x40\x42\x0f\x00".to_vec()] {
        let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
        let mut b = vec![1u8; 2];
        b.extend(filler);
        b.extend(p);
        let mut children = vec![cont(b'4', 1e-7), b];
        children.extend(tables());
        let d = read_cmx(&riff(&children), Limits::default()).unwrap();
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        assert_eq!(d.pages.len(), 1);
        assert_eq!(d.pages[0].layers[0].objects.len(), 1);
    }
}

#[test]
fn after_stray_bytes_a_chunk_the_reader_passes_over_keeps_what_it_holds() {
    // Stray bytes, then a chunk the reader passes over whose body starts with the letters of a
    // page chunk, then the page: only the page is read.
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut inner = b"page".to_vec();
    inner.extend(4u32.to_le_bytes());
    inner.extend([0; 10]);
    let mut b = vec![1u8; 2];
    b.extend(chunk(b"inf1", &inner));
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_cmx(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn a_canvas_or_a_container_is_reported() {
    for bit in [8u8, 16] {
        let mut a = attrs(Some(1), None, false);
        a[0] |= bit;
        let p = page(&[begin_page(None), begin_layer("L", None), square(&a, 0, 0, 1000), end_layer()]);
        let d = read_cmx(&file(&[p]), Limits::default()).unwrap();
        assert_eq!(d.warnings, vec![Warning::UnsupportedObjects], "{bit}");
        assert!(shape(&d.pages[0].layers[0].objects[0]).fill.is_some());
    }
}

#[test]
fn a_library_color_with_its_name_reads_as_a_spot_color() {
    let mut children = vec![cont(b'4', 1e-7)];
    children.push(page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000)]));
    children.push(table(b"rclr", &[color_record_named(5, [0, 0, 255, 0], Some("Brick 12"))]));
    let d = read_cmx(&riff(&children), Limits::default()).unwrap();
    match shape(&d.pages[0].layers[0].objects[0]).fill.clone() {
        Some(Color::Spot { name, tint, look }) => {
            assert_eq!((name.as_str(), tint), ("Brick 12", 1.0));
            assert!(look.is_some());
        }
        other => panic!("{other:?}"),
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn never_panics_on_garbage(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let mut b = b"RIFF\0\0\0\0CMX1".to_vec();
        b.extend(bytes);
        let _ = read_cmx(&b, Limits::default());
    }

    #[test]
    fn never_panics_on_mutated_samples(edits in prop::collection::vec((any::<usize>(), any::<u8>()), 1..24)) {
        let mut b = sample();
        let n = b.len();
        for (at, v) in edits {
            b[at % n] = v;
        }
        let _ = read_cmx(&b, Limits::default());
    }
}

//! Reading synthetic `.cmx` files built from the layouts in `PROVENANCE.md` (R53 to R63), and
//! hostile ones.

use craft_cdr::model::{Cap, Color, Geometry, Join, Object, Page, Shape};
use craft_cdr::synth_cmx::*;
use craft_cdr::{Drawing, Error, Limits, Warning, is_cdr, is_cmx, read_cmx};
use proptest::prelude::*;

/// Read `bytes`, and check that the reader never reads less than it does when it resumes after
/// stray bytes only at the first chunk id found (R53): when that read succeeds, this one does too,
/// and every page that read holds with objects is read here unchanged and in the same order.
fn read_checked(bytes: &[u8], limits: Limits) -> Result<Drawing, Error> {
    read_compared(bytes, limits, |p, q| p == q)
}

/// As [`read_checked`], except that a page only has to keep its layers and the number of objects
/// on each: a header or a table that only the recovery reads may change their colors and sizes
/// (R53).
fn read_kept(bytes: &[u8], limits: Limits) -> Result<Drawing, Error> {
    read_compared(bytes, limits, |p, q| {
        p.layers.len() == q.layers.len() && p.layers.iter().zip(&q.layers).all(|(a, b)| a.name == b.name && a.objects.len() == b.objects.len())
    })
}

fn read_compared(bytes: &[u8], limits: Limits, same: fn(&Page, &Page) -> bool) -> Result<Drawing, Error> {
    let d = read_cmx(bytes, limits);
    if let Ok(first) = read_cmx_without_recovery(bytes, limits) {
        let Ok(read) = &d else { panic!("{d:?} where the first chunk id gives {first:?}") };
        let mut rest = read.pages.iter();
        for p in first.pages.iter().filter(|p| !p.layers.is_empty()) {
            assert!(rest.any(|q| same(q, p)), "{p:?} is not read");
        }
    }
    d
}

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
    let d = read_checked(&sample(), Limits::default()).unwrap();
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
    let d = read_checked(&file(&[p]), Limits::default()).unwrap();
    let page = &d.pages[0];
    assert_eq!(page.size, None);
    // The empty layer is left out, and the UTF-16 name wins over the 8-bit one.
    let names: Vec<&str> = page.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Caf\u{c3}\u{a9}", "Слой 1"]);
}

#[test]
fn objects_before_any_layer_get_one() {
    let p = page(&[begin_page(None), square(&attrs(Some(1), None, false), 0, 0, 1000)]);
    let d = read_checked(&file(&[p]), Limits::default()).unwrap();
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
    let d = read_checked(&file(&[p]), Limits::default()).unwrap();
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
    let d = read_checked(&file(&[p]), Limits::default()).unwrap();
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
    let d = read_checked(&file(&[p]), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
}

#[test]
fn a_layer_name_too_short_to_read_in_utf_16_reads_in_8_bit() {
    let mut d = vec![1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0];
    d.extend(b"Art");
    let layer = instr(11, &fields(&[field(1, &d), field(4, &[3, 0])]));
    let p = page(&[begin_page(None), layer, square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let d = read_checked(&file(&[p]), Limits::default()).unwrap();
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
    let d = read_checked(&file(&[p]), Limits::default()).unwrap();
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
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
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
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
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
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
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
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    // The images jumped over are not read, and the warning says so.
    assert_eq!(d.warnings, [Warning::UnsupportedObjects]);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn bytes_after_a_chunk_that_runs_short_are_passed_over() {
    let stray = vec![0x10, 0x00, 0x16, 0x00, 0x01, 0x0b, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 0xff];
    let mut children = vec![cont(b'4', 1e-7), stray, list(b"pgls", &[page(&[begin_page(None), square(&attrs(Some(1), None, false), 0, 0, 1000)])])];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn other_byte_orders_and_coordinate_sizes_are_refused() {
    let mut f = sample();
    f[..4].copy_from_slice(b"RIFX");
    assert_eq!(read_checked(&f, Limits::default()), Err(Error::Unsupported("big-endian CMX files")));
    let mut children = vec![cont(b'2', 1e-3)];
    children.extend(tables());
    assert_eq!(read_checked(&riff(&children), Limits::default()), Err(Error::Unsupported("CMX files with 16-bit coordinates")));
    assert_eq!(read_checked(&riff(&tables()), Limits::default()), Err(Error::Damaged("CMX header")));
    assert_eq!(read_checked(&riff(&[cont(b'9', 1e-7)]), Limits::default()), Err(Error::Damaged("CMX header")));
}

#[test]
fn a_header_without_a_usable_unit_reads_0_1_micrometers() {
    for meters in [0.0, -1.0, f64::NAN, 2.0] {
        let mut children = vec![cont(b'4', meters), page(&[begin_page(Some((2_100_000, 2_970_000)))])];
        children.extend(tables());
        let d = read_checked(&riff(&children), Limits::default()).unwrap();
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
    let d = read_checked(&file(&[page(&ins)]), Limits { depth: 3, ..Limits::default() }).unwrap();
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
    let d = read_checked(&f, Limits { objects: 10, ..Limits::default() }).unwrap();
    assert_eq!(d.pages[0].layers[0].objects.len(), 10);
    assert!(d.warnings.contains(&Warning::TooDeep));
    let d = read_checked(&f, Limits { coordinates: 12, ..Limits::default() }).unwrap();
    assert_eq!(d.pages[0].layers[0].objects.len(), 2);
    assert_eq!(read_checked(&f, Limits { chunks: 20, ..Limits::default() }), Err(Error::Limit("chunk count")));
    assert_eq!(read_checked(&f, Limits { input: 10, ..Limits::default() }), Err(Error::Limit("input size")));
    let dashed = |n: u16| {
        let mut children = vec![cont(b'4', 1e-7)];
        children.push(page(&[begin_page(None), square(&attrs(None, Some(3), false), 0, 0, 1000), square(&attrs(None, Some(3), false), 0, 0, 1000)]));
        children.extend(tables());
        read_checked(&riff(&children), Limits { dashes: usize::from(n), ..Limits::default() }).unwrap()
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
    let d = read_checked(&file(&[p]), Limits::default()).unwrap();
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
    let d = read_checked(&file(&[page(&[begin_page(None), c])]), Limits::default()).unwrap();
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
    let d = read_checked(&f, Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
}

#[test]
fn a_page_that_ends_inside_an_instruction_is_reported() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer(), vec![1, 0]]);
    let d = read_checked(&file(&[p]), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn a_chunk_64_bytes_past_stray_bytes_is_found() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut children = vec![cont(b'4', 1e-7), [vec![1u8; 64], p].concat()];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
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
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

/// Stray bytes, then letters whose length covers exactly the page chunk after them, then the page.
fn page_inside_stray_letters(stray: &[u8]) -> Vec<u8> {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = stray.to_vec();
    b.extend(b"abcd");
    b.extend((p.len() as u32).to_le_bytes());
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    riff(&children)
}

#[test]
fn after_stray_letters_the_page_is_preferred_to_letters_that_hold_it() {
    let mut stray = b"wxyz".to_vec();
    stray.extend(0xFFFF_FF00u32.to_le_bytes());
    let d = read_checked(&page_inside_stray_letters(&stray), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_the_page_is_preferred_to_letters_that_hold_it() {
    let d = read_checked(&page_inside_stray_letters(&[1, 1]), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn letters_64_bytes_past_stray_bytes_do_not_hide_the_page_they_hold() {
    let d = read_checked(&page_inside_stray_letters(&[1; 64]), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_a_page_after_three_page_headers_that_do_not_read_inside_letters_is_read() {
    // Stray bytes, then letters whose length covers three page headers that reach its end and
    // whose instructions do not fit, then the page.
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let len = 24 + p.len();
    let mut body = Vec::new();
    for k in 0..3 {
        body.extend(b"page");
        body.extend(((len - 8 * k - 8) as u32).to_le_bytes());
    }
    body.extend(p);
    let mut b = vec![1u8; 2];
    b.extend(b"abcd");
    b.extend((body.len() as u32).to_le_bytes());
    b.extend(body);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_recoveries_stop_once_a_read_has_spent_its_allowance() {
    // Many copies of stray bytes followed by a chunk the reader passes over that holds a page:
    // the first pages are read, and once the checks have spent the read's allowance the reader
    // resumes at the chunk that holds each page, as it does without the recovery.
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut unit = vec![1u8; 2];
    unit.extend(chunk(b"inf1", &p));
    let copies = 2000;
    let mut children = vec![cont(b'4', 1e-7), unit.repeat(copies)];
    children.extend(tables());
    let f = riff(&children);
    let d = read_checked(&f, Limits::default()).unwrap();
    assert!(!d.pages.is_empty() && d.pages.len() < copies, "{} of {copies} pages", d.pages.len());
    assert!(read_cmx_without_recovery(&f, Limits::default()).unwrap().pages.is_empty());
}

#[test]
fn after_stray_bytes_a_page_past_a_limit_is_left_out_with_a_warning() {
    let f = page_inside_stray_letters(&[1, 1]);
    let all = read_cmx(&f, Limits::default()).unwrap().pages.len();
    assert_eq!(all, 1);
    let mut cut = 0;
    for chunks in 1..200 {
        let Ok(d) = read_checked(&f, Limits { chunks, ..Limits::default() }) else { continue };
        if d.pages.len() < all {
            cut += 1;
            assert!(d.warnings.contains(&Warning::TooDeep), "{chunks}: {:?}", d.warnings);
        }
    }
    assert!(cut > 0);
}

#[test]
fn a_recovered_page_spends_only_what_the_limits_leave_after_the_other_pages() {
    // Stray bytes, letters whose length covers a page, the page, then another page: with room for
    // one object, the last page keeps it and the warning tells of the page left out; with room for
    // both, the recovered page reads first.
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let q = page(&[begin_page(None), begin_layer("M", None), square(&attrs(Some(1), None, false), 0, 0, 2000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(b"abcd");
    b.extend((p.len() as u32).to_le_bytes());
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b, q];
    children.extend(tables());
    let f = riff(&children);
    let d = read_checked(&f, Limits { objects: 1, ..Limits::default() }).unwrap();
    assert_eq!(d.pages.len(), 1, "{:?}", d.pages);
    let last = d.pages.last().unwrap();
    assert_eq!((last.layers[0].name.as_str(), last.layers[0].objects.len()), ("M", 1));
    assert!(d.warnings.contains(&Warning::TooDeep), "{:?}", d.warnings);
    let d = read_checked(&f, Limits::default()).unwrap();
    assert_eq!(d.pages.iter().map(|p| p.layers[0].name.as_str()).collect::<Vec<_>>(), ["L", "M"]);
}

#[test]
fn chunks_only_a_recovery_reads_do_not_stop_a_read_that_fits_without_them() {
    // A page, then letters spelling `page` whose length runs past the file and that hold another
    // page and an empty chunk: at every chunk limit at which the file opens without the recovery,
    // it opens with it, and where the recovered page does not fit, the warning tells of it.
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let q = page(&[begin_page(None), begin_layer("M", None), square(&attrs(Some(1), None, false), 0, 0, 2000), end_layer()]);
    let mut b = b"page".to_vec();
    b.extend(0xFFFF_FF00u32.to_le_bytes());
    b.extend(q);
    b.extend(chunk(b"junk", &[]));
    let mut children = vec![cont(b'4', 1e-7)];
    children.extend(tables());
    children.push(p);
    children.push(b);
    let f = riff(&children);
    let names = |d: &Drawing| d.pages.iter().filter_map(|p| p.layers.first()).map(|l| l.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&read_checked(&f, Limits::default()).unwrap()), ["L", "M"]);
    let mut cut = 0;
    for chunks in 1..200 {
        let limits = Limits { chunks, ..Limits::default() };
        if read_cmx_without_recovery(&f, limits).is_err() {
            continue;
        }
        let d = read_checked(&f, limits).unwrap();
        if names(&d) != ["L", "M"] {
            cut += 1;
            assert!(d.warnings.contains(&Warning::TooDeep), "{chunks}: {:?}", d.warnings);
        }
    }
    assert!(cut > 0);
}

#[test]
fn after_stray_bytes_a_page_is_read_when_the_chunk_found_lands_inside_a_list() {
    // Stray bytes, then letters whose length ends on the header of an empty page at the end of the
    // page after them. That empty page's length ends on the first page inside the list that
    // follows. Walking on from the page reads the same page inside the list, so the page is read.
    let (name, q) = ["Q", "Qa"]
        .iter()
        .map(|n| {
            let carried = long_instr(-1, &[b"page".as_slice(), &12u32.to_le_bytes()].concat());
            (*n, [begin_page(None), begin_layer(n, None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer(), carried].concat())
        })
        .find(|(_, body)| body.len() % 2 == 0)
        .unwrap();
    let r = page(&[begin_page(None), begin_layer("R", None), square(&attrs(Some(1), None, false), 0, 0, 2000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(b"abcd");
    b.extend((q.len() as u32).to_le_bytes());
    b.extend(chunk(b"page", &q));
    b.extend(list(b"pgls", &[r]));
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    let names: Vec<&str> = d.pages.iter().filter_map(|p| p.layers.first()).map(|l| l.name.as_str()).collect();
    assert_eq!(names, [name, "R"]);
}

#[test]
fn after_stray_bytes_a_page_is_not_read_when_a_list_cuts_short_the_page_both_walks_reach() {
    // As above, except that the list ends after the header of the page inside it, so walking on
    // from the inner page reads that page without its body. Walking on from the letters reads the
    // whole page, so the reader resumes at the letters and the page keeps its square.
    let (name, q) = ["Q", "Qa"]
        .iter()
        .map(|n| {
            let carried = long_instr(-1, &[b"page".as_slice(), &12u32.to_le_bytes()].concat());
            (*n, [begin_page(None), begin_layer(n, None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer(), carried].concat())
        })
        .find(|(_, body)| body.len() % 2 == 0)
        .unwrap();
    let r = page(&[begin_page(None), begin_layer("R", None), square(&attrs(Some(1), None, false), 0, 0, 2000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(b"abcd");
    b.extend((q.len() as u32).to_le_bytes());
    b.extend(chunk(b"page", &q));
    b.extend(b"LIST");
    b.extend(12u32.to_le_bytes());
    b.extend(b"pgls");
    b.extend(r);
    let mut children = vec![cont(b'4', 1e-7)];
    children.extend(tables());
    children.push(b);
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    let names: Vec<&str> = d.pages.iter().filter_map(|p| p.layers.first()).map(|l| l.name.as_str()).collect();
    assert!(!names.contains(&name), "{names:?}");
    assert_eq!(names, ["R"]);
    assert_eq!(d.pages.last().unwrap().layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_a_page_a_list_cuts_short_only_without_the_recovery_is_judged_as_recovered() {
    // Stray bytes, then letters whose length ends on the header of a list at the end of the page
    // after them. The list ends after the header of the next page, so the walk from the letters
    // reads that page without its body, and the walk from the page inside the letters reads the
    // whole page. Only the recovery reads that whole page, and it ends inside an instruction, so the
    // reader leaves it out.
    let (name, q) = ["Q", "Qa"]
        .iter()
        .map(|n| {
            let carried = long_instr(-1, &[b"LIST".as_slice(), &12u32.to_le_bytes(), b"pgls"].concat());
            (*n, [begin_page(None), begin_layer(n, None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer(), carried].concat())
        })
        .find(|(_, body)| body.len() % 2 == 0)
        .unwrap();
    let r = page(&[begin_page(None), begin_layer("R", None), square(&attrs(Some(1), None, false), 0, 0, 2000), end_layer(), vec![1, 0]]);
    let mut b = vec![1u8; 2];
    b.extend(b"abcd");
    b.extend((q.len() as u32 - 4).to_le_bytes());
    b.extend(chunk(b"page", &q));
    b.extend(r);
    let mut children = vec![cont(b'4', 1e-7)];
    children.extend(tables());
    children.push(b);
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    let names: Vec<&str> = d.pages.iter().filter_map(|p| p.layers.first()).map(|l| l.name.as_str()).collect();
    assert_eq!(names, [name]);
}

#[test]
fn after_stray_bytes_a_page_whose_check_stops_at_a_limit_is_not_replaced_by_a_page_inside_it() {
    // Stray bytes, then letters whose body holds the start of a page of one square. That page's last
    // instruction carries the page of thirty squares that follows the letters, and ends where it
    // ends. At a chunk limit that stops the check of the page of thirty squares while the check of
    // the page inside the letters fits, the page inside the letters never takes its place.
    let squares = (1..=30).map(|i| square(&attrs(Some(1), None, false), 0, 0, 100 * i));
    let p = page(&[begin_page(None), begin_layer("P", None)].into_iter().chain(squares).chain([end_layer()]).collect::<Vec<_>>());
    let carried = long_instr(-1, &p);
    let head = &carried[..carried.len() - p.len()];
    let q = ["Q", "Qa"]
        .iter()
        .map(|n| [begin_page(None), begin_layer(n, None), square(&attrs(Some(1), None, false), 0, 0, 2000), end_layer(), head.to_vec()].concat())
        .find(|body| body.len() % 2 == 0)
        .unwrap();
    let mut b = vec![1u8; 2];
    b.extend(b"abcd");
    b.extend((q.len() as u32 + 8).to_le_bytes());
    b.extend(b"page");
    b.extend(((q.len() + p.len()) as u32).to_le_bytes());
    b.extend(q);
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7)];
    children.extend(tables());
    children.push(b);
    let f = riff(&children);
    let names = |d: &Drawing| d.pages.iter().filter_map(|p| p.layers.first()).map(|l| l.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&read_checked(&f, Limits::default()).unwrap()), ["P"]);
    for chunks in 1..200 {
        if let Ok(d) = read_checked(&f, Limits { chunks, ..Limits::default() }) {
            assert_eq!(names(&d), ["P"], "{chunks}: {:?}", d.warnings);
        }
    }
}

/// A page that fills with color 1, then, inside a list, letters spelling `page` whose length runs
/// past the list and that hold another page and a color table of one red entry, then `after`.
fn page_and_recovered_red(after: &[Vec<u8>]) -> Vec<u8> {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let q = page(&[begin_page(None), begin_layer("M", None), square(&attrs(Some(1), None, false), 0, 0, 2000), end_layer()]);
    let mut b = b"page".to_vec();
    b.extend(0xFFFF_FF00u32.to_le_bytes());
    b.extend(q);
    b.extend(table(b"rclr", &[color(5, &[255, 0, 0])]));
    let mut children = vec![cont(b'4', 1e-7)];
    children.extend(tables().into_iter().filter(|t| !t.starts_with(b"rclr")));
    children.push(p);
    children.push(list(b"pgls", &[b]));
    children.extend(after.iter().cloned());
    riff(&children)
}

/// The fill of the first object of each page.
fn fills(d: &Drawing) -> Vec<Option<Color>> {
    d.pages.iter().map(|p| shape(&p.layers[0].objects[0]).fill.clone()).collect()
}

#[test]
fn a_color_table_only_the_recovery_reads_colors_every_page() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000)]);
    let red = fills(&read_checked(&file(&[p]), Limits::default()).unwrap())[0].clone();
    assert!(red.is_some());
    let d = read_kept(&page_and_recovered_red(&[]), Limits::default()).unwrap();
    assert_eq!(fills(&d), [red.clone(), red]);
}

#[test]
fn a_color_table_read_without_the_recovery_wins_over_one_only_the_recovery_reads() {
    let green = table(b"rclr", &[color(5, &[0, 255, 0])]);
    let mut children = vec![cont(b'4', 1e-7)];
    children.push(page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000)]));
    children.extend(tables().into_iter().filter(|t| !t.starts_with(b"rclr")));
    children.push(green.clone());
    let green_fill = fills(&read_checked(&riff(&children), Limits::default()).unwrap())[0].clone();
    let d = read_checked(&page_and_recovered_red(&[green]), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 2);
    assert_eq!(fills(&d), [green_fill.clone(), green_fill]);
}

#[test]
fn after_stray_bytes_a_page_inside_a_chunk_with_more_after_it_is_not_read() {
    // Stray bytes, a chunk the reader passes over that holds a page followed by another chunk,
    // then the page: only the last page is read.
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(chunk(b"inf1", &[p.clone(), chunk(b"junk", &[0; 4])].concat()));
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert_eq!(d.pages.len(), 1);
}

#[test]
fn a_list_between_stray_bytes_is_read_as_a_list() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let b = [vec![1u8; 2], list(b"pgls", &[p.clone(), p]), vec![1u8; 2]].concat();
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert_eq!(d.pages.len(), 2);
}

#[test]
fn after_stray_bytes_an_empty_page_header_inside_another_chunk_is_not_a_page() {
    // Stray bytes, a chunk the reader passes over that holds only an empty page header, then the
    // page: only the page is read.
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(chunk(b"inf1", &chunk(b"page", &[])));
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_an_empty_page_is_kept_before_the_next_page() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(chunk(b"page", &[]));
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 2);
    assert!(d.pages[0].layers.is_empty(), "{:?}", d.pages[0].layers);
    assert_eq!(d.pages[1].layers[0].objects.len(), 1);
}

#[test]
fn a_page_between_stray_bytes_is_preferred_to_letters_that_end_inside_it() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(b"abcd");
    b.extend(8u32.to_le_bytes());
    b.extend(p);
    b.extend([1u8; 2]);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_a_page_is_preferred_to_page_letters_that_end_inside_it() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(b"page");
    b.extend(8u32.to_le_bytes());
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn a_page_between_stray_bytes_is_kept() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let b = [vec![1u8; 2], p, vec![1u8; 2]].concat();
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_a_page_header_that_ends_inside_another_chunk_is_not_a_page() {
    // Stray bytes, a chunk the reader passes over whose body starts with a short page chunk, more
    // stray bytes, then the page: only the last page is read.
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut inner = b"page".to_vec();
    inner.extend(4u32.to_le_bytes());
    inner.extend([0; 10]);
    let mut b = vec![1u8; 2];
    b.extend(chunk(b"inf1", &inner));
    b.extend([1u8; 2]);
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    // The chunk passed over is followed by more stray bytes, and the warning says so.
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_a_header_too_short_to_read_inside_another_chunk_is_passed_over() {
    // Stray bytes, then a chunk the reader passes over that holds a one-byte header, then the
    // header: the drawing opens with the header after it.
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let first = [vec![1u8; 2], chunk(b"inf1", &chunk(b"cont", &[0]))].concat();
    let mut children = vec![first, cont(b'4', 1e-7), p];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_letters_ending_on_a_layer_name_do_not_hide_the_page() {
    // Stray bytes, then letters whose length ends on the letters of a layer name inside the page
    // after them, then the page.
    let names = ["Background", "aBackground"];
    let (p, at) = names
        .iter()
        .map(|n| page(&[begin_page(None), begin_layer(n, None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]))
        .find_map(|p| p.windows(4).position(|w| w == b"Back").filter(|at| at % 2 == 0).map(|at| (p, at)))
        .unwrap();
    let mut b = vec![1u8; 2];
    b.extend(b"abcd");
    b.extend((at as u32).to_le_bytes());
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_a_page_followed_by_a_chunk_inside_another_chunk_is_not_read_before_more_stray_bytes() {
    // Stray bytes, a chunk the reader passes over that holds a page and another chunk, more stray
    // bytes, then the page: only the last page is read.
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(chunk(b"inf1", &[p.clone(), chunk(b"junk", &[0; 4])].concat()));
    b.extend([1u8; 2]);
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
}

#[test]
fn after_stray_bytes_a_header_without_a_coordinate_size_inside_another_chunk_is_passed_over() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let first = [vec![1u8; 2], chunk(b"inf1", &chunk(b"cont", &[0; 60]))].concat();
    let mut children = vec![first, cont(b'4', 1e-7), p];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_a_table_inside_another_chunk_that_ends_inside_the_next_page_is_passed_over() {
    // Stray bytes, a chunk the reader passes over that holds the start of a color table whose
    // length ends inside the page after more stray bytes: the page is read.
    let sq = square(&attrs(Some(1), None, false), 0, 0, 1000);
    let p = page(&[begin_page(None), begin_layer("L", None), sq.clone(), sq.clone(), sq, end_layer()]);
    let mut inner = b"rclr".to_vec();
    inner.extend(14u32.to_le_bytes());
    inner.extend([0; 2]);
    let mut b = vec![1u8; 2];
    b.extend(chunk(b"inf1", &inner));
    b.extend([1u8; 2]);
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 3);
}

#[test]
fn a_page_between_stray_bytes_is_kept_when_its_body_holds_chunk_letters() {
    // A layer name near the start of the page holds a color table chunk and another chunk id.
    let mut tail = b"rclr".to_vec();
    tail.extend([4, 0, 0, 0, 0, 0, 0, 0]);
    tail.extend(b"junk");
    tail.extend([0; 4]);
    let tail = String::from_utf8(tail).unwrap();
    let (p, at) = ["", "x"]
        .iter()
        .map(|pre| {
            page(&[begin_page(None), begin_layer(&format!("{pre}{tail}"), None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()])
        })
        .find_map(|p| p.windows(4).position(|w| w == b"rclr").filter(|at| at % 2 == 0).map(|at| (p, at)))
        .unwrap();
    assert!((8..=64).contains(&at), "{at}");
    let b = [vec![1u8; 2], p, vec![1u8; 2]].concat();
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_a_page_too_short_for_an_instruction_inside_another_chunk_is_not_a_page() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(chunk(b"inf1", &chunk(b"page", &[0; 2])));
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_a_short_page_is_not_hidden_by_a_table_inside_the_chunk_before_it() {
    // Stray bytes, a chunk the reader passes over that holds the start of a color table whose
    // length ends inside the short page after more stray bytes: the page is read.
    let p = page(&[begin_page(None)]);
    let mut inner = b"rclr".to_vec();
    inner.extend(10u32.to_le_bytes());
    inner.extend([0; 2]);
    let mut b = vec![1u8; 2];
    b.extend(chunk(b"inf1", &inner));
    b.extend([1u8; 2]);
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
}

#[test]
fn a_last_page_before_stray_bytes_at_the_end_of_the_file_is_preferred_to_letters_that_end_inside_it() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(b"abcd");
    b.extend(8u32.to_le_bytes());
    b.extend(p);
    b.extend([1u8; 2]);
    let mut children = vec![cont(b'4', 1e-7)];
    children.extend(tables());
    children.push(b);
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn a_page_between_stray_bytes_is_kept_when_its_body_holds_a_table_that_reaches_its_end() {
    // Bytes the page's long instruction carries hold a color table whose length reaches the end
    // of the page.
    let sq = square(&attrs(Some(1), None, false), 0, 0, 1000);
    let mut table = b"rclr".to_vec();
    let rest = [begin_layer("L", None), sq, end_layer()].concat();
    // The table's body: the rest of the long instruction (2 bytes), then the instructions after it.
    table.extend(((2 + rest.len()) as u32).to_le_bytes());
    table.extend([0; 2]);
    let carried = long_instr(-1, &table);
    let p = page(&[begin_page(None), carried, rest]);
    let at = p.windows(4).position(|w| w == b"rclr").unwrap();
    assert!(at % 2 == 0 && (8..=64).contains(&at), "{at}");
    let b = [vec![1u8; 2], p, vec![1u8; 2]].concat();
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_a_page_whose_first_instruction_is_empty_inside_another_chunk_is_not_a_page() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(chunk(b"inf1", &chunk(b"page", &[0; 4])));
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn a_header_between_stray_bytes_is_kept_when_its_body_holds_a_table_that_reaches_its_end() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut c = cont(b'4', 1e-7);
    let inner = (c.len() - 24) as u32;
    c[16..20].copy_from_slice(b"rclr");
    c[20..24].copy_from_slice(&inner.to_le_bytes());
    let mut children = vec![[vec![1u8; 2], c, vec![1u8; 2]].concat(), p];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_a_table_whose_entries_do_not_fit_inside_another_chunk_is_passed_over() {
    // Stray bytes, then a chunk the reader passes over that holds a color table of one entry with
    // no bytes for it, then the real tables: the colors come from the real tables.
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let read = |junk: &[u8]| {
        let mut children = vec![cont(b'4', 1e-7), junk.to_vec()];
        children.extend(tables());
        children.push(p.clone());
        read_checked(&riff(&children), Limits::default()).unwrap()
    };
    let junk = [vec![1u8; 2], chunk(b"inf1", &chunk(b"rclr", &[1, 0]))].concat();
    let d = read(&junk);
    let clean = read(&[]);
    assert_eq!(format!("{:?}", d.pages), format!("{:?}", clean.pages));
}

#[test]
fn after_stray_bytes_a_page_whose_second_instruction_is_empty_inside_another_chunk_is_not_a_page() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(chunk(b"inf1", &chunk(b"page", &[instr(-1, &[]), vec![0; 4]].concat())));
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert_eq!(d.pages.len(), 1);
    assert_eq!(d.pages[0].layers[0].objects.len(), 1);
}

#[test]
fn after_stray_bytes_page_letters_with_a_length_past_the_list_do_not_hide_the_page() {
    let p = page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000), end_layer()]);
    let mut b = vec![1u8; 2];
    b.extend(b"page");
    b.extend(0xFFFF_FF00u32.to_le_bytes());
    b.extend(p);
    let mut children = vec![cont(b'4', 1e-7), b];
    children.extend(tables());
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
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
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
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
    let d = read_checked(&file(&[page(&[begin_page(None), begin_layer("L", None), c])]), Limits::default()).unwrap();
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
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
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
        let d = read_checked(&riff(&children), Limits::default()).unwrap();
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
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
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
        let d = read_checked(&file(&[p]), Limits::default()).unwrap();
        assert_eq!(d.warnings, vec![Warning::UnsupportedObjects], "{bit}");
        assert!(shape(&d.pages[0].layers[0].objects[0]).fill.is_some());
    }
}

#[test]
fn a_library_color_with_its_name_reads_as_a_spot_color() {
    let mut children = vec![cont(b'4', 1e-7)];
    children.push(page(&[begin_page(None), begin_layer("L", None), square(&attrs(Some(1), None, false), 0, 0, 1000)]));
    children.push(table(b"rclr", &[color_record_named(5, [0, 0, 255, 0], Some("Brick 12"))]));
    let d = read_checked(&riff(&children), Limits::default()).unwrap();
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
        let _ = read_kept(&b, Limits::default());
    }

    #[test]
    fn never_panics_on_mutated_samples(edits in prop::collection::vec((any::<usize>(), any::<u8>()), 1..24)) {
        let mut b = sample();
        let n = b.len();
        for (at, v) in edits {
            b[at % n] = v;
        }
        let _ = read_kept(&b, Limits::default());
    }
}

//! Reading synthetic drawings of every version the reader takes, plain and compressed, and
//! hostile files.

use craft_cdr::model::{Blend, Cap, Color, Flags, Geometry, Join, LayerKind, Matrix, Object, Segment};
use craft_cdr::synth::{self, V, chunk, list};
use craft_cdr::{Error, Limits, Warning, preview, read};
use proptest::prelude::*;

fn shapes(objects: &[Object]) -> Vec<&craft_cdr::model::Shape> {
    let mut out = Vec::new();
    let mut stack: Vec<&Object> = objects.iter().rev().collect();
    while let Some(o) = stack.pop() {
        match o {
            Object::Shape(s) => out.push(&**s),
            Object::Group { objects: kids, .. } => stack.extend(kids.iter().rev()),
            Object::Clip { container, objects } => {
                out.push(&**container);
                stack.extend(objects.iter().rev());
            }
        }
    }
    out
}

/// The page and layers of the sample of `version`.
fn check_pages(d: &craft_cdr::Drawing, version: u16) {
    assert_eq!(d.version, version);
    let (w, h) = d.page_size.unwrap();
    assert!((w - 612.0).abs() < 0.1 && (h - 792.0).abs() < 0.1, "{version}: {w} {h}");
    assert_eq!(d.pages.len(), 1, "{version}");
    let art = &d.pages[0].layers[0];
    assert_eq!((art.name.as_str(), art.kind), ("Art", LayerKind::Drawing));
    assert!(matches!(art.objects[3], Object::Group { objects: ref g, .. } if g.len() == 2));
    assert!(d.warnings.contains(&Warning::UnsupportedObjects));
    // The master page: its guideline every page shows, and its desktop object.
    if version >= 600 {
        assert_eq!(d.guides.len(), 1);
        assert!((d.guides[0].angle - 90.0).abs() < 1e-9 && (d.guides[0].at.x - 36.0).abs() < 0.01);
    }
    assert_eq!(d.master.len(), 1);
    assert_eq!(d.master[0].kind, LayerKind::Desktop);
}

/// The shapes of the sample of `version`: rectangle, ellipse, curve, and the group's polygon and
/// arc; the text is left out.
fn check_shapes(d: &craft_cdr::Drawing, version: u16) {
    let s = shapes(&d.pages[0].layers[0].objects);
    assert_eq!(s.len(), 5, "{version}");
    assert!(matches!(s[0].geometry, Geometry::Rect { radii, .. } if radii.iter().all(|r| (r - 5.0).abs() < 0.1)));
    // The samples of versions 3 and 4 store their red as CMYK in percent.
    let red = if version < 500 { Color::Cmyk([0.0, 1.0, 1.0, 0.0]) } else { Color::Rgb([1.0, 0.0, 0.0]) };
    assert_eq!(s[0].fill, Some(red), "{version}");
    let o = s[0].outline.as_ref().unwrap();
    assert!((o.width - 2.0).abs() < 0.05 && o.cap == Cap::Round, "{version}: {o:?}");
    assert!((s[0].matrix.0[4] + 100.0).abs() < 0.1 && (s[0].matrix.0[5] - 100.0).abs() < 0.1);
    assert!(matches!(s[1].geometry, Geometry::Ellipse { .. }) && s[1].outline.is_none());
    assert!(matches!(&s[2].geometry, Geometry::Path(p) if p.subpaths[0].closed && matches!(p.subpaths[0].segments[1], Segment::Cubic(..))));
    assert!(s[2].fill.is_none());
    if version >= 600 {
        let Geometry::Path(p) = &s[3].geometry else { panic!("polygon") };
        assert_eq!(p.subpaths[0].segments.len(), 10, "five sides of two segments");
        // 5 % of 1000 mm.
        let r = p.subpaths[0].start.y;
        assert!((r - 0.05 * synth::POLYGON_SIDE).abs() < 0.5, "{r}");
    }
    assert!(matches!(&s[4].geometry, Geometry::Path(p) if p.subpaths[0].closed));
}

#[test]
fn every_version_from_5_to_13_reads_its_sample() {
    for version in [500, 600, 700, 800, 900, 1000, 1100, 1200, 1300] {
        for compress in [false, version >= 700] {
            let d = read(&synth::sample(version, compress), Limits::default()).unwrap_or_else(|e| panic!("{version} {compress}: {e}"));
            check_pages(&d, version);
            check_shapes(&d, version);
        }
    }
}

#[test]
fn versions_3_and_4_read_their_sample() {
    for version in [300, 400] {
        let file = synth::sample(version, false);
        assert_eq!(&file[8..12], if version == 300 { b"CDR " } else { b"CDR4" });
        assert!(craft_cdr::is_cdr(&file));
        let d = read(&file, Limits::default()).unwrap_or_else(|e| panic!("{version}: {e}"));
        assert!(!d.warnings.contains(&Warning::Damaged), "{version}: {:?}", d.warnings);
        check_pages(&d, version);
        check_shapes(&d, version);
        let s = shapes(&d.pages[0].layers[0].objects);
        let o = s[0].outline.as_ref().unwrap();
        assert_eq!((o.join, &o.color), (Join::Round, &Color::Cmyk([0.0, 0.0, 0.0, 1.0])), "{version}");
        // The ellipse 50 pt right of and below the page centre, and the curve where it was drawn.
        assert!((s[1].matrix.0[4] - 50.0).abs() < 0.1 && (s[1].matrix.0[5] + 50.0).abs() < 0.1, "{version}: {:?}", s[1].matrix);
        assert_eq!(s[2].matrix, Matrix::IDENTITY, "{version}");
    }
    // The form type alone names version 3, and a `vrsn` chunk names it under another form type.
    let mut plain = vec![];
    plain.extend(synth::sample_content(300));
    check_shapes(&read(&synth::riff(300, &plain), Limits::default()).unwrap(), 300);
    let mut relabeled = synth::sample(300, false);
    relabeled[11] = b'5';
    assert_eq!(read(&relabeled, Limits::default()).unwrap().version, 300);
}

/// A version 3 drawing of one layer holding `objects`.
fn drawing3(objects: Vec<synth::Node>) -> Vec<u8> {
    let page = synth::page(false, vec![synth::layer(V(300), [0, 0, 0, 0x98], "L", objects)]);
    synth::riff(300, &[chunk(b"vrsn", 300u16.to_le_bytes().to_vec()), page])
}

/// A version 3 rectangle of 10 × 10 pt with the transforms `matrices`.
fn rect3(matrices: &[[f64; 6]]) -> synth::Node {
    let v = V(300);
    let mut args = vec![(30, v.rect(10.0, 10.0, 0.0))];
    if !matrices.is_empty() {
        args.push((100, synth::transforms3(matrices)));
    }
    synth::object(v, 1, &args, None)
}

#[test]
fn version_3_groups_place_their_objects() {
    let shift = |x: f64, y: f64| [1.0, 0.0, 0.0, 1.0, x, y];
    let double = [2.0, 0.0, 0.0, 2.0, 0.0, 0.0];
    // A rectangle moved 72 pt right, in a group that doubles sizes, in a group that moves 36 pt
    // up; beside the inner group, a rectangle of the outer group alone; then one outside both.
    let inner = synth::group3(&[double], vec![rect3(&[shift(72.0, 0.0)])]);
    let outer = synth::group3(&[shift(0.0, 36.0)], vec![inner, rect3(&[])]);
    let d = read(&drawing3(vec![outer, rect3(&[shift(7.2, 0.0)])]), Limits::default()).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    let objects = &d.pages[0].layers[0].objects;
    assert!(
        matches!(&objects[0], Object::Group { objects: g, .. } if matches!(&g[0], Object::Group { objects: i, .. } if i.len() == 1) && g.len() == 2)
    );
    let s = shapes(objects);
    let close = |m: &Matrix, want: [f64; 6]| m.0.iter().zip(want).all(|(a, b)| (a - b).abs() < 0.01);
    // Its own shift, then the inner group's scale, then the outer group's shift.
    assert!(close(&s[0].matrix, [2.0, 0.0, 0.0, 2.0, 144.0, 36.0]), "{:?}", s[0].matrix);
    assert!(close(&s[1].matrix, [1.0, 0.0, 0.0, 1.0, 0.0, 36.0]), "{:?}", s[1].matrix);
    assert!(close(&s[2].matrix, [1.0, 0.0, 0.0, 1.0, 7.2, 0.0]), "{:?}", s[2].matrix);
    // Two matrices of one object apply in file order.
    let d = read(&drawing3(vec![rect3(&[shift(72.0, 0.0), double])]), Limits::default()).unwrap();
    assert!(close(&shapes(&d.pages[0].layers[0].objects)[0].matrix, [2.0, 0.0, 0.0, 2.0, 144.0, 0.0]));
}

#[test]
fn version_3_objects_and_groups_under_an_envelope_keep_their_outlines_and_matrices() {
    // A transforms argument of one envelope entry (type 0x14).
    let mut envelope = vec![0, 0, 1, 0, 0x14, 0];
    envelope.resize(14 + 24, 0);
    // A matrix that moves 72 pt right, then an envelope entry whose data follows the matrix's.
    let matrix = synth::transforms3(&[[1.0, 0.0, 0.0, 1.0, 72.0, 0.0]]).split_off(14);
    let mut shifted = vec![0, 0, 2, 0, 0x8c, 0, 0, 0, 0, 0, 24, 0, 0, 0, 0x14, 0, 0, 0, 0, 0, 48, 0, 0, 0];
    shifted.extend(matrix);
    shifted.resize(48 + 24, 0);
    let v = V(300);
    let bent = synth::object(v, 1, &[(30, v.rect(10.0, 10.0, 0.0)), (100, envelope.clone())], None);
    let moved = synth::object(v, 1, &[(30, v.rect(10.0, 10.0, 0.0)), (100, shifted)], None);
    let group = list(b"grp ", vec![chunk(b"lobj", v.loda(0, &[(100, envelope)])), rect3(&[])]);
    let d = read(&drawing3(vec![bent, moved, group, rect3(&[])]), Limits::default()).unwrap();
    let objects = &d.pages[0].layers[0].objects;
    assert_eq!(objects.len(), 4);
    assert!(matches!(&objects[2], Object::Group { objects: g, .. } if g.len() == 1));
    let s = shapes(objects);
    assert_eq!(s.len(), 4);
    assert!(matches!(s[0].geometry, Geometry::Rect { .. }) && s[0].matrix == Matrix::IDENTITY);
    assert!((s[1].matrix.0[4] - 72.0).abs() < 0.01 && s[1].matrix.0[5].abs() < 0.01, "{:?}", s[1].matrix);
    assert_eq!(s[2].matrix, Matrix::IDENTITY);
    assert_eq!(d.warnings, vec![Warning::Envelopes]);
}

#[test]
fn version_3_object_types_count_from_0() {
    let v = V(300);
    // `lobj` records written with their stored types: 0 rectangle, 2 curve, 3 text, 4 bitmap.
    let raw = |kind: u32, coords: Vec<u8>| list(b"obj ", vec![chunk(b"lobj", v.loda(kind, &[(30, coords)]))]);
    let curve = v.points(&[(0.0, 0.0, 0x00), (10.0, 0.0, 0x40)]);
    let ellipse = [v.coord(40.0), v.coord(20.0), v.angle(0.0), v.angle(0.0), v.word(0)].concat();
    let d = read(
        &drawing3(vec![raw(0, v.rect(10.0, 10.0, 0.0)), raw(1, ellipse), raw(2, curve), raw(3, vec![0; 8]), raw(4, vec![0; 8])]),
        Limits::default(),
    )
    .unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert_eq!(s.len(), 3);
    assert!(matches!(s[0].geometry, Geometry::Rect { .. }) && matches!(s[1].geometry, Geometry::Ellipse { .. }));
    assert!(matches!(&s[2].geometry, Geometry::Path(p) if p.subpaths.len() == 1));
    assert_eq!(d.warnings, vec![Warning::UnsupportedObjects]);
}

#[test]
fn version_3_fills_and_outlines_read_from_the_object() {
    let v = V(300);
    let styled = |fill: Vec<u8>, outline: Vec<u8>| synth::object(v, 1, &[(20, fill), (10, outline), (30, v.rect(10.0, 10.0, 0.0))], None);
    let pen = |line_type: u16| synth::Pen { line_type, width: 3.0, cap: 2, join: 2 };
    let black = synth::color3(1, [0, 0, 0, 100]);
    let d = read(
        &drawing3(vec![
            styled(synth::fill3(1, &synth::color3(0, [0, 0, 40, 0])), synth::outline3(pen(0), black, &[3, 1])),
            styled(synth::fill3(1, &synth::color3(0, [1, 0, 100, 0])), synth::outline3(pen(1), black, &[])),
            styled(synth::fill3(0, &[]), synth::outline3(pen(0), synth::color3(0, [200, 0, 80, 0]), &[])),
        ]),
        Limits::default(),
    )
    .unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    // Ink 0 is black at its tint, ink 1 white, and another number a library ink.
    assert_eq!((s[0].fill.clone(), s[1].fill.clone(), s[2].fill.clone()), (Some(Color::Gray(0.4)), Some(Color::Gray(0.0)), None));
    let o = s[0].outline.as_ref().unwrap();
    assert!((o.width - 3.0).abs() < 0.05, "{}", o.width);
    assert_eq!((o.cap, o.join, &o.dashes), (Cap::Square, Join::Bevel, &vec![3.0, 1.0]));
    assert!(s[1].outline.is_none());
    assert_eq!(s[2].outline.as_ref().unwrap().color, Color::Spot { name: "Spot 0:200".into(), tint: 0.8, look: None });
    assert_eq!(d.warnings, vec![Warning::SpotLooks]);
    // Fountain, texture and pattern fills are named and leave the shape unfilled; a fill or an
    // outline cut short is damage.
    for kind in [2, 4, 6, 7, 10] {
        let d = read(&drawing3(vec![styled(synth::fill3(kind, &[0; 12]), synth::outline3(pen(1), black, &[]))]), Limits::default()).unwrap();
        assert_eq!(d.warnings, vec![Warning::UnsupportedFills], "{kind}");
        assert!(shapes(&d.pages[0].layers[0].objects)[0].fill.is_none());
    }
    let d = read(&drawing3(vec![styled(vec![1, 1], vec![1, 0])]), Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert!(s[0].fill.is_none() && s[0].outline.is_none());
    assert_eq!(d.warnings, vec![Warning::Damaged]);
}

#[test]
fn version_3_dashes_count_toward_the_dash_limit() {
    let v = V(300);
    let pen = synth::Pen { line_type: 0, width: 1.0, cap: 0, join: 0 };
    let dashed =
        || synth::object(v, 1, &[(10, synth::outline3(pen, synth::color3(1, [0, 0, 0, 100]), &[2, 2, 2])), (30, v.rect(10.0, 10.0, 0.0))], None);
    let d = read(&drawing3(vec![dashed(), dashed()]), Limits { dashes: 4, ..Limits::default() }).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert_eq!((s[0].outline.as_ref().unwrap().dashes.len(), s[1].outline.as_ref().unwrap().dashes.len()), (3, 0));
    assert_eq!(d.warnings, vec![Warning::TooDeep]);
}

#[test]
fn hostile_version_3_transforms_are_damage() {
    let v = V(300);
    let with = |arg: Vec<u8>| synth::object(v, 1, &[(30, v.rect(10.0, 10.0, 0.0)), (100, arg)], None);
    // A matrix whose data offset lies past the argument, and an argument cut inside its header.
    let mut past = synth::transforms3(&[[1.0, 0.0, 0.0, 1.0, 5.0, 5.0]]);
    past[10..12].copy_from_slice(&60000u16.to_le_bytes());
    for arg in [past, vec![0, 0, 9]] {
        let d = read(&drawing3(vec![with(arg)]), Limits::default()).unwrap();
        let s = shapes(&d.pages[0].layers[0].objects);
        assert_eq!(s[0].matrix, Matrix::IDENTITY);
        assert_eq!(d.warnings, vec![Warning::Damaged]);
    }
    // A count of 65,535 over one entry reads that entry.
    let mut many = synth::transforms3(&[[1.0, 0.0, 0.0, 1.0, 72.0, 0.0]]);
    many[2..4].copy_from_slice(&u16::MAX.to_le_bytes());
    let d = read(&drawing3(vec![with(many)]), Limits::default()).unwrap();
    assert!((shapes(&d.pages[0].layers[0].objects)[0].matrix.0[4] - 72.0).abs() < 0.01);
    assert_eq!(d.warnings, vec![Warning::Damaged]);
}

#[test]
fn version_4_transforms_are_fixed_point_and_compose_in_file_order() {
    let v = V(400);
    let own = vec![
        chunk(b"loda", v.loda(1, &[(30, v.rect(10.0, 10.0, 0.0))])),
        list(b"trfl", vec![chunk(b"trfd", v.trfd(&[[1.0, 0.0, 0.0, 1.0, 72.0, 0.0], [0.0, 2.0, -2.0, 0.0, 0.0, 0.0]]))]),
    ];
    let object = list(b"obj ", vec![list(b"lgob", own)]);
    let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![object])]);
    let d = read(&synth::riff(400, &[chunk(b"vrsn", 400u16.to_le_bytes().to_vec()), page]), Limits::default()).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    // Moved 72 pt right, then turned a quarter and doubled: (0, 0) lands at (0, 144).
    let m = shapes(&d.pages[0].layers[0].objects)[0].matrix;
    assert!(m.0.iter().zip([0.0, 2.0, -2.0, 0.0, 0.0, 144.0]).all(|(a, b)| (a - b).abs() < 0.01), "{m:?}");
}

#[test]
fn version_4_colors_are_words() {
    let v = V(400);
    let file = |fill: [u8; 12]| {
        let doc = list(b"doc ", vec![list(b"filt", vec![chunk(b"fill", v.fill(1, &fill))])]);
        let rect = synth::object(v, 1, &[(20, 1u32.to_le_bytes().to_vec()), (30, v.rect(10.0, 10.0, 0.0))], None);
        let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![rect])]);
        synth::riff(400, &[chunk(b"vrsn", 400u16.to_le_bytes().to_vec()), doc, page])
    };
    let fill_of = |fill: [u8; 12]| {
        let d = read(&file(fill), Limits::default()).unwrap();
        (shapes(&d.pages[0].layers[0].objects)[0].fill.clone(), d.warnings)
    };
    assert_eq!(fill_of(synth::color4(1, [20, 40, 60, 80])), (Some(Color::Cmyk([0.2, 0.4, 0.6, 0.8])), vec![]));
    assert_eq!(fill_of(synth::color4(0, [0, 30, 0, 0])), (Some(Color::Gray(0.3)), vec![]));
    assert_eq!(fill_of(synth::color4(0, [1, 30, 0, 0])), (Some(Color::Gray(0.0)), vec![]));
    assert_eq!(
        fill_of(synth::color4(0, [513, 30, 0, 0])),
        (Some(Color::Spot { name: "Spot 0:513".into(), tint: 0.3, look: None }), vec![Warning::SpotLooks])
    );
    // The version 5 record read in the version 4 layout would take these values from other bytes.
    assert_ne!(fill_of(synth::color(2, [20, 40, 60, 80])).0, Some(Color::Cmyk([0.2, 0.4, 0.6, 0.8])));
}

#[test]
fn the_version_3_page_size_is_the_distance_between_its_corners() {
    let v = V(300);
    for corners in [[-306.0, 396.0, 306.0, -396.0], [306.0, -396.0, -306.0, 396.0]] {
        let mut b = vec![0xff; 2];
        b.extend(corners.iter().flat_map(|c| v.coord(*c)));
        let d = read(&synth::riff(300, &[list(b"doc ", vec![chunk(b"mcfg", b)])]), Limits::default()).unwrap();
        let (w, h) = d.page_size.unwrap();
        assert!((w - 612.0).abs() < 0.1 && (h - 792.0).abs() < 0.1, "{w} {h}");
    }
    // Corners that give no width, and a chunk cut inside them, give no page size.
    let mut flat = vec![0; 2];
    flat.extend([v.coord(10.0), v.coord(5.0), v.coord(10.0), v.coord(-5.0)].concat());
    for b in [flat.clone(), flat[..9].to_vec()] {
        assert_eq!(read(&synth::riff(300, &[list(b"doc ", vec![chunk(b"mcfg", b)])]), Limits::default()).unwrap().page_size, None);
    }
}

#[test]
fn a_point_list_before_version_6_counts_its_points_with_a_u16() {
    for version in [300, 400, 500] {
        let v = V(version);
        let mut points = v.points(&[(0.0, 0.0, 0x00), (10.0, 0.0, 0x40), (10.0, 10.0, 0x40)]);
        // The u16 after the count is 1 in some curves; as part of a u32 count it would run the
        // count past the points.
        points[2] = 1;
        let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 3, &[(30, points)], None)])]);
        let d = read(&synth::riff(version, &[chunk(b"vrsn", version.to_le_bytes().to_vec()), page]), Limits::default()).unwrap();
        assert!(d.warnings.is_empty(), "{version}: {:?}", d.warnings);
        let s = shapes(&d.pages[0].layers[0].objects);
        assert!(matches!(&s[0].geometry, Geometry::Path(p) if p.subpaths[0].segments.len() == 2), "{version}");
    }
}

/// The objects of the one layer of a version 1 or 2 drawing.
fn wl_objects(d: &craft_cdr::Drawing) -> &[Object] {
    assert_eq!((d.pages.len(), d.pages[0].layers.len(), d.master.len(), d.guides.len()), (1, 1, 0, 0));
    &d.pages[0].layers[0].objects
}

#[test]
fn versions_1_and_2_read_their_sample() {
    for (marker, version) in [(b'e', 100), (b'l', 200), (b'm', 200)] {
        let file = synth::wl_sample(marker);
        assert!(file.starts_with(b"WL") && craft_cdr::is_cdr(&file));
        let d = read(&file, Limits::default()).unwrap();
        assert_eq!(d.version, version);
        let (w, h) = d.page_size.unwrap();
        assert!((w - 612.0).abs() < 0.1 && (h - 792.0).abs() < 0.1, "{w} {h}");
        // The text is left out, and the group record holds nothing.
        assert_eq!(d.warnings, vec![Warning::UnsupportedObjects]);
        let s = shapes(wl_objects(&d));
        assert_eq!(s.len(), 3);
        // Front to back: the rounded rectangle, the ellipse, the curve.
        assert!(
            matches!(s[0].geometry, Geometry::Rect { w, h, radii } if (w - 100.0).abs() < 0.1 && (h - 50.0).abs() < 0.1 && radii.iter().all(|r| (r - 5.0).abs() < 0.1))
        );
        assert_eq!(s[0].fill, Some(Color::Cmyk([0.0, 1.0, 1.0, 0.0])));
        let o = s[0].outline.as_ref().unwrap();
        assert!((o.width - 2.0).abs() < 0.05, "{}", o.width);
        assert_eq!((&o.color, o.cap, o.join, o.dashes.len()), (&Color::Cmyk([0.0, 0.0, 0.0, 1.0]), Cap::Butt, Join::Miter, 0));
        assert!((s[0].matrix.0[4] + 100.0).abs() < 0.1 && (s[0].matrix.0[5] - 100.0).abs() < 0.1, "{:?}", s[0].matrix);
        assert!(matches!(s[1].geometry, Geometry::Ellipse { w, h } if (w - 40.0).abs() < 0.1 && (h + 40.0).abs() < 0.1));
        assert!(s[1].outline.is_none() && (s[1].matrix.0[4] - 50.0).abs() < 0.1 && (s[1].matrix.0[5] + 50.0).abs() < 0.1);
        assert!(matches!(&s[2].geometry, Geometry::Path(p) if p.subpaths[0].closed && matches!(p.subpaths[0].segments[1], Segment::Cubic(..))));
        assert!(s[2].fill.is_none() && s[2].outline.is_some());
        assert_eq!(&s[2].matrix.0[..4], &[2.0, 0.0, 0.0, 2.0]);
    }
}

#[test]
fn a_version_1_matrix_and_point_place_the_shape() {
    use synth::WlShape;
    // x' = 0 x - 1 y + 72 and y' = 1 x + 0 y + 36: a quarter turn, then a shift.
    let turned = synth::wl_object(WlShape::Rect { w: 10.0, h: 10.0, radius: 0.0 }, [0.0, 1.0, -1.0, 0.0, 72.0, 36.0], None, None);
    let d = read(&synth::wl_file(b'e', (612.0, 792.0), &[turned]), Limits::default()).unwrap();
    let m = shapes(wl_objects(&d))[0].matrix;
    assert!(m.0.iter().zip([0.0, 1.0, -1.0, 0.0, 72.0, 36.0]).all(|(a, b)| (a - b).abs() < 0.01), "{m:?}");
    let at = m.apply(craft_cdr::model::Pt { x: 10.0, y: 0.0 });
    assert!((at.x - 72.0).abs() < 0.01 && (at.y - 46.0).abs() < 0.01, "{at:?}");
}

#[test]
fn version_1_arcs_and_text_are_left_out_with_a_warning() {
    use synth::WlShape;
    let unit = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let arc = synth::wl_object(WlShape::Ellipse { w: 40.0, h: 40.0, angles: (0, 900) }, unit, None, None);
    let whole = synth::wl_object(WlShape::Ellipse { w: 40.0, h: 40.0, angles: (900, 900) }, unit, None, None);
    let mut other = synth::wl_object(WlShape::Text, unit, None, None);
    other[60] = 9;
    for (records, kept) in [(vec![arc], 0), (vec![whole], 1), (vec![synth::wl_object(WlShape::Text, unit, None, None)], 0), (vec![other], 0)] {
        let d = read(&synth::wl_file(b'e', (612.0, 792.0), &records), Limits::default()).unwrap();
        assert_eq!(wl_objects(&d).len(), kept);
        assert_eq!(d.warnings.contains(&Warning::UnsupportedObjects), kept == 0);
        assert!(!d.warnings.contains(&Warning::Damaged));
    }
}

#[test]
fn hostile_version_1_files_stop_within_their_bytes_and_limits() {
    use synth::WlShape;
    let unit = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let rect = || synth::wl_object(WlShape::Rect { w: 10.0, h: 10.0, radius: 0.0 }, unit, None, None);
    let file = synth::wl_sample(b'e');
    // Cut at every length: an error, or what the bytes still hold.
    for len in 0..file.len() {
        let _ = read(&file[..len], Limits::default());
    }
    // A body size that runs past the records' end.
    let mut long = rect();
    long[28..30].copy_from_slice(&u16::MAX.to_le_bytes());
    let d = read(&synth::wl_file(b'e', (612.0, 792.0), &[rect(), long]), Limits::default()).unwrap();
    assert_eq!((wl_objects(&d).len(), d.warnings.clone()), (1, vec![Warning::Damaged]));
    // A point count larger than the record holds reads the points that fit.
    let mut curve = synth::wl_object(WlShape::Curve(vec![(0.0, 0.0, 0x00), (10.0, 0.0, 0x40), (10.0, 10.0, 0x40)]), unit, None, None);
    curve[114..116].copy_from_slice(&u16::MAX.to_le_bytes());
    let d = read(&synth::wl_file(b'e', (612.0, 792.0), &[curve.clone()]), Limits::default()).unwrap();
    assert_eq!((wl_objects(&d).len(), d.warnings.clone()), (1, vec![Warning::Damaged]));
    // Points past the coordinate limit leave the curve out.
    curve[114..116].copy_from_slice(&3u16.to_le_bytes());
    let d = read(&synth::wl_file(b'e', (612.0, 792.0), &[curve]), Limits { coordinates: 2, ..Limits::default() }).unwrap();
    assert_eq!(wl_objects(&d).len(), 0);
    assert_eq!(d.warnings, vec![Warning::TooDeep]);
    // Objects past the object limit are left out, and records past the chunk limit stop the read.
    let many: Vec<Vec<u8>> = (0..10).map(|_| rect()).collect();
    let d = read(&synth::wl_file(b'e', (612.0, 792.0), &many), Limits { objects: 4, ..Limits::default() }).unwrap();
    assert_eq!((wl_objects(&d).len(), d.warnings.clone()), (4, vec![Warning::TooDeep]));
    let groups: Vec<Vec<u8>> = (0..10).map(|_| synth::wl_group()).collect();
    assert_eq!(read(&synth::wl_file(b'e', (612.0, 792.0), &groups), Limits { chunks: 4, ..Limits::default() }), Err(Error::Limit("chunk count")));
    assert_eq!(read(&file, Limits { input: file.len() - 1, ..Limits::default() }), Err(Error::Limit("input size")));
    // Bytes left after the last whole record are damage.
    let mut ragged = vec![rect()];
    ragged.push(vec![0; 5]);
    let d = read(&synth::wl_file(b'e', (612.0, 792.0), &ragged), Limits::default()).unwrap();
    assert_eq!((wl_objects(&d).len(), d.warnings.clone()), (1, vec![Warning::Damaged]));
}

#[test]
fn a_wl_prefix_alone_is_not_a_drawing() {
    for bytes in [
        &b"WL"[..],
        b"WLe\0",
        b"WLAN settings, exported as plain text for the help desk",
        // Records that start inside the header, and a marker and a fourth byte no source gives.
        b"WLe\0\x1c\0\0\0\x10\0\0\0\x1c\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        b"WLAN\x1c\0\0\0\x1c\0\0\0\x1c\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
    ] {
        assert!(!craft_cdr::is_cdr(bytes));
        assert_eq!(read(bytes, Limits::default()), Err(Error::NotCdr));
    }
}

#[test]
fn versions_outside_1_to_27_and_other_files_are_refused() {
    // No source shows a RIFF file of version 1 or 2: those versions are `WL` files.
    for version in [100, 200] {
        let refused = read(&synth::riff(version, &[]), Limits::default()).unwrap_err();
        assert_eq!(refused, Error::Unsupported("RIFF files of versions before 3"), "{version}");
        assert_eq!(refused.to_string(), "RIFF files of versions before 3 are not read");
    }
    assert_eq!(read(&synth::riff(2800, &[]), Limits::default()), Err(Error::Version(2800)));
    assert_eq!(read(b"RIFF\x04\0\0\0WAVE", Limits::default()), Err(Error::NotCdr));
    assert_eq!(read(b"RIFF\x04\0\0\0CDRX", Limits::default()), Err(Error::NotCdr));
    assert!(!craft_cdr::is_cdr(b"PK\x03\x04"));
    assert!(craft_cdr::is_cdr(&synth::sample(1100, true)));
}

#[test]
fn letters_that_count_differently_take_the_vrsn_version() {
    // Version 27 saves the letter `U`, which counts as 29 the older way; the next letter with a
    // `vrsn` of 2800 is refused as version 28.
    let mut file = synth::sample(2700, false);
    file[11] = b'U';
    let d = read(&file, Limits::default()).unwrap();
    check_pages(&d, 2700);
    check_shapes(&d, 2700);
    let mut file = synth::riff(2800, &[chunk(b"vrsn", 2800u16.to_le_bytes().to_vec())]);
    file[11] = b'V';
    assert_eq!(read(&file, Limits::default()), Err(Error::Version(2800)));
}

#[test]
fn every_version_from_14_to_27_reads_its_package() {
    for version in [1400, 1500, 1600, 1700, 1800, 2000, 2200, 2400, 2430, 2600, 2700] {
        for compress in [false, true] {
            let file = synth::sample_file(version, compress);
            assert!(file.starts_with(b"PK\x03\x04") && craft_cdr::is_cdr(&file), "{version}");
            let d = read(&file, Limits::default()).unwrap_or_else(|e| panic!("{version} {compress}: {e}"));
            assert!(!d.warnings.contains(&Warning::Damaged), "{version} {compress}: {:?}", d.warnings);
            check_pages(&d, version);
            check_shapes(&d, version);
        }
    }
}

/// The version 17 sample's content with `extra` added after it, as a package.
fn package17(extra: Vec<synth::Node>, members: &[synth::Member]) -> Vec<u8> {
    let mut body = vec![chunk(b"vrsn", 1700u16.to_le_bytes().to_vec())];
    body.extend(synth::sample_content(1700));
    body.extend(extra);
    synth::package16(1700, &body, true, members)
}

#[test]
fn lists_whose_bodies_move_to_a_data_stream_read_the_same() {
    let mut body = vec![chunk(b"vrsn", 1700u16.to_le_bytes().to_vec())];
    body.extend(synth::sample_content(1700));
    for moved in [*b"layr", *b"obj ", *b"page"] {
        let file = synth::package16_with(1700, &body, true, &[], &[moved]);
        let d = read(&file, Limits::default()).unwrap();
        assert!(!d.warnings.contains(&Warning::Damaged), "{moved:?}: {:?}", d.warnings);
        check_pages(&d, 1700);
        check_shapes(&d, 1700);
    }
}

#[test]
fn a_data_stream_no_chunk_reads_is_not_inflated() {
    // 32 MiB of picture data that the reader doesn't use, in a stream of its own.
    let bitmaps = list(b"bmpt", vec![chunk(b"bmp ", vec![0; 32 << 20])]);
    let file = package17(vec![bitmaps], &[]);
    assert!(file.len() < 1 << 20);
    let d = read(&file, Limits { inflated: 4 << 20, ..Limits::default() }).unwrap();
    assert_eq!(d.warnings, vec![Warning::UnsupportedObjects]);
    check_shapes(&d, 1700);
}

#[test]
fn a_redirected_list_is_inflated_and_counts_toward_the_limit() {
    // A list whose body is in a stream must be inflated to be split; past the limit it is left
    // out with a warning, and the rest of the drawing reads.
    let mut body = vec![chunk(b"vrsn", 1700u16.to_le_bytes().to_vec())];
    body.extend(synth::sample_content(1700));
    body.push(list(b"bmpt", vec![chunk(b"bmp ", vec![0; 32 << 20])]));
    let file = synth::package16_with(1700, &body, true, &[], &[*b"bmpt"]);
    let d = read(&file, Limits { inflated: 4 << 20, ..Limits::default() }).unwrap();
    assert!(d.warnings.contains(&Warning::TooDeep) && !d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
    check_shapes(&d, 1700);
    let d = read(&file, Limits::default()).unwrap();
    assert!(!d.warnings.contains(&Warning::TooDeep));
}

#[test]
fn chunks_in_a_data_stream_past_a_limit_are_not_damage() {
    // A record and a list in a 2 MiB stream, read with a 1 MiB member limit, stored and deflated
    // with a 1 MiB inflate limit: only the limit warning.
    let mcfg = synth::mcfg(V(1700), 300.0, 200.0);
    let synth::Node::Chunk(_, mcfg) = mcfg else { panic!() };
    let n = mcfg.len() as u32;
    let mut stream = mcfg.clone();
    stream.resize(2 << 20, 0);
    stream[(1 << 20)..(1 << 20) + 4].copy_from_slice(b"fntt");
    let mut root = b"RIFF".to_vec();
    let body = doc_of(&[redirect(b"mcfg", 0, n, 0), redirect(b"LIST", 0, 4, 1 << 20)]);
    root.extend(((body.len() + 4) as u32).to_le_bytes());
    root.extend(b"CDRH");
    root.extend(body);
    let m = |name: &str, data: Vec<u8>, deflate: bool| synth::Member { name: name.into(), data, deflate };
    for (deflate, limits) in [(false, Limits { member: 1 << 20, ..Limits::default() }), (true, Limits { inflated: 1 << 20, ..Limits::default() })] {
        let file = synth::zip(
            &[
                m("content/root.dat", root.clone(), false),
                m("content/dataFileList.dat", b"data1.dat".to_vec(), false),
                m("content/data/data1.dat", stream.clone(), deflate),
            ],
            b"",
        );
        let d = read(&file, limits).unwrap();
        assert_eq!(d.warnings, vec![Warning::TooDeep], "{deflate}");
        assert!(d.page_size.is_none());
        let d = read(&file, Limits::default()).unwrap();
        assert!(d.page_size.is_some() && d.warnings.is_empty(), "{deflate}: {:?}", d.warnings);
    }
}

#[test]
fn encrypted_data_streams_and_other_methods_are_refused() {
    let good = synth::sample_file(1700, false);
    assert!(read(&good, Limits::default()).is_ok());
    // The directory entry of a data stream: its name's last place, 46 bytes after the entry start.
    let name = b"content/data/data1.dat";
    let entry = good.windows(name.len()).rposition(|w| w == name).unwrap() - 46;
    assert_eq!(&good[entry..entry + 4], b"PK\x01\x02");
    let mut encrypted = good.clone();
    encrypted[entry + 8] |= 1;
    assert_eq!(read(&encrypted, Limits::default()), Err(Error::Unsupported("encrypted packages")));
    let mut other = good.clone();
    other[entry + 10] = 12;
    assert_eq!(read(&other, Limits::default()), Err(Error::Unsupported("package compression methods other than storing and deflate")));
}

/// A version 17 package whose `root.dat` is `root` (the RIFF body after the form type) and whose
/// one data stream is `stream`.
fn raw_package(root: &[u8], stream: &[u8], list: &[u8]) -> Vec<u8> {
    let mut riff = b"RIFF".to_vec();
    riff.extend(((root.len() + 4) as u32).to_le_bytes());
    riff.extend(b"CDRH");
    riff.extend(root);
    let m = |name: &str, data: &[u8]| synth::Member { name: name.into(), data: data.to_vec(), deflate: false };
    synth::zip(&[m("content/root.dat", &riff), m("content/dataFileList.dat", list), m("content/data/data1.dat", stream)], b"")
}

/// A chunk header for a redirect record of `stream`, `len` and `offset`.
fn redirect(id: &[u8; 4], stream: u32, len: u32, offset: u32) -> Vec<u8> {
    let mut out = id.to_vec();
    out.extend(16u32.to_le_bytes());
    for v in [stream, len, offset, 0] {
        out.extend(v.to_le_bytes());
    }
    out
}

#[test]
fn a_list_redirected_into_its_own_bytes_is_read_once() {
    // The stream holds a `grp ` body whose children are lists redirected to that body again:
    // the bytes are taken, so each child is damage and the list is read once.
    for children in [1, 2] {
        let len = 4 + 24 * children;
        let mut stream = b"grp ".to_vec();
        for _ in 0..children {
            stream.extend(redirect(b"LIST", 0, len, 0));
        }
        let file = raw_package(&redirect(b"LIST", 0, len, 0), &stream, b"data1.dat");
        let d = read(&file, Limits { chunks: 100, ..Limits::default() }).unwrap();
        assert_eq!(d.warnings, vec![Warning::Damaged], "{children}");
    }
}

/// A `doc ` list holding `records`, as `root.dat`'s body.
fn doc_of(records: &[Vec<u8>]) -> Vec<u8> {
    let records = records.concat();
    let mut body = b"LIST".to_vec();
    body.extend(((records.len() + 4) as u32).to_le_bytes());
    body.extend(b"doc ");
    body.extend(records);
    body
}

#[test]
fn records_share_bytes_within_the_member_limit() {
    let mcfg = synth::mcfg(V(1700), 300.0, 200.0);
    let synth::Node::Chunk(_, mcfg) = mcfg else { panic!() };
    let n = mcfg.len() as u32;
    let mut stream = mcfg.clone();
    stream.extend(&mcfg);
    // The same bytes, bytes that overlap them, bytes that only touch them, and the same bytes
    // through a second line naming the same member: both redirects read.
    for (second, list) in [
        (redirect(b"mcfg", 0, n, 0), &b"data1.dat"[..]),
        (redirect(b"mcfg", 0, n, n - 1), b"data1.dat"),
        (redirect(b"mcfg", 0, n, n), b"data1.dat"),
        (redirect(b"mcfg", 1, n, 0), b"data1.dat\ndata1.dat"),
    ] {
        let file = raw_package(&doc_of(&[redirect(b"mcfg", 0, n, 0), second.clone()]), &stream, list);
        let d = read(&file, Limits::default()).unwrap();
        assert!(d.page_size.is_some() && d.warnings.is_empty(), "{:?}: {:?}", &second[8..], d.warnings);
    }
    // Bytes read again take from the member limit: with a limit of the stream's 1,024 bytes, a
    // record of all of them reads twice, and a third read is left out with the limit warning.
    let mut stream = mcfg.clone();
    stream.resize(1024, 0);
    let limits = Limits { member: 1024, ..Limits::default() };
    for (reads, warnings) in [(2, vec![]), (3, vec![Warning::TooDeep])] {
        let records = vec![redirect(b"mcfg", 0, 1024, 0); reads];
        let d = read(&raw_package(&doc_of(&records), &stream, b"data1.dat"), limits).unwrap();
        assert!(d.page_size.is_some(), "{reads}");
        assert_eq!(d.warnings, warnings, "{reads}");
    }
}

#[test]
fn a_member_two_streams_name_is_inflated_once() {
    // One deflated member of 2 MiB named on two lines, a list read from one and the page size
    // from the other: within a 3 MiB inflate limit it is inflated once.
    let mcfg = synth::mcfg(V(1700), 300.0, 200.0);
    let synth::Node::Chunk(_, mcfg) = mcfg else { panic!() };
    let n = mcfg.len() as u32;
    let mut stream = mcfg.clone();
    stream.resize(2 << 20, 0);
    stream[(1 << 20)..(1 << 20) + 4].copy_from_slice(b"fntt");
    let mut root = b"RIFF".to_vec();
    let body = doc_of(&[redirect(b"LIST", 1, 4, 1 << 20), redirect(b"mcfg", 0, n, 0)]);
    root.extend(((body.len() + 4) as u32).to_le_bytes());
    root.extend(b"CDRH");
    root.extend(body);
    let m = |name: &str, data: Vec<u8>, deflate: bool| synth::Member { name: name.into(), data, deflate };
    let file = synth::zip(
        &[
            m("content/root.dat", root, false),
            m("content/dataFileList.dat", b"data1.dat\ndata1.dat".to_vec(), false),
            m("content/data/data1.dat", stream, true),
        ],
        b"",
    );
    let d = read(&file, Limits { inflated: 3 << 20, ..Limits::default() }).unwrap();
    assert!(d.page_size.is_some() && d.warnings.is_empty(), "{:?}", d.warnings);
}

#[test]
fn redirects_past_their_stream_or_to_a_missing_one_are_damage() {
    let mcfg = synth::mcfg(V(1700), 300.0, 200.0);
    let synth::Node::Chunk(_, mcfg) = mcfg else { panic!() };
    let n = mcfg.len() as u32;
    for (root, list, damaged) in [
        (redirect(b"mcfg", 0, n, 0), &b"data1.dat"[..], false),
        (redirect(b"mcfg", 0, n, 0), b"data1.dat\r\n", false),
        (redirect(b"mcfg", 0, n + 1, 0), b"data1.dat", true),
        (redirect(b"mcfg", 0, n, u32::MAX), b"data1.dat", true),
        (redirect(b"mcfg", 1, n, 0), b"data1.dat", true),
        (redirect(b"mcfg", 0, n, 0), b"gone.dat", true),
        (redirect(b"mcfg", 0, n, 0), b"", true),
    ] {
        let mut body = b"LIST".to_vec();
        body.extend(((root.len() + 4) as u32).to_le_bytes());
        body.extend(b"doc ");
        body.extend(&root);
        let d = read(&raw_package(&body, &mcfg, list), Limits::default()).unwrap();
        assert_eq!(d.warnings.contains(&Warning::Damaged), damaged, "{list:?} {:?}", &root[8..]);
        assert_eq!(d.page_size.is_some(), !damaged);
    }
}

#[test]
fn a_redirect_record_holds_up_to_8_bytes_itself() {
    let mut inline = b"vrsn".to_vec();
    inline.extend(16u32.to_le_bytes());
    inline.extend(u32::MAX.to_le_bytes());
    inline.extend(2u32.to_le_bytes());
    inline.extend(1800u16.to_le_bytes());
    inline.extend([0; 6]);
    let d = read(&raw_package(&inline, b"", b""), Limits::default()).unwrap();
    assert_eq!(d.version, 1800);
    // A length past the record's 8 bytes is damage, and the form type's version stands.
    inline[12] = 9;
    let d = read(&raw_package(&inline, b"", b""), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged));
    assert_eq!(d.version, 1700);
}

fn zip_of(members: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let m: Vec<_> = members.iter().map(|(n, d)| synth::Member { name: (*n).into(), data: d.clone(), deflate: false }).collect();
    synth::zip(&m, b"")
}

/// Where the first central directory entry starts.
fn directory(bytes: &[u8]) -> usize {
    bytes.windows(4).position(|w| w == b"PK\x01\x02").unwrap()
}

#[test]
fn hostile_packages_are_refused() {
    let riff = synth::sample(1400, false);
    let good = zip_of(&[("content/riffData.cdr", riff.clone())]);
    assert!(read(&good, Limits::default()).is_ok());
    // More members than a read takes.
    let many: Vec<(String, Vec<u8>)> = (0..4097).map(|i| (format!("m{i}"), vec![])).collect();
    let many: Vec<(&str, Vec<u8>)> = many.iter().map(|(n, d)| (n.as_str(), d.clone())).collect();
    assert_eq!(read(&zip_of(&many), Limits::default()), Err(Error::Limit("package member count")));
    assert_eq!(read(&zip_of(&many[..4096]), Limits::default()), Err(Error::NotCdr));
    // A member named twice.
    let twice = zip_of(&[("content/riffData.cdr", riff.clone()), ("content/riffData.cdr", riff.clone())]);
    assert_eq!(read(&twice, Limits::default()), Err(Error::Damaged("package member named twice")));
    // A member whose bytes run into the next member's header.
    let two = zip_of(&[("a", vec![1; 10]), ("content/riffData.cdr", riff.clone())]);
    let mut overlap = two.clone();
    let at = directory(&two) + 20;
    overlap[at..at + 4].copy_from_slice(&20u32.to_le_bytes());
    assert_eq!(read(&overlap, Limits::default()), Err(Error::Damaged("package members overlap")));
    // A central directory cut short, and an archive without its end record.
    let mut short = good.clone();
    let n = short.len();
    let len_at = n - 22 + 12;
    let len = u32::from_le_bytes(short[len_at..len_at + 4].try_into().unwrap());
    short[len_at..len_at + 4].copy_from_slice(&(len - 10).to_le_bytes());
    assert_eq!(read(&short, Limits::default()), Err(Error::Damaged("package directory")));
    assert_eq!(read(&good[..directory(&good) + 30], Limits::default()), Err(Error::Damaged("package directory")));
    // Encrypted members, ZIP64 fields and archives split across files.
    let mut encrypted = good.clone();
    encrypted[directory(&good) + 8] |= 1;
    assert_eq!(read(&encrypted, Limits::default()), Err(Error::Unsupported("encrypted packages")));
    let mut zip64 = good.clone();
    zip64[n - 22 + 10..n - 22 + 12].copy_from_slice(&[0xff, 0xff]);
    assert_eq!(read(&zip64, Limits::default()), Err(Error::Unsupported("ZIP64 packages")));
    let mut split = good.clone();
    split[n - 22 + 4] = 1;
    assert_eq!(read(&split, Limits::default()), Err(Error::Unsupported("packages split across files")));
}

#[test]
fn a_package_member_bomb_stops_at_its_stated_size_or_the_limit() {
    let riff = synth::sample(1400, false);
    let m = synth::Member { name: "content/riffData.cdr".into(), data: riff.clone(), deflate: true };
    let good = synth::zip(std::slice::from_ref(&m), b"");
    assert!(read(&good, Limits::default()).is_ok());
    // A member that inflates past the size its entry states stops one byte past it.
    let bomb = synth::zip(&[synth::Member { data: [riff.clone(), vec![0; 64 << 20]].concat(), ..m.clone() }], b"");
    assert!(bomb.len() < 1 << 20);
    let mut lying = bomb.clone();
    let at = directory(&bomb) + 24;
    lying[at..at + 4].copy_from_slice(&(riff.len() as u32).to_le_bytes());
    assert_eq!(read(&lying, Limits::default()), Err(Error::Damaged("package member")));
    // One that states more than a member may hold, or than is left to inflate, is not inflated.
    assert_eq!(read(&bomb, Limits { member: 1 << 20, ..Limits::default() }), Err(Error::Limit("package member size")));
    assert_eq!(read(&bomb, Limits { inflated: 1 << 20, ..Limits::default() }), Err(Error::Limit("inflated size")));
}

#[test]
fn stored_package_members_take_nothing_from_the_inflate_limit() {
    // Nothing may be inflated, and the stored members of either package layout still read, as
    // the same drawing does as a plain RIFF file.
    let none = Limits { inflated: 0, ..Limits::default() };
    assert!(read(&synth::sample(1400, false), none).is_ok());
    for version in [1400, 1700] {
        let d = read(&synth::sample_file(version, false), none).unwrap_or_else(|e| panic!("{version}: {e}"));
        assert!(d.warnings.iter().all(|w| *w == Warning::UnsupportedObjects), "{version}: {:?}", d.warnings);
        check_shapes(&d, version);
    }
    // Deflated members do take from it.
    assert_eq!(read(&synth::sample_file(1400, true), none), Err(Error::Limit("inflated size")));
}

#[test]
fn redirect_records_that_lead_nowhere_count_toward_the_chunk_limit() {
    // 3,000 redirects to a stream the package lacks: each lost record is counted.
    let root: Vec<u8> = (0..3000).flat_map(|_| redirect(b"mcfg", 7, 4, 0)).collect();
    let file = raw_package(&root, b"", b"data1.dat");
    assert_eq!(read(&file, Limits { chunks: 2_000, ..Limits::default() }), Err(Error::Limit("chunk count")));
    let d = read(&file, Limits { chunks: 4_000, ..Limits::default() }).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
}

#[test]
fn rectangles_from_version_15_give_radii_as_shares_of_half_the_shorter_side() {
    let v = V(1500);
    let mut c = synth::V(1500).rect(100.0, 50.0, 0.0);
    // The four shares: 0.2, 0.4, 0, and 3, which half the shorter side holds to 25 pt.
    for (k, share) in [0.2f64, 0.4, 0.0, 3.0].iter().enumerate() {
        let at = 40 + 24 * k;
        c[at..at + 8].copy_from_slice(&share.to_le_bytes());
    }
    let file =
        synth::riff(1500, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &[(30, c.clone())], None)])])]);
    let d = read(&file, Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    let Geometry::Rect { w, h, radii } = s[0].geometry else { panic!("{:?}", s[0].geometry) };
    assert!((w - 100.0).abs() < 1e-6 && (h - 50.0).abs() < 1e-6);
    for (got, want) in radii.iter().zip([5.0, 10.0, 0.0, 25.0]) {
        assert!((got - want).abs() < 1e-6, "{radii:?}");
    }
    // With the scale-with flag set the radii are inches, held to half the shorter side as well.
    c[32] = 1;
    let file = synth::riff(1500, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &[(30, c)], None)])])]);
    let d = read(&file, Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    let Geometry::Rect { radii, .. } = s[0].geometry else { panic!("{:?}", s[0].geometry) };
    for (got, want) in radii.iter().zip([14.4, 25.0, 0.0, 25.0]) {
        assert!((got - want).abs() < 1e-6, "{radii:?}");
    }
}

#[test]
fn paths_from_version_10_read_their_points() {
    let v = V(1700);
    let mut c = vec![0; 4];
    c.extend(2u16.to_le_bytes());
    c.extend(1u16.to_le_bytes());
    c.extend([0; 16]);
    c.extend(&v.points(&[(0.0, 0.0, 0x08), (50.0, 0.0, 0x40), (50.0, 20.0, 0x48)])[4..]);
    let objects = |version: u16| {
        let v = V(version);
        synth::riff(
            version,
            &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 0x25, &[(30, c.clone())], None)])])],
        )
    };
    for version in [1000, 1300, 1500, 1700] {
        let d = read(&objects(version), Limits::default()).unwrap();
        let s = shapes(&d.pages[0].layers[0].objects);
        let Geometry::Path(p) = &s[0].geometry else { panic!("{version}: {:?}", s[0].geometry) };
        assert_eq!(p.subpaths[0].segments.len(), 2);
        assert!(d.warnings.is_empty(), "{version}: {:?}", d.warnings);
    }
    // No sample before version 10 holds the type, so it is not read as a path there.
    let d = read(&objects(900), Limits::default()).unwrap();
    assert!(d.pages[0].layers[0].objects.is_empty() && d.warnings.contains(&Warning::UnsupportedObjects));
}

#[test]
fn a_template_takes_its_version_from_the_vrsn_chunk() {
    let mut file = synth::sample(2100, false);
    file[8..12].copy_from_slice(b"CDTQ");
    assert!(craft_cdr::is_cdr(&file));
    let d = read(&file, Limits::default()).unwrap();
    assert_eq!(d.version, 2100);
    check_shapes(&d, 2100);
    let mut bare = synth::riff(2100, &[]);
    bare[8..12].copy_from_slice(b"CDTQ");
    assert_eq!(read(&bare, Limits::default()), Err(Error::NotCdr));
    // A template of a version after 27 is refused by its version, and one before 3 as a RIFF
    // file of those versions.
    for (version, refused) in [(2800u16, Error::Version(2800)), (200, Error::Unsupported("RIFF files of versions before 3"))] {
        let mut t = synth::riff(2100, &[chunk(b"vrsn", version.to_le_bytes().to_vec())]);
        t[8..12].copy_from_slice(b"CDTQ");
        assert_eq!(read(&t, Limits::default()), Err(refused));
    }
}

#[test]
fn layers_take_their_kind_from_their_object_type() {
    for version in [300, 800, 1300, 1700] {
        let v = V(version);
        let rect = || synth::object(v, 1, &[(30, v.rect(10.0, 10.0, 0.0))], None);
        // Version 3 counts its types from 0 (R32).
        let ty = |t: u32| if version < 400 { t - 1 } else { t };
        // Master page layers flagged in the older way, with the types of guides and desktop layers,
        // and a drawing layer whose flags would make it a desktop layer.
        let master = synth::page(
            true,
            vec![
                synth::layer_of_type(v, [2, 1, 1, 0x98], ty(12), "G", vec![rect()]),
                synth::layer_of_type(v, [0, 1, 1, 0x98], ty(17), "D", vec![rect()]),
                synth::layer_of_type(v, [8, 1, 0, 0x98], 0, "M", vec![rect()]),
            ],
        );
        // A drawing layer with flag bit 0x10 set, and a grid layer.
        let own = synth::page(
            false,
            vec![
                synth::layer_of_type(v, [0x10, 0, 0, 0x98], 0, "L", vec![rect()]),
                synth::layer_of_type(v, [0x1a, 0, 1, 0x98], ty(11), "Grid", vec![rect()]),
            ],
        );
        let body = [chunk(b"vrsn", version.to_le_bytes().to_vec()), master, own];
        let file = if version < 1400 { synth::riff(version, &body) } else { synth::package16(version, &body, true, &[]) };
        let d = read(&file, Limits::default()).unwrap();
        let kinds: Vec<_> = d.master.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, [LayerKind::Guides, LayerKind::Desktop, LayerKind::Drawing], "{version}");
        let kinds: Vec<_> = d.pages[0].layers.iter().map(|l| (l.kind, l.objects.len())).collect();
        assert_eq!(kinds, [(LayerKind::Drawing, 1), (LayerKind::Grid, 1)], "{version}");
    }
}

#[test]
fn a_lower_case_form_type_names_the_same_versions() {
    let mut file = synth::sample(600, false);
    file[8..12].copy_from_slice(b"cdr6");
    // The `vrsn` chunk agrees; without it the form type alone decides.
    assert_eq!(read(&file, Limits::default()).unwrap().version, 600);
    assert!(craft_cdr::is_cdr(&file));
    file[8..12].copy_from_slice(b"cdrX");
    assert_eq!(read(&file, Limits::default()), Err(Error::NotCdr));
}

#[test]
fn rectangles_keep_four_corner_radii_from_version_9() {
    let v = V(900);
    let c = [v.coord(100.0), v.coord(50.0), v.coord(1.0), v.coord(2.0), v.coord(3.0), v.coord(4.0)].concat();
    let file = synth::riff(900, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &[(30, c)], None)])])]);
    let d = read(&file, Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    let Geometry::Rect { radii, .. } = s[0].geometry else { panic!("{:?}", s[0].geometry) };
    for (got, want) in radii.iter().zip([1.0, 2.0, 3.0, 4.0]) {
        assert!((got - want).abs() < 0.01, "{radii:?}");
    }
    // Before version 9 one radius rounds every corner.
    let v = V(800);
    let c = [v.coord(100.0), v.coord(50.0), v.coord(6.0)].concat();
    let file = synth::riff(800, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &[(30, c)], None)])])]);
    let s = read(&file, Limits::default()).unwrap();
    let s = shapes(&s.pages[0].layers[0].objects);
    assert!(matches!(s[0].geometry, Geometry::Rect { radii, .. } if radii.iter().all(|r| (r - 6.0).abs() < 0.01)));
}

#[test]
fn version_7_guidelines_count_toward_the_object_limit() {
    let v = V(700);
    let mut guid = 1000u32.to_le_bytes().to_vec();
    guid.extend(1000u32.to_le_bytes());
    for i in 0..1000 {
        guid.extend([0; 12]);
        guid.extend(90_000_000i32.to_le_bytes());
        guid.extend([v.coord(f64::from(i)), v.coord(0.0), v.coord(f64::from(i)), v.coord(1.0), v.coord(f64::from(i)), v.coord(0.0)].concat());
    }
    let file = synth::riff(700, &[chunk(b"vrsn", 700u16.to_le_bytes().to_vec()), list(b"doc ", vec![chunk(b"guid", guid)])]);
    let d = read(&file, Limits::default()).unwrap();
    assert_eq!(d.guides.len(), 1000);
    let d = read(&file, Limits { objects: 10, ..Limits::default() }).unwrap();
    assert_eq!(d.guides.len(), 10);
    assert!(d.warnings.contains(&Warning::TooDeep));
}

/// Guidelines on any layer of the master page show on every page, not only those of its guides
/// layer: one on a drawing layer that also holds art, and one alone on the desktop layer.
#[test]
fn master_guidelines_on_any_layer_show_on_every_page() {
    let v = V(1100);
    let guide = |x: f64| {
        let mut g = vec![0; 12];
        g.extend(90_000_000i32.to_le_bytes());
        g.extend([v.coord(x), v.coord(0.0)].concat());
        synth::object(v, 0x16, &[(16000, g)], None)
    };
    let rect = synth::object(v, 1, &[(30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    let master = synth::page(
        true,
        vec![synth::layer(v, [0, 0, 1, 0x98], "Art", vec![guide(10.0), rect]), synth::layer(v, [0x08, 0, 1, 0x98], "Desktop", vec![guide(20.0)])],
    );
    let file = synth::riff(1100, &[master, synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "Art", vec![])])]);
    let d = read(&file, Limits::default()).unwrap();
    let mut xs: Vec<f64> = d.guides.iter().map(|g| g.at.x).collect();
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs.len(), 2, "{:?}", d.guides);
    assert!((xs[0] - 10.0).abs() < 0.01 && (xs[1] - 20.0).abs() < 0.01, "{xs:?}");
    assert_eq!(d.master.len(), 1);
    assert_eq!(d.master[0].objects.len(), 1);
}

/// An object with more arguments than the limit keeps the ones read paired with their own types:
/// the types table runs from the stored count, not from the limit.
#[test]
fn arguments_past_the_limit_leave_the_first_ones_paired_with_their_types() {
    let v = V(1100);
    let mut args = vec![(30, [v.coord(40.0), v.coord(20.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())];
    args.extend((0..69).map(|i| (500 + i, vec![0; 4])));
    let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &args, None)])])]);
    let d = read(&file, Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert!(
        matches!(s.first().map(|s| &s.geometry), Some(Geometry::Rect { w, h, .. }) if (w - 40.0).abs() < 0.01 && (h - 20.0).abs() < 0.01),
        "{s:?}"
    );
    assert!(d.warnings.contains(&Warning::Damaged));
}

/// Groups count toward the object limit; a group past it leaves its objects ungrouped.
#[test]
fn groups_count_toward_the_object_limit() {
    fn count(objects: &[Object]) -> usize {
        objects.iter().map(|o| 1 + if let Object::Group { objects: kids, .. } = o { count(kids) } else { 0 }).sum()
    }
    let v = V(1100);
    let rect = synth::object(v, 1, &[(30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    let nested = list(b"grp ", vec![list(b"grp ", vec![rect])]);
    let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![nested])])]);
    let d = read(&file, Limits { objects: 2, ..Limits::default() }).unwrap();
    let objects = &d.pages[0].layers[0].objects;
    assert_eq!(count(objects), 2, "{objects:?}");
    assert!(matches!(objects.as_slice(), [Object::Group { objects: kids, .. }] if matches!(kids.as_slice(), [Object::Shape(_)])), "{objects:?}");
    assert!(d.warnings.contains(&Warning::TooDeep));
}

/// A rectangle, ellipse, curve or polygon stored without its coordinates is left out as damage.
#[test]
fn shapes_without_their_coordinates_are_damage() {
    let v = V(1100);
    for kind in [1, 2, 3, 0x14] {
        let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, kind, &[], None)])])]);
        let d = read(&file, Limits::default()).unwrap();
        assert!(d.pages[0].layers[0].objects.is_empty(), "{kind}");
        assert!(d.warnings.contains(&Warning::Damaged), "{kind}: {:?}", d.warnings);
    }
}

/// An object list without its `loda`, a fill or outline record that can't be read, and a `guid`
/// chunk that counts more guidelines than it holds are each damage.
#[test]
fn unreadable_records_and_lists_are_damage() {
    let v = V(1300);
    let rect = |args: Vec<(u32, Vec<u8>)>| {
        let mut args = args;
        args.push((30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat()));
        synth::object(v, 1, &args, None)
    };
    let on_page = |objects: Vec<synth::Node>, styles: Vec<synth::Node>| {
        let mut kids = vec![chunk(b"vrsn", 1300u16.to_le_bytes().to_vec())];
        kids.extend(styles);
        kids.push(synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", objects)]));
        read(&synth::riff(1300, &kids), Limits::default()).unwrap()
    };
    let d = on_page(vec![list(b"obj ", vec![chunk(b"flgs", vec![0, 0, 0, 0x08])])], vec![]);
    assert!(d.warnings.contains(&Warning::Damaged), "no loda: {:?}", d.warnings);
    // A fill record of only its id.
    let d = on_page(vec![rect(vec![(20, 4u32.to_le_bytes().to_vec())])], vec![list(b"filt", vec![chunk(b"fild", 4u32.to_le_bytes().to_vec())])]);
    assert_eq!(shapes(&d.pages[0].layers[0].objects)[0].fill, None);
    assert!(d.warnings.contains(&Warning::Damaged), "fill: {:?}", d.warnings);
    // A tagged outline whose first records are none of record 1.
    let mut outl = 9u32.to_le_bytes().to_vec();
    for _ in 0..16 {
        outl.extend(5u32.to_le_bytes());
        outl.extend(0u32.to_le_bytes());
    }
    let d = on_page(vec![rect(vec![(10, 9u32.to_le_bytes().to_vec())])], vec![list(b"otlt", vec![chunk(b"outl", outl)])]);
    assert!(shapes(&d.pages[0].layers[0].objects)[0].outline.is_none());
    assert!(d.warnings.contains(&Warning::Damaged), "outline: {:?}", d.warnings);
    // Version 7: a `guid` chunk counting three guidelines and holding one.
    let mut guid = 3u32.to_le_bytes().to_vec();
    guid.extend(3u32.to_le_bytes());
    guid.extend([0; 40]);
    let d = read(&synth::riff(700, &[chunk(b"vrsn", 700u16.to_le_bytes().to_vec()), list(b"doc ", vec![chunk(b"guid", guid)])]), Limits::default())
        .unwrap();
    assert_eq!(d.guides.len(), 1);
    assert!(d.warnings.contains(&Warning::Damaged), "guid: {:?}", d.warnings);
}

/// Pages keep their layers in a `gobj` list, and layers and objects their `loda` (and objects
/// their transform) in a `lgob` list. The layout with those chunks directly in the page, layer and
/// object lists reads the same.
#[test]
fn layers_and_objects_read_from_their_gobj_and_lgob_lists() {
    let v = V(1000);
    let c = [v.coord(10.0), v.coord(20.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat();
    let m = [1.0, 0.0, 0.0, 1.0, 5.0, 6.0];
    let nested = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &[(30, c.clone())], Some(m))])]);
    let plain_object = list(b"obj ", vec![chunk(b"loda", v.loda(1, &[(30, c)])), list(b"trfl", vec![chunk(b"trfd", v.trfd(&[m]))])]);
    let plain_layer = list(b"layr", vec![chunk(b"flgs", vec![0, 0, 0, 0x98]), chunk(b"loda", v.loda(0, &[(1000, b"L\0".to_vec())])), plain_object]);
    let plain = list(b"page", vec![chunk(b"flgs", vec![0, 0, 0, 0x90]), plain_layer]);
    for page in [nested, plain] {
        let d = read(&synth::riff(1000, &[page]), Limits::default()).unwrap();
        let layer = &d.pages[0].layers[0];
        assert_eq!(layer.name, "L");
        let s = shapes(&layer.objects);
        assert_eq!(s.len(), 1, "{:?}", layer.objects);
        assert!(matches!(s[0].geometry, Geometry::Rect { w, .. } if (w - 10.0).abs() < 0.01));
        assert!((s[0].matrix.0[4] - 5.0).abs() < 0.01 && (s[0].matrix.0[5] - 6.0).abs() < 0.01, "{:?}", s[0].matrix);
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    }
}

/// From version 7 a `LIST stlt` holds the style tables, which don't parse as chunks: it is not
/// damage.
#[test]
fn a_style_table_list_is_not_read_as_chunks() {
    let v = V(1000);
    let stlt = chunk(b"LIST", [b"stlt".to_vec(), 3u32.to_le_bytes().to_vec(), vec![0xff; 30]].concat());
    let d = read(&synth::riff(1000, &[list(b"doc ", vec![stlt, synth::mcfg(v, 100.0, 50.0)])]), Limits::default()).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert!(d.page_size.is_some_and(|(w, h)| (w - 100.0).abs() < 0.01 && (h - 50.0).abs() < 0.01));
}

/// Before version 7 a `LIST stlt` holds chunks (R4), which are read like the document's others.
#[test]
fn a_style_list_before_version_7_is_read_as_chunks() {
    for version in [400, 500, 600] {
        let v = V(version);
        let red = if version < 500 { synth::color4(1, [0, 100, 100, 0]) } else { synth::color(5, [0, 0, 255, 0]) };
        let styles = list(b"stlt", vec![list(b"filt", vec![chunk(b"fill", v.fill(1, &red))])]);
        let rect = synth::object(v, 1, &[(20, 1u32.to_le_bytes().to_vec()), (30, v.rect(10.0, 10.0, 0.0))], None);
        let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![rect])]);
        let content = vec![list(b"doc ", vec![styles]), page];
        let d = read(&synth::file_of(version, content.clone()), Limits::default()).unwrap();
        let s = shapes(&d.pages[0].layers[0].objects);
        assert!(s[0].fill.is_some(), "{version}");
        assert!(d.warnings.is_empty(), "{version}: {:?}", d.warnings);
        // A template, whose form type names no version, by its `vrsn` chunk.
        let mut template = synth::file_of(version, content);
        template[8..12].copy_from_slice(b"CDTA");
        let d = read(&template, Limits::default()).unwrap();
        assert!(shapes(&d.pages[0].layers[0].objects)[0].fill.is_some(), "{version}");
    }
}

/// The objects of a styled drawing: a rectangle with only its style (argument 200), one whose
/// style leaves the fill and outline to its parent, and one with an outline argument of its own.
fn graphic_style_drawing(version: u16, table: synth::Node) -> Vec<u8> {
    let v = V(version);
    let black = synth::color(2, [0, 0, 0, 100]);
    let doc = list(
        b"doc ",
        vec![
            synth::mcfg(v, 612.0, 792.0),
            list(b"filt", vec![chunk(if version >= 700 { b"fild" } else { b"fill" }, v.fill(1, &synth::color(5, [0, 0, 255, 0])))]),
            list(
                b"otlt",
                vec![
                    chunk(b"outl", v.outline(2, synth::Pen { line_type: 0, width: 2.0, cap: 1, join: 1 }, &black, &[])),
                    chunk(b"outl", v.outline(3, synth::Pen { line_type: 1, width: 1.0, cap: 0, join: 0 }, &black, &[])),
                ],
            ),
            table,
        ],
    );
    let rect = (30, v.rect(20.0, 10.0, 0.0));
    let styled = |style: u32, own: &[(u32, Vec<u8>)]| {
        let mut args = vec![(200, style.to_le_bytes().to_vec())];
        args.extend(own.iter().cloned());
        args.push(rect.clone());
        synth::object(v, 1, &args, None)
    };
    let objects = vec![styled(0x10, &[]), styled(0x11, &[]), styled(0x10, &[(10, 3u32.to_le_bytes().to_vec())])];
    let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", objects)]);
    synth::file_of(version, vec![doc, page])
}

/// From version 7 an object without a fill or outline argument of its own takes its style's
/// (argument 200, `LIST stlt`), through the style's parent where the style names none; its own
/// argument wins over the style's.
#[test]
fn objects_without_their_own_fill_or_outline_take_their_styles() {
    let styles = [synth::Style { id: 0x10, parent: 0, fill: 0x100, outline: 0x200 }, synth::Style { id: 0x11, parent: 0x10, fill: 0, outline: 0 }];
    for version in [700, 800, 801, 900, 1000, 1100, 1200, 1300, 1400, 1500] {
        for text in [false, true] {
            let table = synth::stlt(V(version), &[(0x100, 1)], &[(0x200, 2)], &styles, text);
            let d = read(&graphic_style_drawing(version, table), Limits::default()).unwrap_or_else(|e| panic!("{version}: {e}"));
            let s = shapes(&d.pages[0].layers[0].objects);
            assert_eq!(s.len(), 3, "{version}");
            for shape in &s[..2] {
                assert_eq!(shape.fill, Some(Color::Rgb([1.0, 0.0, 0.0])), "{version} {text}");
                assert!(shape.outline.as_ref().is_some_and(|o| (o.width - 2.0).abs() < 0.05), "{version} {text}: {:?}", shape.outline);
            }
            assert_eq!(s[2].fill, Some(Color::Rgb([1.0, 0.0, 0.0])), "{version} {text}");
            assert!(s[2].outline.is_none(), "{version} {text}");
            assert!(d.warnings.is_empty(), "{version} {text}: {:?}", d.warnings);
        }
    }
}

/// A template whose style table comes before its `vrsn` chunk still reads the table by the
/// version that chunk names.
#[test]
fn a_style_table_before_the_version_chunk_reads_by_that_version() {
    let styles = [synth::Style { id: 0x10, parent: 0, fill: 0x100, outline: 0x200 }, synth::Style { id: 0x11, parent: 0x10, fill: 0, outline: 0 }];
    let table = synth::stlt(V(1000), &[(0x100, 1)], &[(0x200, 2)], &styles, true);
    let mut file = graphic_style_drawing(1000, chunk(b"junk", vec![]));
    // The drawing's chunks, the table put first, ahead of `vrsn`.
    let body = file.split_off(12);
    file.extend(synth::plain(&table));
    file.extend(body);
    file[8..12].copy_from_slice(b"CDTA");
    let len = (file.len() - 8) as u32;
    file[4..8].copy_from_slice(&len.to_le_bytes());
    let d = read(&file, Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert_eq!(s[0].fill, Some(Color::Rgb([1.0, 0.0, 0.0])));
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
}

/// A style table that doesn't parse to its end is damage when an object needs a style from it;
/// the object is drawn with what it holds itself.
#[test]
fn a_style_table_that_does_not_parse_is_damage_when_an_object_needs_it() {
    let styles = [synth::Style { id: 0x10, parent: 0, fill: 0x100, outline: 0x200 }, synth::Style { id: 0x11, parent: 0x10, fill: 0, outline: 0 }];
    let mut body = synth::stlt_body(V(1000), &[(0x100, 1)], &[(0x200, 2)], &styles, true);
    body.push(0);
    let table = chunk(b"LIST", [b"stlt".to_vec(), body].concat());
    let d = read(&graphic_style_drawing(1000, table), Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert_eq!(s.len(), 3);
    assert!(s[0].fill.is_none() && s[0].outline.is_none());
    assert_eq!(d.warnings, vec![Warning::Damaged]);
}

/// A drawing with fill 1 (red), whose `LIST clpt` holds `groups` and whose page holds `objects`.
fn clip_drawing(version: u16, groups: Vec<(u32, Vec<synth::Node>)>, objects: Vec<synth::Node>) -> Vec<u8> {
    let v = V(version);
    let doc = list(
        b"doc ",
        vec![
            synth::mcfg(v, 612.0, 792.0),
            list(b"filt", vec![chunk(if version >= 700 { b"fild" } else { b"fill" }, v.fill(1, &synth::color(5, [0, 0, 255, 0])))]),
            synth::clpt(groups),
        ],
    );
    let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", objects)]);
    synth::file_of(version, vec![doc, page])
}

fn clip_rect(v: V, w: f64, extra: &[(u32, Vec<u8>)], at: [f64; 2]) -> synth::Node {
    let mut args = vec![(20, 1u32.to_le_bytes().to_vec()), (30, v.rect(w, w, 0.0))];
    args.extend(extra.iter().cloned());
    synth::object(v, 1, &args, Some([1.0, 0.0, 0.0, 1.0, at[0], at[1]]))
}

/// A container (argument 8005) clips the group of `LIST clpt` whose `spnd` id it names; the
/// group's objects keep their own transforms, front to back.
#[test]
fn a_container_clips_the_group_it_names() {
    for version in [700, 900, 1200, 1300, 1500, 1600, 2000] {
        let v = V(version);
        let contents = vec![clip_rect(v, 10.0, &[], [5.0, 6.0]), clip_rect(v, 20.0, &[], [-50.0, 0.0])];
        let container = clip_rect(v, 40.0, &[synth::container(0x77)], [0.0, 0.0]);
        let d = read(&clip_drawing(version, vec![(0x77, contents)], vec![container]), Limits::default()).unwrap_or_else(|e| panic!("{version}: {e}"));
        let objects = &d.pages[0].layers[0].objects;
        let [Object::Clip { container, objects: clipped }] = objects.as_slice() else { panic!("{version}: {objects:?}") };
        assert!(matches!(container.geometry, Geometry::Rect { w, .. } if (w - 40.0).abs() < 0.01), "{version}");
        let s = shapes(clipped);
        assert_eq!(s.len(), 2, "{version}");
        assert!((s[0].matrix.0[4] - 5.0).abs() < 0.01 && (s[1].matrix.0[4] + 50.0).abs() < 0.01, "{version}: {:?}", s[0].matrix);
        assert!(d.warnings.is_empty(), "{version}: {:?}", d.warnings);
    }
}

/// The feature sample, the seed of the importer's fuzz tests, reads its style table, clipping
/// container, lens, flags, winding flag, transparency and named outline color in every version
/// from 7 to 13, plain and compressed.
#[test]
fn the_feature_sample_reads_its_styles_container_and_lens() {
    for version in [700, 800, 900, 1000, 1100, 1200, 1300] {
        for compress in [false, true] {
            let d = read(&synth::feature_sample(version, compress), Limits::default()).unwrap_or_else(|e| panic!("{version}: {e}"));
            let objects = &d.pages[0].layers[0].objects;
            let [
                Object::Shape(styled),
                Object::Clip { objects: clipped, .. },
                Object::Shape(_),
                Object::Shape(flagged),
                Object::Group { objects: grouped, flags },
            ] = objects.as_slice()
            else {
                panic!("{version}: {objects:?}")
            };
            assert_eq!(styled.fill, Some(Color::Rgb([1.0, 0.0, 0.0])), "{version}");
            assert!(styled.outline.as_ref().is_some_and(|o| o.dashes == [3.0, 1.0]), "{version}");
            let s = shapes(clipped);
            assert_eq!(s.len(), 2, "{version}");
            assert!(s[1].outline.as_ref().is_some_and(|o| o.behind_fill), "{version}");
            assert_eq!(flagged.flags, Flags { hidden: true, locked: true }, "{version}");
            assert!(flagged.nonzero, "{version}");
            assert_eq!(*flags, Flags { hidden: true, locked: false }, "{version}");
            let named = shapes(grouped)[0].outline.as_ref().map(|o| o.color.clone());
            // A transparency and a named outline color are read from version 13.
            if version >= 1300 {
                assert_eq!((flagged.opacity, flagged.blend), (0.75, Blend::Multiply), "{version}");
                assert_eq!(named, Some(Color::Spot { name: "Sky Blue".into(), tint: 1.0, look: Some([1.0, 0.0, 0.0]) }), "{version}");
                assert_eq!(d.warnings, vec![Warning::Lenses], "{version} {compress}");
            } else {
                assert_eq!((flagged.opacity, flagged.blend), (1.0, Blend::Normal), "{version}");
                assert_eq!(named, Some(Color::Spot { name: "Spot 0:7".into(), tint: 1.0, look: None }), "{version}");
                assert_eq!(d.warnings, vec![Warning::Lenses, Warning::SpotLooks], "{version} {compress}");
            }
        }
    }
}

/// A container whose group isn't there is damage, and one whose group holds a container naming
/// it again is read once, with the limit warning; the containers stay as plain shapes.
#[test]
fn a_missing_or_repeating_clipped_group_keeps_the_container() {
    let v = V(1100);
    let missing = clip_rect(v, 40.0, &[synth::container(0x78)], [0.0, 0.0]);
    let d = read(&clip_drawing(1100, vec![(0x77, vec![clip_rect(v, 10.0, &[], [0.0, 0.0])])], vec![missing]), Limits::default()).unwrap();
    assert!(matches!(d.pages[0].layers[0].objects.as_slice(), [Object::Shape(_)]));
    assert_eq!(d.warnings, vec![Warning::Damaged]);
    let inner = clip_rect(v, 10.0, &[synth::container(0x77)], [0.0, 0.0]);
    let outer = clip_rect(v, 40.0, &[synth::container(0x77)], [0.0, 0.0]);
    let d = read(&clip_drawing(1100, vec![(0x77, vec![inner])], vec![outer]), Limits::default()).unwrap();
    let [Object::Clip { objects: clipped, .. }] = d.pages[0].layers[0].objects.as_slice() else { panic!("{:?}", d.pages[0].layers[0].objects) };
    assert!(matches!(clipped.as_slice(), [Object::Shape(_)]), "{clipped:?}");
    assert_eq!(d.warnings, vec![Warning::TooDeep]);
}

/// Each chunk of a clipped group counts toward the clipped chunk limit every time a container
/// names the group, empty groups included; past it the containers stay as plain shapes, and a
/// group cut off at the limit leaves out its guidelines too.
#[test]
fn containers_naming_one_group_stop_at_the_clipped_chunk_limit() {
    let v = V(1100);
    // The group walks 53 chunks: its `spnd`, a guideline, 50 empty groups and a rectangle.
    let mut guide = vec![0; 12];
    guide.extend(90_000_000i32.to_le_bytes());
    guide.extend([v.coord(36.0), v.coord(0.0)].concat());
    let mut contents = vec![synth::object(v, 0x16, &[(16000, guide)], None)];
    contents.extend((0..50).map(|_| list(b"grp ", vec![])));
    contents.push(clip_rect(v, 10.0, &[], [0.0, 0.0]));
    let containers: Vec<synth::Node> = (0..20).map(|_| clip_rect(v, 40.0, &[synth::container(0x77)], [0.0, 0.0])).collect();
    let file = clip_drawing(1100, vec![(0x77, contents)], containers);
    let count = |objects: &[Object]| objects.iter().filter(|o| matches!(o, Object::Clip { .. })).count();
    let d = read(&file, Limits::default()).unwrap();
    assert_eq!((count(&d.pages[0].layers[0].objects), d.pages[0].layers[0].guides.len()), (20, 20));
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    // Looking through the guideline's and the rectangle's lists for what they keep for themselves
    // takes 13 chunks, counted once. The sixth container's walk stops after its guideline and 8
    // empty groups.
    let d = read(&file, Limits { clipped_chunks: 53 * 5 + 13 + 10, ..Limits::default() }).unwrap();
    let layer = &d.pages[0].layers[0];
    assert_eq!((layer.objects.len(), count(&layer.objects), layer.guides.len()), (20, 5, 5));
    assert_eq!(d.warnings, vec![Warning::TooDeep]);
}

/// The chunks an object of a clipped group keeps for itself count toward the clipped chunk limit
/// the first time a container names the group, and are not looked through again for the next
/// containers.
#[test]
fn a_clipped_objects_own_chunks_count_once() {
    let v = V(1100);
    // A rectangle whose list holds 1,000 chunks before its own 5.
    let synth::Node::List(form, own) = clip_rect(v, 10.0, &[], [0.0, 0.0]) else { panic!("an object is a list") };
    let rect = synth::Node::List(form, (0..1000).map(|_| chunk(b"pad ", vec![])).chain(own).collect());
    let containers: Vec<synth::Node> = (0..20).map(|_| clip_rect(v, 40.0, &[synth::container(0x77)], [0.0, 0.0])).collect();
    let file = clip_drawing(1100, vec![(0x77, vec![rect.clone()])], containers);
    let count = |d: &craft_cdr::Drawing| d.pages[0].layers[0].objects.iter().filter(|o| matches!(o, Object::Clip { .. })).count();
    // Each container walks the group's `spnd` and the rectangle's list, and the first also looks
    // through the rectangle's chunks for its `loda` (1,003 chunks), `flgs` (1,001) and `trfd`
    // (1,005).
    let first = 1003 + 1001 + 1005;
    let d = read(&file, Limits { clipped_chunks: first + 20 * 2, ..Limits::default() }).unwrap();
    assert_eq!(count(&d), 20);
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    // Past the limit while looking through them, no container keeps its group, nor the objects
    // and guidelines read before it.
    let mut guide = vec![0; 12];
    guide.extend(90_000_000i32.to_le_bytes());
    guide.extend([v.coord(36.0), v.coord(0.0)].concat());
    let before = vec![clip_rect(v, 5.0, &[], [0.0, 0.0]), synth::object(v, 0x16, &[(16000, guide)], None), rect];
    let file = clip_drawing(1100, vec![(0x77, before)], vec![clip_rect(v, 40.0, &[synth::container(0x77)], [0.0, 0.0])]);
    let d = read(&file, Limits { clipped_chunks: 1000, ..Limits::default() }).unwrap();
    let layer = &d.pages[0].layers[0];
    assert!(matches!(layer.objects.as_slice(), [Object::Shape(_)]), "{:?}", layer.objects);
    assert!(layer.guides.is_empty() && d.guides.is_empty(), "{:?}", layer.guides);
    assert_eq!(d.warnings, vec![Warning::TooDeep]);
    // The same past the limit while looking for an object's `loda`, `flgs` or `trfd` after 1,000
    // chunks.
    let synth::Node::List(form, own) = clip_rect(v, 10.0, &[], [0.0, 0.0]) else { panic!("an object is a list") };
    let [flgs, lgob @ synth::Node::List(lgob_form, kids)] = own.as_slice() else { panic!("{own:?}") };
    let [loda, transform @ synth::Node::List(trfl, trfd)] = kids.as_slice() else { panic!("{kids:?}") };
    let pads = || (0..1000).map(|_| chunk(b"pad ", vec![]));
    let loda_late = synth::Node::List(
        form,
        vec![flgs.clone(), synth::Node::List(*lgob_form, std::iter::once(transform.clone()).chain(pads()).chain([loda.clone()]).collect())],
    );
    let flags_late = synth::Node::List(form, std::iter::once(lgob.clone()).chain(pads()).chain([flgs.clone()]).collect());
    let trfd_late = synth::Node::List(
        form,
        vec![flgs.clone(), synth::Node::List(*lgob_form, vec![loda.clone(), synth::Node::List(*trfl, pads().chain(trfd.iter().cloned()).collect())])],
    );
    for late in [loda_late, flags_late, trfd_late] {
        let file = clip_drawing(1100, vec![(0x77, vec![late])], vec![clip_rect(v, 40.0, &[synth::container(0x77)], [0.0, 0.0])]);
        let d = read(&file, Limits { clipped_chunks: 500, ..Limits::default() }).unwrap();
        assert!(matches!(d.pages[0].layers[0].objects.as_slice(), [Object::Shape(_)]), "{:?}", d.pages[0].layers[0].objects);
        assert_eq!(d.warnings, vec![Warning::TooDeep]);
    }
    // Chunks after the ones an object keeps for itself are not looked through.
    let synth::Node::List(form, own) = clip_rect(v, 10.0, &[], [0.0, 0.0]) else { panic!("an object is a list") };
    let rect = synth::Node::List(form, own.into_iter().chain((0..1000).map(|_| chunk(b"pad ", vec![]))).collect());
    let containers: Vec<synth::Node> = (0..20).map(|_| clip_rect(v, 40.0, &[synth::container(0x77)], [0.0, 0.0])).collect();
    let file = clip_drawing(1100, vec![(0x77, vec![rect])], containers);
    let d = read(&file, Limits { clipped_chunks: 3 + 1 + 5 + 20 * 2, ..Limits::default() }).unwrap();
    assert_eq!(count(&d), 20);
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
}

/// A container, fill, outline or style argument too short for the id it names is damage: the
/// shape comes in as a plain shape without that id.
#[test]
fn an_id_argument_cut_short_is_damage() {
    for version in [700, 1100, 1600] {
        let v = V(version);
        let group = vec![(0x77, vec![clip_rect(v, 10.0, &[], [0.0, 0.0])])];
        let container = clip_rect(v, 40.0, &[(8005, vec![0x77, 0, 0])], [0.0, 0.0]);
        let d = read(&clip_drawing(version, group, vec![container]), Limits::default()).unwrap_or_else(|e| panic!("{version}: {e}"));
        assert!(matches!(d.pages[0].layers[0].objects.as_slice(), [Object::Shape(_)]), "{version}: {:?}", d.pages[0].layers[0].objects);
        assert_eq!(d.warnings, vec![Warning::Damaged], "{version}");
        for kind in [20, 10, 200] {
            let short = synth::object(v, 1, &[(kind, vec![1, 0]), (30, v.rect(10.0, 10.0, 0.0))], None);
            let d = read(&clip_drawing(version, vec![], vec![short]), Limits::default()).unwrap_or_else(|e| panic!("{version} {kind}: {e}"));
            let s = shapes(&d.pages[0].layers[0].objects);
            assert!(s.len() == 1 && s[0].fill.is_none() && s[0].outline.is_none(), "{version} {kind}");
            assert_eq!(d.warnings, vec![Warning::Damaged], "{version} {kind}");
        }
    }
}

/// A fill or outline id that no record has is damage: the shape comes in without that paint. Id 0
/// without a record is no paint.
#[test]
fn a_fill_or_outline_without_its_record_is_damage() {
    for version in [700, 1100, 1600] {
        let v = V(version);
        for kind in [20, 10] {
            for (id, want) in [(9u32, vec![Warning::Damaged]), (0, vec![])] {
                let object = synth::object(v, 1, &[(kind, id.to_le_bytes().to_vec()), (30, v.rect(10.0, 10.0, 0.0))], None);
                let d =
                    read(&clip_drawing(version, vec![], vec![object]), Limits::default()).unwrap_or_else(|e| panic!("{version} {kind} {id}: {e}"));
                let s = shapes(&d.pages[0].layers[0].objects);
                assert!(s.len() == 1 && s[0].fill.is_none() && s[0].outline.is_none(), "{version} {kind} {id}");
                assert_eq!(d.warnings, want, "{version} {kind} {id}");
            }
        }
    }
}

/// An object with a lens or transparency (argument 8000) comes in opaque, with the lenses
/// warning; one without the argument gives no warning.
#[test]
fn lenses_and_transparency_are_named_in_the_warnings() {
    for (version, lens) in [(1000, [1u8, 0, 0, 0].to_vec()), (1200, [5, 0, 0, 0].to_vec()), (1500, [0x14, 5, 0, 0, 0x75, 0, 0, 0, 5].to_vec())] {
        let v = V(version);
        let plain = clip_rect(v, 10.0, &[], [0.0, 0.0]);
        let d = read(&clip_drawing(version, vec![], vec![plain]), Limits::default()).unwrap();
        assert!(d.warnings.is_empty(), "{version}: {:?}", d.warnings);
        let see_through = clip_rect(v, 10.0, &[(8000, lens)], [0.0, 0.0]);
        let d = read(&clip_drawing(version, vec![], vec![see_through]), Limits::default()).unwrap();
        assert_eq!(shapes(&d.pages[0].layers[0].objects).len(), 1, "{version}");
        assert_eq!(d.warnings, vec![Warning::Lenses], "{version}");
    }
}

/// From version 13 a uniform transparency (a 117-byte record of version 1300 in argument 8000)
/// gives the shape its opacity and blend mode, records of other versions before it skipped. A
/// lens record, a value outside 0 to 100, a merge mode without a blend mode, and the record in an
/// earlier version are not applied: the shape stays opaque, with the lenses warning.
#[test]
fn a_uniform_transparency_from_version_13_applies() {
    let record = |since: u32, body: Vec<u8>| [since.to_le_bytes().to_vec(), (body.len() as u32).to_le_bytes().to_vec(), body].concat();
    for version in [1300, 1500, 1600, 2000] {
        let v = V(version);
        let read_with = |arg: Vec<u8>| {
            let d = read(&clip_drawing(version, vec![], vec![clip_rect(v, 10.0, &[(8000, arg)], [0.0, 0.0])]), Limits::default())
                .unwrap_or_else(|e| panic!("{version}: {e}"));
            let s = shapes(&d.pages[0].layers[0].objects)[0];
            ((s.opacity, s.blend), d.warnings)
        };
        for (percent, merge, want) in [(25.0, 0, Blend::Normal), (0.0, 9, Blend::Multiply), (100.0, 28, Blend::Screen), (60.0, 36, Blend::Exclusion)]
        {
            let opacity = (1.0 - percent / 100.0) as f32;
            assert_eq!(read_with(synth::transparency(percent, merge).1), ((opacity, want), vec![]), "{version} {percent} {merge}");
        }
        // A record of a later version comes first.
        let later = [record(1600, vec![0; 20]), synth::transparency(25.0, 0).1].concat();
        assert_eq!(read_with(later), ((0.75, Blend::Normal), vec![]), "{version}");
        let opaque = ((1.0, Blend::Normal), vec![Warning::Lenses]);
        assert_eq!(read_with(record(1300, vec![0; 84])), opaque, "{version}: a lens");
        for bad in [150.0, -1.0, f64::NAN] {
            assert_eq!(read_with(synth::transparency(bad, 0).1), opaque, "{version} {bad}");
        }
        for merge in [1, 2, 3, 6, 7, 8, 10, 13, 18, 19, 20, 27, 37, 255] {
            assert_eq!(read_with(synth::transparency(25.0, merge).1), opaque, "{version} {merge}");
        }
        assert_eq!(read_with(vec![]), opaque, "{version}: empty");
    }
    let d =
        read(&clip_drawing(1200, vec![], vec![clip_rect(V(1200), 10.0, &[synth::transparency(25.0, 0)], [0.0, 0.0])]), Limits::default()).unwrap();
    assert_eq!(shapes(&d.pages[0].layers[0].objects)[0].opacity, 1.0);
    assert_eq!(d.warnings, vec![Warning::Lenses]);
}

/// Bit 0x01 of byte 1 of a layer's, group's or object's flags hides it and bit 0x10 of byte 0
/// locks it, in every version; a group left ungrouped at the object limit passes its flags to its
/// objects.
#[test]
fn flags_hide_and_lock_layers_groups_and_objects() {
    for version in [300, 700, 1100, 1300, 1600] {
        let v = V(version);
        let rect = |flags: [u8; 4]| synth::flagged(synth::object(v, 1, &[(30, v.rect(10.0, 10.0, 0.0))], None), flags);
        let group = |flags: [u8; 4], objects: Vec<synth::Node>| {
            let mut kids = vec![chunk(b"flgs", flags.to_vec())];
            kids.extend(objects);
            list(b"grp ", kids)
        };
        let objects = vec![
            rect([0, 0, 0, 8]),
            rect([0, 1, 0, 8]),
            rect([0x10, 0, 0, 8]),
            rect([0x10, 1, 0, 8]),
            group([0x10, 1, 0, 0x10], vec![rect([0, 0, 0, 8])]),
        ];
        let layers =
            vec![synth::layer(v, [0, 0, 0, 0x98], "Shown", objects), synth::layer(v, [0x10, 1, 0, 0x98], "Hidden", vec![rect([0, 0, 0, 8])])];
        let file = synth::file_of(version, vec![synth::page(false, layers)]);
        let d = read(&file, Limits::default()).unwrap_or_else(|e| panic!("{version}: {e}"));
        let [shown, hidden] = d.pages[0].layers.as_slice() else { panic!("{version}: {:?}", d.pages[0].layers) };
        assert_eq!(shown.flags, Flags::default(), "{version}");
        assert_eq!(hidden.flags, Flags { hidden: true, locked: true }, "{version}");
        let f = |hidden: bool, locked: bool| Flags { hidden, locked };
        let own: Vec<Flags> = shown
            .objects
            .iter()
            .map(|o| match o {
                Object::Shape(s) => s.flags,
                Object::Group { flags, .. } => *flags,
                Object::Clip { container, .. } => container.flags,
            })
            .collect();
        assert_eq!(own, [f(false, false), f(true, false), f(false, true), f(true, true), f(true, true)], "{version}");
        assert!(
            matches!(&shown.objects[4], Object::Group { objects, .. } if matches!(&objects[0], Object::Shape(s) if s.flags == Flags::default())),
            "{version}"
        );
        assert!(d.warnings.contains(&Warning::UnsupportedObjects) || d.warnings.is_empty(), "{version}: {:?}", d.warnings);
        // Past the object limit the group's rectangle comes in ungrouped, hidden and locked.
        let d = read(&file, Limits { objects: 5, ..Limits::default() }).unwrap_or_else(|e| panic!("{version}: {e}"));
        let last = d.pages[0].layers[0].objects.last();
        assert!(matches!(last, Some(Object::Shape(s)) if s.flags == f(true, true)), "{version}: {last:?}");
    }
}

/// A guideline is hidden when its own object, a group around it or its layer is.
#[test]
fn hidden_guidelines_say_so() {
    for version in [700, 1300, 1600] {
        let v = V(version);
        let guide = |flags: [u8; 4]| {
            let mut arg = vec![0; 12];
            arg.extend(90_000_000i32.to_le_bytes());
            arg.extend([v.coord(36.0), v.coord(0.0)].concat());
            synth::flagged(synth::object(v, 0x16, &[(16000, arg)], None), flags)
        };
        let group = list(b"grp ", vec![chunk(b"flgs", vec![0, 1, 0, 0x10]), guide([0, 0, 0, 8])]);
        let layers = vec![
            synth::layer(v, [0, 0, 0, 0x98], "Shown", vec![guide([0, 0, 0, 8]), guide([0, 1, 0, 8]), group]),
            synth::layer(v, [0, 1, 0, 0x98], "Hidden", vec![guide([0, 0, 0, 8])]),
        ];
        let d = read(&synth::file_of(version, vec![synth::page(false, layers)]), Limits::default()).unwrap_or_else(|e| panic!("{version}: {e}"));
        let hidden: Vec<Vec<bool>> = d.pages[0].layers.iter().map(|l| l.guides.iter().map(|g| g.hidden).collect()).collect();
        assert_eq!(hidden, [vec![false, true, true], vec![true]], "{version}");
    }
}

/// Byte 0 of argument 14001 other than 0 fills the shape nonzero; 0, or no argument, fills it
/// even-odd; an argument without its byte is damage.
#[test]
fn a_winding_flag_fills_nonzero() {
    for version in [300, 700, 1200, 1600] {
        let v = V(version);
        let read_with = |extra: &[(u32, Vec<u8>)]| {
            let mut args = vec![(30, v.rect(10.0, 10.0, 0.0))];
            args.extend(extra.iter().cloned());
            let file = synth::file_of(
                version,
                vec![synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &args, None)])])],
            );
            let d = read(&file, Limits::default()).unwrap_or_else(|e| panic!("{version}: {e}"));
            (shapes(&d.pages[0].layers[0].objects)[0].nonzero, d.warnings)
        };
        assert_eq!(read_with(&[(14001, vec![1, 0, 0, 0])]), (true, vec![]), "{version}");
        assert_eq!(read_with(&[(14001, vec![2])]), (true, vec![]), "{version}");
        assert_eq!(read_with(&[(14001, vec![0, 0, 0, 0])]), (false, vec![]), "{version}");
        assert_eq!(read_with(&[]), (false, vec![]), "{version}");
        assert_eq!(read_with(&[(14001, vec![])]), (false, vec![Warning::Damaged]), "{version}");
    }
}

/// A `cdr8` form type makes the file the 8.01 variant whatever its `vrsn` chunk names: its style
/// table is read with the 8.01 layout.
#[test]
fn a_cdr8_file_whose_version_chunk_names_800_reads_as_801() {
    let styles = [synth::Style { id: 0x10, parent: 0, fill: 0x100, outline: 0x200 }, synth::Style { id: 0x11, parent: 0x10, fill: 0, outline: 0 }];
    for named in [800u16, 900, 1300] {
        let table = synth::stlt(V(801), &[(0x100, 1)], &[(0x200, 2)], &styles, true);
        let mut file = graphic_style_drawing(801, table);
        file[8..12].copy_from_slice(b"cdr8");
        let at = file.windows(4).position(|w| w == b"vrsn").unwrap() + 8;
        assert_eq!(&file[at..at + 2], &801u16.to_le_bytes());
        file[at..at + 2].copy_from_slice(&named.to_le_bytes());
        let d = read(&file, Limits::default()).unwrap_or_else(|e| panic!("{named}: {e}"));
        assert_eq!(d.version, 801, "{named}");
        let s = shapes(&d.pages[0].layers[0].objects);
        assert_eq!(s[0].fill, Some(Color::Rgb([1.0, 0.0, 0.0])), "{named}");
        assert!(d.warnings.is_empty(), "{named}: {:?}", d.warnings);
    }
}

/// Before version 6 an object names its style in 2 bytes, which is not damage; an argument too
/// short for them is.
#[test]
fn a_style_id_before_version_6_is_two_bytes() {
    for version in [400, 500] {
        let v = V(version);
        let read_with = |style: Vec<u8>| {
            let object = synth::object(v, 1, &[(200, style), (30, v.rect(10.0, 10.0, 0.0))], None);
            let file = synth::file_of(version, vec![synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![object])])]);
            let d = read(&file, Limits::default()).unwrap_or_else(|e| panic!("{version}: {e}"));
            (shapes(&d.pages[0].layers[0].objects).len(), d.warnings)
        };
        assert_eq!(read_with(vec![0x10, 0]), (1, vec![]), "{version}");
        assert_eq!(read_with(vec![0x10]), (1, vec![Warning::Damaged]), "{version}");
    }
}

/// A hidden or locked group of `LIST clpt` hides or locks the objects a container clips from it.
#[test]
fn a_clipped_groups_flags_reach_its_objects() {
    for version in [700, 1300, 1600] {
        let v = V(version);
        let group = list(
            b"grp ",
            vec![chunk(b"flgs", vec![0x10, 1, 0x80, 0x10]), chunk(b"spnd", 0x77u32.to_le_bytes().to_vec()), clip_rect(v, 10.0, &[], [0.0, 0.0])],
        );
        let doc = list(b"doc ", vec![synth::mcfg(v, 612.0, 792.0), list(b"clpt", vec![group])]);
        let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![clip_rect(v, 40.0, &[synth::container(0x77)], [0.0, 0.0])])]);
        let d = read(&synth::file_of(version, vec![doc, page]), Limits::default()).unwrap_or_else(|e| panic!("{version}: {e}"));
        let [Object::Clip { container, objects }] = d.pages[0].layers[0].objects.as_slice() else {
            panic!("{version}: {:?}", d.pages[0].layers[0].objects)
        };
        assert_eq!(container.flags, Flags::default(), "{version}");
        assert!(matches!(objects.as_slice(), [Object::Shape(s)] if s.flags == Flags { hidden: true, locked: true }), "{version}: {objects:?}");
    }
}

/// Before version 6 a clipped group's `spnd` id is 2 bytes.
#[test]
fn a_clipped_group_id_before_version_6_is_two_bytes() {
    for version in [400, 500] {
        let v = V(version);
        let rect = |w: f64, extra: Vec<(u32, Vec<u8>)>| {
            let mut args = vec![(30, v.rect(w, w, 0.0))];
            args.extend(extra);
            synth::object(v, 1, &args, None)
        };
        let clpt = list(b"clpt", vec![list(b"grp ", vec![chunk(b"spnd", vec![0x77, 0]), rect(10.0, vec![])])]);
        let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![rect(40.0, vec![synth::container(0x77)])])]);
        let d = read(&synth::file_of(version, vec![list(b"doc ", vec![synth::mcfg(v, 612.0, 792.0), clpt]), page]), Limits::default())
            .unwrap_or_else(|e| panic!("{version}: {e}"));
        let objects = &d.pages[0].layers[0].objects;
        assert!(matches!(objects.as_slice(), [Object::Clip { objects, .. }] if objects.len() == 1), "{version}: {objects:?}");
        assert!(d.warnings.is_empty(), "{version}: {:?}", d.warnings);
    }
}

/// A `trfd` entry ends at the next offset: one too short for its matrix makes the transform
/// damage and leaves the shape in place, even when the bytes after it would read as a matrix. A
/// table that stops before the last entry's end offset leaves that entry the rest of the record.
#[test]
fn a_transform_entry_ends_at_the_next_offset() {
    for version in [400, 1100, 1300, 1600] {
        let v = V(version);
        let w = if version < 600 { 2 } else { 4 };
        let moved = [1.0, 0.0, 0.0, 1.0, 72.0, 0.0];
        let object = |trfd: Vec<u8>| {
            let own = vec![chunk(b"loda", v.loda(1, &[(30, v.rect(10.0, 10.0, 0.0))])), list(b"trfl", vec![chunk(b"trfd", trfd)])];
            list(b"obj ", vec![chunk(b"flgs", vec![0, 0, 0, 8]), list(b"lgob", own)])
        };
        let read_with = |trfd: Vec<u8>| {
            let file = synth::file_of(version, vec![synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![object(trfd)])])]);
            let d = read(&file, Limits::default()).unwrap_or_else(|e| panic!("{version}: {e}"));
            (shapes(&d.pages[0].layers[0].objects)[0].matrix.0[4], d.warnings)
        };
        let whole = v.trfd(&[moved]);
        assert!((read_with(whole.clone()).0 - 72.0).abs() < 0.01, "{version}");
        // The end offset, right after the first, made 4 bytes past the entry's start.
        let mut short = whole.clone();
        let start = if w == 2 {
            usize::from(u16::from_le_bytes([short[3 * w], short[3 * w + 1]]))
        } else {
            u32::from_le_bytes(short[12..16].try_into().unwrap()) as usize
        };
        short[4 * w..5 * w].copy_from_slice(&v.word(start + 4));
        assert_eq!(read_with(short), (0.0, vec![Warning::Damaged]), "{version}");
        // The entry first, then a table of its start alone at the record's end.
        let entry = whole[5 * w..].to_vec();
        let at = 3 * w + entry.len();
        let mut end = [v.word(at + w), v.word(1), v.word(at)].concat();
        end.extend(entry);
        end.extend(v.word(3 * w));
        let (x, warnings) = read_with(end);
        assert!((x - 72.0).abs() < 0.01 && warnings.is_empty(), "{version}: {x} {warnings:?}");
    }
}

/// From version 13 an outline's record of type 2 that names a library color gives the outline
/// that color; one without a name leaves the fixed color field.
#[test]
fn an_outline_takes_a_named_library_color_from_its_property_list() {
    for version in [1300, 1500, 1600] {
        let v = V(version);
        let read_with = |props: Vec<u8>| {
            let mut outl = v.outline(4, synth::Pen { line_type: 0, width: 1.0, cap: 0, join: 0 }, &synth::color(2, [0, 0, 0, 100]), &[]);
            let mut record = 2u32.to_le_bytes().to_vec();
            record.extend((props.len() as u32).to_le_bytes());
            record.extend(props);
            outl.splice(4..4, record);
            let doc = list(b"doc ", vec![synth::mcfg(v, 612.0, 792.0), list(b"otlt", vec![chunk(b"outl", outl)])]);
            let object = synth::object(v, 1, &[(10, 4u32.to_le_bytes().to_vec()), (30, v.rect(10.0, 10.0, 0.0))], None);
            let file = synth::file_of(version, vec![doc, synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![object])])]);
            let d = read(&file, Limits::default()).unwrap_or_else(|e| panic!("{version}: {e}"));
            (shapes(&d.pages[0].layers[0].objects)[0].outline.as_ref().map(|o| o.color.clone()), d.warnings)
        };
        let named = synth::properties(&synth::color(5, [0, 0, 255, 0]), Some("Sky Blue"));
        assert_eq!(read_with(named), (Some(Color::Spot { name: "Sky Blue".into(), tint: 1.0, look: Some([1.0, 0.0, 0.0]) }), vec![]), "{version}");
        let plain = synth::properties(&synth::color(5, [0, 0, 255, 0]), None);
        assert_eq!(read_with(plain), (Some(Color::Cmyk([0.0, 0.0, 0.0, 1.0])), vec![]), "{version}");
    }
}

#[test]
fn linked_groups_are_left_out_with_a_warning() {
    let v = V(1100);
    let rect = synth::object(v, 1, &[(30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![list(b"lnkg", vec![rect])])])]);
    let d = read(&file, Limits::default()).unwrap();
    assert!(d.pages[0].layers[0].objects.is_empty());
    assert!(d.warnings.contains(&Warning::UnsupportedObjects));
}

#[test]
fn a_file_cut_short_keeps_what_it_holds() {
    let full = synth::sample(900, false);
    let cut = &full[..full.len() - 40];
    let d = read(cut, Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged));
    // The last page is cut inside its layer: the lists that hold the cut keep their whole
    // children, so the rectangle, ellipse and curve before the cut are read.
    assert_eq!(d.pages.len(), 1);
    let s = shapes(&d.pages[0].layers[0].objects);
    assert!(s.len() >= 3, "{}", s.len());
    assert!(matches!(s[0].geometry, Geometry::Rect { .. }) && matches!(s[1].geometry, Geometry::Ellipse { .. }));
}

#[test]
fn a_riff_length_shorter_than_the_form_type_is_damage() {
    for declared in 0u32..4 {
        let mut file = synth::sample(900, false);
        file[4..8].copy_from_slice(&declared.to_le_bytes());
        let d = read(&file, Limits::default()).unwrap();
        assert!(d.warnings.contains(&Warning::Damaged), "{declared}");
        check_shapes(&d, 900);
    }
}

#[test]
fn a_zip_bomb_stops_at_the_inflate_limit() {
    let v = V(1100);
    let big = chunk(b"junk", vec![0; 4 << 20]);
    let file = synth::riff(1100, &[chunk(b"vrsn", 1100u16.to_le_bytes().to_vec()), synth::cmpr(&[big, synth::mcfg(v, 10.0, 10.0)])]);
    assert!(file.len() < 64 << 10);
    let limits = Limits { inflated: 1 << 20, ..Limits::default() };
    assert_eq!(read(&file, limits), Err(Error::Limit("inflated size")));
}

/// What a compressed stream inflates before it fails counts toward the inflate limit, so that
/// streams failing at their checksum can't each inflate up to the limit.
#[test]
fn bytes_inflated_by_a_failing_stream_count_toward_the_limit() {
    let failing = || {
        let mut cmpr = synth::cmpr(&[chunk(b"junk", vec![0; 600 << 10])]);
        if let synth::Node::Chunk(_, body) = &mut cmpr {
            // The last byte of the first block's zlib stream is part of its checksum.
            let first = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize;
            body[4 + 16 + first - 1] ^= 0xff;
        }
        cmpr
    };
    let file = synth::riff(1100, &[chunk(b"vrsn", 1100u16.to_le_bytes().to_vec()), failing(), failing()]);
    let one =
        read(&synth::riff(1100, &[chunk(b"vrsn", 1100u16.to_le_bytes().to_vec()), failing()]), Limits { inflated: 1 << 20, ..Limits::default() });
    assert!(one.is_ok_and(|d| d.warnings == vec![Warning::Damaged]));
    assert_eq!(read(&file, Limits { inflated: 1 << 20, ..Limits::default() }), Err(Error::Limit("inflated size")));
}

#[test]
fn a_million_empty_chunks_stop_at_the_chunk_limit() {
    let kids: Vec<_> = (0..5000).map(|_| chunk(b"junk", vec![])).collect();
    let file = synth::riff(1100, &[list(b"doc ", kids)]);
    let limits = Limits { chunks: 1000, ..Limits::default() };
    assert_eq!(read(&file, limits), Err(Error::Limit("chunk count")));
    assert!(read(&file, Limits::default()).is_ok());
}

#[test]
fn deep_nesting_is_left_out_with_a_warning() {
    let v = V(1100);
    let mut inner = synth::object(v, 1, &[(30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    for _ in 0..300 {
        inner = list(b"grp ", vec![inner]);
    }
    let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![inner])])]);
    let d = read(&file, Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::TooDeep));
}

#[test]
fn a_size_pool_index_out_of_range_is_damage() {
    // A compressed stream whose one chunk names pool entry 7 of a pool of one.
    let mut cmpr = synth::cmpr(&[chunk(b"mcfg", vec![0; 8])]);
    if let synth::Node::Chunk(_, body) = &mut cmpr {
        // Rebuild the stream with a bad index: the pool keeps its one size.
        let mut stream = b"mcfg".to_vec();
        stream.extend(7u32.to_le_bytes());
        stream.extend([0; 8]);
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut e, &stream).unwrap();
        let z = e.finish().unwrap();
        let pool_block = body[4 + 16 + u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize..].to_vec();
        let mut b = b"cmpr".to_vec();
        b.extend(((z.len() + 8) as u32).to_le_bytes());
        b.extend((stream.len() as u32).to_le_bytes());
        b.extend(body[12..20].to_vec());
        b.extend(b"CPng\x01\x00\x04\x00");
        b.extend(z);
        b.extend(pool_block);
        *body = b;
    }
    let d = read(&synth::riff(1100, &[cmpr]), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
}

#[test]
fn backward_and_huge_loda_offsets_do_not_panic() {
    let v = V(1100);
    let mut loda = v.loda(3, &[(30, v.points(&[(0.0, 0.0, 0), (1.0, 1.0, 0x40)]))]);
    // The offsets table now points before the header and far past the end.
    let at = u32::from_le_bytes(loda[8..12].try_into().unwrap()) as usize;
    loda[at..at + 4].copy_from_slice(&40u32.to_le_bytes());
    loda[at + 4..at + 8].copy_from_slice(&2u32.to_le_bytes());
    let obj = list(b"obj ", vec![chunk(b"loda", loda)]);
    let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![obj])])]);
    let d = read(&file, Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged));
}

#[test]
fn outlines_flagged_none_and_dashes_read() {
    let v = V(1300);
    let black = synth::color(2, [0, 0, 0, 100]);
    let dashed = v.outline(9, synth::Pen { line_type: 4, width: 1.0, cap: 2, join: 2 }, &black, &[3, 1]);
    let style = vec![(10, 9u32.to_le_bytes().to_vec()), (30, v.points(&[(0.0, 0.0, 0), (10.0, 0.0, 0x40)]))];
    let file = synth::riff(
        1300,
        &[
            chunk(b"vrsn", 1300u16.to_le_bytes().to_vec()),
            list(b"otlt", vec![chunk(b"outl", dashed)]),
            synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 3, &style, None)])]),
        ],
    );
    let d = read(&file, Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    let o = s[0].outline.as_ref().unwrap();
    assert_eq!(o.dashes, vec![3.0, 1.0]);
    assert_eq!((o.cap, o.join), (Cap::Square, craft_cdr::model::Join::Bevel));
}

/// A curve or polygon left out at the coordinate limit gives the limit warning, not the damage
/// warning.
#[test]
fn shapes_past_the_coordinate_limit_are_not_damage() {
    let v = V(1100);
    let curve = synth::object(v, 3, &[(30, v.points(&[(0.0, 0.0, 0), (10.0, 0.0, 0x40), (10.0, 10.0, 0x40)]))], None);
    let side = synth::POLYGON_SIDE;
    let mut sides = vec![0; 4];
    sides.extend(5u32.to_le_bytes());
    sides.extend([0; 8]);
    sides.extend(0.05f64.to_le_bytes());
    sides.extend(0.05f64.to_le_bytes());
    sides.extend([v.coord(0.0), v.coord(0.0)].concat());
    let base = v.points(&[(0.0, side, 0x0C), (side * 0.587, side * 0.809, 0x44)]);
    let polygon = synth::object(v, 0x14, &[(30, base.clone()), (11000, sides)], None);
    for (objects, limit) in [(vec![curve], 2), (vec![polygon], 4)] {
        let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", objects)])]);
        let d = read(&file, Limits { coordinates: limit, ..Limits::default() }).unwrap();
        assert!(d.pages[0].layers[0].objects.is_empty());
        assert_eq!(d.warnings, vec![Warning::TooDeep]);
    }
    // A polygon without its sides is damage, past the limit too.
    let sideless = synth::object(v, 0x14, &[(30, base)], None);
    let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![sideless])])]);
    let d = read(&file, Limits { coordinates: 1, ..Limits::default() }).unwrap();
    assert_eq!(d.warnings, vec![Warning::Damaged]);
}

/// Dashes stored in an outline whose line type doesn't mark it dashed (bit 2) are not drawn: the
/// outline is solid.
#[test]
fn dashes_are_drawn_only_when_the_line_type_marks_them() {
    let black = synth::color(2, [0, 0, 0, 100]);
    for version in [700, 1200, 1500] {
        for (line_type, dashed) in [(2, false), (0x22, false), (0x42, false), (4, true), (0x24, true), (0x44, true)] {
            let v = V(version);
            let outline = v.outline(9, synth::Pen { line_type, width: 1.0, cap: 0, join: 0 }, &black, &[2, 1]);
            let style = vec![(10, 9u32.to_le_bytes().to_vec()), (30, v.points(&[(0.0, 0.0, 0), (10.0, 0.0, 0x40)]))];
            let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 3, &style, None)])]);
            let d = read(&synth::file_of(version, vec![list(b"otlt", vec![chunk(b"outl", outline)]), page]), Limits::default()).unwrap();
            let s = shapes(&d.pages[0].layers[0].objects);
            let o = s[0].outline.as_ref().unwrap();
            assert_eq!(o.dashes.is_empty(), !dashed, "{version} {line_type:#x}: {:?}", o.dashes);
        }
    }
}

/// Bit 4 of the line type puts the outline behind the fill.
#[test]
fn outlines_flagged_behind_the_fill_say_so() {
    let black = synth::color(2, [0, 0, 0, 100]);
    for version in [700, 1300, 1600] {
        for (line_type, behind) in [(2, false), (0x22, false), (0x12, true), (0x14, true)] {
            let v = V(version);
            let outline = v.outline(9, synth::Pen { line_type, width: 4.0, cap: 0, join: 0 }, &black, &[]);
            let style = vec![(10, 9u32.to_le_bytes().to_vec()), (30, v.rect(10.0, 10.0, 0.0))];
            let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &style, None)])]);
            let d = read(&synth::file_of(version, vec![list(b"otlt", vec![chunk(b"outl", outline)]), page]), Limits::default()).unwrap();
            let s = shapes(&d.pages[0].layers[0].objects);
            assert_eq!(s[0].outline.as_ref().map(|o| o.behind_fill), Some(behind), "{version} {line_type:#x}");
        }
    }
}

/// Bit 5 of the line type scales the outline with the shape.
#[test]
fn outlines_flagged_to_scale_with_the_shape_say_so() {
    let black = synth::color(2, [0, 0, 0, 100]);
    for version in [700, 1300, 1600] {
        for (line_type, scale_with) in [(2, false), (0x22, true), (0x12, false), (0x26, true)] {
            let v = V(version);
            let outline = v.outline(9, synth::Pen { line_type, width: 4.0, cap: 0, join: 0 }, &black, &[]);
            let style = vec![(10, 9u32.to_le_bytes().to_vec()), (30, v.rect(10.0, 10.0, 0.0))];
            let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &style, None)])]);
            let d = read(&synth::file_of(version, vec![list(b"otlt", vec![chunk(b"outl", outline)]), page]), Limits::default()).unwrap();
            let s = shapes(&d.pages[0].layers[0].objects);
            assert_eq!(s[0].outline.as_ref().map(|o| o.scale_with), Some(scale_with), "{version} {line_type:#x}");
        }
    }
}

#[test]
fn library_colors_from_version_13_keep_their_names_and_tints() {
    // A model 21 color (blue, green, red, tint) shown at 50 %, and its library name.
    let shown = synth::color(21, [255, 191, 128, 50]);
    let mut props = vec![1];
    props.extend(12u32.to_le_bytes());
    props.extend(shown);
    props.push(8);
    let name: Vec<u8> = "Sky Blue".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    props.extend((16 + 4 + name.len() as u32).to_le_bytes());
    props.extend([0; 16]);
    props.extend(8u32.to_le_bytes());
    props.extend(name);
    props.extend([0, 0, 0, 0, 0]);
    let mut solid = 1300u32.to_le_bytes().to_vec();
    solid.extend((props.len() as u32).to_le_bytes());
    solid.extend(props);
    let mut body = 1u16.to_le_bytes().to_vec();
    body.extend(solid);
    let mut fill = 4u32.to_le_bytes().to_vec();
    // A record of a later layout first, which the reader steps over.
    fill.extend(2400u32.to_le_bytes());
    fill.extend(16u32.to_le_bytes());
    fill.extend([0; 16]);
    fill.extend(1300u32.to_le_bytes());
    fill.extend((body.len() as u32).to_le_bytes());
    fill.extend(body);
    // The same records in a plain file of version 13 and the packages of later versions.
    for version in [1300, 1400, 1700, 2700] {
        let v = V(version);
        let style = vec![(20, 4u32.to_le_bytes().to_vec()), (30, v.rect(1.0, 1.0, 0.0))];
        let body = [
            chunk(b"vrsn", version.to_le_bytes().to_vec()),
            list(b"filt", vec![chunk(b"fild", fill.clone())]),
            synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &style, None)])]),
        ];
        let file = match version {
            ..1400 => synth::riff(version, &body),
            1400..1600 => synth::package14(synth::riff(version, &body), true),
            _ => synth::package16(version, &body, true, &[]),
        };
        let d = read(&file, Limits::default()).unwrap();
        let s = shapes(&d.pages[0].layers[0].objects);
        let Some(Color::Spot { name, tint, look: Some(look) }) = &s[0].fill else { panic!("{version}: {:?}", s[0].fill) };
        assert_eq!((name.as_str(), *tint), ("Sky Blue", 0.5));
        // 128/255 shown at half strength comes from about 1/255 at full strength.
        assert!(look[0] < 0.01 && (look[2] - 1.0).abs() < 1e-6, "{version}: {look:?}");
    }
}

/// One rectangle of version `v` that uses fill `fill` and outline `outline` (0: none).
fn styled_rect(v: V, fill: u32, outline: u32) -> synth::Node {
    let mut args = vec![(30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())];
    if fill != 0 {
        args.push((20, fill.to_le_bytes().to_vec()));
    }
    if outline != 0 {
        args.push((10, outline.to_le_bytes().to_vec()));
    }
    synth::object(v, 1, &args, None)
}

/// A drawing of version `v` with `styles` in its `doc` list and `objects` on one page.
fn styled_drawing(v: V, styles: Vec<synth::Node>, objects: Vec<synth::Node>) -> Vec<u8> {
    let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", objects)]);
    synth::riff(v.0, &[chunk(b"vrsn", v.0.to_le_bytes().to_vec()), list(b"doc ", styles), page])
}

/// An outline's dashes count toward the read's dashes for every shape that uses it: one outline
/// of 65,535 dashes on 260 shapes would copy 17 million dashes, past the default limit.
#[test]
fn a_shared_dashed_outline_counts_its_dashes_for_every_shape() {
    let v = V(1300);
    let pen = synth::Pen { line_type: 4, width: 1.0, cap: 0, join: 0 };
    let outl = |dashes: &[u16]| list(b"otlt", vec![chunk(b"outl", v.outline(9, pen, &synth::color(2, [0, 0, 0, 100]), dashes))]);
    let file = styled_drawing(v, vec![outl(&[2, 1].repeat(10))], (0..20).map(|_| styled_rect(v, 0, 9)).collect());
    let d = read(&file, Limits { dashes: 100, ..Limits::default() }).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert_eq!(s.len(), 20);
    let dashed = s.iter().filter(|s| s.outline.as_ref().is_some_and(|o| o.dashes.len() == 20)).count();
    assert_eq!(dashed, 5, "100 dashes hold five outlines of 20");
    assert!(s.iter().all(|s| s.outline.as_ref().is_some_and(|o| o.dashes.is_empty() || o.dashes.len() == 20)));
    assert!(d.warnings.contains(&Warning::TooDeep), "{:?}", d.warnings);
    // An outline with more dashes than the limit is solid, not cut short.
    let file = styled_drawing(v, vec![outl(&[2, 1])], vec![styled_rect(v, 0, 9)]);
    let d = read(&file, Limits { dashes: 1, ..Limits::default() }).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert!(s[0].outline.as_ref().is_some_and(|o| o.dashes.is_empty()), "{:?}", s[0].outline);
    assert_eq!(d.warnings, vec![Warning::TooDeep]);
    // At the default limit.
    let file = styled_drawing(v, vec![outl(&[1; 65_535])], (0..260).map(|_| styled_rect(v, 0, 9)).collect());
    assert!(file.len() < 256 << 10);
    let d = read(&file, Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    let total: usize = s.iter().filter_map(|s| s.outline.as_ref()).map(|o| o.dashes.len()).sum();
    assert!(total <= Limits::default().dashes, "{total}");
    assert!(d.warnings.contains(&Warning::TooDeep), "{:?}", d.warnings);
}

/// A library color's name is copied for every shape that uses it: a long one keeps its first 256
/// units and a hash of the whole name.
#[test]
fn a_library_color_keeps_the_first_256_units_of_its_name() {
    let v = V(1300);
    let mut props = vec![1];
    props.extend(12u32.to_le_bytes());
    props.extend(synth::color(21, [255, 191, 128, 100]));
    props.push(8);
    let name: Vec<u8> = "N".repeat(10_000).encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    props.extend((16 + 4 + name.len() as u32).to_le_bytes());
    props.extend([0; 16]);
    props.extend(10_000u32.to_le_bytes());
    props.extend(name);
    props.extend([0, 0, 0, 0, 0]);
    let mut body = 1u16.to_le_bytes().to_vec();
    body.extend(1300u32.to_le_bytes());
    body.extend((props.len() as u32).to_le_bytes());
    body.extend(props);
    let mut fill = 4u32.to_le_bytes().to_vec();
    fill.extend(1300u32.to_le_bytes());
    fill.extend((body.len() as u32).to_le_bytes());
    fill.extend(body);
    let d = read(&styled_drawing(v, vec![list(b"filt", vec![chunk(b"fild", fill)])], vec![styled_rect(v, 4, 0)]), Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    let Some(Color::Spot { name, .. }) = &s[0].fill else { panic!("{:?}", s[0].fill) };
    assert_eq!(name.chars().count(), 256 + 17, "{name}");
    assert!(name.starts_with(&"N".repeat(256)), "{name}");
    assert!(d.warnings.contains(&Warning::TooDeep), "{:?}", d.warnings);
}

/// A transform or a uniform fill's color that is there but can't be read is damage: the shape
/// stays, without its transform or its fill, and the warnings say so.
#[test]
fn unreadable_transforms_and_fill_colors_are_damage() {
    let v = V(1100);
    let rect = [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat();
    let obj = list(b"obj ", vec![chunk(b"loda", v.loda(1, &[(30, rect)])), list(b"trfl", vec![chunk(b"trfd", vec![1, 2])])]);
    let d = read(&styled_drawing(v, vec![], vec![obj]), Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert_eq!((s.len(), s[0].matrix), (1, craft_cdr::model::Matrix::IDENTITY));
    assert!(d.warnings.contains(&Warning::Damaged), "trfd: {:?}", d.warnings);
    // A uniform fill cut off in its color record, before version 13 and from it.
    for v in [V(1100), V(1300)] {
        let mut fill = v.fill(4, &synth::color(2, [0, 100, 0, 0]));
        fill.truncate(fill.len() - 10);
        let d = read(&styled_drawing(v, vec![list(b"filt", vec![chunk(b"fild", fill)])], vec![styled_rect(v, 4, 0)]), Limits::default()).unwrap();
        let s = shapes(&d.pages[0].layers[0].objects);
        assert_eq!(s[0].fill, None, "{}", v.0);
        assert!(d.warnings.contains(&Warning::Damaged), "fill {}: {:?}", v.0, d.warnings);
    }
}

/// The inflate limit may be as large as `usize` holds.
#[test]
fn the_largest_inflate_limit_reads_compressed_lists() {
    let d = read(&synth::sample(1100, true), Limits { inflated: usize::MAX, ..Limits::default() }).unwrap();
    check_shapes(&d, 1100);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn never_panics_on_garbage(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let mut b = b"RIFF".to_vec();
        b.extend(bytes);
        let _ = read(&b, Limits::default());
        let _ = preview(&b, Limits::default());
    }

    #[test]
    fn never_panics_on_mutated_samples(version in prop::sample::select(vec![300u16, 400, 500, 700, 900, 1100, 1300]), compress: bool, edits in prop::collection::vec((any::<usize>(), any::<u8>()), 1..24)) {
        let mut b = synth::sample(version, compress && version >= 700);
        let n = b.len();
        for (at, v) in edits {
            b[at % n] = v;
        }
        let _ = read(&b, Limits::default());
        let _ = preview(&b, Limits::default());
    }

    #[test]
    fn never_panics_on_garbage_after_a_wl_header(marker in prop::sample::select(vec![b'e', b'l', b'm']), bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        // Offsets that fit the file, so the garbage is walked as records.
        let mut b = vec![b'W', b'L', marker, 0];
        b.extend(28u32.to_le_bytes());
        b.extend(28u32.to_le_bytes());
        b.extend((28 + bytes.len() as u32).to_le_bytes());
        b.extend([0; 12]);
        b.extend(bytes);
        prop_assert!(craft_cdr::is_cdr(&b));
        let _ = read(&b, Limits::default());
    }

    #[test]
    fn never_panics_on_mutated_wl_files(marker in prop::sample::select(vec![b'e', b'l', b'm']), cut in any::<usize>(), edits in prop::collection::vec((any::<usize>(), any::<u8>()), 1..24)) {
        let mut b = synth::wl_sample(marker);
        let n = b.len();
        for (at, v) in edits {
            b[at % n] = v;
        }
        let _ = read(&b, Limits::default());
        let _ = read(&b[..cut % (n + 1)], Limits::default());
    }

    #[test]
    fn never_panics_on_mutated_packages(version in prop::sample::select(vec![1400u16, 1500, 1700, 2700]), edits in prop::collection::vec((any::<usize>(), any::<usize>(), any::<u8>()), 1..24)) {
        // The members are edited before they are zipped, so that the edits reach the chunks and
        // redirect records rather than stop at the CRC-32.
        let mut body = vec![chunk(b"vrsn", version.to_le_bytes().to_vec())];
        body.extend(synth::sample_content(version));
        let mut members = if version < 1600 {
            vec![synth::Member { name: "content/riffData.cdr".into(), data: synth::sample(version, false), deflate: false }]
        } else {
            synth::package16_members(version, &body, false, &[])
        };
        for (m, at, v) in edits {
            let k = m % members.len();
            let data = &mut members[k].data;
            if !data.is_empty() {
                let n = data.len();
                data[at % n] = v;
            }
        }
        let zip = synth::zip(&members, b"");
        let _ = read(&zip, Limits::default());
        // The package has no picture member, and `preview` does not read the damaged drawing.
        prop_assert_eq!(preview(&zip, Limits::default()), Ok(None));
    }

    #[test]
    fn preview_never_panics_on_mutated_pictures(version in prop::sample::select(vec![700u16, 1100, 1300, 1500, 1700]), deflate: bool, zipped: bool, edits in prop::collection::vec((any::<usize>(), any::<u8>()), 1..24)) {
        let bitmap = disp_body(5, 3, 8);
        let mut b = if version < 1400 {
            // A compressed list before the `DISP` chunk, which the walk passes over.
            let mut content = vec![chunk(b"vrsn", version.to_le_bytes().to_vec())];
            content.push(synth::cmpr(&synth::sample_content(version)));
            content.push(chunk(b"DISP", bitmap));
            synth::riff(version, &content)
        } else {
            let (name, picture) = if version < 1700 {
                ("metadata/thumbnails/thumbnail.bmp", [b"BM".as_slice(), &bitmap].concat())
            } else {
                ("previews/thumbnail.png", [b"\x89PNG\r\n\x1a\n".as_slice(), &bitmap].concat())
            };
            let mut picture = synth::Member { name: name.into(), data: picture, deflate };
            // Edits to the member before it is zipped reach past its CRC-32.
            if !zipped {
                let n = picture.data.len();
                for &(at, v) in &edits {
                    picture.data[at % n] = v;
                }
            }
            if version < 1600 {
                synth::zip(&[synth::Member { name: "content/riffData.cdr".into(), data: synth::sample(version, false), deflate }, picture], b"")
            } else {
                let mut body = vec![chunk(b"vrsn", version.to_le_bytes().to_vec())];
                body.extend(synth::sample_content(version));
                synth::package16(version, &body, deflate, &[picture])
            }
        };
        if zipped || version < 1400 {
            let n = b.len();
            for (at, v) in edits {
                b[at % n] = v;
            }
        }
        let _ = preview(&b, Limits::default());
    }
}

/// A `DISP` body: 4 bytes, a 40-byte bitmap info header for `bits` per pixel, a full color table
/// and the rows.
fn disp_body(width: u32, rows: u32, bits: u16) -> Vec<u8> {
    let mut b = 8u32.to_le_bytes().to_vec();
    b.extend(40u32.to_le_bytes());
    b.extend(width.to_le_bytes());
    b.extend(rows.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(bits.to_le_bytes());
    b.extend([0; 24]);
    b.extend((0..(1u32 << bits) * 4).map(|i| i as u8));
    let stride = (width * u32::from(bits)).div_ceil(32) * 4;
    b.extend((0..stride * rows).map(|i| (i * 3) as u8));
    b
}

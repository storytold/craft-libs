//! Writing drawings and reading them back.

use std::io::Read as _;

use craft_cdr::WriteError as Error;
use craft_cdr::model::{
    Blend, Cap, Color, Drawing, Flags, Geometry, Guideline, Join, Layer, LayerKind, Matrix, Object, Outline, Page, Path, Pt, Segment, Shape, SubPath,
};
use craft_cdr::{Limits, WRITTEN_VERSION, is_cdr, read, write};
use proptest::prelude::*;

/// `n` coordinate units (1/254000 inch) in points, which the file stores exactly.
fn u(n: i32) -> f64 {
    f64::from(n) * (72.0 / 254_000.0)
}

fn pt(x: i32, y: i32) -> Pt {
    Pt { x: u(x), y: u(y) }
}

/// A byte value as a color component, which the file stores exactly.
fn c(v: u8) -> f32 {
    f32::from(v) / 255.0
}

fn moved(x: f64, y: f64) -> Matrix {
    Matrix([1.0, 0.0, 0.0, 1.0, x, y])
}

fn layer(name: &str, objects: Vec<Object>) -> Layer {
    Layer { name: name.into(), kind: LayerKind::Drawing, flags: Flags::default(), objects, guides: Vec::new() }
}

fn rect(w: i32, h: i32, fill: Option<Color>) -> Object {
    Object::Shape(Box::new(Shape::new(Geometry::Rect { w: u(w), h: u(h), radii: [0.0; 4] }, moved(10.0, -20.0), fill, None)))
}

/// A drawing of one page and one layer holding `objects`.
fn one_layer(objects: Vec<Object>) -> Drawing {
    Drawing {
        version: WRITTEN_VERSION,
        page_size: None,
        pages: vec![Page { size: None, layers: vec![layer("A", objects)] }],
        master: Vec::new(),
        guides: Vec::new(),
        warnings: Vec::new(),
    }
}

/// A drawing with every kind of thing the writer writes.
fn everything() -> Drawing {
    let black = Outline {
        width: u(5080),
        color: Color::Gray(1.0),
        cap: Cap::Butt,
        join: Join::Miter,
        dashes: Vec::new(),
        behind_fill: false,
        scale_with: false,
    };
    let dashed = Outline {
        width: u(2540),
        color: Color::Cmyk([c(0), c(128), c(255), c(13)]),
        cap: Cap::Round,
        join: Join::Bevel,
        dashes: vec![3.0, 1.0],
        behind_fill: true,
        scale_with: true,
    };
    let curve = Path {
        subpaths: vec![
            SubPath {
                start: pt(0, 0),
                segments: vec![Segment::Line(pt(254_000, 0)), Segment::Cubic(pt(254_000, 50_000), pt(100_000, 254_000), pt(0, 254_000))],
                closed: true,
            },
            SubPath { start: pt(-1000, -1000), segments: vec![Segment::Line(pt(-5000, -9000))], closed: false },
        ],
    };
    let mut path =
        Shape::new(Geometry::Path(curve), Matrix([0.5, 0.25, -0.25, 2.0, 100.0, -40.0]), Some(Color::Rgb([c(255), c(64), c(0)])), Some(dashed));
    path.nonzero = true;
    path.opacity = 0.75;
    path.blend = Blend::Multiply;
    let mut rounded = Shape::new(
        Geometry::Rect { w: u(200_000), h: u(-100_000), radii: [u(10_000), u(0), u(25_000), u(50_000)] },
        moved(0.0, 0.0),
        Some(Color::Registration),
        Some(black.clone()),
    );
    rounded.flags = Flags { hidden: true, locked: true };
    let spot = Color::Spot { name: "Sky Blue".into(), tint: 1.0, look: Some([0.0, c(255), 0.0]) };
    let ellipse = Shape::new(
        Geometry::Ellipse { w: u(80_000), h: u(40_000) },
        moved(-30.0, 30.0),
        Some(spot.clone()),
        Some(Outline { color: spot, ..black.clone() }),
    );
    let container = Shape::new(Geometry::Rect { w: u(100_000), h: u(100_000), radii: [0.0; 4] }, moved(0.0, 0.0), None, Some(black.clone()));
    let inner = Object::Clip { container: Box::new(container.clone()), objects: vec![rect(10_000, 10_000, Some(Color::Gray(c(128))))] };
    let clip = Object::Clip { container: Box::new(container), objects: vec![rect(20_000, 30_000, None), inner] };
    let group = Object::Group {
        objects: vec![Object::Shape(Box::new(ellipse)), rect(1000, 2000, Some(Color::Gray(0.0)))],
        flags: Flags { hidden: true, locked: false },
    };
    let mut art = layer("Art", vec![Object::Shape(Box::new(path)), Object::Shape(Box::new(rounded)), group, clip]);
    art.guides.push(Guideline { at: pt(0, 254_000), angle: -45.0, hidden: false });
    let mut back = layer("Back ünïcode", vec![rect(5000, 5000, Some(Color::Rgb([c(1), c(2), c(3)])))]);
    back.flags = Flags { hidden: true, locked: true };
    let mut desktop = layer("Desktop", vec![rect(3000, 3000, None)]);
    desktop.kind = LayerKind::Desktop;
    Drawing {
        version: WRITTEN_VERSION,
        page_size: Some((u(2_159_000), u(2_794_000))),
        pages: vec![
            Page { size: Some((u(2_159_000), u(2_794_000))), layers: vec![art, back] },
            Page { size: Some((u(1_000_000), u(500_000))), layers: vec![layer("Second", vec![rect(7000, 8000, None)])] },
        ],
        master: vec![desktop],
        guides: vec![Guideline { at: pt(254_000, 0), angle: 90.0, hidden: false }],
        warnings: Vec::new(),
    }
}

/// Rounds every number of `d` past the precision a round trip keeps.
fn rounded(mut d: Drawing) -> Drawing {
    fn r(v: &mut f64) {
        *v = (*v * 1e4).round() / 1e4 + 0.0;
    }
    fn rc(c: &mut Color) {
        let f = |v: &mut f32| *v = (*v * 1e3).round() / 1e3 + 0.0;
        match c {
            Color::Rgb(v) => v.iter_mut().for_each(f),
            Color::Cmyk(v) => v.iter_mut().for_each(f),
            Color::Gray(v) => f(v),
            Color::Registration => {}
            Color::Spot { tint, look, .. } => {
                f(tint);
                if let Some(l) = look {
                    l.iter_mut().for_each(f);
                }
            }
        }
    }
    fn rp(p: &mut Pt) {
        r(&mut p.x);
        r(&mut p.y);
    }
    fn rs(s: &mut Shape) {
        s.matrix.0.iter_mut().for_each(r);
        match &mut s.geometry {
            Geometry::Rect { w, h, radii } => {
                r(w);
                r(h);
                radii.iter_mut().for_each(r);
            }
            Geometry::Ellipse { w, h } => {
                r(w);
                r(h);
            }
            Geometry::Path(p) => {
                for sp in &mut p.subpaths {
                    rp(&mut sp.start);
                    for seg in &mut sp.segments {
                        match seg {
                            Segment::Line(a) => rp(a),
                            Segment::Cubic(a, b, e) => [a, b, e].into_iter().for_each(rp),
                        }
                    }
                }
            }
        }
        s.fill.iter_mut().for_each(rc);
        if let Some(o) = &mut s.outline {
            r(&mut o.width);
            rc(&mut o.color);
        }
        s.opacity = (s.opacity * 1e3).round() / 1e3;
    }
    let mut stack: Vec<&mut Object> = Vec::new();
    for l in d.pages.iter_mut().flat_map(|p| p.layers.iter_mut()).chain(d.master.iter_mut()) {
        for g in &mut l.guides {
            rp(&mut g.at);
        }
        stack.extend(l.objects.iter_mut());
    }
    while let Some(o) = stack.pop() {
        match o {
            Object::Shape(s) => rs(s),
            Object::Group { objects, .. } => stack.extend(objects.iter_mut()),
            Object::Clip { container, objects } => {
                rs(container);
                stack.extend(objects.iter_mut());
            }
        }
    }
    for g in &mut d.guides {
        rp(&mut g.at);
    }
    for (w, h) in d.pages.iter_mut().filter_map(|p| p.size.as_mut()).chain(d.page_size.as_mut()) {
        r(w);
        r(h);
    }
    d
}

/// `d` as it reads back once written: rounded, and each closed subpath whose last segment ends away
/// from its start given a closing line to its start.
fn as_read(d: Drawing) -> Drawing {
    let mut d = rounded(d);
    let mut stack: Vec<&mut Object> = d.pages.iter_mut().flat_map(|p| p.layers.iter_mut()).flat_map(|l| l.objects.iter_mut()).collect();
    stack.extend(d.master.iter_mut().flat_map(|l| l.objects.iter_mut()));
    while let Some(o) = stack.pop() {
        let shape = match o {
            Object::Shape(s) => s,
            Object::Group { objects, .. } => {
                stack.extend(objects.iter_mut());
                continue;
            }
            Object::Clip { container, objects } => {
                stack.extend(objects.iter_mut());
                container
            }
        };
        let Geometry::Path(p) = &mut shape.geometry else { continue };
        for sp in &mut p.subpaths {
            let end = sp.segments.last().map(|s| match s {
                Segment::Line(e) | Segment::Cubic(_, _, e) => *e,
            });
            if sp.closed && end.is_some_and(|e| e != sp.start) {
                sp.segments.push(Segment::Line(sp.start));
            }
        }
    }
    d
}

/// The RIFF file inside a written package.
fn riff_of(file: &[u8]) -> Vec<u8> {
    // The second member, after the stored `mimetype`.
    let (name, data) = member(file, 0);
    assert_eq!((name, data), (&b"mimetype"[..], &b"application/x-vnd.corel.draw.document+zip"[..]));
    let next = 30 + name.len() + data.len();
    let (name, data) = member(file, next);
    assert_eq!(name, b"content/riffData.cdr");
    let mut out = Vec::new();
    flate2::read::DeflateDecoder::new(data).read_to_end(&mut out).unwrap();
    out
}

/// The name and packed bytes of the member whose local header is at `at`.
fn member(file: &[u8], at: usize) -> (&[u8], &[u8]) {
    let le16 = |i: usize| usize::from(u16::from_le_bytes([file[at + i], file[at + i + 1]]));
    let packed = u32::from_le_bytes(file[at + 18..at + 22].try_into().unwrap()) as usize;
    let name = &file[at + 30..at + 30 + le16(26)];
    let start = at + 30 + le16(26) + le16(28);
    (name, &file[start..start + packed])
}

fn count(hay: &[u8], needle: &[u8]) -> usize {
    hay.windows(needle.len()).filter(|w| *w == needle).count()
}

#[test]
fn a_written_drawing_reads_back_as_written() {
    let d = everything();
    let file = write(&d, Limits::default()).unwrap();
    let back = read(&file, Limits::default()).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_eq!(rounded(back), as_read(d));
}

#[test]
fn a_written_file_is_a_version_15_package_and_the_same_each_time() {
    let file = write(&everything(), Limits::default()).unwrap();
    assert!(is_cdr(&file) && file.starts_with(b"PK\x03\x04"));
    let riff = riff_of(&file);
    assert_eq!(&riff[8..12], b"CDRF");
    assert_eq!(&riff[12..18], b"vrsn\x02\0");
    assert_eq!(read(&file, Limits::default()).unwrap().version, 1500);
    assert_eq!(file, write(&everything(), Limits::default()).unwrap());
    // `mcfg` starts with the tag 1300 and the length of the rest (R72).
    let le32 = |at: usize| u32::from_le_bytes(riff[at..at + 4].try_into().unwrap()) as usize;
    let mcfg = riff.windows(4).position(|w| w == b"mcfg").unwrap();
    assert_eq!((le32(mcfg + 8), le32(mcfg + 12)), (1300, le32(mcfg + 4) - 8));
    // Every child of `LIST filt` is a `LIST filc` that holds one `fild` and nothing else (R72).
    let padded = |size: usize| 8 + size + (size & 1);
    let filt = riff.windows(12).position(|w| &w[..4] == b"LIST" && &w[8..] == b"filt").unwrap();
    let (mut at, end) = (filt + 12, filt + 8 + le32(filt + 4));
    let mut lists = 0;
    while at < end {
        assert_eq!((&riff[at..at + 4], &riff[at + 8..at + 12], &riff[at + 12..at + 16]), (&b"LIST"[..], &b"filc"[..], &b"fild"[..]));
        assert_eq!(at + 12 + padded(le32(at + 16)), at + padded(le32(at + 4)));
        at += padded(le32(at + 4));
        lists += 1;
    }
    assert_eq!((at, lists), (end, count(&riff, b"fild")));
    assert!(lists > 0);
}

#[test]
fn each_container_names_its_own_clipped_group_by_the_stored_id() {
    let riff = riff_of(&write(&everything(), Limits::default()).unwrap());
    let ids: Vec<u32> =
        (0..riff.len() - 12).filter(|&p| &riff[p..p + 4] == b"spnd").map(|p| u32::from_le_bytes(riff[p + 8..p + 12].try_into().unwrap())).collect();
    let mut distinct = ids.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(ids.len() > 1 && distinct.len() == ids.len(), "{ids:?}");
    // The first u32 of each container argument (8005), from every `loda` record: a `loda` chunk's
    // body starts with its own length.
    let le32 = |at: usize| u32::from_le_bytes(riff[at..at + 4].try_into().unwrap()) as usize;
    let mut named: Vec<u32> = (0..riff.len() - 28)
        .filter(|&p| &riff[p..p + 4] == b"loda" && le32(p + 4) == le32(p + 8))
        .flat_map(|p| {
            let loda = p + 8;
            let (n, offsets, types) = (le32(loda + 4), loda + le32(loda + 8), loda + le32(loda + 12));
            (0..n).filter(move |i| le32(types + 4 * (n - 1 - i)) == 8005).map(move |i| le32(loda + le32(offsets + 4 * i)) as u32)
        })
        .collect();
    named.sort_unstable();
    // Each container names its group by the group's `spnd` id unchanged, and no two name one group.
    assert_eq!(named, distinct);
}

#[test]
fn shapes_with_the_same_fill_and_outline_share_one_record_of_each() {
    let fill = || Some(Color::Rgb([c(10), c(20), c(30)]));
    let records = |d: &Drawing| {
        let riff = riff_of(&write(d, Limits::default()).unwrap());
        (count(&riff, b"fild"), count(&riff, b"outl"))
    };
    assert_eq!(records(&one_layer((0..50).map(|i| rect(1000 + i, 1000, fill())).collect())), records(&one_layer(vec![rect(1000, 1000, fill())])));
}

#[test]
fn a_closed_subpath_ends_at_its_start_and_its_other_nodes_can_be_edited() {
    let start = pt(1234, 5678);
    let open_end = SubPath { start, segments: vec![Segment::Line(pt(90_000, 5678)), Segment::Line(pt(1234, 90_000))], closed: true };
    let triangle =
        |sp: SubPath| one_layer(vec![Object::Shape(Box::new(Shape::new(Geometry::Path(Path { subpaths: vec![sp] }), Matrix::IDENTITY, None, None)))]);
    let riff = riff_of(&write(&triangle(open_end.clone()), Limits::default()).unwrap());
    // The closing point at the start, then the type bytes: an editable closed move, two editable
    // line ends and the closing line end, which is not editable.
    let tail: Vec<u8> = [1234i32, 5678].iter().flat_map(|v| v.to_le_bytes()).chain([0x0C, 0x44, 0x44, 0x48]).collect();
    assert_eq!(count(&riff, &tail), 1);
    // A subpath that already ends at its start is written as it is.
    let mut ends_at_start = open_end;
    ends_at_start.segments.push(Segment::Line(start));
    assert_eq!(riff_of(&write(&triangle(ends_at_start), Limits::default()).unwrap()), riff);
}

#[test]
fn every_page_names_the_master_pages_guides_desktop_and_grid_by_their_ids() {
    let mut d = everything();
    // A drawing layer of the master page, which the other pages do not name. The drawing has no
    // grid layer of its own, as a read drawing has none.
    d.master.push(layer("Master art", vec![rect(100, 100, None)]));
    let riff = riff_of(&write(&d, Limits::default()).unwrap());
    let layers = layer_entries(&riff);
    // Each page names the master page's guides, desktop and grid layers (object types 12, 17 and
    // 11) once each, by kind, name and id: the guides and desktop before the page's own layers,
    // the grid after them.
    let named = |page: usize, kinds: &[usize]| {
        let kept = |l: &&Entry| l.0 == page && l.1 == 1 && kinds.contains(&l.2);
        let mut v: Vec<_> = layers.iter().filter(kept).map(|l| (l.2, &l.3, &l.4)).collect();
        v.sort();
        v
    };
    assert_eq!(named(0, &[11, 12, 17]).len(), 3);
    for page in 1..=d.pages.len() {
        assert_eq!(named(page, &[11, 12, 17, 0]), named(0, &[11, 12, 17]));
        let kinds: Vec<usize> = layers.iter().filter(|l| l.0 == page).map(|l| if l.1 == 0 { 0 } else { l.2 }).collect();
        let own = kinds.iter().position(|k| *k == 0).unwrap();
        assert!(kinds[..own].iter().all(|k| [12, 17].contains(k)) && kinds[own..].iter().filter(|k| **k != 0).all(|k| *k == 11), "{kinds:?}");
        assert!(kinds.ends_with(&[11]), "{kinds:?}");
    }
    // The master page's layers and the pages' own layers each have their own 16-byte id.
    let mut own: Vec<&[u8]> = layers.iter().filter(|l| l.0 == 0 || l.1 == 0).map(|l| &l.4[..]).collect();
    assert!(own.iter().all(|id| id.len() == 16));
    let count = own.len();
    own.sort();
    own.dedup();
    assert_eq!(own.len(), count);
    // The guides layer, the first layer written, gets another id in a drawing that differs only in
    // a fill color.
    // A guides layer of the drawing's own master page takes the drawing's guidelines too, and is
    // the only guides layer there; a hidden or locked one keeps them off it.
    let guide = |x: i32| Guideline { at: pt(x, 0), angle: 90.0, hidden: false };
    let with_guides = |flags: Flags, drawing_guides: Vec<Guideline>| {
        let mut guides = layer("Guides", Vec::new());
        guides.kind = LayerKind::Guides;
        guides.flags = flags;
        guides.guides = vec![guide(100)];
        Drawing { master: vec![guides], guides: drawing_guides, ..one_layer(Vec::new()) }
    };
    for (flags, drawing_guides, layers) in [
        (Flags::default(), vec![guide(200)], 1),
        (Flags { hidden: true, locked: false }, Vec::new(), 1),
        (Flags { hidden: true, locked: false }, vec![guide(200)], 2),
        (Flags { hidden: false, locked: true }, vec![guide(200)], 2),
    ] {
        let d = with_guides(flags, drawing_guides.clone());
        let file = write(&d, Limits::default()).unwrap();
        let entries = layer_entries(&riff_of(&file));
        assert_eq!(entries.iter().filter(|l| l.0 == 0 && l.2 == 12).count(), layers, "{flags:?}");
        // Each guideline once, the drawing's in a layer that is shown and unlocked: each guideline
        // object's x (in units), and bytes 0 and 1 of the flags of the layer it is in.
        let riff = riff_of(&file);
        let le32 = |at: usize| u32::from_le_bytes(riff[at..at + 4].try_into().unwrap()) as usize;
        let mut placed: Vec<(i32, u8, u8)> = (0..riff.len() - 28)
            .filter(|&p| &riff[p..p + 4] == b"loda" && le32(p + 8 + 16) == 0x16)
            .map(|p| {
                let arg = p + 8 + le32(p + 8 + le32(p + 8 + 8));
                let layr = riff[..p].windows(8).rposition(|w| w == b"layrflgs").unwrap();
                (i32::from_le_bytes(riff[arg + 16..arg + 20].try_into().unwrap()), riff[layr + 12], riff[layr + 13])
            })
            .collect();
        placed.sort();
        let xs: Vec<i32> = placed.iter().map(|g| g.0).collect();
        assert_eq!(xs, if drawing_guides.is_empty() { vec![100] } else { vec![100, 200] }, "{flags:?}");
        assert!(placed.iter().filter(|g| g.0 == 200).all(|g| g.1 & 0x10 == 0 && g.2 == 0), "{flags:?}");
    }
    let first = |c: Color| layer_entries(&riff_of(&write(&one_layer(vec![rect(100, 100, Some(c))]), Limits::default()).unwrap()))[0].4.clone();
    assert_ne!(first(Color::Gray(0.0)), first(Color::Gray(1.0)));
}

/// A layer list of a written file: its page (0 the master page), byte 2 of its flags, its object
/// type, and its name and id arguments.
type Entry = (usize, u8, usize, Vec<u8>, Vec<u8>);

/// The layer lists of `riff`, each `layr` with its `flgs` chunk, then `LIST lgob` with its `loda`.
fn layer_entries(riff: &[u8]) -> Vec<Entry> {
    let le32 = |at: usize| u32::from_le_bytes(riff[at..at + 4].try_into().unwrap()) as usize;
    let pages: Vec<usize> = riff.windows(12).enumerate().filter(|(_, w)| &w[..4] == b"LIST" && &w[8..] == b"page").map(|(p, _)| p).collect();
    riff.windows(8)
        .enumerate()
        .filter(|(_, w)| w == b"layrflgs")
        .map(|(p, _)| {
            let page = pages.iter().filter(|&&at| at < p).count() - 1;
            let loda = p + 36;
            let (n, offsets, types) = (le32(loda + 4), loda + le32(loda + 8), loda + le32(loda + 12));
            let arg = |t: usize| {
                let i = (0..n).find(|i| le32(types + 4 * (n - 1 - i)) == t).unwrap();
                riff[loda + le32(offsets + 4 * i)..loda + le32(offsets + 4 * i + 4)].to_vec()
            };
            (page, riff[p + 14], le32(loda + 16), arg(1000), arg(40050))
        })
        .collect()
}

#[test]
fn a_guideline_runs_along_the_angle_it_stores() {
    // Starts `by` units inside a corner of the coordinate range.
    let near = |by: f64| Pt { x: u(i32::MAX) - by * u(1), y: u(i32::MAX) - by * u(1) };
    let guide = |at: Pt, angle: f64| Drawing { guides: vec![Guideline { at, angle, hidden: false }], ..one_layer(Vec::new()) };
    // A quarter of a unit past the corner, on the corner itself, and less than half a unit inside
    // it, the start rounds onto the corner, and no other point of the line rounds within the range
    // along it. The last line runs almost level, so every point of it in the range rounds onto the
    // corner too, as does the line that starts on the corner and half a unit past it.
    let on_corner = Pt { x: 608_735.521_984_251_9, y: 608_735.521_984_251_9 };
    let level = [(near(-0.49), -0.9), (Pt { x: 608_735.521_984_252, y: 608_735.522_125_984_2 }, -0.00004)];
    for (at, angle) in [near(-0.25), on_corner, near(0.1), near(0.01), near(0.00025)].map(|at| (at, -45.0)).into_iter().chain(level) {
        assert!(matches!(write(&guide(at, angle), Limits::default()), Err(Error::Unwritable(_))), "{at:?}");
    }
    let inside = [(pt(1000, 2000), 30.0), (pt(1000, 2000), -45.0), (pt(1000, 2000), 1e20)];
    // A unit or more inside the corner, a point a whole unit or more away on each axis fits;
    // at 1.25 units the farthest points in the range round off the angle and a nearer one does not.
    // At -44 degrees the only point along the line turns from it by exactly a degree. At -18
    // degrees the one point along the line lies between whole units on both axes.
    let between = Pt { x: u(i32::MAX) - 2.0 * u(1), y: u(i32::MAX) - 1.48 * u(1) };
    // A start a unit and a half and half a unit inside the corner lies on the edges of rounding.
    let edges = Pt { x: 608_735.521_559_055_1, y: 608_735.521_842_519_8 };
    let near_corner = [(near(1000.0), -45.0), (near(1.25), -45.0), (near(1.0), -45.0), (near(1.0), -44.0), (between, -18.0), (edges, -45.0)];
    for (at, angle) in inside.into_iter().chain(near_corner) {
        let corner = at.x > u(1_000_000);
        let d = Drawing { guides: vec![Guideline { at, angle, hidden: false }], ..one_layer(Vec::new()) };
        let riff = riff_of(&write(&d, Limits::default()).unwrap_or_else(|e| panic!("{at:?} {angle}: {e}")));
        let le32 = |at: usize| u32::from_le_bytes(riff[at..at + 4].try_into().unwrap()) as usize;
        let i32_at = |at: usize| f64::from(i32::from_le_bytes(riff[at..at + 4].try_into().unwrap()));
        // The guideline object's `loda` (object type 0x16) and its first argument: 12 bytes, the
        // angle in millionths of a degree, then the point and a second point on the line.
        let loda = riff.windows(4).enumerate().map(|(p, _)| p).find(|&p| &riff[p..p + 4] == b"loda" && le32(p + 8 + 16) == 0x16).unwrap() + 8;
        let arg = loda + le32(loda + le32(loda + 8));
        let a = (i32_at(arg + 12) / 1e6).to_radians();
        let (dx, dy) = (i32_at(arg + 24) - i32_at(arg + 16), i32_at(arg + 28) - i32_at(arg + 20));
        assert!(dx.hypot(dy) > if corner { 0.0 } else { 1000.0 });
        // Off the stored direction by no more than the coordinates' rounding, and, on either
        // side, by no more than a degree.
        assert!((dx * a.sin() - dy * a.cos()).abs() <= 1.5, "{angle}: {dx} {dy}");
        let turn = (dx * a.sin() - dy * a.cos()).abs().atan2((dx * a.cos() + dy * a.sin()).abs());
        assert!(turn <= 1f64.to_radians() + 1e-12, "{angle}: {dx} {dy}");
    }
}

#[test]
fn an_outline_record_ends_8_bytes_before_its_chunk_and_its_color_list_at_its_empty_property() {
    let spot = Color::Spot { name: "Ink".into(), tint: 1.0, look: Some([0.0; 3]) };
    let o = Outline { width: u(2540), color: spot, cap: Cap::Butt, join: Join::Miter, dashes: Vec::new(), behind_fill: false, scale_with: false };
    let outlined = one_layer(vec![Object::Shape(Box::new(Shape::new(rect_geometry(), Matrix::IDENTITY, None, Some(o))))]);
    let riff = riff_of(&write(&outlined, Limits::default()).unwrap());
    let le32 = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as usize;
    let at = riff.windows(4).position(|w| w == b"outl").unwrap();
    let chunk = &riff[at + 8..at + 8 + le32(&riff, at + 4)];
    // The outline's id, then records (type, length, body) up to the one of type 1.
    let mut p = 4;
    let mut types = Vec::new();
    loop {
        let (kind, len) = (le32(chunk, p), le32(chunk, p + 4));
        types.push(kind);
        if kind == 1 {
            assert_eq!(len, chunk.len() - p - 8 - 8);
            // The body as sample files lay it out (R71): 130 bytes, an f64 1 at bytes 16 and 48,
            // 45 degrees at byte 84 and 60 at byte 88.
            let body = &chunk[p + 8..p + 8 + len];
            let f64_at = |at: usize| f64::from_le_bytes(body[at..at + 8].try_into().unwrap());
            assert_eq!((len, f64_at(16), f64_at(48), le32(body, 84), le32(body, 88)), (130, 1.0, 1.0, 45_000_000, 60));
            break;
        }
        // The color property list (type byte, length, body) ends at its first empty property.
        if kind == 2 {
            let mut q = p + 8;
            while chunk[q] != 0 {
                q += 5 + le32(chunk, q + 1);
            }
            assert_eq!((q + 5, le32(chunk, q + 1)), (p + 8 + len, 0));
        }
        p += 8 + len;
    }
    assert_eq!(types, [2, 1]);
}

#[test]
fn a_transparency_names_a_white_fill_record_of_the_file() {
    let mut s = Shape::new(rect_geometry(), Matrix::IDENTITY, Some(Color::Gray(1.0)), None);
    s.opacity = 0.5;
    let riff = riff_of(&write(&one_layer(vec![Object::Shape(Box::new(s))]), Limits::default()).unwrap());
    let head: Vec<u8> = [1300u32, 117].iter().flat_map(|v| v.to_le_bytes()).collect();
    let at = riff.windows(head.len()).position(|w| w == head).unwrap() + head.len();
    let id = &riff[at + 105..at + 109];
    // The `fild` chunk of that id (chunk id, size, record id) holds the RGB color record of white.
    let fild = riff.windows(12).position(|w| &w[..4] == b"fild" && &w[8..] == id).unwrap();
    let size = u32::from_le_bytes(riff[fild + 4..fild + 8].try_into().unwrap()) as usize;
    let white = [5, 0, 0, 0, 0, 0, 0, 0, 255, 255, 255, 0];
    assert_eq!(count(&riff[fild + 8..fild + 8 + size], &white), 1);
}

#[test]
fn empty_groups_are_left_out_and_a_container_without_objects_is_a_shape() {
    let container = Shape::new(Geometry::Ellipse { w: u(10_000), h: u(10_000) }, Matrix::IDENTITY, None, None);
    let objects = vec![
        Object::Group { objects: Vec::new(), flags: Flags::default() },
        Object::Clip { container: Box::new(container.clone()), objects: Vec::new() },
    ];
    let back = read(&write(&one_layer(objects), Limits::default()).unwrap(), Limits::default()).unwrap();
    assert_eq!(back.pages[0].layers[0].objects, vec![Object::Shape(Box::new(container.clone()))]);
    assert!(back.warnings.is_empty());
    // A container whose objects are groups that draw nothing is written as the plain shape is.
    let hollow = Object::Group { objects: vec![Object::Group { objects: Vec::new(), flags: Flags::default() }], flags: Flags::default() };
    let clips_hollow = one_layer(vec![Object::Clip { container: Box::new(container.clone()), objects: vec![hollow] }]);
    let file = write(&clips_hollow, Limits::default()).unwrap();
    assert_eq!(file, write(&one_layer(vec![Object::Shape(Box::new(container))]), Limits::default()).unwrap());
    // A group that holds only groups that draw nothing is written as no group at all.
    assert_eq!(write(&one_layer(vec![hollow_group()]), Limits::default()).unwrap(), write(&one_layer(Vec::new()), Limits::default()).unwrap());
}

fn hollow_group() -> Object {
    Object::Group { objects: vec![Object::Group { objects: Vec::new(), flags: Flags::default() }], flags: Flags::default() }
}

#[test]
fn a_drawing_without_pages_is_written_with_one_empty_page() {
    let d = Drawing { pages: Vec::new(), ..one_layer(Vec::new()) };
    let back = read(&write(&d, Limits::default()).unwrap(), Limits::default()).unwrap();
    assert_eq!(back.pages.len(), 1);
    assert!(back.pages[0].layers.iter().all(|l| l.objects.is_empty()));
    // Without a page size of its own, the drawing reads back without one.
    assert_eq!(back.page_size, None);
}

/// At the smallest value of a limit at which the reader reads `d`'s file whole, the writer writes
/// it; one below, the writer refuses it.
fn writes_at_the_readers_limit(d: &Drawing, set: fn(&mut Limits, usize)) -> usize {
    let limits = |n| {
        let mut l = Limits::default();
        set(&mut l, n);
        l
    };
    let file = write(d, Limits::default()).unwrap();
    let whole = |n| read(&file, limits(n)).is_ok_and(|b| b.warnings.is_empty());
    // The smallest value that reads whole: a read that fits a limit fits every larger one.
    let (mut n, mut above) = (0, 100_000);
    while n < above {
        let mid = (n + above) / 2;
        if whole(mid) { above = mid } else { n = mid + 1 }
    }
    assert!(whole(n), "{n}");
    assert_eq!(write(d, limits(n)).as_ref(), Ok(&file), "{n}");
    assert!(n > 0 && matches!(write(d, limits(n - 1)), Err(Error::Limit(_))), "{n}");
    n
}

#[test]
fn every_limit_a_read_applies_is_applied_when_writing() {
    let d = everything();
    let set: [fn(&mut Limits, usize); 10] = [
        |l, n| l.input = n,
        |l, n| l.inflated = n,
        |l, n| l.arguments = n,
        |l, n| l.clipped_chunks = n,
        |l, n| l.chunks = n,
        |l, n| l.member = n,
        |l, n| l.dashes = n,
        |l, n| l.objects = n,
        |l, n| l.coordinates = n,
        |l, n| l.members = n,
    ];
    for set in set {
        writes_at_the_readers_limit(&d, set);
    }
    // Without clipped groups the outline table ends the file: an empty drawing, and one whose
    // last record is an outline.
    let o = Outline {
        width: u(2540),
        color: Color::Gray(0.0),
        cap: Cap::Butt,
        join: Join::Miter,
        dashes: Vec::new(),
        behind_fill: false,
        scale_with: false,
    };
    let outlined = one_layer(vec![Object::Shape(Box::new(Shape::new(rect_geometry(), Matrix::IDENTITY, None, Some(o))))]);
    let bytes: [fn(&mut Limits, usize); 3] = [|l, n| l.input = n, |l, n| l.inflated = n, |l, n| l.member = n];
    for d in [one_layer(Vec::new()), outlined] {
        for set in bytes {
            writes_at_the_readers_limit(&d, set);
        }
    }
    // A group that holds only an empty group is left out, and takes nothing of the object count.
    let hollow =
        one_layer(vec![Object::Group { objects: vec![Object::Group { objects: Vec::new(), flags: Flags::default() }], flags: Flags::default() }]);
    assert!(write(&hollow, Limits { objects: 0, ..Limits::default() }).is_ok());
}

#[test]
fn a_path_past_the_coordinate_count_stops_before_its_points_are_written() {
    // The closing point's coordinates are past the file's range; the count is refused first.
    let mut segments: Vec<Segment> = (1..10).map(|i| Segment::Line(pt(i * 100, 0))).collect();
    segments.push(Segment::Line(Pt { x: 1e30, y: 0.0 }));
    let path = Path { subpaths: vec![SubPath { start: pt(0, 0), segments, closed: true }] };
    let d = one_layer(vec![Object::Shape(Box::new(Shape::new(Geometry::Path(path), Matrix::IDENTITY, None, None)))]);
    assert!(matches!(write(&d, Limits::default()), Err(Error::Unwritable(_))));
    assert!(matches!(write(&d, Limits { coordinates: 5, ..Limits::default() }), Err(Error::Limit(_))));
    // A shape takes at least three arguments, counted before its points.
    assert_eq!(write(&d, Limits { arguments: 2, ..Limits::default() }), Err(Error::Limit("argument count")));
    // So is the file's size, from the segment count: a long path refused at half its bytes.
    let long = |end: Pt| {
        let mut segments: Vec<Segment> = (1..1000).map(|i| Segment::Line(pt(i * 100, i % 2 * 100))).collect();
        segments.push(Segment::Line(end));
        let path = Path { subpaths: vec![SubPath { start: pt(0, 0), segments, closed: true }] };
        one_layer(vec![Object::Shape(Box::new(Shape::new(Geometry::Path(path), Matrix::IDENTITY, None, None)))])
    };
    let member = riff_of(&write(&long(pt(0, 100)), Limits::default()).unwrap()).len() - 1000 * 9 / 2;
    assert!(matches!(write(&long(Pt { x: 1e30, y: 0.0 }), Limits::default()), Err(Error::Unwritable(_))));
    assert!(matches!(write(&long(Pt { x: 1e30, y: 0.0 }), Limits { member, ..Limits::default() }), Err(Error::Limit(_))));
    // So are the shape's nested lists: one level short of what a rectangle in its place takes.
    let fits = |n| write(&one_layer(vec![rect(100, 100, None)]), Limits { depth: n, ..Limits::default() }).is_ok();
    let depth = (1..64).find(|&n| fits(n)).unwrap();
    assert_eq!(write(&d, Limits { depth: depth - 1, ..Limits::default() }), Err(Error::Limit("nesting depth")));
    // In a clipped group, every other chunk of the file counts too: one chunk short of what the
    // drawing takes with a path in range, the clipped path is refused before its points.
    let clipped = |path: Path| {
        let container = Shape::new(rect_geometry(), Matrix::IDENTITY, None, None);
        // A fill of its own, whose record counts as well.
        let shape = Shape::new(Geometry::Path(path), Matrix::IDENTITY, Some(Color::Gray(0.5)), None);
        one_layer(vec![Object::Clip { container: Box::new(container), objects: vec![Object::Shape(Box::new(shape))] }])
    };
    let Some(Object::Shape(s)) = d.pages[0].layers[0].objects.first() else { panic!("no path") };
    let Geometry::Path(past) = s.geometry.clone() else { panic!("no path") };
    let mut within = past.clone();
    within.subpaths[0].segments.pop();
    let chunks = (1..1000).find(|&n| write(&clipped(within.clone()), Limits { chunks: n, ..Limits::default() }).is_ok()).unwrap();
    assert!(matches!(write(&clipped(past.clone()), Limits::default()), Err(Error::Unwritable(_))));
    assert_eq!(write(&clipped(past), Limits { chunks: chunks - 1, ..Limits::default() }), Err(Error::Limit("chunk count")));
}

#[test]
fn a_rectangle_repeats_the_scale_of_its_transform() {
    // Scaled by 2 and 3, then turned a quarter turn: the scale fields hold 2 and 3 (R29).
    for m in [Matrix([2.0, 0.0, 0.0, 3.0, 5.0, 7.0]), Matrix([0.0, 2.0, -3.0, 0.0, 5.0, 7.0])] {
        let shape = Shape::new(Geometry::Rect { w: u(1000), h: u(2000), radii: [0.0; 4] }, m, None, None);
        let riff = riff_of(&write(&one_layer(vec![Object::Shape(Box::new(shape))]), Limits::default()).unwrap());
        // The record's width and height in coordinate units, then the two scale fields.
        let units = |v: f64| (v * (254_000.0 / 72.0)).to_le_bytes();
        let fields = [units(u(1000)), units(u(2000)), 2.0f64.to_le_bytes(), 3.0f64.to_le_bytes()].concat();
        assert_eq!(count(&riff, &fields), 1, "{m:?}");
    }
}

#[test]
fn the_pages_clipped_groups_and_records_share_the_byte_limit() {
    // A long path on the page and another in a clipped group each fit the limit; together they do
    // not, and writing stops at the second before it reaches the unwritable shape after it.
    let long = || {
        let segments = (1..4000).map(|i| Segment::Line(pt(i * 100, (i % 7) * 100))).collect();
        let path = Path { subpaths: vec![SubPath { start: pt(0, 0), segments, closed: false }] };
        Object::Shape(Box::new(Shape::new(Geometry::Path(path), Matrix::IDENTITY, None, None)))
    };
    let container = || Box::new(Shape::new(rect_geometry(), Matrix::IDENTITY, None, None));
    let fits = one_layer(vec![long(), Object::Clip { container: container(), objects: vec![long()] }]);
    let past = Shape::new(Geometry::Rect { w: 1e30, h: 1.0, radii: [0.0; 4] }, Matrix::IDENTITY, None, None);
    let stops = one_layer(vec![long(), Object::Clip { container: container(), objects: vec![long(), Object::Shape(Box::new(past))] }]);
    assert!(matches!(write(&stops, Limits::default()), Err(Error::Unwritable(_))));
    let member = riff_of(&write(&fits, Limits::default()).unwrap()).len() - 4000 * 9 / 2;
    assert!(matches!(write(&stops, Limits { member, ..Limits::default() }), Err(Error::Limit(_))));
    // Shapes whose fills all differ: the page fits the limit, and the fill table takes it past
    // before the writer reaches the unwritable shape at the end of the page.
    let shapes = || (0..=255u8).map(|v| rect(1000, 1000, Some(Color::Gray(c(v))))).collect::<Vec<_>>();
    let riff = riff_of(&write(&one_layer(shapes()), Limits::default()).unwrap());
    let filt = riff.windows(12).position(|w| &w[..4] == b"LIST" && &w[8..] == b"filt").unwrap();
    let table = u32::from_le_bytes(riff[filt + 4..filt + 8].try_into().unwrap()) as usize;
    let member = riff.len() - table / 2;
    let past = Shape::new(Geometry::Rect { w: 1e30, h: 1.0, radii: [0.0; 4] }, Matrix::IDENTITY, None, None);
    let stops = one_layer(shapes().into_iter().chain([Object::Shape(Box::new(past))]).collect());
    assert!(matches!(write(&stops, Limits::default()), Err(Error::Unwritable(_))));
    assert!(matches!(write(&stops, Limits { member, ..Limits::default() }), Err(Error::Limit(_))));
}

#[test]
fn clips_nest_as_deep_as_a_read_takes() {
    let nested = |depth: usize| {
        let mut o = rect(100, 100, None);
        for _ in 0..depth {
            let container = Shape::new(Geometry::Ellipse { w: u(1000), h: u(1000) }, Matrix::IDENTITY, None, None);
            o = Object::Clip { container: Box::new(container), objects: vec![o] };
        }
        one_layer(vec![o])
    };
    let back = read(&write(&nested(64), Limits::default()).unwrap(), Limits::default()).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_eq!(rounded(back), as_read(nested(64)));
    assert_eq!(write(&nested(65), Limits::default()), Err(Error::Limit("clip nesting")));
}

#[test]
fn ten_dashes_fit_before_the_arrowheads_and_eleven_do_not() {
    let dashed = |n: usize| {
        let o = Outline {
            width: u(2540),
            color: Color::Gray(1.0),
            cap: Cap::Butt,
            join: Join::Miter,
            dashes: (1..=n).map(|d| d as f64).collect(),
            behind_fill: false,
            scale_with: false,
        };
        one_layer(vec![Object::Shape(Box::new(Shape::new(Geometry::Ellipse { w: u(1000), h: u(1000) }, Matrix::IDENTITY, None, Some(o))))])
    };
    let back = read(&write(&dashed(10), Limits::default()).unwrap(), Limits::default()).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_eq!(rounded(back), as_read(dashed(10)));
    assert!(matches!(write(&dashed(11), Limits::default()), Err(Error::Unwritable(_))));
}

#[test]
fn names_and_values_the_reader_would_change_are_refused() {
    let named = |name: String| Drawing { pages: vec![Page { size: None, layers: vec![layer(&name, Vec::new())] }], ..one_layer(Vec::new()) };
    let back = read(&write(&named("é".repeat(256)), Limits::default()).unwrap(), Limits::default()).unwrap();
    assert_eq!(back.pages[0].layers[0].name, "é".repeat(256));
    assert!(back.warnings.is_empty());
    for name in ["é".repeat(257), "A\0B".to_string()] {
        assert!(matches!(write(&named(name), Limits::default()), Err(Error::Unwritable(_))));
    }
    let filled = |c: Color| one_layer(vec![rect(100, 100, Some(c))]);
    // At a 10% tint, the stored byte nearest a black look reads back a step off, and the next one
    // reads back black.
    let black_tint = Color::Spot { name: "Ink".into(), tint: 0.1, look: Some([0.0; 3]) };
    let back = read(&write(&filled(black_tint.clone()), Limits::default()).unwrap(), Limits::default()).unwrap();
    assert_eq!(rounded(back), rounded(filled(black_tint)));
    for c in [
        Color::Rgb([f32::NAN, 0.0, 0.0]),
        Color::Gray(1.5),
        Color::Spot { name: String::new(), tint: 1.0, look: None },
        Color::Spot { name: "Red".into(), tint: 0.0, look: Some([1.0, 0.0, 0.0]) },
        Color::Spot { name: "Gray".into(), tint: 0.01, look: Some([0.5, 0.5, 0.5]) },
    ] {
        assert!(matches!(write(&filled(c), Limits::default()), Err(Error::Unwritable(_))));
    }
    // A corner past half the shorter side, a page of no width, a dash past what a u16 holds.
    let corner = Shape::new(Geometry::Rect { w: 10.0, h: 10.0, radii: [9.0, 0.0, 0.0, 0.0] }, Matrix::IDENTITY, None, None);
    assert!(matches!(write(&one_layer(vec![Object::Shape(Box::new(corner))]), Limits::default()), Err(Error::Unwritable(_))));
    let flat = Drawing { page_size: Some((0.0, 10.0)), ..one_layer(Vec::new()) };
    assert!(matches!(write(&flat, Limits::default()), Err(Error::Unwritable(_))));
    let long = Outline {
        width: 1.0,
        color: Color::Gray(1.0),
        cap: Cap::Butt,
        join: Join::Miter,
        dashes: vec![100_000.0],
        behind_fill: false,
        scale_with: false,
    };
    let fine = Outline { dashes: vec![0.4, 0.4], ..long.clone() };
    for o in [long, fine] {
        let dashed = Shape::new(rect_geometry(), Matrix::IDENTITY, None, Some(o));
        assert!(matches!(write(&one_layer(vec![Object::Shape(Box::new(dashed))]), Limits::default()), Err(Error::Unwritable(_))));
    }
    let mut see_through = Shape::new(rect_geometry(), Matrix::IDENTITY, None, None);
    see_through.opacity = f32::NAN;
    assert!(matches!(write(&one_layer(vec![Object::Shape(Box::new(see_through))]), Limits::default()), Err(Error::Unwritable(_))));
    let mut grid = layer("Grid", vec![rect(100, 100, None)]);
    grid.kind = LayerKind::Grid;
    assert!(matches!(write(&Drawing { master: vec![grid], ..one_layer(Vec::new()) }, Limits::default()), Err(Error::Unwritable(_))));
}

fn rect_geometry() -> Geometry {
    Geometry::Rect { w: u(1000), h: u(1000), radii: [0.0; 4] }
}

#[test]
fn a_grid_layer_keeps_its_lock_state() {
    for locked in [false, true] {
        let mut grid = layer("Grid", vec![rect(100, 100, None)]);
        grid.kind = LayerKind::Grid;
        grid.flags.locked = locked;
        let d = Drawing { pages: vec![Page { size: None, layers: vec![grid] }], ..one_layer(Vec::new()) };
        let back = read(&write(&d, Limits::default()).unwrap(), Limits::default()).unwrap();
        assert_eq!((back.pages[0].layers[0].kind, back.pages[0].layers[0].flags.locked), (LayerKind::Grid, locked));
    }
}

/// A drawing at a limit is written and read whole; one past it is refused.
#[test]
fn drawings_past_what_a_read_takes_are_refused() {
    let flat = |n: i32| one_layer((0..n).map(|i| rect(100 + i, 100, None)).collect());
    let limits = Limits { objects: 10, ..Limits::default() };
    let back = read(&write(&flat(10), limits).unwrap(), limits).unwrap();
    assert!(back.warnings.is_empty());
    assert_eq!(write(&flat(11), limits), Err(Error::Limit("object count")));
    // Groups nested in groups: each one takes a list.
    let nested = |depth: usize| {
        let mut o = rect(100, 100, None);
        for _ in 0..depth {
            o = Object::Group { objects: vec![o], flags: Flags::default() };
        }
        one_layer(vec![o])
    };
    // The smallest depth a read of eight nested groups takes whole is the smallest the writer
    // takes, and a ninth group is refused at it.
    let depth = writes_at_the_readers_limit(&nested(8), |l, n| l.depth = n);
    assert_eq!(write(&nested(9), Limits { depth, ..Limits::default() }), Err(Error::Limit("nesting depth")));
    let limits = Limits { depth: 12, ..Limits::default() };
    // Groups that hold nothing are refused past the depth too, in a layer and in a clip.
    let hollow = |depth: usize| {
        let mut o = Object::Group { objects: Vec::new(), flags: Flags::default() };
        for _ in 0..depth {
            o = Object::Group { objects: vec![o], flags: Flags::default() };
        }
        o
    };
    // `hollow(n)` is n + 1 groups deep.
    let clip = |o| Object::Clip { container: Box::new(Shape::new(rect_geometry(), Matrix::IDENTITY, None, None)), objects: vec![o] };
    for wrap in [|o| o, clip] {
        assert!(write(&one_layer(vec![wrap(hollow(11))]), limits).is_ok());
        assert_eq!(write(&one_layer(vec![wrap(hollow(12))]), limits), Err(Error::Limit("nesting depth")));
    }
    let limits = Limits { coordinates: 3, ..Limits::default() };
    let line = |n: i32| {
        let segments = (1..n).map(|i| Segment::Line(pt(i, i))).collect();
        let path = Path { subpaths: vec![SubPath { start: pt(0, 0), segments, closed: false }] };
        one_layer(vec![Object::Shape(Box::new(Shape::new(Geometry::Path(path), Matrix::IDENTITY, None, None)))])
    };
    assert!(read(&write(&line(3), limits).unwrap(), limits).unwrap().warnings.is_empty());
    assert_eq!(write(&line(4), limits), Err(Error::Limit("coordinate count")));
}

#[test]
fn numbers_the_file_cant_hold_are_refused() {
    let with = |s: Shape| one_layer(vec![Object::Shape(Box::new(s))]);
    let far = Shape::new(Geometry::Ellipse { w: 1e9, h: 1.0 }, Matrix::IDENTITY, None, None);
    assert!(matches!(write(&with(far), Limits::default()), Err(Error::Unwritable(_))));
    let nan = Shape::new(Geometry::Rect { w: f64::NAN, h: 1.0, radii: [0.0; 4] }, Matrix::IDENTITY, None, None);
    assert!(matches!(write(&with(nan), Limits::default()), Err(Error::Unwritable(_))));
    let inf = Shape::new(Geometry::Ellipse { w: 1.0, h: 1.0 }, Matrix([f64::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0]), None, None);
    assert!(matches!(write(&with(inf), Limits::default()), Err(Error::Unwritable(_))));
}

#[test]
fn values_outside_their_range_are_refused() {
    let with = |s: Shape| one_layer(vec![Object::Shape(Box::new(s))]);
    let outlined = |width: f64| Outline {
        width,
        color: Color::Gray(1.0),
        cap: Cap::Butt,
        join: Join::Miter,
        dashes: Vec::new(),
        behind_fill: false,
        scale_with: false,
    };
    let mut opaque = Shape::new(rect_geometry(), Matrix::IDENTITY, None, None);
    opaque.opacity = 1.5;
    let far = Path { subpaths: vec![SubPath { start: Pt { x: 1e9, y: 0.0 }, segments: vec![Segment::Line(pt(0, 0))], closed: false }] };
    let cases = [
        ("negative outline width", with(Shape::new(rect_geometry(), Matrix::IDENTITY, None, Some(outlined(-1.0))))),
        // A width of 0 reads back as no width.
        ("positive outline width that rounds to 0", with(Shape::new(rect_geometry(), Matrix::IDENTITY, None, Some(outlined(u(1) / 4.0))))),
        ("negative corner radius", with(Shape::new(Geometry::Rect { w: 10.0, h: 10.0, radii: [-1.0, 0.0, 0.0, 0.0] }, Matrix::IDENTITY, None, None))),
        ("opacity past 1", with(opaque)),
        ("tint past 1", one_layer(vec![rect(100, 100, Some(Color::Spot { name: "Red".into(), tint: 1.5, look: Some([1.0, 1.0, 1.0]) }))])),
        ("path point past 32 bits", with(Shape::new(Geometry::Path(far), Matrix::IDENTITY, None, None))),
        ("page size past 32 bits", Drawing { page_size: Some((1e9, 10.0)), ..one_layer(Vec::new()) }),
    ];
    let written: Vec<_> =
        cases.iter().filter(|(_, d)| !matches!(write(d, Limits::default()), Err(Error::Unwritable(_)))).map(|(what, _)| *what).collect();
    assert!(written.is_empty(), "{written:?}");
}

fn arb_color() -> impl Strategy<Value = Color> {
    prop_oneof![
        any::<[u8; 3]>().prop_map(|v| Color::Rgb(v.map(c))),
        any::<[u8; 4]>().prop_map(|v| Color::Cmyk(v.map(c))),
        any::<u8>().prop_map(|v| Color::Gray(c(v))),
        Just(Color::Registration),
    ]
}

fn arb_shape() -> impl Strategy<Value = Shape> {
    let size = || (-1_000_000i32..1_000_000, -1_000_000i32..1_000_000);
    let geometry = prop_oneof![
        size().prop_map(|(w, h)| Geometry::Rect { w: u(w), h: u(h), radii: [0.0; 4] }),
        size().prop_map(|(w, h)| Geometry::Ellipse { w: u(w), h: u(h) }),
        (prop::collection::vec(size(), 1..20), any::<bool>()).prop_map(|(pts, closed)| {
            let segments = pts.iter().map(|&(x, y)| Segment::Line(pt(x, y))).collect();
            Geometry::Path(Path { subpaths: vec![SubPath { start: pt(0, 0), segments, closed }] })
        }),
    ];
    (geometry, prop::option::of(arb_color()), any::<bool>(), -500i32..500)
        .prop_map(|(g, fill, nonzero, x)| Shape { nonzero, ..Shape::new(g, moved(f64::from(x), -f64::from(x)), fill, None) })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 128, ..ProptestConfig::default() })]

    #[test]
    fn random_drawings_read_back_as_written(shapes in prop::collection::vec(arb_shape(), 0..30), grouped in 0usize..30) {
        let mut objects: Vec<Object> = shapes.into_iter().map(|s| Object::Shape(Box::new(s))).collect();
        let tail = objects.split_off(grouped.min(objects.len()));
        if !tail.is_empty() {
            objects.push(Object::Group { objects: tail, flags: Flags::default() });
        }
        let d = Drawing { page_size: Some((u(1_000_000), u(1_000_000))), ..one_layer(objects) };
        let back = read(&write(&d, Limits::default()).unwrap(), Limits::default()).unwrap();
        prop_assert!(back.warnings.is_empty());
        prop_assert_eq!(rounded(back), as_read(d));
    }

    #[test]
    fn writing_never_panics(
        w in prop_oneof![-1e4..1e4f64, any::<f64>()],
        h in prop_oneof![-1e4..1e4f64, any::<f64>()],
        m in prop_oneof![prop::array::uniform6(-1e3..1e3f64), any::<[f64; 6]>()],
        dashes in prop::collection::vec(prop_oneof![-1.0..20.0f64, any::<f64>()], 0..12),
        opacity in prop_oneof![-0.5..1.5f32, any::<f32>()],
    ) {
        let outline =
            Outline { width: w, color: Color::Gray(opacity), cap: Cap::Square, join: Join::Round, dashes, behind_fill: false, scale_with: false };
        // Corners within half the shorter side, so a rectangle of a size the file holds is written.
        let r = w.abs().min(h.abs()) / 4.0;
        let mut s = Shape::new(Geometry::Rect { w, h, radii: [r, 0.0, r, 0.0] }, Matrix(m), Some(Color::Rgb([opacity; 3])), Some(outline));
        s.opacity = opacity;
        let d = Drawing { pages: vec![Page { size: Some((w, h)), layers: vec![layer("A", vec![Object::Shape(Box::new(s))])] }], ..everything() };
        if let Ok(file) = write(&d, Limits::default()) {
            prop_assert!(read(&file, Limits::default()).is_ok());
        }
    }
}

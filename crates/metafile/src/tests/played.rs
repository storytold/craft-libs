//! Hand-built files from VectorCraft's reader tests, played into the picture model: what GDI
//! draws for them is what the model must hold.

use super::*;
use crate::{Cap, Kind, sniff};

/// An EMF of `records` (type, body) with a `w` x `h` frame in 0.01 mm and a reference device of
/// 100 pixels a millimetre, so one device unit is 0.01 mm.
fn hand_emf(w: i32, h: i32, records: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut head = vec![0u8; 16];
    head.extend(le(&[0, 0, w, h]));
    head.extend(0x464D_4520u32.to_le_bytes());
    head.extend(0x10000u32.to_le_bytes());
    head.extend([0; 8]);
    head.extend([1, 0, 0, 0]);
    head.extend([0; 12]);
    head.extend(le(&[10000, 10000, 100, 100]));
    let mut out = Vec::new();
    let eof = (14, vec![0, 0, 0, 0, 16, 0, 0, 0, 20, 0, 0, 0]);
    for (kind, body) in [(1, head)].iter().chain(records).chain([&eof]) {
        out.extend(kind.to_le_bytes());
        out.extend((8 + body.len() as u32).to_le_bytes());
        out.extend(body);
    }
    out
}

fn le(values: &[i32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn le16(values: &[i16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// A placeable WMF of `records` (function, parameters) in a `w` x `h` box at 1440 units an inch.
fn hand_wmf(w: i16, h: i16, records: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut o = 0x9AC6_CDD7u32.to_le_bytes().to_vec();
    o.extend(le16(&[0, 0, 0, w, h, 1440]));
    o.extend([0; 4]);
    let sum = o.as_chunks::<2>().0.iter().fold(0u16, |a, w| a ^ u16::from_le_bytes(*w));
    o.extend(sum.to_le_bytes());
    o.extend([1, 0, 9, 0, 0, 3]);
    o.extend([0; 12]);
    for (f, body) in records.iter().chain(&[(0, vec![])]) {
        o.extend((3 + body.len() as u32 / 2).to_le_bytes());
        o.extend(f.to_le_bytes());
        o.extend(body);
    }
    o
}

/// The bounding box `[x0, y0, x1, y1]` of a path item's points.
fn bounds(item: &Item) -> [f32; 4] {
    let (segs, ..) = path_segs(item);
    let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    let mut grow = |x: f32, y: f32| b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    for s in segs {
        match *s {
            Seg::Move(x, y) | Seg::Line(x, y) => grow(x, y),
            Seg::Cubic(x1, y1, x2, y2, x, y) => {
                grow(x1, y1);
                grow(x2, y2);
                grow(x, y);
            }
            Seg::Close => {}
        }
    }
    b
}

fn near(a: [f32; 4], b: [f64; 4], tol: f64) -> bool {
    a.iter().zip(b).all(|(a, b)| (f64::from(*a) - b).abs() <= tol)
}

const MM: f64 = 72.0 / 25.4;

#[test]
fn hand_built_emf_plays_like_gdi() {
    let mut text = vec![0; 16];
    text.extend(le(&[1, 0, 0]));
    // At (1000, 1500), 2 characters at offset 76 (from the record start).
    text.extend(le(&[1000, 1500, 2, 76, 0, 0, 0, 0, 0, 0]));
    text.extend("Hi".encode_utf16().flat_map(u16::to_le_bytes));
    let mut font = le(&[2, -1000, 0, 0, 0, 700]);
    font.extend([0u8; 8]);
    font.extend("Arial".encode_utf16().chain(std::iter::repeat_n(0, 27)).flat_map(u16::to_le_bytes));
    let records = vec![
        // A green brush in slot 1, selected; a null pen.
        (39, le(&[1, 0, 0x00_ff_00, 0])),
        (37, le(&[1])),
        (37, le(&[0x8000_0008u32 as i32])),
        // A rectangle of 10 x 5 mm at (10, 10) mm.
        (43, le(&[1000, 1000, 2000, 1500])),
        // A record nobody knows, and one of a kind not read (ANGLEARC).
        (9999, le(&[1, 2])),
        (41, le(&[0, 0, 10, 0, 0])),
        // Clip to a rectangle; a black pen draws two lines one after the other.
        (30, le(&[0, 0, 1500, 1500])),
        (37, le(&[0x8000_0007u32 as i32])),
        (27, le(&[0, 0])),
        (54, le(&[500, 0])),
        (54, le(&[500, 500])),
        // A 10 mm bold font and red text.
        (82, font),
        (37, le(&[2])),
        (24, le(&[0xff])),
        (84, text),
    ];
    let file = hand_emf(5000, 3000, &records);
    assert_eq!(sniff(&file), Some(Kind::Emf));
    let p = parse(&file).unwrap();
    assert!((f64::from(p.width) - 50.0 * MM).abs() < 0.01 && (f64::from(p.height) - 30.0 * MM).abs() < 0.01);
    assert_eq!(p.skipped_records, 2);
    assert_eq!(p.items.len(), 4, "the rectangle, two lines and the text");
    let (_, fill, stroke, _) = path_segs(&p.items[0]);
    assert_eq!((fill, stroke), (Some([0, 255, 0, 255]), None), "green, with the null pen");
    assert!(near(bounds(&p.items[0]), [10.0 * MM, 10.0 * MM, 20.0 * MM, 15.0 * MM], 0.01));
    let Item::Path { region: None, .. } = &p.items[0] else { panic!("drawn before the clip") };
    // The lines and the text are clipped to the rectangle.
    for item in &p.items[1..3] {
        let Item::Path { region: Some(r), fill: None, stroke: Some(([0, 0, 0, 255], _)), .. } = item else { panic!("a clipped black line") };
        assert!(r.len() == 1 && near(r[0], [0.0, 0.0, 15.0 * MM, 15.0 * MM], 0.01));
    }
    assert!(near(bounds(&p.items[1]), [0.0, 0.0, 5.0 * MM, 0.0], 0.01));
    assert!(near(bounds(&p.items[2]), [5.0 * MM, 0.0, 5.0 * MM, 5.0 * MM], 0.01));
    let Item::Text(t) = &p.items[3] else { panic!("text") };
    assert_eq!((t.text.as_str(), t.font.family.as_str(), t.font.weight), ("Hi", "Arial", 700));
    assert_eq!(t.color, [255, 0, 0, 255]);
    // The em is 1000 text units of 0.01 mm: 10 mm.
    let em = f64::from(-t.font.height) * t.transform[3];
    assert!((em - 10.0 * MM).abs() < 0.01, "{em}");
    assert!((t.transform[4] - 10.0 * MM).abs() < 0.01 && (t.transform[5] - 15.0 * MM).abs() < 0.01);
    assert!(t.region.as_ref().is_some_and(|r| r.len() == 1));
}

#[test]
fn emf_mapping_modes_and_world_transforms() {
    let records = vec![
        // MM_ANISOTROPIC: a 100 x 100 window on a 1000 x 2000 viewport.
        (17, le(&[8])),
        (9, le(&[100, 100])),
        (11, le(&[1000, 2000])),
        (43, le(&[0, 0, 10, 10])),
        // Then a world transform moving by (5, 0) logical units.
        (35, [1.0f32, 0.0, 0.0, 1.0, 5.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect()),
        (43, le(&[0, 0, 10, 10])),
    ];
    let p = parse(&hand_emf(10000, 10000, &records)).unwrap();
    // 10 units are 100 x 200 pixels: 1 x 2 mm.
    assert!(near(bounds(&p.items[0]), [0.0, 0.0, MM, 2.0 * MM], 0.01));
    assert!(near(bounds(&p.items[1]), [0.5 * MM, 0.0, 1.5 * MM, 2.0 * MM], 0.01));
}

#[test]
fn hand_built_wmf_plays_like_gdi() {
    let records = vec![
        // A blue brush (slot 0), a 20-unit red pen with square caps (slot 1), selected.
        (0x02FC, [le16(&[0]), 0x00ff_0000u32.to_le_bytes().to_vec(), le16(&[0])].concat()),
        (0x02FA, [le16(&[0x100, 20, 0]), 0x0000_00ffu32.to_le_bytes().to_vec()].concat()),
        (0x012D, le16(&[0])),
        (0x012D, le16(&[1])),
        // A rectangle (bottom, right, top, left): 1 x 0.5 inches at the origin.
        (0x041B, le16(&[720, 1440, 0, 0])),
        (0x0999, le16(&[1, 2])),
        // Delete the brush: the next object takes slot 0 again.
        (0x01F0, le16(&[0])),
        (0x02FC, [le16(&[0]), 0x0000_ff00u32.to_le_bytes().to_vec(), le16(&[0])].concat()),
        (0x012D, le16(&[0])),
        (0x0418, le16(&[1440, 1440, 720, 0])),
    ];
    let file = hand_wmf(1440, 1440, &records);
    assert_eq!(sniff(&file), Some(Kind::Wmf));
    let p = parse(&file).unwrap();
    assert_eq!((p.width, p.height), (72.0, 72.0));
    assert_eq!(p.skipped_records, 1);
    assert_eq!(p.items.len(), 2);
    let Item::Path { fill, stroke, line, .. } = &p.items[0] else { panic!("a path") };
    assert_eq!(*fill, Some([0, 0, 255, 255]));
    assert!(near(bounds(&p.items[0]), [0.0, 0.0, 72.0, 36.0], 1e-4));
    assert_eq!((*stroke, line.cap), (Some(([255, 0, 0, 255], 1.0)), Cap::Square), "20 units of 1/1440 inch are a point");
    assert_eq!(path_segs(&p.items[1]).1, Some([0, 255, 0, 255]), "the new brush took the freed slot");
}

#[test]
fn wmf_window_is_mapped_onto_the_placeable_box() {
    // The box is 1000 x 500 units at 1440 an inch: 50 x 25 pt.
    let want = [12.5, 6.25, 37.5, 18.75];
    // Window 0,0 - 4000 x 2000: the rectangle 1000,500 - 3000,1500 is the middle half.
    let ext = vec![(0x020C, le16(&[2000, 4000])), (0x041B, le16(&[1500, 3000, 500, 1000]))];
    // Window 10000,20000 - 1000 x 500 (the box): the rectangle 10250,20125 - 10750,20375.
    let org = vec![(0x020B, le16(&[20000, 10000])), (0x020C, le16(&[500, 1000])), (0x041B, le16(&[20375, 10750, 20125, 10250]))];
    // Window 0,2000 - 4000 x -2000 (y up): the same rectangle, flipped onto the same place.
    let flip = vec![(0x020B, le16(&[2000, 0])), (0x020C, le16(&[-2000, 4000])), (0x041B, le16(&[1500, 3000, 500, 1000]))];
    for records in [ext, org, flip] {
        let p = parse(&hand_wmf(1000, 500, &records)).unwrap();
        assert_eq!((p.width, p.height), (50.0, 25.0));
        assert_eq!(p.items.len(), 1);
        assert!(near(bounds(&p.items[0]), want, 1e-4), "{:?}", bounds(&p.items[0]));
    }
}

#[test]
fn huge_counts_sizes_and_offsets_are_refused_not_allocated() {
    let huge = hand_emf(100, 100, &[(3, le(&[0, 0, 0, 0, i32::MAX])), (81, le(&[0; 18])), (8, le(&[0, 0, 0, 0, i32::MAX, i32::MAX]))]);
    assert!(parse(&huge).unwrap().items.is_empty());
    let huge = hand_wmf(100, 100, &[(0x0324, le16(&[i16::MAX])), (0x0538, le16(&[-1]))]);
    assert!(parse(&huge).unwrap().items.is_empty());
    assert!(parse(b"not a metafile").is_err());
}

#[test]
fn a_run_of_clip_rectangles_stays_one_region() {
    // 200,000 INTERSECTCLIPRECT records nest nothing: the region is still one rectangle.
    let mut records = vec![(30, le(&[0, 0, 1500, 1500])); 200_000];
    records.push((43, le(&[100, 100, 200, 200])));
    let p = parse(&hand_emf(5000, 3000, &records)).unwrap();
    let Item::Path { region: Some(r), clips, .. } = &p.items[0] else { panic!("a clipped path") };
    assert_eq!((r.len(), clips.len()), (1, 0));
}

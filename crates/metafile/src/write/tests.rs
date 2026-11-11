//! The writers' output record by record, and round trips through the reader.

use std::sync::Arc;

use kurbo::{Affine, BezPath, Ellipse, Point, Rect, Shape};

use super::*;
use crate::{Item, Kind, Seg, parse, sniff};

const RED: [u8; 3] = [255, 0, 0];
const BLUE: [u8; 3] = [0, 0, 255];

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}

/// The EMF's records: (type, the whole record).
fn emf_records(b: &[u8]) -> Vec<(u32, &[u8])> {
    let mut out = vec![];
    let mut at = 0;
    while at < b.len() {
        let size = u32_at(b, at + 4) as usize;
        assert!(size >= 8 && size.is_multiple_of(4), "record size {size} at {at}");
        out.push((u32_at(b, at), &b[at..at + size]));
        at += size;
    }
    assert_eq!(at, b.len(), "records end at the end of the file");
    out
}

/// The WMF's records after both headers: (function, the whole record).
fn wmf_records(b: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = vec![];
    let mut at = 22 + 18;
    while at < b.len() {
        let words = u32_at(b, at) as usize;
        assert!(words >= 3, "record of {words} words at {at}");
        out.push((u16_at(b, at + 4), &b[at..at + words * 2]));
        at += words * 2;
    }
    assert_eq!(at, b.len());
    out
}

fn kinds(recs: &[(u32, &[u8])]) -> Vec<u32> {
    recs.iter().map(|r| r.0).collect()
}

fn count(k: &[u32], kind: u32) -> usize {
    k.iter().filter(|x| **x == kind).count()
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    Rect::new(x0, y0, x1, y1).to_path(0.1)
}

fn ellipse(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
    let mut p = Ellipse::from_rect(Rect::new(x0, y0, x1, y1)).to_path(0.01);
    if !matches!(p.elements().last(), Some(PathEl::ClosePath)) {
        p.close_path();
    }
    p
}

fn line(a: (f64, f64), b: (f64, f64)) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(a);
    p.line_to(b);
    p
}

fn fill(path: BezPath, rgb: [u8; 3]) -> Op {
    Op::Fill { path, even_odd: false, rgb }
}

fn pen(rgb: [u8; 3], width: f64) -> Pen {
    Pen { rgb, width, cap: Cap::Round, join: Join::Round, miter: 10.0, dash: None }
}

fn stroke(path: BezPath, pen: Pen) -> Op {
    Op::Stroke { path, pen }
}

fn image(width: u32, height: u32, px: [u8; 4]) -> Arc<Image> {
    Arc::new(Image { width, height, rgba: px.repeat((width * height) as usize) })
}

fn page(w: f64, h: f64) -> Rect {
    Rect::new(0.0, 0.0, w, h)
}

fn write_emf(ops: &[Op], w: f64, h: f64) -> Vec<u8> {
    emf(ops, page(w, h), "Craft", "Picture").unwrap()
}

/// Red, green, blue; then half-transparent black, transparent, white.
fn sample() -> Image {
    Image { width: 3, height: 2, rgba: vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 0, 0, 0, 128, 9, 9, 9, 0, 255, 255, 255, 255] }
}

#[test]
fn emf_header_frame_is_the_region_in_hundredths_of_a_millimetre() {
    // 72 x 144 pt is 25.4 x 50.8 mm; a rectangle from (36, 36) to (54, 72) pt.
    let b = &write_emf(&[fill(rect(36.0, 36.0, 54.0, 72.0), RED)], 72.0, 144.0);
    assert_eq!(sniff(b), Some(Kind::Emf));
    assert_eq!(u32_at(b, 0), 1);
    assert_eq!(u32_at(b, 40), 0x464D_4520, "the \" EMF\" signature");
    // rclFrame.
    assert_eq!([i32_at(b, 24), i32_at(b, 28), i32_at(b, 32), i32_at(b, 36)], [0, 0, 2540, 5080]);
    // rclBounds, in device units of 0.01 mm: the rectangle.
    assert_eq!([i32_at(b, 8), i32_at(b, 12), i32_at(b, 16), i32_at(b, 20)], [1270, 1270, 1905, 2540]);
    // The reference device has 100 pixels a millimetre.
    let (dev, mm) = ((i32_at(b, 72), i32_at(b, 76)), (i32_at(b, 80), i32_at(b, 84)));
    assert_eq!(dev, (mm.0 * 100, mm.1 * 100));
    assert_eq!((u32_at(b, 100), u32_at(b, 104)), (mm.0 as u32 * 1000, mm.1 as u32 * 1000), "micrometres");
    // The description names the application and the picture.
    let n = u32_at(b, 60) as usize;
    let off = u32_at(b, 64) as usize;
    assert_eq!(off, 108);
    let desc: Vec<u16> = (0..n).map(|i| u16_at(b, off + 2 * i)).collect();
    assert_eq!(String::from_utf16_lossy(&desc), "Craft\0Picture\0\0");
    // An empty picture has empty bounds.
    let empty = write_emf(&[], 72.0, 72.0);
    assert_eq!([i32_at(&empty, 8), i32_at(&empty, 16)], [0, -1]);
    // The region's origin is the picture's.
    let moved = emf(&[fill(rect(136.0, 36.0, 154.0, 72.0), RED)], Rect::new(100.0, 0.0, 172.0, 144.0), "", "").unwrap();
    assert_eq!([i32_at(&moved, 8), i32_at(&moved, 16)], [1270, 1905]);
}

#[test]
fn emf_record_sizes_add_up_to_the_header_totals() {
    let ops = [
        Op::Image { image: image(3, 2, [0, 128, 0, 255]), xf: Affine::translate((10.0, 10.0)) * Affine::scale(10.0), opacity: 1.0 },
        fill(ellipse(50.0, 10.0, 150.0, 90.0), RED),
        stroke(ellipse(50.0, 10.0, 150.0, 90.0), pen(BLUE, 2.0)),
    ];
    let b = write_emf(&ops, 200.0, 100.0);
    let recs = emf_records(&b);
    assert_eq!(u32_at(&b, 48) as usize, b.len(), "nBytes");
    assert_eq!(u32_at(&b, 52) as usize, recs.len(), "nRecords");
    assert_eq!(u16_at(&b, 56), 3, "nHandles: the reserved slot, a brush and a pen");
    let (last, eof) = recs.last().copied().unwrap();
    assert_eq!((last, eof.len()), (14, 20), "EMR_EOF ends the file");
    assert_eq!(u32_at(eof, 16), 20, "nSizeLast");
    // Fixed-size records have their fixed sizes.
    for (k, r) in &recs {
        let fixed = match k {
            18 | 19 | 34 | 37 | 40 | 58 | 67 => Some(12),
            27 | 54 => Some(16),
            33 | 59 | 60 | 61 => Some(8),
            39 | 62 | 64 => Some(24),
            35 => Some(32),
            36 => Some(36),
            _ => None,
        };
        if let Some(f) = fixed {
            assert_eq!(r.len(), f, "record {k}");
        }
    }
    // StretchDIBits: its bitmap where its offsets say, 3 x 2 pixels of 24 bits (rows of 12 bytes).
    let sdib = recs.iter().find(|r| r.0 == 81).unwrap().1;
    assert_eq!([u32_at(sdib, 48), u32_at(sdib, 52), u32_at(sdib, 56), u32_at(sdib, 60)], [80, 40, 120, 24]);
    assert_eq!(sdib.len(), 144);
}

#[test]
fn emf_paths_keep_their_curves_brushes_and_pens() {
    let e = ellipse(10.0, 10.0, 60.0, 40.0);
    let p = Pen { join: Join::Bevel, ..pen(BLUE, 4.0) };
    let b = write_emf(&[fill(e.clone(), RED), stroke(e, p)], 100.0, 100.0);
    let recs = emf_records(&b);
    let k = kinds(&recs);
    assert!(k.contains(&5), "EMR_POLYBEZIERTO keeps the curves");
    assert!(k.contains(&62) && k.contains(&64), "filled, then stroked");
    let brush = recs.iter().find(|r| r.0 == 39).unwrap().1;
    assert_eq!(u32_at(brush, 16), 0x0000_00ff, "a red COLORREF");
    let pen_rec = recs.iter().find(|r| r.0 == 95).unwrap().1;
    let style = u32_at(pen_rec, 28);
    assert_eq!(style & 0x000f_0000, 0x0001_0000, "geometric");
    assert_eq!(style & 0xf00, 0, "round caps");
    assert_eq!(style & 0xf000, 0x1000, "bevel joins");
    assert_eq!(u32_at(pen_rec, 32), (4.0 * 2540.0 / 72.0f64).round() as u32, "4 pt in 0.01 mm");
    assert_eq!(u32_at(pen_rec, 40), 0x00ff_0000, "a blue COLORREF");
    assert!(recs.iter().any(|r| r.0 == 19 && u32_at(r.1, 8) == 2), "WINDING for non-zero");
    // One brush and one pen for two shapes of the same colours.
    let two = [
        fill(rect(0.0, 0.0, 10.0, 10.0), RED),
        stroke(rect(0.0, 0.0, 10.0, 10.0), pen(BLUE, 1.0)),
        fill(rect(20.0, 0.0, 30.0, 10.0), RED),
        stroke(rect(20.0, 0.0, 30.0, 10.0), pen(BLUE, 1.0)),
    ];
    let k = kinds(&emf_records(&write_emf(&two, 100.0, 100.0)));
    assert_eq!((count(&k, 39), count(&k, 95)), (1, 1));
    // A miter join sets the miter limit.
    let miter = Pen { join: Join::Miter, miter: 4.0, ..pen(BLUE, 1.0) };
    let recs_bytes = write_emf(&[stroke(rect(0.0, 0.0, 10.0, 10.0), miter)], 100.0, 100.0);
    assert!(emf_records(&recs_bytes).iter().any(|r| r.0 == 58 && u32_at(r.1, 8) == 4));
}

#[test]
fn emf_dashes_are_pen_styles() {
    let l = line((10.0, 10.0), (90.0, 10.0));
    let dashed = |dash: Dash| write_emf(&[stroke(l.clone(), Pen { dash: Some(dash), ..pen(BLUE, 2.0) })], 100.0, 100.0);
    let native = Dash { pattern: vec![6.0, 3.0], offset: 0.0 };
    assert!(native.emf_pen_draws());
    let b = dashed(native);
    let recs = emf_records(&b);
    let pen_rec = recs.iter().find(|r| r.0 == 95).unwrap().1;
    assert_eq!(u32_at(pen_rec, 28) & 0xf, 7, "PS_USERSTYLE");
    assert_eq!(u32_at(pen_rec, 48), 2);
    assert_eq!([u32_at(pen_rec, 52), u32_at(pen_rec, 56)], [(6.0 * 2540.0 / 72.0f64).round() as u32, (3.0 * 2540.0 / 72.0f64).round() as u32]);
    // With an offset, the dashes are separate lines drawn with a solid pen.
    let offset = Dash { pattern: vec![6.0, 3.0], offset: 2.0 };
    assert!(!offset.emf_pen_draws());
    let b = dashed(offset);
    let recs = emf_records(&b);
    assert_eq!(u32_at(recs.iter().find(|r| r.0 == 95).unwrap().1, 28) & 0xf, 0);
    assert!(count(&kinds(&recs), 27) > 5, "a move-to for each dash");
    // An odd pattern repeats to an even one; more than 16 entries, or one under a device unit,
    // are not a pen's.
    assert!(Dash { pattern: vec![4.0; 8], offset: 0.0 }.emf_pen_draws());
    assert!(!Dash { pattern: vec![4.0; 9], offset: 0.0 }.emf_pen_draws());
    assert!(!Dash { pattern: vec![4.0, 0.001], offset: 0.0 }.emf_pen_draws());
}

#[test]
fn emf_clips_and_images() {
    let ops = [
        Op::Clip { path: rect(10.0, 10.0, 60.0, 30.0), even_odd: true },
        fill(rect(0.0, 0.0, 100.0, 100.0), RED),
        Op::Image { image: image(4, 4, [10, 20, 30, 255]), xf: Affine::translate((0.0, 50.0)), opacity: 1.0 },
        Op::Unclip,
        Op::Image { image: image(4, 4, [10, 20, 30, 100]), xf: Affine::translate((50.0, 50.0)) * Affine::rotate(0.3), opacity: 1.0 },
    ];
    let b = write_emf(&ops, 100.0, 100.0);
    let recs = emf_records(&b);
    let k = kinds(&recs);
    assert_eq!(count(&k, 67), 1);
    assert!(recs.iter().any(|r| r.0 == 67 && u32_at(r.1, 8) == 1), "RGN_AND: inside the clips around it");
    assert_eq!(count(&k, 33), count(&k, 34), "every save is restored");
    // The opaque image is StretchDIBits; the translucent one AlphaBlend with per-pixel alpha.
    assert_eq!(count(&k, 81), 1);
    let ab = recs.iter().find(|r| r.0 == 114).unwrap().1;
    assert_eq!(&ab[40..44], &[0, 0, 255, 1]);
    // Images are placed by a world transform, reset after each.
    assert_eq!((count(&k, 35), count(&k, 36)), (2, 2));
}

#[test]
fn wmf_placeable_header_checksum_and_record_sizes() {
    let e = ellipse(10.0, 10.0, 60.0, 40.0);
    let out = wmf(&[fill(e.clone(), RED), stroke(e, pen(BLUE, 1.0))], page(72.0, 144.0)).unwrap();
    let b = &out.bytes;
    assert_eq!(sniff(b), Some(Kind::Wmf));
    assert_eq!(u32_at(b, 0), 0x9AC6_CDD7);
    // The box in logical units at 1440 an inch: 1 x 2 inches.
    assert_eq!([u16_at(b, 6), u16_at(b, 8), u16_at(b, 10), u16_at(b, 12)], [0, 0, 1440, 2880]);
    assert_eq!(u16_at(b, 14), 1440);
    let xor = (0..10).fold(0u16, |a, i| a ^ u16_at(b, 2 * i));
    assert_eq!(u16_at(b, 20), xor, "the checksum");
    // The WMF header: a memory metafile of 9 words, its size in words, the largest record.
    assert_eq!([u16_at(b, 22), u16_at(b, 24), u16_at(b, 26)], [1, 9, 0x0300]);
    assert_eq!(u32_at(b, 28) as usize * 2, b.len() - 22);
    let recs = wmf_records(b);
    let largest = recs.iter().map(|r| r.1.len() / 2).max().unwrap();
    assert_eq!(u32_at(b, 34) as usize, largest, "MaxRecord");
    assert_eq!(recs.last().map(|r| (r.0, r.1.len())), Some((0, 6)), "META_EOF ends the file");
    // Objects: the null pen and brush, the brush, the pen.
    assert_eq!(u16_at(b, 32), 4);
    // The curve is flattened into a polygon, its outline a polygon drawn with the null brush.
    let poly = recs.iter().find(|r| r.0 == 0x0538).unwrap().1;
    assert_eq!(u16_at(poly, 6), 1);
    assert!(u16_at(poly, 8) > 16, "many points");
    assert!(recs.iter().any(|r| r.0 == 0x0324));
    assert_eq!(out, Wmf { bytes: out.bytes.clone(), ..Wmf::default() }, "nothing was lost");
}

#[test]
fn wmf_sizes_fit_sixteen_bits() {
    assert_eq!(super::wmf::units_per_inch(612.0, 792.0), 1440);
    let inch = super::wmf::units_per_inch(3000.0, 1000.0);
    assert!(inch < 1440 && f64::from(inch) * 3000.0 / 72.0 <= 32767.0);
    let b = wmf(&[fill(rect(0.0, 0.0, 3000.0, 1000.0), RED)], page(3000.0, 1000.0)).unwrap().bytes;
    assert_eq!(u16_at(&b, 14), inch);
    assert!(u16_at(&b, 10) <= 32767);
}

#[test]
fn wmf_reports_what_it_leaves_out() {
    let clipped = [Op::Clip { path: rect(0.0, 0.0, 20.0, 20.0), even_odd: false }, fill(rect(5.0, 5.0, 40.0, 40.0), RED), Op::Unclip];
    let out = wmf(&clipped, page(100.0, 100.0)).unwrap();
    assert!(out.clips_ignored && !out.translucent_images && !out.turned_images && !out.oversized_images);
    // The art the clip covers is written whole.
    assert!(wmf_records(&out.bytes).iter().any(|r| r.0 == 0x0538));
    let turned = Op::Image { image: image(2, 2, [0, 0, 0, 10]), xf: Affine::translate((50.0, 50.0)) * Affine::rotate(0.5), opacity: 1.0 };
    let out = wmf(&[turned], page(100.0, 100.0)).unwrap();
    assert!(out.translucent_images && out.turned_images && !out.clips_ignored);
    let faded = Op::Image { image: image(2, 2, [0, 0, 0, 255]), xf: Affine::IDENTITY, opacity: 0.5 };
    assert!(wmf(&[faded], page(100.0, 100.0)).unwrap().translucent_images);
    // An image whose side does not fit the record's 16-bit sizes is left out.
    let wide = Op::Image { image: image(40_000, 1, [0, 0, 0, 255]), xf: Affine::IDENTITY, opacity: 1.0 };
    let out = wmf(&[wide], page(100.0, 100.0)).unwrap();
    assert!(out.oversized_images);
    assert!(!wmf_records(&out.bytes).iter().any(|r| r.0 == 0x0F43));
}

#[test]
fn an_empty_or_unbounded_region_is_refused() {
    for r in [Rect::new(0.0, 0.0, 0.0, 10.0), Rect::new(0.0, 0.0, f64::NAN, 10.0), Rect::new(0.0, 0.0, f64::INFINITY, 10.0)] {
        assert!(emf(&[], r, "", "").is_none());
        assert!(wmf(&[], r).is_none());
    }
    // Corners in either order are one region.
    assert_eq!(emf(&[], Rect::new(72.0, 72.0, 0.0, 0.0), "", ""), emf(&[], page(72.0, 72.0), "", ""));
}

#[test]
fn rgb24_is_bottom_up_bgr_over_white() {
    let (bmi, bits) = dib::rgb24(&sample(), 1.0);
    assert_eq!(bmi.len(), 40);
    assert_eq!(&bmi[14..16], &[24, 0], "24 bits a pixel");
    // Rows of 9 bytes padded to 12, the last row first: half-transparent black over white is
    // mid grey, transparent is white.
    assert_eq!(bits, vec![127, 127, 127, 255, 255, 255, 255, 255, 255, 0, 0, 0, 0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 0, 0]);
}

#[test]
fn bgra32_is_premultiplied() {
    let (bmi, bits) = dib::bgra32(&sample(), 1.0);
    assert_eq!(&bmi[14..16], &[32, 0]);
    assert_eq!(&bits[..12], &[0, 0, 0, 128, 0, 0, 0, 0, 255, 255, 255, 255]);
    assert_eq!(&bits[12..16], &[0, 0, 255, 255], "red, as BGRA");
    // At half opacity.
    let (_, half) = dib::bgra32(&sample(), 0.5);
    assert_eq!(&half[12..16], &[0, 0, 128, 128]);
}

#[test]
fn written_dibs_decode_with_the_reader() {
    let decode =
        |(bmi, bits): (Vec<u8>, Vec<u8>), alpha: bool| crate::dib::decode(crate::bytes::Bytes(&bmi), crate::bytes::Bytes(&bits), alpha).unwrap();
    let back = decode(dib::rgb24(&sample(), 1.0), false);
    assert_eq!((back.w, back.h), (3, 2));
    assert_eq!(&back.px[..12], &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]);
    assert_eq!(&back.px[12..20], &[127, 127, 127, 255, 255, 255, 255, 255], "over white");
    let back = decode(dib::bgra32(&sample(), 1.0), true);
    assert_eq!(&back.px[..12], &sample().rgba[..12]);
    assert_eq!((back.px[15], back.px[19]), (128, 0), "the alpha is kept");
    // A short pixel buffer writes the rows it has and no more.
    let short = Image { width: 3, height: 2, rgba: vec![255; 12] };
    assert_eq!(dib::rgb24(&short, 1.0).1.len(), 24);
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// The bounding box of a path item's points.
fn bounds(segs: &[Seg]) -> Rect {
    let mut pts = Vec::new();
    for s in segs {
        match *s {
            Seg::Move(x, y) | Seg::Line(x, y) => pts.push(Point::new(f64::from(x), f64::from(y))),
            Seg::Cubic(a, b, c, d, x, y) => pts.extend([(a, b), (c, d), (x, y)].map(|(x, y)| Point::new(f64::from(x), f64::from(y)))),
            Seg::Close => {}
        }
    }
    pts.iter().skip(1).fold(Rect::from_points(pts[0], pts[0]), |r, p| r.union_pt(*p))
}

fn rect_near(a: Rect, b: Rect, tol: f64) -> bool {
    close(a.x0, b.x0, tol) && close(a.y0, b.y0, tol) && close(a.x1, b.x1, tol) && close(a.y1, b.y1, tol)
}

#[test]
fn emf_round_trips_through_the_reader() {
    let turned = Affine::translate((120.0, 50.0)) * Affine::rotate(0.3) * Affine::scale(5.0);
    let ops = [
        Op::Fill { path: ellipse(10.0, 10.0, 60.0, 40.0), even_odd: true, rgb: RED },
        stroke(
            line((10.0, 60.0), (90.0, 60.0)),
            Pen { rgb: BLUE, width: 2.0, cap: Cap::Flat, join: Join::Miter, miter: 4.0, dash: Some(Dash { pattern: vec![6.0, 3.0], offset: 0.0 }) },
        ),
        Op::Clip { path: rect(100.0, 10.0, 150.0, 30.0), even_odd: false },
        fill(rect(90.0, 0.0, 160.0, 40.0), [0, 255, 0]),
        Op::Clip { path: rect(110.0, 0.0, 140.0, 40.0), even_odd: true },
        fill(rect(90.0, 0.0, 160.0, 40.0), BLUE),
        Op::Unclip,
        Op::Unclip,
        Op::Image { image: image(4, 2, [200, 100, 50, 128]), xf: turned, opacity: 1.0 },
        Op::Image { image: image(3, 2, [1, 2, 3, 255]), xf: Affine::translate((10.0, 70.0)) * Affine::scale(4.0), opacity: 1.0 },
        fill(rect(0.0, 90.0, 5.0, 95.0), RED),
    ];
    let p = parse(&write_emf(&ops, 200.0, 100.0)).unwrap();
    assert!(close(f64::from(p.width), 200.0, 0.02) && close(f64::from(p.height), 100.0, 0.02), "{} x {}", p.width, p.height);
    assert_eq!(p.skipped_records, 0, "the reader knows every record the writer writes");
    // A device unit is 0.01 mm, 0.028 pt.
    let tol = 0.05;
    let Item::Path { segs, fill: Some([255, 0, 0, 255]), stroke: None, even_odd: true, clips, .. } = &p.items[0] else { panic!("the ellipse") };
    assert!(segs.iter().any(|s| matches!(s, Seg::Cubic(..))), "curves stay curves");
    assert!(rect_near(bounds(segs), Rect::new(10.0, 10.0, 60.0, 40.0), tol) && clips.is_empty());
    let Item::Path { fill: None, stroke: Some(([0, 0, 255, 255], w)), line, .. } = &p.items[1] else { panic!("the dashed line") };
    assert!(close(f64::from(*w), 2.0, tol));
    assert_eq!((line.cap, line.join, line.miter_limit), (crate::Cap::Flat, crate::Join::Miter, 4.0));
    assert!(line.dash.len() == 2 && close(f64::from(line.dash[0]), 6.0, tol) && close(f64::from(line.dash[1]), 3.0, tol), "{:?}", line.dash);
    // One clip, then a second inside it; both gone after the two restores.
    let Item::Path { fill: Some([0, 255, 0, 255]), even_odd: false, clips: outer, .. } = &p.items[2] else { panic!("the clipped fill") };
    assert_eq!(outer.len(), 1);
    assert!(rect_near(bounds(&outer[0].segs), Rect::new(100.0, 10.0, 150.0, 30.0), tol) && !outer[0].even_odd);
    let Item::Path { fill: Some([0, 0, 255, 255]), clips: both, .. } = &p.items[3] else { panic!("the twice clipped fill") };
    assert_eq!(both.len(), 2);
    assert!(Arc::ptr_eq(&both[0], &outer[0]) && both[1].even_odd);
    // The turned image keeps its pixels, its alpha and its turn.
    let Item::Bitmap { transform, width: 4, height: 2, rgba, clips, .. } = &p.items[4] else { panic!("the turned image") };
    assert!(clips.is_empty());
    assert!(rgba.as_chunks::<4>().0.iter().all(|c| c[3] == 128 && c[0].abs_diff(200) <= 2 && c[1].abs_diff(100) <= 2 && c[2].abs_diff(50) <= 2));
    let want = (turned * Affine::scale_non_uniform(4.0, 2.0)).as_coeffs();
    assert!(transform.iter().zip(want).all(|(a, b)| close(*a, b, tol)), "{transform:?} {want:?}");
    let Item::Bitmap { rect: r, width: 3, height: 2, rgba, .. } = &p.items[5] else { panic!("the upright image") };
    assert!(rect_near(Rect::new(r[0].into(), r[1].into(), (r[0] + r[2]).into(), (r[1] + r[3]).into()), Rect::new(10.0, 70.0, 22.0, 78.0), tol));
    assert_eq!(rgba.as_slice(), [1, 2, 3, 255].repeat(6));
    let Item::Path { clips, .. } = &p.items[6] else { panic!("the last fill") };
    assert!(clips.is_empty());
    assert_eq!(p.items.len(), 7);
}

#[test]
fn wmf_round_trips_through_the_reader() {
    let mut tri = BezPath::new();
    tri.move_to((10.0, 10.0));
    tri.line_to((60.0, 10.0));
    tri.line_to((10.0, 40.0));
    tri.close_path();
    let ops = [
        Op::Fill { path: tri, even_odd: true, rgb: RED },
        stroke(line((10.0, 50.0), (100.0, 50.0)), Pen { cap: Cap::Square, join: Join::Bevel, ..pen(BLUE, 2.0) }),
        fill(ellipse(70.0, 10.0, 130.0, 40.0), [0, 255, 0]),
        Op::Image { image: image(3, 2, [1, 2, 3, 255]), xf: Affine::translate((10.0, 55.0)) * Affine::scale(4.0), opacity: 1.0 },
    ];
    let out = wmf(&ops, page(144.0, 72.0)).unwrap();
    let p = parse(&out.bytes).unwrap();
    assert_eq!((p.width, p.height), (144.0, 72.0));
    assert_eq!(p.skipped_records, 0);
    // A logical unit is 1/1440 inch, 0.05 pt.
    let tol = 0.06;
    let Item::Path { segs, fill: Some([255, 0, 0, 255]), stroke: None, even_odd: true, .. } = &p.items[0] else { panic!("the triangle") };
    assert!(rect_near(bounds(segs), Rect::new(10.0, 10.0, 60.0, 40.0), tol));
    let Item::Path { segs, fill: None, stroke: Some(([0, 0, 255, 255], w)), line, .. } = &p.items[1] else { panic!("the line") };
    assert!(close(f64::from(*w), 2.0, tol) && rect_near(bounds(segs), Rect::new(10.0, 50.0, 100.0, 50.0), tol));
    assert_eq!((line.cap, line.join), (crate::Cap::Square, crate::Join::Bevel));
    let Item::Path { segs, fill: Some([0, 255, 0, 255]), even_odd: false, .. } = &p.items[2] else { panic!("the ellipse") };
    assert!(rect_near(bounds(segs), Rect::new(70.0, 10.0, 130.0, 40.0), 0.3), "flattened to within a quarter unit");
    let Item::Bitmap { rect: r, width: 3, height: 2, rgba, .. } = &p.items[3] else { panic!("the image") };
    assert!(rect_near(Rect::new(r[0].into(), r[1].into(), (r[0] + r[2]).into(), (r[1] + r[3]).into()), Rect::new(10.0, 55.0, 22.0, 63.0), tol));
    assert_eq!(rgba.as_slice(), [1, 2, 3, 255].repeat(6));
}

#[test]
fn every_prefix_of_a_written_file_reads_without_a_panic() {
    let ops = [
        Op::Clip { path: rect(0.0, 0.0, 50.0, 50.0), even_odd: false },
        fill(ellipse(10.0, 10.0, 60.0, 40.0), RED),
        stroke(line((0.0, 0.0), (50.0, 50.0)), Pen { dash: Some(Dash { pattern: vec![3.0, 2.0], offset: 0.0 }), ..pen(BLUE, 1.0) }),
        Op::Unclip,
        Op::Image { image: image(2, 2, [9, 9, 9, 90]), xf: Affine::rotate(0.2), opacity: 0.5 },
    ];
    let e = write_emf(&ops, 100.0, 100.0);
    let w = wmf(&ops, page(100.0, 100.0)).unwrap().bytes;
    for file in [&e, &w] {
        for n in 0..=file.len() {
            let _ = parse(&file[..n]);
            let _ = crate::size(&file[..n]);
        }
        // Each byte in turn set to 0xFF.
        for at in 0..file.len() {
            let mut bad = file.clone();
            bad[at] = 0xFF;
            let _ = parse(&bad);
        }
    }
}

#[test]
fn a_tiny_dash_on_a_long_path_is_written_solid() {
    // Not a pen's pattern (an offset), and a hundred million dashes if cut: the line stays whole.
    let tiny = Dash { pattern: vec![0.01, 0.01], offset: 0.005 };
    let ops = [stroke(line((0.0, 0.0), (1.0e6, 0.0)), Pen { dash: Some(tiny), ..pen(BLUE, 1.0) })];
    let b = write_emf(&ops, 100.0, 100.0);
    assert!(count(&kinds(&emf_records(&b)), 27) == 1, "one move-to");
    assert!(wmf(&ops, page(100.0, 100.0)).unwrap().bytes.len() < 1000);
    // Up to the cap the dashes are cut.
    let some = Dash { pattern: vec![1.0, 1.0], offset: 0.5 };
    let b = write_emf(&[stroke(line((0.0, 0.0), (100.0, 0.0)), Pen { dash: Some(some), ..pen(BLUE, 1.0) })], 100.0, 100.0);
    assert_eq!(count(&kinds(&emf_records(&b)), 27), 51);
    // A pattern that draws nothing (all zero, negative, not finite) is a solid line.
    for bad in [vec![0.0, 0.0], vec![-1.0, 2.0], vec![f64::NAN, 1.0], vec![]] {
        let d = Dash { pattern: bad, offset: 1.0 };
        let b = write_emf(&[stroke(line((0.0, 0.0), (100.0, 0.0)), Pen { dash: Some(d), ..pen(BLUE, 1.0) })], 100.0, 100.0);
        assert_eq!(count(&kinds(&emf_records(&b)), 27), 1);
    }
}

#[test]
fn far_out_curves_flatten_into_a_bounded_number_of_points() {
    let curve = |h: f64| {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((h, 0.0), (h, 50.0), (50.0, 50.0));
        p.close_path();
        p
    };
    for h in [1.0e9, 1.0e70, f64::INFINITY, f64::NAN] {
        let out = wmf(&[fill(curve(h), RED), stroke(curve(h), pen(BLUE, 1.0))], page(100.0, 100.0)).unwrap();
        assert!(out.bytes.len() < 100_000, "{h}: {} bytes", out.bytes.len());
        let _ = parse(&out.bytes).unwrap();
        let e = write_emf(&[fill(curve(h), RED)], 100.0, 100.0);
        let _ = parse(&e).unwrap();
    }
}

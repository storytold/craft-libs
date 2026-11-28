//! QuickDraw pictures read from hand-built opcodes: versions 1 and 2, with and without the file's
//! application data; shapes, lines, regions, patterns, text, bitmaps, pixel maps and QuickTime
//! JPEG images; and pictures that are cut short, damaged or never end.

use kurbo::{Rect, Shape};

use craft_metafile::{Item, TruncationReason, bez_path, parse_pict};

/// Big-endian words.
fn be(v: &[i16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_be_bytes()).collect()
}

/// A QuickDraw rectangle: top, left, bottom, right.
fn rect(top: i16, left: i16, bottom: i16, right: i16) -> Vec<u8> {
    be(&[top, left, bottom, right])
}

/// A version 2 PICT file: 512 bytes of application data, then a picture of `frame` (top, left,
/// bottom, right) holding `ops`, each padded to an even length, and the end opcode.
fn v2(frame: [i16; 4], ops: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut b = vec![0xA5; 512];
    b.extend([0, 0]);
    b.extend(be(&frame));
    b.extend([0x00, 0x11, 0x02, 0xFF, 0x0C, 0x00]);
    b.extend(be(&[-1, 0]));
    b.extend([0; 20]);
    for (op, data) in ops {
        b.extend(op.to_be_bytes());
        b.extend(data);
        if b.len() % 2 == 1 {
            b.push(0);
        }
    }
    b.extend([0x00, 0xFF]);
    b
}

/// An RGB foreground color.
fn fg(r: u16, g: u16, b: u16) -> (u16, Vec<u8>) {
    (0x1A, [r, g, b].iter().flat_map(|c| c.to_be_bytes()).collect())
}

fn near(a: Rect, b: Rect, tol: f64) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() <= tol)
}

fn near_box(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= tol)
}

fn path_bounds(item: &Item) -> Rect {
    let Item::Path { segs, .. } = item else { panic!("expected a path") };
    bez_path(segs).bounding_box()
}

fn fill_rgb(item: &Item) -> Option<[u8; 3]> {
    match item {
        Item::Path { fill: Some(c), .. } => Some([c[0], c[1], c[2]]),
        Item::Pattern { tile, .. } => {
            let px = tile.rgba.as_chunks::<4>().0;
            let n = px.len().max(1) as f64;
            let mut sum = [0.0; 3];
            for p in px {
                sum[0] += f64::from(p[0]);
                sum[1] += f64::from(p[1]);
                sum[2] += f64::from(p[2]);
            }
            Some([(sum[0] / n).round() as u8, (sum[1] / n).round() as u8, (sum[2] / n).round() as u8])
        }
        _ => None,
    }
}

fn stroke(item: &Item) -> Option<([u8; 3], f32)> {
    match item {
        Item::Path { stroke: Some((c, w)), .. } => Some(([c[0], c[1], c[2]], *w)),
        _ => None,
    }
}

#[test]
fn a_version_2_picture_opens_as_shapes_in_its_frame() {
    let bytes = v2(
        [10, 20, 110, 220],
        &[
            fg(0xFFFF, 0, 0),
            // paintRect
            (0x31, rect(20, 30, 60, 80)),
            (0x07, be(&[4, 4])),
            fg(0, 0, 0xFFFF),
            // frameRect: stroked inside the rectangle.
            (0x30, rect(20, 120, 80, 200)),
            // A line, and one from its end: one path.
            (0x20, be(&[90, 40, 90, 140])),
            (0x21, be(&[100, 140])),
        ],
    );
    assert!(craft_metafile::is_pict(&bytes));
    let p = parse_pict(&bytes).unwrap();
    assert_eq!((p.width, p.height), (200.0, 100.0));
    assert_eq!(p.items.len(), 3);
    assert_eq!(fill_rgb(&p.items[0]), Some([255, 0, 0]));
    let b0 = path_bounds(&p.items[0]);
    assert!(near(b0, Rect::new(10.0, 10.0, 60.0, 50.0), 1e-5), "{b0:?}");
    assert_eq!(stroke(&p.items[1]), Some(([0, 0, 255], 4.0)));
    let b1 = path_bounds(&p.items[1]);
    assert!(near(b1, Rect::new(102.0, 12.0, 178.0, 68.0), 1e-5), "{b1:?}");
    assert_eq!(stroke(&p.items[2]), Some(([0, 0, 255], 4.0)));
    let b2 = path_bounds(&p.items[2]);
    assert!(near(b2, Rect::new(22.0, 82.0, 122.0, 92.0), 1e-5), "{b2:?}");
}

#[test]
fn a_bare_picture_and_a_version_1_picture_open() {
    // A version 2 picture without the file's application data.
    let file = v2([0, 0, 50, 50], &[(0x31, rect(10, 10, 20, 20))]);
    let bare = &file[512..];
    assert!(craft_metafile::is_pict(bare));
    assert_eq!(parse_pict(bare).unwrap().items.len(), 1);

    // Version 1: byte opcodes, known by name only.
    let mut v1 = vec![0, 0];
    v1.extend(rect(0, 0, 40, 60));
    v1.extend([0x11, 0x01, 0x31]);
    v1.extend(rect(5, 5, 15, 25));
    v1.extend([0xA0, 0x00, 0x05, 0x50]);
    v1.extend(rect(20, 20, 30, 30));
    v1.push(0xFF);
    assert!(craft_metafile::is_pict(&v1));
    let p = parse_pict(&v1).unwrap();
    assert_eq!((p.width, p.height), (60.0, 40.0));
    assert_eq!(p.items.len(), 2);
    let b0 = path_bounds(&p.items[0]);
    assert!(near(b0, Rect::new(5.0, 5.0, 25.0, 15.0), 1e-5));
    assert_eq!(stroke(&p.items[1]), Some(([0, 0, 0], 1.0)));

    let mut file = vec![0; 512];
    file.extend(&v1);
    assert_eq!(parse_pict(&file).unwrap().items.len(), 2);
}

#[test]
fn files_that_are_not_pictures_are_refused() {
    assert_eq!(parse_pict(&[0; 600]).unwrap_err(), craft_metafile::Error::NotPict);
    assert_eq!(parse_pict(b"").unwrap_err(), craft_metafile::Error::NotPict);
    let mut b = vec![0, 0];
    b.extend(rect(10, 10, 10, 50));
    b.extend([0x11, 0x01, 0xFF]);
    assert_eq!(parse_pict(&b).unwrap_err(), craft_metafile::Error::NotPict);
}

#[test]
fn rounded_rectangles_ovals_arcs_and_same_shapes() {
    let bytes = v2(
        [0, 0, 100, 100],
        &[
            (0x0B, be(&[20, 20])),
            (0x41, rect(0, 0, 50, 50)),
            (0x51, rect(50, 0, 100, 50)),
            // A wedge from 12 to 3 o'clock.
            (0x61, [rect(0, 50, 100, 150), be(&[0, 90])].concat()),
            fg(0, 0xFFFF, 0),
            // paintSameRect after paintRect: the same rectangle.
            (0x31, rect(60, 60, 70, 80)),
            (0x39, vec![]),
        ],
    );
    let p = parse_pict(&bytes).unwrap();
    assert_eq!(p.items.len(), 5);
    assert!(near(path_bounds(&p.items[0]), Rect::new(0.0, 0.0, 50.0, 50.0), 1e-5));
    assert!(near(path_bounds(&p.items[1]), Rect::new(0.0, 50.0, 50.0, 100.0), 1e-3));
    assert!(near(path_bounds(&p.items[2]), Rect::new(100.0, 0.0, 150.0, 50.0), 1e-3));
    assert_eq!(path_bounds(&p.items[3]), path_bounds(&p.items[4]));
    assert_eq!(fill_rgb(&p.items[4]), Some([0, 255, 0]));
}

#[test]
fn polygons_and_regions_fill_and_clip() {
    let region = [be(&[36]), rect(0, 0, 20, 20), be(&[0, 0, 20, 0x7FFF, 10, 10, 20, 0x7FFF, 20, 0, 10, 0x7FFF, 0x7FFF])].concat();
    let poly = [be(&[22]), rect(0, 0, 40, 40), be(&[0, 0, 40, 40, 0, 40])].concat();
    let bytes = v2([0, 0, 100, 100], &[(0x81, region.clone()), (0x71, poly), (0x01, region), (0x31, rect(0, 0, 100, 100))]);
    let p = parse_pict(&bytes).unwrap();
    assert_eq!(p.items.len(), 3);
    assert!(near(path_bounds(&p.items[0]), Rect::new(0.0, 0.0, 20.0, 20.0), 1e-5));
    assert!(near(path_bounds(&p.items[1]), Rect::new(0.0, 0.0, 40.0, 40.0), 1e-5));
    let Item::Path { region: Some(r), .. } = &p.items[2] else { panic!("expected clip region on item 2") };
    assert!(!r.is_empty());
}

#[test]
fn patterns_fill_with_their_average_color() {
    let gray = vec![0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55];
    let bytes = v2(
        [0, 0, 10, 10],
        &[(0x0A, gray), (0x34, rect(0, 0, 10, 10)), (0x12, [be(&[2]), vec![0; 8], be(&[0, -1, 0])].concat()), (0x32, rect(0, 0, 5, 5))],
    );
    let p = parse_pict(&bytes).unwrap();
    assert!(matches!(&p.items[0], Item::Pattern { .. }));
    assert_eq!(fill_rgb(&p.items[0]), Some([128, 128, 128]));
    assert_eq!(fill_rgb(&p.items[1]), Some([0, 255, 0]));
}

#[test]
fn text_comes_in_as_point_type_in_the_named_font() {
    let name = [be(&[9, 300]), vec![6], b"Geneva".to_vec()].concat();
    let bytes = v2(
        [0, 0, 100, 200],
        &[(0x2C, name), (0x03, be(&[300])), (0x0D, be(&[18])), (0x04, vec![1]), (0x28, [be(&[50, 10]), vec![4], b"Caf\x8E".to_vec()].concat())],
    );
    let p = parse_pict(&bytes).unwrap();
    let Item::Text(t) = &p.items[0] else { panic!("expected text") };
    assert_eq!(t.text, "Café");
    assert_eq!(t.font.family, "Geneva");
    assert_eq!(t.font.weight, 700);
    assert_eq!(t.font.height, -18.0);
    assert_eq!((t.transform[4], t.transform[5]), (10.0, 50.0));
}

fn pixmap_fields(pack: i16, size: i16, cmps: i16) -> Vec<u8> {
    be(&[0, pack, 0, 0, 72, 0, 72, 0, 0, size, cmps, 8, 0, 0, 0, 0, 0, 0])
}

#[test]
fn a_packed_indexed_pixel_map_becomes_an_image() {
    let table = be(&[0, 0, 0, 1, 0, -1, 0, 0, 1, 0, 0, -1]);
    let rows = [vec![2, 0xF9, 0], vec![9, 7, 1, 0, 1, 0, 1, 0, 1, 0]].concat();
    let data =
        [be(&[0x8008u16 as i16]), rect(0, 0, 2, 8), pixmap_fields(0, 8, 1), table, rect(0, 0, 2, 8), rect(10, 20, 14, 36), be(&[0]), rows].concat();
    let p = parse_pict(&v2([0, 0, 50, 50], &[(0x98, data)])).unwrap();
    let Item::Bitmap { width, height, rect, rgba, .. } = &p.items[0] else { panic!("expected bitmap") };
    assert_eq!((*width, *height), (8, 2));
    assert_eq!(rgba[0..4], [255, 0, 0, 255]);
    assert_eq!(rgba[32..36], [0, 0, 255, 255]);
    assert_eq!(rgba[36..40], [255, 0, 0, 255]);
    assert!(near_box(*rect, [20.0, 10.0, 16.0, 4.0], 1e-5));
}

#[test]
fn direct_pixel_maps_and_masked_bitmaps() {
    let row = vec![7, 5, 255, 0, 0, 255, 0, 0];
    let direct =
        [be(&[0, 0xFF, 0x8008u16 as i16]), rect(0, 0, 1, 2), pixmap_fields(4, 32, 3), rect(0, 0, 1, 2), rect(0, 0, 1, 2), be(&[0]), row].concat();
    let mask = [be(&[10]), rect(5, 0, 6, 1)].concat();
    let bitmap = [be(&[2]), rect(0, 0, 1, 2), rect(0, 0, 1, 2), rect(5, 0, 6, 2), be(&[1]), mask, vec![0x80, 0]].concat();
    let p = parse_pict(&v2([0, 0, 10, 10], &[(0x9A, direct), (0x91, bitmap)])).unwrap();
    assert_eq!(p.items.len(), 2);
    let Item::Bitmap { rgba: rgba0, .. } = &p.items[0] else { panic!("expected bitmap 0") };
    assert_eq!(rgba0[0..4], [255, 0, 0, 255]);
    assert_eq!(rgba0[4..8], [0, 255, 0, 255]);
    let Item::Bitmap { rgba: rgba1, region: Some(r), .. } = &p.items[1] else { panic!("expected bitmap 1 with region") };
    assert_eq!(rgba1[0..4], [0, 0, 0, 255]);
    assert_eq!(rgba1[7], 0, "the clear bit is left out");
    assert_eq!(r, &vec![[0.0, 5.0, 1.0, 6.0]]);
}

#[test]
fn rows_far_longer_than_their_pixels_are_refused() {
    let table = be(&[0, 0, 0, 1, 0, -1, 0, 0, 1, -1, 0, 0]);
    let mut row = be(&[256]);
    (0..127).for_each(|_| row.extend([0x81, 1]));
    row.extend([0x82, 1]);
    let map = [be(&[0xBFFFu16 as i16]), rect(0, 0, 1, 1), pixmap_fields(0, 8, 1), table.clone()].concat();
    let data = [map, rect(0, 0, 1, 1), rect(0, 0, 1, 1), be(&[0]), row.clone()].concat();
    let p = parse_pict(&v2([0, 0, 10, 10], &[(0x98, data), (0x31, rect(0, 0, 5, 5))])).unwrap();
    assert_eq!(p.dropped_images, 1);
    assert_eq!(p.items.len(), 1);
    assert!(!p.truncated);

    let pattern = [be(&[1]), vec![0xFF; 8], be(&[0xBFFFu16 as i16]), rect(0, 0, 1, 1), pixmap_fields(0, 8, 1), table, row].concat();
    let p = parse_pict(&v2([0, 0, 10, 10], &[(0x12, pattern), (0x32, rect(0, 0, 5, 5))])).unwrap();
    assert_eq!(fill_rgb(&p.items[0]), Some([0, 0, 0]));
}

#[test]
fn bitmap_rows_of_eight_bytes_or_more_are_read_unpacked() {
    let row = vec![0x80, 0, 0, 0, 0, 0, 0, 0];
    let bitmap = [be(&[8]), rect(0, 0, 1, 16), rect(0, 0, 1, 16), rect(0, 0, 1, 16), be(&[0]), row].concat();
    let p = parse_pict(&v2([0, 0, 10, 20], &[(0x90, bitmap), (0x31, rect(2, 2, 4, 4))])).unwrap();
    assert_eq!(p.items.len(), 2);
    let Item::Bitmap { width, height, rgba, .. } = &p.items[0] else { panic!("expected bitmap") };
    assert_eq!((*width, *height), (16, 1));
    assert_eq!(rgba[0..4], [0, 0, 0, 255]);
    assert_eq!(rgba[4..8], [255, 255, 255, 255]);
}

#[test]
fn an_empty_clip_region_hides_what_follows() {
    let empty = [be(&[10]), rect(0, 0, 0, 0)].concat();
    let p = parse_pict(&v2([0, 0, 50, 50], &[(0x01, empty), (0x31, rect(0, 0, 10, 10))])).unwrap();
    let Item::Path { region: Some(r), .. } = &p.items[0] else { panic!("expected item with clip") };
    assert_eq!(r, &vec![[0.0, 0.0, 0.0, 0.0]]);
}

fn quicktime(ctype: &[u8; 4], matrix: [f64; 6], w: i16, h: i16, data: &[u8]) -> Vec<u8> {
    let fixed = |v: f64| ((v * 65536.0) as i32).to_be_bytes();
    let [a, b, c, d, tx, ty] = matrix;
    let mut q = be(&[0]);
    for v in [a, b, 0.0, c, d, 0.0, tx, ty] {
        q.extend(fixed(v));
    }
    q.extend((1i32 << 30).to_be_bytes());
    q.extend([0; 4]);
    q.extend([0; 8]);
    q.extend(be(&[0]));
    q.extend(rect(0, 0, h, w));
    q.extend([0; 4]);
    q.extend([0; 4]);
    let mut desc = vec![];
    desc.extend(86u32.to_be_bytes());
    desc.extend(ctype);
    desc.extend([0; 24]);
    desc.extend(be(&[w, h]));
    desc.extend([0; 8]);
    desc.extend((data.len() as u32).to_be_bytes());
    desc.resize(86, 0);
    q.extend(desc);
    q.extend(data);
    [(q.len() as u32).to_be_bytes().to_vec(), q].concat()
}

fn red_jpeg() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(4, 2, image::Rgb([255, 0, 0]));
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
    buf.into_inner()
}

#[test]
fn quicktime_jpeg_images_are_placed_by_their_matrix() {
    let jpeg = red_jpeg();
    let ops = [
        (0x8200, quicktime(b"jpeg", [2.0, 0.0, 0.0, 2.0, 10.0, 5.0], 4, 2, &jpeg)),
        (0x8200, quicktime(b"rle ", [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 4, 2, &[1, 2, 3])),
    ];
    let p = parse_pict(&v2([0, 0, 50, 50], &ops)).unwrap();
    assert_eq!(p.items.len(), 1);
    let Item::Bitmap { width, height, rgba, rect, .. } = &p.items[0] else { panic!("expected bitmap") };
    assert_eq!((*width, *height), (4, 2));
    assert!(rgba[0] > 240 && rgba[1] < 16 && rgba[2] < 16);
    assert!(near_box(*rect, [10.0, 5.0, 8.0, 4.0], 1e-5));
    assert_eq!(p.dropped_images, 1);
}

#[test]
fn a_flattened_image_adds_no_image_data() {
    let p = parse_pict(&v2([0, 0, 50, 50], &[(0x8200, quicktime(b"jpeg", [0.0; 6], 4, 2, &red_jpeg()))])).unwrap();
    assert!(p.items.is_empty());
}

#[test]
fn pictures_cut_short_keep_what_came_before() {
    let full = v2([0, 0, 50, 50], &[(0x31, rect(0, 0, 10, 10)), (0x31, rect(20, 20, 30, 30))]);
    let cut = &full[..full.len() - 6];
    let p = parse_pict(cut).unwrap();
    assert_eq!(p.items.len(), 1);
    assert!(p.truncated);
    assert_eq!(p.truncation_reason, Some(TruncationReason::PrematureEof));

    let p = parse_pict(&v2([0, 0, 50, 50], &[(0x31, rect(0, 0, 10, 10)), (0xA1, be(&[100, 5000]))])).unwrap();
    assert_eq!(p.items.len(), 1);
    assert!(p.truncated);
    assert_eq!(p.truncation_reason, Some(TruncationReason::PrematureEof));

    let p = parse_pict(&v2([0, 0, 50, 50], &[(0xA0, be(&[100]))])).unwrap();
    assert!(p.items.is_empty());
    assert!(!p.truncated);
    assert_eq!(p.truncation_reason, None);
}

#[test]
fn reserved_opcodes_are_skipped_by_their_lengths() {
    let ops = [
        (0x0024, be(&[4, 1, 2])),
        (0x0200, be(&[1, 2])),
        (0x00D0, [8u32.to_be_bytes().to_vec(), vec![9; 8]].concat()),
        (0x8201, [2u32.to_be_bytes().to_vec(), vec![9; 2]].concat()),
        (0x31, rect(0, 0, 10, 10)),
    ];
    let p = parse_pict(&v2([0, 0, 50, 50], &ops)).unwrap();
    assert_eq!(p.items.len(), 1);
    assert_eq!(p.skipped_records, 4);
}

#[test]
fn a_picture_that_never_ends_stops_at_the_opcode_limit() {
    let mut b = v2([0, 0, 50, 50], &[(0x31, rect(0, 0, 10, 10))]);
    b.truncate(b.len() - 2);
    b.resize(b.len() + 2 * ((1 << 22) + 1), 0);
    b.extend([0x00, 0x31]);
    b.extend(rect(20, 20, 30, 30));
    b.extend([0x00, 0xFF]);
    let p = parse_pict(&b).unwrap();
    assert_eq!(p.items.len(), 1);
    assert!(p.truncated);
    assert_eq!(p.truncation_reason, Some(TruncationReason::OpcodeLimitExceeded));
}

#[test]
fn regions_past_the_picture_budget_are_left_out() {
    let mut words = vec![0];
    words.extend(0..16_000i16);
    words.extend([0x7FFF, 1, 0x7FFF, 0x7FFF]);
    let body = [rect(0, 0, 1, 16_000), be(&words)].concat();
    let region = [be(&[(body.len() + 2) as i16]), body].concat();
    let p = parse_pict(&v2([0, 0, 300, 300], &[(0x81, region.clone()), (0x81, region.clone()), (0x81, region)])).unwrap();
    assert_eq!(p.skipped_records, 1);
    assert_eq!(p.ignored_clips, 0);
    assert_eq!(p.items.len(), 2);
}

#[test]
fn pict_size_works_from_header() {
    let file = v2([10, 20, 110, 220], &[(0x31, rect(20, 30, 60, 80))]);
    assert_eq!(craft_metafile::size(&file).unwrap(), (200.0, 100.0));
    assert_eq!(craft_metafile::size(&file[512..]).unwrap(), (200.0, 100.0));
}

#[test]
fn bare_v1_pict_with_10x10_frame_is_parsed_as_pict() {
    let mut v1 = vec![0, 0];
    v1.extend(rect(0, 0, 10, 10));
    v1.extend([0x11, 0x01, 0x31]);
    v1.extend(rect(0, 0, 10, 10));
    v1.push(0xFF);
    v1.resize(256, 0);
    assert!(craft_metafile::is_pict(&v1));
    assert_eq!(craft_metafile::size(&v1).unwrap(), (10.0, 10.0));
    let pic = craft_metafile::parse(&v1).unwrap();
    assert_eq!((pic.width, pic.height), (10.0, 10.0));
    assert_eq!(pic.items.len(), 1);
}

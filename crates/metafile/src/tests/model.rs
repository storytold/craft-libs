//! Line styles, clip paths and the device-context records behind them, the DIB forms and the
//! header fields that came with VectorCraft's reader.

use std::sync::Arc;

use super::*;
use crate::canvas::MAX_CLIP_PATHS;
use crate::{Cap, ClipPath, Join, LineStyle, PlacedItem};

fn line_of(item: &Item) -> &LineStyle {
    match item {
        Item::Path { line, .. } => line,
        _ => panic!("expected a path"),
    }
}

fn clips_of(item: &Item) -> &Vec<Arc<ClipPath>> {
    match item {
        Item::Path { clips, .. } | Item::Bitmap { clips, .. } | Item::Pattern { clips, .. } => clips,
        Item::Text(t) => &t.clips,
    }
}

fn rect(l: i32, t: i32, r: i32, b: i32) -> Vec<u8> {
    erec(43, &cat(vec![e32(l), e32(t), e32(r), e32(b)]))
}

fn move_to(x: i32, y: i32) -> Vec<u8> {
    erec(27, &cat(vec![e32(x), e32(y)]))
}

fn line_to(x: i32, y: i32) -> Vec<u8> {
    erec(54, &cat(vec![e32(x), e32(y)]))
}

/// EMR_EXTCREATEPEN in slot 1 with `entries` style entries.
fn ext_pen(style: u32, width: u32, entries: &[u32]) -> Vec<u8> {
    let mut fields = vec![(8, 1), (28, style), (32, width), (36, 0), (40, 0x0000_00FF), (48, entries.len() as u32)];
    fields.extend(entries.iter().enumerate().map(|(i, e)| (52 + i * 4, *e)));
    erec_at(95, 52 + entries.len() * 4, &fields, &[])
}

/// A rectangle path bracket selected as the clip in `mode`.
fn clip_to_rect(l: i32, t: i32, r: i32, b: i32, mode: u32) -> Vec<Vec<u8>> {
    vec![erec(59, &[]), rect(l, t, r, b), erec(60, &[]), erec(67, &h32(mode))]
}

fn parse_emf(recs: &[Vec<u8>]) -> crate::Picture {
    parse(&emf(recs, [0; 4], [0; 4])).unwrap()
}

const PS_GEOMETRIC: u32 = 0x0001_0000;

#[test]
fn emf_geometric_pen_keeps_its_cap_join_miter_limit_and_dashes() {
    // PS_USERSTYLE with flat caps and miter joins, 4 wide, dashes of 8 and gaps of 4.
    let pen = ext_pen(PS_GEOMETRIC | 7 | 0x200 | 0x2000, 4, &[8, 4]);
    let recs = [pen, erec(37, &h32(1)), erec(58, &h32(3)), move_to(0, 0), line_to(100, 0)];
    let p = parse_emf(&recs);
    let (_, _, stroke, _) = path_segs(&p.items[0]);
    assert_eq!(stroke, Some(([255, 0, 0, 255], 4.0)));
    assert_eq!(line_of(&p.items[0]), &LineStyle { cap: Cap::Flat, join: Join::Miter, miter_limit: 3.0, dash: vec![8.0, 4.0] });
}

#[test]
fn select_clip_path_clips_what_follows_and_items_share_the_path() {
    let mut recs = vec![rect(0, 0, 100, 100)];
    recs.extend(clip_to_rect(10, 10, 60, 60, 1));
    recs.extend([rect(0, 0, 50, 50), rect(20, 20, 80, 80)]);
    let p = parse_emf(&recs);
    assert_eq!(p.items.len(), 3);
    assert!(clips_of(&p.items[0]).is_empty(), "drawn before the clip");
    let (a, b) = (clips_of(&p.items[1]), clips_of(&p.items[2]));
    assert_eq!((a.len(), b.len()), (1, 1));
    assert!(Arc::ptr_eq(&a[0], &b[0]), "one clip, shared");
    assert_eq!(a[0].segs, vec![Seg::Move(10.0, 10.0), Seg::Line(60.0, 10.0), Seg::Line(60.0, 60.0), Seg::Line(10.0, 60.0), Seg::Close]);
    assert!(a[0].even_odd, "the path takes the fill mode in force (ALTERNATE)");
    // The clip path is not painted, and the bracket is empty afterwards: FILLPATH draws nothing.
    let mut recs = clip_to_rect(0, 0, 10, 10, 1);
    recs.push(erec(62, &[0; 16]));
    assert!(parse_emf(&recs).items.is_empty());
}

#[test]
fn clip_paths_nest_with_and_replace_with_copy_and_end_with_restore_dc() {
    let mut recs = vec![erec(33, &[])];
    recs.extend(clip_to_rect(0, 0, 90, 90, 1));
    recs.extend(clip_to_rect(10, 10, 80, 80, 1));
    recs.push(rect(0, 0, 100, 100));
    recs.extend(clip_to_rect(20, 20, 70, 70, 5));
    recs.push(rect(0, 0, 100, 100));
    recs.push(erec(34, &e32(-1)));
    recs.push(rect(0, 0, 100, 100));
    let p = parse_emf(&recs);
    let n: Vec<usize> = p.items.iter().map(|i| clips_of(i).len()).collect();
    assert_eq!(n, vec![2, 1, 0], "RGN_AND nests, RGN_COPY replaces, RESTOREDC removes");
    let outer = &clips_of(&p.items[0])[0];
    assert_eq!(outer.segs.first(), Some(&Seg::Move(0.0, 0.0)), "outermost first");
    assert_eq!(clips_of(&p.items[1])[0].segs.first(), Some(&Seg::Move(20.0, 20.0)));
}

#[test]
fn clip_path_ops_count_against_the_byte_budget() {
    // Each clip path is charged once, when it is selected, so RGN_COPY in a loop cannot hold more
    // than the budget (the earlier paths stay alive while items drawn under them do).
    let mut cv = Canvas::new();
    let big = crate::MAX_PATH_OPS;
    let mut selected = 0;
    while !cv.full() && selected < 100 {
        cv.begin_path();
        cv.line_chain(&vec![(1.0, 1.0); big - 1], false, false);
        cv.end_path();
        cv.select_clip_path(5);
        selected += 1;
    }
    assert!(cv.full(), "the budget must stop a file that keeps selecting large clip paths");
    assert!(selected * big * std::mem::size_of::<crate::geom::Op>() <= 2 * MAX_BYTES);
}

#[test]
fn arc_direction_picks_the_other_part_of_the_ellipse() {
    // From the right-most point to the top-most one: counter-clockwise is the quarter between
    // them, clockwise the three quarters round the other way.
    let arc = erec(45, &cat(vec![e32(0), e32(0), e32(100), e32(100), e32(100), e32(50), e32(50), e32(0)]));
    let ccw = parse_emf(std::slice::from_ref(&arc));
    assert!(close(ccw.width, 50.0) && close(ccw.height, 50.0), "{} x {}", ccw.width, ccw.height);
    let cw = parse_emf(&[erec(57, &h32(2)), arc.clone()]);
    assert!(close(cw.width, 100.0) && close(cw.height, 100.0), "{} x {}", cw.width, cw.height);
    // RESTOREDC brings the direction back.
    let p = parse_emf(&[erec(33, &[]), erec(57, &h32(2)), erec(34, &e32(-1)), arc]);
    assert!(close(p.width, 50.0));
}

#[test]
fn stroke_and_fill_path_closes_open_figures_but_stroke_path_does_not() {
    let figure = |paint: u32| {
        let recs =
            [erec(59, &[]), move_to(0, 0), line_to(10, 0), line_to(10, 10), move_to(20, 0), line_to(30, 0), erec(60, &[]), erec(paint, &[0; 16])];
        let p = parse_emf(&recs);
        path_segs(&p.items[0]).0.clone()
    };
    let open = vec![Seg::Move(0.0, 0.0), Seg::Line(10.0, 0.0), Seg::Line(10.0, 10.0), Seg::Move(20.0, 0.0), Seg::Line(30.0, 0.0)];
    assert_eq!(figure(64), open, "STROKEPATH");
    assert_eq!(figure(62), open, "FILLPATH: a fill closes by itself");
    let closed =
        vec![Seg::Move(0.0, 0.0), Seg::Line(10.0, 0.0), Seg::Line(10.0, 10.0), Seg::Close, Seg::Move(20.0, 0.0), Seg::Line(30.0, 0.0), Seg::Close];
    assert_eq!(figure(63), closed, "STROKEANDFILLPATH");
}

#[test]
fn close_figure_moves_the_current_position_to_the_figures_start() {
    let recs = [erec(59, &[]), move_to(0, 0), line_to(10, 0), line_to(10, 10), erec(61, &[]), erec(60, &[]), erec(64, &[0; 16]), line_to(5, 20)];
    let p = parse_emf(&recs);
    assert_eq!(path_segs(&p.items[1]).0, &vec![Seg::Move(0.0, 0.0), Seg::Line(5.0, 20.0)]);
}

#[test]
fn emf_header_micrometres_set_the_device_resolution() {
    // The 88-byte header says 4 pixels a millimetre; the extension says 5 (200 mm for 1000 pixels).
    // A 25.4 mm frame is then 127 pixels wide, and a 127-pixel rectangle fills its 72 points.
    let short = emf(&[rect(0, 0, 127, 10)], [0, 0, 127, 10], [0, 0, 2540, 1270]);
    let mut file = short[..88].to_vec();
    file.extend([0u8; 12]);
    file.extend(200_000u32.to_le_bytes());
    file.extend(100_000u32.to_le_bytes());
    file.extend(&short[88..]);
    file[4..8].copy_from_slice(&108u32.to_le_bytes());
    let right = |bytes: &[u8]| {
        let p = parse(bytes).unwrap();
        assert!(close(p.width, 72.0));
        path_segs(&p.items[0]).0.iter().filter_map(|s| if let Seg::Line(x, _) = s { Some(*x) } else { None }).fold(0.0, f32::max)
    };
    assert!(close(right(&file), 72.0), "{}", right(&file));
    assert!(close(right(&short), 90.0), "without the extension the millimetres stand");
    // A zero in the extension falls back to the millimetres.
    file[100..104].copy_from_slice(&0u32.to_le_bytes());
    assert!(close(right(&file), 90.0));
    assert_eq!(crate::size(&file).unwrap(), (parse(&file).unwrap().width, parse(&file).unwrap().height));
}

/// STRETCHDIB of a 2 x 2 source into 20 x 10 with `dib` as the packed bitmap.
fn stretch_dib(dib: Vec<u8>) -> Vec<u8> {
    wmf(&[wrec(0x0F43, &cat(vec![h32(0x00CC_0020), h16(0), h16(2), h16(2), h16(0), h16(0), h16(10), h16(20), h16(0), h16(0), dib]))], None)
}

#[test]
fn core_header_dibs_decode() {
    // BITMAPCOREHEADER: size 12, 16-bit width and height, planes, bits per pixel; rows bottom-up.
    let core = |bpp: u16| cat(vec![h32(12), h16(2), h16(2), h16(1), h16(i32::from(bpp))]);
    let mut dib = core(24);
    dib.extend(dib24_bottom_up().get(40..).unwrap_or_default());
    let p = parse(&stretch_dib(dib)).unwrap();
    let Item::Bitmap { width, height, rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!((*width, *height), (2, 2));
    assert_eq!(rgba.as_slice(), DIB24_RGBA);
    // 1 bpp: two palette entries of three bytes (blue, green, red), then padded rows.
    let mut dib = core(1);
    dib.extend([10, 20, 30, 40, 50, 60]);
    dib.extend([0b0100_0000, 0, 0, 0, 0b1000_0000, 0, 0, 0]);
    let p = parse(&stretch_dib(dib)).unwrap();
    let Item::Bitmap { rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
    let (a, b) = ([30, 20, 10, 255], [60, 50, 40, 255]);
    assert_eq!(rgba.as_slice(), [b, a, a, b].concat(), "the file's second row is the top one");
    // 16 bpp is not a core-header depth.
    assert!(parse(&stretch_dib(core(16))).unwrap().items.is_empty());
}

fn png(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(px));
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// EMR_STRETCHDIBITS of the whole `w` x `h` source into (0, 0, 30, 10) with a header of
/// `compression` and `bits`.
fn stretch_dibits(w: i32, h: i32, compression: u32, bits: &[u8]) -> Vec<u8> {
    let mut info = info_header(w, h, 32, bits.len() as u32);
    info[16..20].copy_from_slice(&compression.to_le_bytes());
    let mut payload = vec![0u8; 16];
    let fields = [0, 0, 0, 0, w, h, 80, 40, 120, bits.len() as i32, 0, 0x00CC_0020, 30, 10];
    payload.extend(cat(fields.iter().map(|v| e32(*v)).collect()));
    payload.extend(info);
    payload.extend(bits);
    erec(81, &payload)
}

#[test]
fn bitmaps_that_are_not_read_are_counted() {
    let dropped = |recs: &[Vec<u8>]| {
        let p = parse_emf(recs);
        (p.items.len(), p.dropped_images)
    };
    assert_eq!(dropped(&[stretch_dibits(3, 1, 5, &png(3, 1, [1, 2, 3, 255]))]), (1, 0), "a bitmap that is drawn");
    assert_eq!(dropped(&[stretch_dibits(3, 1, 5, &[1, 2, 3, 4])]), (0, 1), "bits that are no image");
    let wide = png(crate::MAX_BITMAP_SIDE + 1, 1, [0; 4]);
    assert_eq!(dropped(&[stretch_dibits(crate::MAX_BITMAP_SIDE as i32 + 1, 1, 5, &wide)]), (0, 1), "a side past the cap");
    // A source rectangle beside the bitmap selects none of its pixels: nothing to draw, nothing lost.
    let mut beside = stretch_dibits(1, 1, 0, &[1, 2, 3, 255]);
    beside[32..36].copy_from_slice(&50i32.to_le_bytes());
    assert_eq!(dropped(&[beside.clone()]), (0, 0));
    // The same for a compression the decoder does not read (BI_RLE8), and counted when it is inside.
    beside[80 + 16..80 + 20].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(dropped(&[beside.clone()]), (0, 0));
    beside[32..36].copy_from_slice(&0i32.to_le_bytes());
    assert_eq!(dropped(&[beside]), (0, 1));
    // A BI_PNG header that says 10 x 1 over a 1 x 1 PNG: the PNG is what is cropped.
    let mut png_dib = stretch_dibits(10, 1, 5, &png(1, 1, [1, 2, 3, 255]));
    png_dib[32..36].copy_from_slice(&5i32.to_le_bytes());
    png_dib[40..44].copy_from_slice(&1i32.to_le_bytes());
    assert_eq!(dropped(&[png_dib]), (0, 0));
    // A STRETCHDIBITS without bits is not a bitmap.
    assert_eq!(dropped(&[stretch_dibits(1, 1, 0, &[])]), (0, 0));
    // WMF: a DIB of a depth the core header does not have.
    let core16 = cat(vec![h32(12), h16(2), h16(2), h16(1), h16(16)]);
    let p = parse(&stretch_dib(core16)).unwrap();
    assert_eq!((p.items.len(), p.dropped_images), (0, 1));
    let p = parse(&stretch_dib(dib24_bottom_up())).unwrap();
    assert_eq!((p.items.len(), p.dropped_images), (1, 0));
    // WMF: BI_RLE8, which the decoder does not read, from beside the bitmap and from inside it.
    let mut file = stretch_dib(dib24_bottom_up());
    // The 18-byte WMF header, then the record: the DIB starts 28 bytes in, after the source x at 18.
    let at = 18 + 28;
    assert_eq!(file[at..at + 4], 40u32.to_le_bytes());
    file[at + 16..at + 20].copy_from_slice(&1u32.to_le_bytes());
    let x_src = at - 10;
    file[x_src..x_src + 2].copy_from_slice(&50i16.to_le_bytes());
    assert_eq!(parse(&file).unwrap().dropped_images, 0);
    file[x_src..x_src + 2].copy_from_slice(&0i16.to_le_bytes());
    assert_eq!(parse(&file).unwrap().dropped_images, 1);
    // A header that gives the bitmap no pixels draws nothing.
    assert_eq!(dropped(&[stretch_dibits(0, 1, 0, &[1, 2, 3, 4])]), (0, 0));
    let empty = cat(vec![h32(12), h16(0), h16(2), h16(1), h16(16)]);
    assert_eq!(parse(&stretch_dib(empty)).unwrap().dropped_images, 0);
}

/// EMR_CREATEDIBPATTERNBRUSHPT in slot 1 with the DIB `info` and `bits`.
fn dib_pattern_brush(info: &[u8], bits: &[u8]) -> Vec<u8> {
    let fields = [1, 0, 32, info.len() as u32, 32 + info.len() as u32, bits.len() as u32];
    erec(94, &cat(vec![fields.iter().flat_map(|v| v.to_le_bytes()).collect(), info.to_vec(), bits.to_vec()]))
}

#[test]
fn pattern_and_texture_brushes_that_are_not_read_are_counted() {
    let dib = dib24_bottom_up();
    let (info, bits) = dib.split_at(40);
    assert_eq!(parse_emf(&[dib_pattern_brush(info, bits)]).dropped_images, 0);
    // BI_RLE8, which the decoder does not read.
    let mut rle = info_header(2, 2, 8, 4);
    rle[16..20].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(parse_emf(&[dib_pattern_brush(&rle, &[0; 4])]).dropped_images, 1);
    // A tile past the side cap.
    let wide = info_header(crate::canvas::MAX_TILE_SIDE as i32 + 1, 1, 24, 0);
    let wide_bits = vec![0u8; (crate::canvas::MAX_TILE_SIDE + 1) * 3 + 4];
    assert_eq!(parse_emf(&[dib_pattern_brush(&wide, &wide_bits)]).dropped_images, 1);
    // A header with no pixels.
    assert_eq!(parse_emf(&[dib_pattern_brush(&info_header(0, 2, 24, 0), &[0; 4])]).dropped_images, 0);
    // WMF DIBCREATEPATTERNBRUSH.
    let wmf_brush = |dib: Vec<u8>| parse(&wmf(&[wrec(0x0142, &cat(vec![h16(5), h16(0), dib]))], None)).unwrap().dropped_images;
    assert_eq!(wmf_brush(dib24_bottom_up()), 0);
    assert_eq!(wmf_brush(cat(vec![rle.clone(), vec![0; 4]])), 1);
    assert_eq!(wmf_brush(info_header(0, 2, 24, 0)), 0);
    // An EMF+ texture brush whose bitmap runs past the record, one with no pixels, and one that is read.
    let texture = |image: Vec<u8>| {
        let brush = cat(vec![h32(0xDBC0_1002), h32(2), h32(0), h32(0), image]);
        emf(&[plus_comment(&[plus_header(false), prec(0x4008, 0x0100 | 5, &brush)])], [0; 4], [0; 4])
    };
    let short = cat(vec![h32(0xDBC0_1002), h32(1), h32(4), h32(4), h32(16), h32(0x0026_200A), h32(0), vec![1, 2, 3]]);
    assert_eq!(parse(&texture(short)).unwrap().dropped_images, 1);
    let none = cat(vec![h32(0xDBC0_1002), h32(1), h32(0), h32(4), h32(16), h32(0x0026_200A), h32(0)]);
    assert_eq!(parse(&texture(none)).unwrap().dropped_images, 0);
    assert_eq!(parse(&texture(bitmap_2x2())).unwrap().dropped_images, 0);
    // A pen whose brush is such a texture brush.
    let pen = |image: Vec<u8>| {
        let brush = cat(vec![h32(0xDBC0_1002), h32(2), h32(0), h32(0), image]);
        let pen = cat(vec![h32(0xDBC0_1002), h32(0), h32(0), h32(0), f32s(&[1.0]), brush]);
        emf(&[plus_comment(&[plus_header(false), prec(0x4008, 0x0200 | 5, &pen)])], [0; 4], [0; 4])
    };
    let short = cat(vec![h32(0xDBC0_1002), h32(1), h32(4), h32(4), h32(16), h32(0x0026_200A), h32(0), vec![1, 2, 3]]);
    assert_eq!(parse(&pen(short)).unwrap().dropped_images, 1);
    let none = cat(vec![h32(0xDBC0_1002), h32(1), h32(0), h32(4), h32(16), h32(0x0026_200A), h32(0)]);
    assert_eq!(parse(&pen(none)).unwrap().dropped_images, 0);
    assert_eq!(parse(&pen(bitmap_2x2())).unwrap().dropped_images, 0);
}

#[test]
fn an_emfplus_image_refused_by_a_limit_is_counted_unless_the_crop_misses_it() {
    // A PNG one pixel wider than the compressed-side cap: its header is read, its pixels are not.
    let side = crate::dib::MAX_COMPRESSED_SIDE;
    let wide = png(side + 1, 1, [0; 4]);
    let obj = cat(vec![h32(0xDBC0_1002), h32(1), h32(0), h32(0), h32(0), h32(0), h32(1), wide]);
    let drawn = |sx: f32| {
        let draw = prec(0x401A, 2, &cat(vec![h32(0), h32(2), f32s(&[sx, 0.0, 1.0, 1.0]), f32s(&[0.0, 0.0, 40.0, 20.0])]));
        let p = parse(&emf(&[plus_comment(&[plus_header(false), prec(0x4008, 0x0500 | 2, &obj), draw])], [0; 4], [0; 4])).unwrap();
        (p.items.len(), p.dropped_images)
    };
    assert_eq!(drawn(0.0), (0, 1));
    assert_eq!(drawn(side as f32 + 10.0), (0, 0), "a source rectangle past the right edge");
}

#[test]
fn raster_operations_that_are_not_played_are_counted() {
    let copied = |rop: u32| {
        let mut rec = stretch_dibits(1, 1, 0, &[10, 20, 30, 255]);
        rec[68..72].copy_from_slice(&rop.to_le_bytes());
        let p = parse_emf(&[rec]);
        (p.items.len(), p.unplayed_raster_ops)
    };
    // SRCCOPY, SRCAND, SRCPAINT and SRCINVERT are played.
    for rop in [0x00CC_0020, 0x0088_00C6, 0x00EE_0086, 0x0066_0046] {
        assert_eq!(copied(rop), (1, 0), "{rop:#x}");
    }
    // NOTSRCCOPY and MERGEPAINT are drawn as the source.
    assert_eq!(copied(0x0033_0008), (1, 1));
    assert_eq!(copied(0x00BB_0226), (1, 1));
    // A bitmap that is clipped away is not drawn, so nothing is copied.
    let mut rec = stretch_dibits(1, 1, 0, &[10, 20, 30, 255]);
    rec[68..72].copy_from_slice(&0x0033_0008u32.to_le_bytes());
    let recs = [erec(59, &[]), erec(60, &[]), erec(67, &h32(1)), rec];
    assert_eq!(parse_emf(&recs).unplayed_raster_ops, 0);
    // WMF STRETCHDIB with NOTSRCCOPY.
    let mut file = stretch_dib(dib24_bottom_up());
    let at = file.windows(4).position(|w| w == 0x00CC_0020u32.to_le_bytes()).unwrap();
    file[at..at + 4].copy_from_slice(&0x0033_0008u32.to_le_bytes());
    assert_eq!(parse(&file).unwrap().unplayed_raster_ops, 1);
    // Operations without a source: BLACKNESS is played; DSTINVERT and PATINVERT are left out.
    let bitblt = |rop: u32| {
        let mut payload = vec![0u8; 92];
        payload[16..32].copy_from_slice(&cat(vec![e32(0), e32(0), e32(10), e32(10)]));
        payload[32..36].copy_from_slice(&rop.to_le_bytes());
        let p = parse_emf(&[erec(76, &payload)]);
        (p.items.len(), p.unplayed_raster_ops)
    };
    assert_eq!(bitblt(0x0000_0042), (1, 0));
    assert_eq!(bitblt(0x0055_0009), (0, 1));
    assert_eq!(bitblt(0x005A_0049), (0, 1));
    assert_eq!(bitblt(0x00CC_0020), (0, 0), "a copy without a bitmap draws nothing and leaves nothing out");
    // STRETCHDIBITS with an operation without a source fills the destination, with or without bits.
    assert_eq!(copied(0x0000_0042), (1, 0), "BLACKNESS with bits");
    assert_eq!(copied(0x0055_0009), (0, 1), "DSTINVERT with bits");
    let mut bare = stretch_dibits(1, 1, 0, &[]);
    bare[68..72].copy_from_slice(&0x00FF_0062u32.to_le_bytes());
    assert_eq!(parse_emf(&[bare]).items.len(), 1, "WHITENESS without bits");
    // WMF STRETCHDIB with BLACKNESS fills instead of drawing the bitmap.
    let mut file = stretch_dib(dib24_bottom_up());
    let at = file.windows(4).position(|w| w == 0x00CC_0020u32.to_le_bytes()).unwrap();
    file[at..at + 4].copy_from_slice(&0x0000_0042u32.to_le_bytes());
    let p = parse(&file).unwrap();
    assert!(matches!(p.items.as_slice(), [Item::Path { fill: Some([0, 0, 0, 255]), .. }]), "{:?}", p.items);
    let patblt = |rop: u32| {
        let p = parse(&wmf(&[wrec(0x061D, &cat(vec![h32(rop), h16(10), h16(10), h16(0), h16(0)]))], None)).unwrap();
        (p.items.len(), p.unplayed_raster_ops)
    };
    assert_eq!(patblt(0x00FF_0062), (1, 0));
    assert_eq!(patblt(0x0055_0009), (0, 1));
    assert!(!crate::dib::uses_source(0x00F0_0021) && crate::dib::uses_source(0x00CC_0020) && crate::dib::uses_source(0x00BB_0226));
}

#[test]
fn clips_that_are_not_applied_are_counted() {
    let ignored = |recs: Vec<Vec<u8>>| parse_emf(&recs).ignored_clips;
    let after = |mode: u32| {
        let mut recs = clip_to_rect(0, 0, 10, 10, mode);
        recs.push(rect(0, 0, 100, 100));
        recs
    };
    assert_eq!(ignored(after(1)), 0, "RGN_AND is applied");
    assert_eq!(ignored(after(5)), 0, "RGN_COPY is applied");
    assert_eq!(ignored(after(3)), 1, "RGN_XOR");
    assert_eq!(ignored(after(4)), 1, "RGN_DIFF");
    assert_eq!(ignored(after(2)), 0, "RGN_OR on the whole surface leaves the whole surface");
    assert_eq!(ignored(after(0)), 0, "an invalid mode selects nothing");
    let mut recs = clip_to_rect(0, 0, 50, 50, 1);
    recs.extend(after(2));
    assert_eq!(ignored(recs), 1, "RGN_OR on a clip path is counted, whether or not it widens it");
    let recs = vec![erec(59, &[]), erec(60, &[]), erec(67, &h32(4)), rect(0, 0, 100, 100)];
    assert_eq!(ignored(recs), 0, "an empty path cuts nothing");
    let line = vec![erec(59, &[]), move_to(0, 0), line_to(50, 50), move_to(60, 0), erec(60, &[]), erec(67, &h32(3)), rect(0, 0, 100, 100)];
    assert_eq!(ignored(line), 0, "a path on one line encloses nothing");
}

#[test]
fn emfplus_clips_that_are_not_applied_are_counted() {
    // The combine mode is in bits 8 to 11 of the flags.
    let plus = |recs: Vec<Vec<u8>>| {
        let mut all = vec![plus_header(false)];
        all.extend(recs);
        parse(&emf(&[plus_comment(&all)], [0; 4], [0; 4])).unwrap().ignored_clips
    };
    let clip = |mode: u16, r: [f32; 4]| prec(0x4032, mode << 8, &f32s(&r));
    for mode in 3..=5 {
        assert_eq!(plus(vec![clip(0, [0.0, 0.0, 50.0, 50.0]), clip(mode, [10.0, 10.0, 10.0, 10.0])]), 1, "mode {mode}");
        assert_eq!(plus(vec![clip(mode, [10.0, 10.0, 0.0, 10.0])]), u32::from(mode == 5), "mode {mode} with no area");
        let empty_on_clip = vec![clip(0, [0.0, 0.0, 50.0, 50.0]), clip(mode, [10.0, 10.0, 0.0, 10.0])];
        assert_eq!(plus(empty_on_clip), u32::from(mode == 5), "mode {mode} with no area on a clip");
    }
    assert_eq!(plus(vec![clip(0, [0.0, 0.0, 50.0, 50.0]), clip(2, [10.0, 10.0, 10.0, 10.0])]), 0, "a union inside the clip");
    assert_eq!(plus(vec![clip(0, [0.0, 0.0, 50.0, 50.0]), clip(2, [60.0, 60.0, 10.0, 10.0])]), 1, "a union apart from it");
    assert_eq!(plus(vec![clip(2, [60.0, 60.0, 10.0, 10.0])]), 0, "a union with no clip set");
    assert_eq!(plus(vec![clip(0, [0.0, 0.0, 10.0, 10.0]), clip(2, [10.0, 0.0, 10.0, 10.0])]), 0, "a union that is its rectangle");
    // A region whose area is a stand-in: an XOR node, unless its right area is empty.
    let node = |r: [f32; 4]| cat(vec![h32(0x1000_0000), f32s(&r)]);
    let region = |mode: u32, right: Vec<u8>| {
        let obj = cat(vec![h32(0xDBC0_1002), h32(2), h32(mode), node([0.0, 0.0, 30.0, 30.0]), right]);
        vec![prec(0x4008, 0x0400 | 6, &obj), prec(0x4034, 6, &[])]
    };
    assert_eq!(plus(region(3, node([10.0, 10.0, 90.0, 90.0]))), 1, "an XOR region");
    assert_eq!(plus(region(3, h32(0x1000_0002))), 0, "XOR with an empty region");
    let empty_pair = cat(vec![h32(0xDBC0_1002), h32(3), h32(5), h32(0x1000_0002), h32(0x1000_0002)]);
    assert_eq!(plus(vec![prec(0x4008, 0x0400 | 6, &empty_pair), prec(0x4034, 6, &[])]), 0, "the complement of two empty regions");
    let cut_empty = vec![clip(0, [0.0, 0.0, 0.0, 10.0]), clip(4, [0.0, 0.0, 10.0, 10.0])];
    assert_eq!(plus(cut_empty), 0, "an exclude from an empty clip");
    assert_eq!(plus(region(1, node([10.0, 10.0, 90.0, 90.0]))), 0, "an intersection");
    assert_eq!(plus(region(2, node([30.0, 0.0, 30.0, 30.0]))), 0, "a union that is its rectangle");
    assert_eq!(plus(region(2, node([40.0, 40.0, 30.0, 30.0]))), 1, "a union drawn as a larger rectangle");
    // A clip path that is not one convex figure is drawn as its rectangle.
    let path = |pts: &[f32], types: Vec<u8>| {
        let obj = cat(vec![h32(0xDBC0_1002), h32(types.len() as u32), h32(0), f32s(pts), types]);
        vec![prec(0x4008, 0x0300 | 9, &obj), prec(0x4033, 9, &[])]
    };
    assert_eq!(plus(path(&[0.0, 50.0, 50.0, 0.0, 100.0, 50.0, 50.0, 100.0], vec![0, 1, 1, 0x81])), 0, "a convex path");
    let l_shape = [0.0, 0.0, 20.0, 0.0, 20.0, 10.0, 10.0, 10.0, 10.0, 20.0, 0.0, 20.0];
    assert_eq!(plus(path(&l_shape, vec![0, 1, 1, 1, 1, 0x81])), 1, "an L shape");
    // A stand-in that cannot show: intersected with an empty clip, or added to an unlimited one.
    let combined = |mode: u16, first: Vec<Vec<u8>>| {
        let mut recs = first;
        recs.extend(path(&l_shape, vec![0, 1, 1, 1, 1, 0x81]));
        let last = recs.len() - 1;
        recs[last] = prec(0x4033, (mode << 8) | 9, &[]);
        plus(recs)
    };
    assert_eq!(combined(1, vec![clip(0, [0.0, 0.0, 0.0, 10.0])]), 0, "an L shape inside an empty clip");
    assert_eq!(combined(2, Vec::new()), 0, "an L shape added to no clip");
    assert_eq!(combined(1, Vec::new()), 1, "an L shape inside no clip");
    let inf = h32(0x1000_0003);
    let xor = cat(vec![h32(3), node([0.0, 0.0, 30.0, 30.0]), node([10.0, 10.0, 90.0, 90.0])]);
    let obj = cat(vec![h32(0xDBC0_1002), h32(4), h32(2), xor, inf]);
    assert_eq!(plus(vec![prec(0x4008, 0x0400 | 6, &obj), prec(0x4034, 6, &[])]), 0, "an XOR region added to an unlimited one");
    assert_eq!(plus(vec![clip(1, [0.0, 0.0, 50.0, 50.0]), clip(0, [1.0, 1.0, 5.0, 5.0])]), 0, "intersect and replace are applied");
}

#[test]
fn an_emfplus_clip_on_a_diagonal_line_encloses_nothing() {
    let fill = prec(0x400A, 0x8000, &cat(vec![h32(0xFF00_0000), h32(1), f32s(&[0.0, 0.0, 100.0, 100.0])]));
    let drawn = |clip: Vec<Vec<u8>>| {
        let mut all = vec![plus_header(false)];
        all.extend(clip);
        all.push(fill.clone());
        let p = parse(&emf(&[plus_comment(&all)], [0; 4], [0; 4])).unwrap();
        (p.items.len(), p.ignored_clips)
    };
    let line = cat(vec![h32(0xDBC0_1002), h32(2), h32(0), f32s(&[0.0, 0.0, 10.0, 10.0]), vec![0, 1]]);
    // SetClipPath with the line replaces the clip with nothing.
    assert_eq!(drawn(vec![prec(0x4008, 0x0300 | 9, &line), prec(0x4033, 9, &[])]), (0, 0));
    // A region whose one node is the line.
    let node = cat(vec![h32(0x1000_0001), h32(line.len() as u32), line]);
    let region = cat(vec![h32(0xDBC0_1002), h32(1), node]);
    assert_eq!(drawn(vec![prec(0x4008, 0x0400 | 6, &region), prec(0x4034, 6, &[])]), (0, 0));
}

#[test]
fn a_parse_stopped_at_a_limit_is_truncated() {
    assert!(!parse_emf(&[rect(0, 0, 10, 10)]).truncated);
    // SAVEDC past the stack depth.
    let mut recs = vec![erec(33, &[]); crate::MAX_STACK + 1];
    recs.push(rect(0, 0, 10, 10));
    let p = parse_emf(&recs);
    assert!(p.truncated && p.items.is_empty());
    let recs = vec![erec(33, &[]); crate::MAX_STACK];
    assert!(!parse_emf(&recs).truncated, "a full stack is not past it");
    // One clip path past the nesting cap.
    let mut recs = Vec::new();
    for _ in 0..=MAX_CLIP_PATHS {
        recs.extend(clip_to_rect(0, 0, 100, 100, 1));
    }
    assert!(parse_emf(&recs).truncated);
}

#[test]
fn placed_paths_carry_their_line_style_and_shared_clips() {
    let mut recs = clip_to_rect(0, 0, 50, 50, 1);
    recs.extend([ext_pen(PS_GEOMETRIC | 7 | 0x100, 2, &[6, 3]), erec(37, &h32(1)), rect(0, 0, 100, 100), rect(10, 10, 20, 20)]);
    let p = parse_emf(&recs);
    assert_eq!((p.width, p.height), (100.0, 100.0));
    let placed = p.place(kurbo::Rect::new(0.0, 0.0, 200.0, 200.0), [0.0; 4]);
    let (PlacedItem::Path { line, clips: a, stroke, .. }, PlacedItem::Path { clips: b, .. }) = (&placed[0], &placed[1]) else {
        panic!("expected two paths")
    };
    assert_eq!(stroke.map(|s| s.1), Some(4.0));
    assert_eq!((line.cap, &line.dash), (Cap::Square, &vec![12.0, 6.0]), "dashes scale with the width");
    assert!(Arc::ptr_eq(&a[0], &b[0]));
    assert_eq!(kurbo::Shape::bounding_box(&a[0].path), kurbo::Rect::new(0.0, 0.0, 100.0, 100.0));
    assert!(a[0].even_odd);
}

#[test]
fn a_turned_bitmap_keeps_its_pixels_and_carries_the_turn() {
    // A quarter turn (x' = -y, y' = x) moved 10 right: the 30 x 10 destination stands 10 x 30.
    let xf: Vec<u8> = [0.0f32, 1.0, -1.0, 0.0, 10.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
    let pixels = png(3, 1, [9, 8, 7, 255]);
    let p = parse(&emf(&[erec(35, &xf), stretch_dibits(3, 1, 5, &pixels)], [0, 0, 10, 30], [0, 0, 2540, 7620])).unwrap();
    let Item::Bitmap { transform, width, height, .. } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!((*width, *height), (3, 1), "the pixels are not resampled");
    // 4 device pixels a millimetre and a 25.4 x 76.2 mm frame of 72 x 216 points: 0.709 points a pixel.
    let k = 72.0 / 101.6;
    let want = [0.0, 30.0 * k, -10.0 * k, 0.0, 10.0 * k, 0.0];
    assert!(transform.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9), "{transform:?}");
    let placed = p.place(kurbo::Rect::new(0.0, 0.0, 72.0, 216.0), [0.0; 4]);
    let PlacedItem::Bitmap { transform, .. } = &placed[0] else { panic!("expected a bitmap") };
    let corner = *transform * kurbo::Point::new(1.0, 1.0);
    assert!((corner.x - 0.0).abs() < 1e-6 && (corner.y - 30.0 * k).abs() < 1e-6, "{corner:?}");
}

#[test]
fn compressed_bitmaps_share_one_decode_budget_per_file() {
    let small = png(3, 1, [0, 0, 0, 255]);
    crate::dib::set_decode_budget(7);
    assert!(crate::dib::decode_compressed(&small).is_some());
    assert!(crate::dib::decode_compressed(&small).is_some());
    assert!(crate::dib::decode_compressed(&small).is_none(), "one pixel left: the third image is not decoded");
    // Every parse starts with the whole budget again.
    let p = parse_emf(&[stretch_dibits(3, 1, 5, &small)]);
    assert_eq!(p.items.len(), 1);
}

#[test]
fn truncated_and_mutated_files_with_these_records_never_panic() {
    let mut recs = vec![
        ext_pen(PS_GEOMETRIC | 7 | 0x200 | 0x2000, 4, &[8, 4, 2, 1]),
        erec(37, &h32(1)),
        erec(58, &h32(3)),
        erec(57, &h32(2)),
        erec(17, &h32(8)),
        erec(9, &cat(vec![e32(100), e32(100)])),
        erec(11, &cat(vec![e32(100), e32(100)])),
        erec(31, &cat(vec![e32(2), e32(1), e32(3), e32(1)])),
        erec(32, &cat(vec![e32(1), e32(2), e32(1), e32(3)])),
        erec(33, &[]),
    ];
    recs.extend(clip_to_rect(0, 0, 50, 50, 1));
    recs.extend(clip_to_rect(5, 5, 40, 40, 5));
    recs.extend([
        erec(45, &cat(vec![e32(0), e32(0), e32(100), e32(100), e32(100), e32(50), e32(50), e32(0)])),
        erec(59, &[]),
        move_to(0, 0),
        line_to(10, 0),
        line_to(10, 10),
        erec(61, &[]),
        erec(60, &[]),
        erec(63, &[0; 16]),
        erec(59, &[]),
        rect(0, 0, 5, 5),
        erec(68, &[]),
        stretch_dibits(3, 1, 5, &png(3, 1, [1, 2, 3, 255])),
        erec(34, &e32(-1)),
        erec(200, &[0; 8]),
    ]);
    let efile = emf(&recs, [0, 0, 100, 100], [0, 0, 2540, 2540]);
    let mut core = cat(vec![h32(12), h16(2), h16(2), h16(1), h16(24)]);
    core.extend(dib24_bottom_up().get(40..).unwrap_or_default());
    let wfile = stretch_dib(core);
    assert!(!parse(&efile).unwrap().items.is_empty() && !parse(&wfile).unwrap().items.is_empty());
    let mut rng = Lcg(0x9E37_79B9_7F4A_7C15);
    for file in [&efile, &wfile] {
        for n in 0..=file.len() {
            let _ = parse(&file[..n]);
            let _ = crate::size(&file[..n]);
        }
        for at in 0..file.len() {
            for v in [0x00, 0x7F, 0x80, 0xFF] {
                let mut bad = file.clone();
                bad[at] = v;
                let _ = parse(&bad);
            }
        }
        for _ in 0..2000 {
            let mut bad = file.clone();
            for _ in 0..6 {
                let at = rng.rnd() as usize % bad.len();
                bad[at] = rng.rnd() as u8;
            }
            if let Ok(p) = parse(&bad) {
                let _ = p.place(kurbo::Rect::new(0.0, 0.0, 100.0, 100.0), [0.0; 4]);
            }
        }
    }
}

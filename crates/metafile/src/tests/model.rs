//! Line styles, clip paths and the device-context records behind them, the DIB forms and the
//! header fields that came with VectorCraft's reader.

use std::sync::Arc;

use super::*;
use crate::canvas::{MAX_CLIP_PATHS, MAX_DASH_ENTRIES};
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
fn dash_lengths_follow_the_mapping_like_the_width() {
    // A world transform of 2: the width and the dashes double.
    let xf: Vec<u8> = [2.0f32, 0.0, 0.0, 2.0, 0.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
    let recs = [erec(35, &xf), ext_pen(PS_GEOMETRIC | 7, 4, &[8, 4]), erec(37, &h32(1)), move_to(0, 0), line_to(100, 0)];
    let p = parse_emf(&recs);
    let (_, _, stroke, _) = path_segs(&p.items[0]);
    assert_eq!(stroke, Some(([255, 0, 0, 255], 8.0)));
    assert_eq!(line_of(&p.items[0]).dash, vec![16.0, 8.0]);
}

#[test]
fn pen_styles_become_dash_patterns_of_the_pen_width() {
    // CREATEPEN: slot, style, width (x, y), colour.
    let pen = |style: u32, width: u32| erec_at(38, 28, &[(8, 1), (12, style), (16, width)], &[]);
    let draw = |style: u32, width: u32| {
        let p = parse_emf(&[pen(style, width), erec(37, &h32(1)), move_to(0, 0), line_to(100, 0)]);
        line_of(&p.items[0]).clone()
    };
    assert_eq!(draw(0, 2), LineStyle::default(), "PS_SOLID: round caps and joins, miter limit 10, no dashes");
    assert_eq!(draw(1, 2).dash, vec![6.0, 2.0], "PS_DASH");
    assert_eq!(draw(2, 2).dash, vec![2.0, 2.0], "PS_DOT");
    assert_eq!(draw(3, 2).dash, vec![6.0, 2.0, 2.0, 2.0], "PS_DASHDOT");
    assert_eq!(draw(4, 2).dash, vec![6.0, 2.0, 2.0, 2.0, 2.0, 2.0], "PS_DASHDOTDOT");
    // A cosmetic pen's dashes are in device pixels.
    assert_eq!(draw(2, 0).dash, vec![1.0, 1.0]);
    // PS_INSIDEFRAME and unknown styles are solid.
    assert!(draw(6, 2).dash.is_empty());
}

#[test]
fn a_dash_pattern_that_draws_nothing_is_a_solid_line() {
    let draw = |entries: &[u32]| {
        let p = parse_emf(&[ext_pen(PS_GEOMETRIC | 7, 2, entries), erec(37, &h32(1)), move_to(0, 0), line_to(100, 0)]);
        line_of(&p.items[0]).dash.clone()
    };
    assert!(draw(&[0, 0]).is_empty());
    assert!(draw(&[]).is_empty());
    assert_eq!(draw(&[0, 5]), vec![0.0, 5.0], "a zero dash with round caps draws dots");
}

#[test]
fn a_pen_keeps_at_most_sixteen_style_entries() {
    let entries: Vec<u32> = (1..=100).collect();
    let p = parse_emf(&[ext_pen(PS_GEOMETRIC | 7, 1, &entries), erec(37, &h32(1)), move_to(0, 0), line_to(100, 0)]);
    assert_eq!(line_of(&p.items[0]).dash.len(), MAX_DASH_ENTRIES);
    // A count that runs past the record keeps the entries the record holds.
    let mut rec = ext_pen(PS_GEOMETRIC | 7, 1, &[3, 2]);
    rec[48..52].copy_from_slice(&1000u32.to_le_bytes());
    let p = parse_emf(&[rec, erec(37, &h32(1)), move_to(0, 0), line_to(100, 0)]);
    assert_eq!(line_of(&p.items[0]).dash, vec![3.0, 2.0]);
}

#[test]
fn wmf_pen_style_word_carries_cap_and_join() {
    // CREATEPENINDIRECT: style (PS_SOLID, flat caps, bevel joins), width (x, y), colour.
    let pen = wrec(0x02FA, &cat(vec![h16(0x1200), h16(3), h16(0), h32(0)]));
    let recs = [pen, wrec(0x012D, &h16(0)), wrec(0x0214, &cat(vec![h16(0), h16(0)])), wrec(0x0213, &cat(vec![h16(0), h16(50)]))];
    let p = parse(&wmf(&recs, None)).unwrap();
    let line = line_of(&p.items[0]);
    assert_eq!((line.cap, line.join), (Cap::Flat, Join::Bevel));
}

#[test]
fn restore_dc_brings_back_the_miter_limit() {
    let pen = ext_pen(PS_GEOMETRIC | 0x2000, 2, &[]);
    let recs =
        [pen, erec(37, &h32(1)), erec(33, &[]), erec(58, &h32(4)), move_to(0, 0), line_to(9, 0), erec(34, &e32(-1)), move_to(0, 5), line_to(9, 5)];
    let p = parse_emf(&recs);
    assert_eq!(line_of(&p.items[0]).miter_limit, 4.0);
    assert_eq!(line_of(&p.items[1]).miter_limit, 10.0);
    // A limit below 1 is invalid: the limit stays.
    let p = parse_emf(&[ext_pen(PS_GEOMETRIC | 0x2000, 2, &[]), erec(37, &h32(1)), erec(58, &h32(0)), move_to(0, 0), line_to(9, 0)]);
    assert_eq!(line_of(&p.items[0]).miter_limit, 10.0);
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
fn clip_modes_that_add_or_cut_leave_the_clip_and_a_region_copy_drops_clip_paths() {
    for mode in [2, 3, 4, 0, 99] {
        let mut recs = clip_to_rect(0, 0, 10, 10, mode);
        recs.push(rect(0, 0, 100, 100));
        let p = parse_emf(&recs);
        assert!(clips_of(&p.items[0]).is_empty(), "mode {mode}");
    }
    // EXTSELECTCLIPRGN with no region data selects the default clip: no region, no clip paths.
    let mut recs = clip_to_rect(0, 0, 10, 10, 1);
    recs.push(erec(75, &cat(vec![h32(0), h32(5)])));
    recs.push(rect(0, 0, 100, 100));
    let p = parse_emf(&recs);
    assert!(clips_of(&p.items[0]).is_empty());
}

#[test]
fn an_empty_clip_path_clips_everything_away() {
    let recs = [erec(59, &[]), erec(60, &[]), erec(67, &h32(1)), rect(0, 0, 100, 100)];
    assert!(parse_emf(&recs).items.is_empty());
}

#[test]
fn one_clip_path_past_the_cap_stops_the_parse() {
    let nested = |n: usize| {
        let mut recs = Vec::new();
        for _ in 0..n {
            recs.extend(clip_to_rect(0, 0, 50, 50, 1));
        }
        recs.push(rect(0, 0, 100, 100));
        parse_emf(&recs)
    };
    let p = nested(MAX_CLIP_PATHS);
    assert_eq!(clips_of(&p.items[0]).len(), MAX_CLIP_PATHS);
    assert!(nested(MAX_CLIP_PATHS + 1).items.is_empty(), "what follows can no longer be clipped as the file asks");
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
fn abort_path_discards_the_bracket() {
    let recs = [erec(59, &[]), rect(0, 0, 10, 10), erec(68, &[]), erec(62, &[0; 16]), rect(0, 0, 5, 5)];
    let p = parse_emf(&recs);
    assert_eq!(p.items.len(), 1, "only the rectangle drawn after the bracket was aborted");
    assert_eq!((p.width, p.height), (5.0, 5.0));
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
fn scale_viewport_and_window_ext_change_the_mapping() {
    let setup = [erec(17, &h32(8)), erec(9, &cat(vec![e32(100), e32(100)])), erec(11, &cat(vec![e32(100), e32(100)]))];
    let scaled = |typ: u32, v: [i32; 4]| {
        let mut recs = setup.to_vec();
        recs.push(erec(typ, &cat(v.iter().map(|x| e32(*x)).collect())));
        recs.push(rect(0, 0, 10, 10));
        let p = parse_emf(&recs);
        (p.width, p.height)
    };
    assert_eq!(scaled(31, [2, 1, 3, 1]), (20.0, 30.0), "SCALEVIEWPORTEXTEX");
    assert_eq!(scaled(32, [2, 1, 5, 1]), (5.0, 2.0), "SCALEWINDOWEXTEX");
    assert_eq!(scaled(31, [2, 0, 3, 1]), (10.0, 10.0), "a zero denominator leaves the extent");
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
fn records_of_unknown_types_are_counted_and_known_ones_are_not() {
    // 200 is no EMF record; 21 (SETSTRETCHBLTMODE) and 70 (a plain comment) change nothing kept.
    let recs = [rect(0, 0, 10, 10), erec(200, &[0; 8]), erec(21, &h32(1)), erec(70, &cat(vec![h32(4), h32(0)])), erec(201, &[])];
    assert_eq!(parse_emf(&recs).skipped_records, 2);
    let rect16 = wrec(0x041B, &cat(vec![h16(10), h16(10), h16(0), h16(0)]));
    let recs = [rect16, wrec(0x0999, &[0; 4]), wrec(0x0107, &h16(1))];
    assert_eq!(parse(&wmf(&recs, None)).unwrap().skipped_records, 1);
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
fn png_and_jpeg_dibs_decode() {
    let p = parse_emf(&[stretch_dibits(3, 1, 5, &png(3, 1, [1, 2, 3, 255]))]);
    let Item::Bitmap { rect, width, height, rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!((*width, *height, rect), (3, 1, &[0.0, 0.0, 30.0, 10.0]));
    assert_eq!(rgba.as_slice(), [1, 2, 3, 255].repeat(3));
    let mut jpeg = Vec::new();
    let img = image::RgbImage::from_pixel(8, 8, image::Rgb([200, 200, 200]));
    image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
    let p = parse_emf(&[stretch_dibits(8, 8, 4, &jpeg)]);
    let Item::Bitmap { width, height, rgba, .. } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!((*width, *height), (8, 8));
    assert!(rgba.as_chunks::<4>().0.iter().all(|c| c[0].abs_diff(200) < 4 && c[3] == 255));
    // Bits that are no image, and a truncated file, draw nothing.
    assert!(parse_emf(&[stretch_dibits(3, 1, 5, &[1, 2, 3, 4])]).items.is_empty());
    let whole = png(3, 1, [1, 2, 3, 255]);
    assert!(parse_emf(&[stretch_dibits(3, 1, 5, &whole[..whole.len() / 2])]).items.is_empty());
}

#[test]
fn a_compressed_dib_over_the_side_cap_is_refused_before_decoding() {
    let side = crate::dib::MAX_COMPRESSED_SIDE;
    assert!(crate::dib::decode_compressed(&png(side, 1, [0; 4])).is_some());
    assert!(crate::dib::decode_compressed(&png(side + 1, 1, [0; 4])).is_none());
    // A side no renderer draws (MAX_BITMAP_SIDE) makes no item either.
    let wide = png(crate::MAX_BITMAP_SIDE + 1, 1, [0; 4]);
    assert!(parse_emf(&[stretch_dibits(crate::MAX_BITMAP_SIDE as i32 + 1, 1, 5, &wide)]).items.is_empty());
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
fn an_item_under_a_clip_far_off_the_page_is_left_out() {
    let clip = Arc::new(ClipPath { segs: vec![Seg::Move(0.0, 0.0), Seg::Line(f32::MAX, 0.0), Seg::Line(0.0, 10.0), Seg::Close], even_odd: false });
    let bitmap = |clips| Item::Bitmap {
        rect: [0.0, 0.0, 10.0, 10.0],
        transform: [10.0, 0.0, 0.0, 10.0, 0.0, 0.0],
        width: 1,
        height: 1,
        rgba: vec![0; 4],
        region: None,
        clips,
    };
    let pic = crate::Picture { width: 10.0, height: 10.0, items: vec![bitmap(vec![clip]), bitmap(Vec::new())], skipped_records: 0 };
    let placed = pic.place(kurbo::Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]);
    assert_eq!(placed.len(), 1);
}

#[test]
fn an_upright_bitmap_has_its_rectangle_as_its_transform() {
    let p = parse(&stretch_dib(dib24_bottom_up())).unwrap();
    let Item::Bitmap { rect, transform, .. } = &p.items[0] else { panic!("expected a bitmap") };
    assert_eq!((rect, transform), (&[0.0, 0.0, 20.0, 10.0], &[20.0, 0.0, 0.0, 10.0, 0.0, 0.0]));
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

#[test]
fn a_turned_bitmaps_rectangle_is_the_box_around_it() {
    let xf: Vec<u8> = [0.0f32, 1.0, -1.0, 0.0, 10.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
    let p = parse(&emf(&[erec(35, &xf), stretch_dibits(3, 1, 5, &png(3, 1, [9, 8, 7, 255]))], [0, 0, 10, 30], [0, 0, 2540, 7620])).unwrap();
    let Item::Bitmap { rect, .. } = &p.items[0] else { panic!("expected a bitmap") };
    let k = 72.0 / 101.6;
    assert!(rect.iter().zip([0.0, 0.0, 10.0 * k, 30.0 * k]).all(|(a, b)| (f64::from(*a) - b).abs() < 1e-4), "{rect:?}");
}

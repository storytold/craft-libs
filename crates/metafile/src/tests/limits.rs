//! One test for each cap that the other test modules do not already hold to its value.

use super::*;
use crate::emfplus;

#[test]
fn strings_are_cut_at_the_character_cap() {
    let cap = crate::text::MAX_CHARS;
    // EXTTEXTOUTW: the count says more than the cap; the string holds that many.
    let units = vec![0x41u16; cap + 100];
    let p = parse(&emf(&[etext(true, 0, 0, &units, 0, &[])], [0, 0, 200, 100], [0, 0, 5000, 2500])).unwrap();
    assert_eq!(texts(&p)[0].text.chars().count(), cap);
    // META_EXTTEXTOUT and META_TEXTOUT.
    let bytes = vec![b'a'; cap + 100];
    let p = wmf200(&[wtext(0, 0, &bytes, 0, None, &[])]);
    assert_eq!(texts(&p)[0].text.chars().count(), cap);
    let mut v = h16((cap + 100) as i32 as i16 as i32);
    v.extend(&bytes);
    v.extend(cat(vec![h16(0), h16(0)]));
    let p = wmf200(&[wrec(0x0521, &v)]);
    assert!(texts(&p).iter().all(|t| t.text.chars().count() <= cap));
}

#[test]
fn emfplus_strings_are_cut_at_their_character_cap() {
    let cap = emfplus::MAX_CHARS;
    let font = prec(0x4008, 0x0600 | 3, &font_obj("Calibri", 12.0, 2, 0));
    let long: String = "a".repeat(cap + 50);
    let s = prec(0x401C, 0x8000 | 3, &cat(vec![h32(0xFF00_0000), h32(9), h32((cap + 50) as u32), f32s(&[0.0, 0.0, 0.0, 0.0]), u16s(&long)]));
    let p = plus_pic(vec![font, s]);
    let total: usize = texts(&p).iter().map(|t| t.text.chars().count()).sum();
    assert!(total > 0 && total <= cap, "{total}");
}

#[test]
fn a_clip_region_keeps_at_most_its_rectangle_cap() {
    let cap = crate::clip::MAX_CLIP_RECTS;
    // EXTSELECTCLIPRGN (RGN_COPY) with twice the cap of separate rectangles in its RGNDATA.
    let n = cap * 2;
    let mut data = cat(vec![h32(32), h32(1), h32(n as u32), h32((n * 16) as u32), vec![0; 16]]);
    for i in 0..n as i32 {
        data.extend(cat(vec![e32(i * 10), e32(0), e32(i * 10 + 5), e32(5)]));
    }
    let rgn = erec(75, &cat(vec![h32(data.len() as u32), h32(5), data]));
    let draw = erec(43, &cat(vec![e32(0), e32(0), e32(10_000), e32(10)]));
    let p = parse(&emf(&[rgn, draw], [0; 4], [0; 4])).unwrap();
    let Item::Path { region: Some(r), .. } = &p.items[0] else { panic!("a clipped path") };
    assert_eq!(r.len(), cap);
}

#[test]
fn placing_stops_at_the_element_cap() {
    let segs: Vec<Seg> = std::iter::once(Seg::Move(0.0, 0.0)).chain((0..100_000).map(|i| Seg::Line((i % 7) as f32, (i % 5) as f32))).collect();
    let path = || Item::Path {
        segs: segs.clone(),
        fill: Some([0, 0, 0, 255]),
        stroke: None,
        line: crate::LineStyle::default(),
        even_odd: false,
        region: None,
        clips: Vec::new(),
    };
    let pic = crate::Picture { width: 10.0, height: 10.0, items: (0..8).map(|_| path()).collect(), skipped_records: 0 };
    let placed = pic.place(kurbo::Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]);
    // Five paths of 100,001 elements reach the 500,000 cap; the rest are left out.
    assert_eq!(placed.len(), 5);
}

#[test]
fn a_pattern_tile_over_the_side_cap_is_no_brush() {
    let side = crate::canvas::MAX_TILE_SIDE;
    let tile = |w: usize, h: usize| crate::canvas::Brush::pattern(Rgba { w, h, px: vec![0; w * h * 4] }, false);
    assert!(tile(side, side).is_some());
    assert!(tile(side + 1, 1).is_none());
    assert!(tile(1, side + 1).is_none());
}

#[test]
fn a_dib_palette_holds_at_most_256_entries() {
    // 24 bpp with a colour table of 1000 entries claimed: 256 are skipped before the bits.
    let mut info = info_header(1, 1, 24, 4);
    info[32..36].copy_from_slice(&1000u32.to_le_bytes());
    assert_eq!(crate::dib::info_len(crate::bytes::Bytes(&info)), Some(40 + 256 * 4));
}

/// `inner` as an EMF+ metafile image object drawn into (0, 0, 40, 20), in a file of its own.
fn wrap(inner: Vec<u8>) -> Vec<u8> {
    let obj = cat(vec![h32(0xDBC0_1002), h32(2), h32(3), h32(inner.len() as u32), inner]);
    emf(&[plus_comment(&[plus_header(false), prec(0x4008, 0x0500 | 3, &obj), draw_image(3, [0.0, 0.0, 40.0, 20.0])])], [0; 4], [0; 4])
}

#[test]
fn metafiles_nest_only_so_deep() {
    let nested = |levels: usize| {
        let mut file = emf(&[erec(43, &cat(vec![e32(0), e32(0), e32(10), e32(10)]))], [0; 4], [0; 4]);
        for _ in 0..levels {
            file = wrap(file);
        }
        parse(&file).map(|p| p.items.len()).unwrap_or(0)
    };
    assert_eq!(nested(emfplus::MAX_NESTING), 1);
    assert_eq!(nested(emfplus::MAX_NESTING + 1), 0, "the innermost picture is not read");
}

#[test]
fn emfplus_object_ids_past_the_table_are_ignored() {
    let draw = |id: u16| {
        let pen = prec(0x4008, 0x0200 | id, &solid_pen(0xFF00_FF00, 2.0));
        let lines = prec(0x400D, id, &cat(vec![h32(2), f32s(&[1.0, 1.0, 4.0, 2.0])]));
        plus_pic(vec![pen, lines]).items.len()
    };
    assert_eq!(draw(emfplus::MAX_OBJECTS as u16 - 1), 1);
    assert_eq!(draw(emfplus::MAX_OBJECTS as u16), 0);
}

#[test]
fn emfplus_saves_past_the_stack_cap_are_not_kept() {
    // Saves 0..=cap, a move, then a restore of the last save: it was not kept, so the move stays.
    let cap = emfplus::MAX_STACK as u32;
    let fill = || prec(0x400A, 0x8000, &cat(vec![h32(0xFF00_0000), h32(1), f32s(&[0.0, 0.0, 10.0, 10.0])]));
    let run = |saves: u32| {
        let mut recs: Vec<Vec<u8>> = (0..saves).map(|i| prec(0x4025, 0, &h32(i))).collect();
        recs.extend([fill(), prec(0x402D, 0, &f32s(&[100.0, 0.0])), prec(0x4026, 0, &h32(saves - 1)), fill()]);
        plus_pic(recs).width
    };
    assert_eq!(run(cap), 10.0, "the last save was kept: the restore undoes the move");
    assert_eq!(run(cap + 1), 110.0, "one save too many: nothing to restore");
}

#[test]
fn emfplus_point_lists_are_cut_at_the_point_cap() {
    let n = emfplus::MAX_POINTS + 10;
    let pts: Vec<f32> = (0..n).flat_map(|i| [(i % 100) as f32, (i % 37) as f32]).collect();
    let pen = prec(0x4008, 0x0200 | 3, &solid_pen(0xFF00_FF00, 1.0));
    let lines = prec(0x400D, 3, &cat(vec![h32(n as u32), f32s(&pts)]));
    let p = plus_pic(vec![pen, lines]);
    let (segs, ..) = path_segs(&p.items[0]);
    assert_eq!(segs.len(), emfplus::MAX_POINTS);
}

#[test]
fn a_continued_emfplus_object_reserves_nothing_for_the_size_it_claims() {
    // 4 GB claimed (cut to the cap), a few bytes sent: the object never completes and no memory
    // is set aside for it.
    let first = prec(0x4008, 0x8000 | 0x0500 | 1, &cat(vec![h32(u32::MAX), vec![1, 2, 3, 4]]));
    let p = plus_pic(vec![first.clone(), first, draw_image(1, [0.0, 0.0, 4.0, 4.0])]);
    assert!(p.items.is_empty());
}

#[test]
fn emfplus_gradients_keep_at_most_their_stop_cap() {
    // Preset colours (brush flag 4) with far more stops claimed than the data holds: the count is
    // cut to the cap first, so the read is bounded and the short data makes no brush.
    let cap = emfplus::MAX_STOPS as u32;
    let brush = |n: u32, held: u32| {
        let mut b =
            cat(vec![h32(0xDBC0_1002), h32(4), h32(4), h32(0), f32s(&[0.0, 0.0, 100.0, 10.0]), h32(0xFFFF_0000), h32(0xFF00_00FF), h32(0), h32(0)]);
        b.extend(h32(n));
        b.extend(f32s(&(0..held).map(|i| i as f32 / held as f32).collect::<Vec<_>>()));
        b.extend((0..held).flat_map(|_| h32(0xFF00_FF00)));
        b
    };
    let fill = || prec(0x400A, 0, &cat(vec![h32(5), h32(1), f32s(&[0.0, 0.0, 100.0, 10.0])]));
    let drawn = |n: u32, held: u32| plus_pic(vec![prec(0x4008, 0x0100 | 5, &brush(n, held)), fill()]).items.len();
    assert_eq!(drawn(cap, cap), 1);
    assert_eq!(drawn(u32::MAX, cap), 1, "a count past the cap reads the cap's worth");
    assert_eq!(drawn(u32::MAX, 4), 0, "and no more than the record holds");
}

#[test]
fn emfplus_regions_nest_only_so_deep_and_only_so_many_nodes() {
    let rect = || cat(vec![h32(0x1000_0000), f32s(&[0.0, 0.0, 30.0, 30.0])]);
    // A chain of intersections `depth` deep: and(and(and(..., rect), rect), rect).
    let chain = |depth: usize| {
        let mut body = Vec::new();
        for _ in 0..depth {
            body.extend(h32(1));
        }
        for _ in 0..=depth {
            body.extend(rect());
        }
        cat(vec![h32(0xDBC0_1002), h32(depth as u32), body])
    };
    let clipped = |region: Vec<u8>| {
        let fill = prec(0x400A, 0x8000, &cat(vec![h32(0xFF00_0000), h32(1), f32s(&[0.0, 0.0, 100.0, 100.0])]));
        plus_pic(vec![prec(0x4008, 0x0400 | 6, &region), prec(0x4034, 6, &[]), fill]).width
    };
    assert_eq!(clipped(chain(emfplus::MAX_REGION_DEPTH)), 30.0, "a region at the depth cap clips");
    assert_eq!(clipped(chain(emfplus::MAX_REGION_DEPTH + 1)), 100.0, "one deeper is no region: the fill is whole");
    // A balanced tree of intersections stays shallow and has 2^(levels + 1) - 1 nodes.
    fn tree(levels: usize, out: &mut Vec<u8>) {
        if levels == 0 {
            out.extend(cat(vec![h32(0x1000_0000), f32s(&[0.0, 0.0, 30.0, 30.0])]));
        } else {
            out.extend(h32(1));
            tree(levels - 1, out);
            tree(levels - 1, out);
        }
    }
    let balanced = |levels: usize| {
        let mut body = Vec::new();
        tree(levels, &mut body);
        cat(vec![h32(0xDBC0_1002), h32(0), body])
    };
    let levels = (emfplus::MAX_REGION_WORK + 1).ilog2() as usize;
    assert_eq!(clipped(balanced(levels - 1)), 30.0, "fewer nodes than the cap");
    assert_eq!(clipped(balanced(levels)), 100.0, "more nodes than the cap is no region");
}

#[test]
fn masking_a_bitmap_to_a_clip_stops_at_the_work_cap() {
    // A triangle clip over a bitmap: three corners times the pixels inside the clip's box.
    let tri = [(0.0, 0.0), (1000.0, 0.0), (0.0, 1000.0)];
    let side = |n: usize| crate::cliprect::crop([0.0, 0.0, 1000.0, 1000.0], Rgba { w: n, h: n, px: vec![255; n * n * 4] }, &tri).is_some();
    let cap = crate::cliprect::MAX_MASK_WORK;
    let fits = ((cap / 3) as f64).sqrt() as usize;
    assert!(side(fits));
    assert!(!side(fits + 2));
}

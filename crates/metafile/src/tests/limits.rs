//! One test for each cap that the other test modules do not already hold to its value.

use super::*;
use crate::emfplus;

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
    let pic = crate::Picture { width: 10.0, height: 10.0, items: (0..8).map(|_| path()).collect(), ..Default::default() };
    let placed = pic.place(kurbo::Rect::new(0.0, 0.0, 10.0, 10.0), [0.0; 4]);
    // Five paths of 100,001 elements reach the 500,000 cap; the rest are left out.
    assert_eq!(placed.len(), 5);
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
fn wmf_wrapped_emf_shares_nesting_depth_cap() {
    let square = emf(&[erec(43, &cat(vec![e32(0), e32(0), e32(10), e32(10)]))], [0; 4], [0; 4]);
    let wmf_wrap = |inner: Vec<u8>| wmf(&wmfc_chunks(&inner, &[inner.len()]), None);
    // WMF (0) -> EMF (1) -> EMF+ WMF (2) -> EMF (3): at MAX_NESTED_DEPTH, still drawn.
    let ok = wmf_wrap(wrap(wmf_wrap(square.clone())));
    let p_ok = parse(&ok).unwrap();
    assert_eq!(p_ok.items.len(), 1);
    assert!(!p_ok.truncated);
    // One more EMF+ level exceeds MAX_NESTED_DEPTH: stops cleanly without stack overflow.
    let too_deep = wmf_wrap(wrap(wmf_wrap(wrap(square))));
    let p_deep = parse(&too_deep).unwrap();
    assert_eq!(p_deep.items.len(), 0);
    assert_eq!(p_deep.dropped_images, 1);
}

#[test]
fn emfplus_images_that_are_not_read_are_counted() {
    let square = || emf(&[erec(43, &cat(vec![e32(0), e32(0), e32(10), e32(10)]))], [0; 4], [0; 4]);
    let nested = |levels: usize| {
        let mut file = square();
        for _ in 0..levels {
            file = wrap(file);
        }
        parse(&file).unwrap()
    };
    assert_eq!(nested(emfplus::MAX_NESTING).dropped_images, 0);
    assert_eq!(nested(emfplus::MAX_NESTING + 1).dropped_images, 1, "the picture past the nesting cap");
    // A bitmap image object whose pixels run past the record.
    let obj = cat(vec![h32(0xDBC0_1002), h32(1), h32(4), h32(4), h32(16), h32(0x0026_200A), h32(0), vec![1, 2, 3]]);
    let file = emf(&[plus_comment(&[plus_header(false), prec(0x4008, 0x0500 | 2, &obj), draw_image(2, [0.0, 0.0, 40.0, 20.0])])], [0; 4], [0; 4]);
    assert_eq!(parse(&file).unwrap().dropped_images, 1);
    // The same object drawn from a source rectangle beside its 4 x 4 pixels selects none of them.
    let beside = prec(0x401A, 2, &cat(vec![h32(0), h32(2), f32s(&[50.0, 0.0, 1.0, 1.0]), f32s(&[0.0, 0.0, 40.0, 20.0])]));
    let file = emf(&[plus_comment(&[plus_header(false), prec(0x4008, 0x0500 | 2, &obj), beside])], [0; 4], [0; 4]);
    assert_eq!(parse(&file).unwrap().dropped_images, 0);
    // What a nested picture leaves out is reported by the file that draws it.
    let inner = emf(&[stretch_bad_png(), erec(43, &cat(vec![e32(0), e32(0), e32(10), e32(10)]))], [0; 4], [0; 4]);
    assert_eq!(parse(&inner).unwrap().dropped_images, 1);
    assert_eq!(parse(&wrap(inner)).unwrap().dropped_images, 1);
}

#[test]
fn a_metafile_image_adds_its_counts_once_per_object() {
    // A square and one record of a type the reader does not know.
    let inner = emf(&[erec(43, &cat(vec![e32(0), e32(0), e32(10), e32(10)])), erec(200, &[0; 8])], [0; 4], [0; 4]);
    let obj = cat(vec![h32(0xDBC0_1002), h32(2), h32(3), h32(inner.len() as u32), inner]);
    let draw = || draw_image(3, [0.0, 0.0, 40.0, 20.0]);
    let store = || prec(0x4008, 0x0500 | 3, &obj);
    let skipped = |recs: Vec<Vec<u8>>| {
        let mut all = vec![plus_header(false)];
        all.extend(recs);
        let p = parse(&emf(&[plus_comment(&all)], [0; 4], [0; 4])).unwrap();
        (p.items.len(), p.skipped_records)
    };
    assert_eq!(skipped(vec![store()]), (0, 0), "an image that is never drawn");
    let cut = prec(0x401A, 3, &cat(vec![h32(0), h32(2), f32s(&[0.0, 0.0, 2.0, 2.0])]));
    assert_eq!(skipped(vec![store(), cut]), (0, 0), "a draw record without its destination");
    assert_eq!(skipped(vec![store(), draw()]), (1, 1));
    assert_eq!(skipped(vec![store(), draw(), draw(), draw()]), (3, 1), "drawn three times, counted once");
    assert_eq!(skipped(vec![store(), draw(), store(), draw()]), (2, 2), "stored again, counted again");
}

#[test]
fn a_nested_picture_stopped_at_a_limit_truncates_the_file_that_draws_it() {
    // The square comes first, so the inner picture has a size and is drawn.
    let inner = |saves: usize| {
        let mut recs = vec![erec(43, &cat(vec![e32(0), e32(0), e32(10), e32(10)]))];
        recs.extend(vec![erec(33, &[]); saves]);
        emf(&recs, [0; 4], [0; 4])
    };
    assert!(!parse(&wrap(inner(crate::MAX_STACK))).unwrap().truncated);
    assert!(parse(&inner(crate::MAX_STACK + 1)).unwrap().truncated);
    let p = parse(&wrap(inner(crate::MAX_STACK + 1))).unwrap();
    assert!(p.truncated && !p.items.is_empty());
    // An inner picture that stops before it draws anything has no size and draws nothing, and still
    // reports that it stopped.
    let empty = emf(&vec![erec(33, &[]); crate::MAX_STACK + 1], [0; 4], [0; 4]);
    let p = parse(&wrap(empty)).unwrap();
    assert!(p.truncated && p.items.is_empty());
}

/// EMR_STRETCHDIBITS of a 1 x 1 BI_PNG bitmap whose bits are no image.
fn stretch_bad_png() -> Vec<u8> {
    let bits = [1u8, 2, 3, 4];
    let mut info = info_header(1, 1, 32, bits.len() as u32);
    info[16..20].copy_from_slice(&5u32.to_le_bytes());
    let mut payload = vec![0u8; 16];
    let fields = [0, 0, 0, 0, 1, 1, 80, 40, 120, bits.len() as i32, 0, 0x00CC_0020, 10, 10];
    payload.extend(cat(fields.iter().map(|v| e32(*v)).collect()));
    payload.extend(info);
    payload.extend(bits);
    erec(81, &payload)
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

#[test]
fn pict_truncated_halfway_through_rectangle_reports_premature_eof() {
    let mut pict = vec![0xA5; 512];
    pict.extend([0, 0]);
    pict.extend([0, 0, 0, 0, 0, 10, 0, 10]);
    pict.extend([0x00, 0x11, 0x02, 0xFF, 0x0C, 0x00]);
    pict.extend([-1i16, 0].iter().flat_map(|x| x.to_be_bytes()));
    pict.extend([0; 20]);
    // Truncated halfway through a rectangle opcode (paintRect 0x0031):
    // Rectangle opcode expects 8 bytes [top, left, bottom, right], but only 4 bytes are provided.
    pict.extend([0x00, 0x31]);
    pict.extend([0, 0, 0, 5]);

    let p = crate::parse(&pict).unwrap();
    assert!(p.truncated);
    assert_eq!(p.truncation_reason, Some(crate::TruncationReason::PrematureEof));
}

#[test]
fn pict_records_exceeding_max_opcode_budget_reports_opcode_limit_exceeded() {
    let mut pict = vec![0xA5; 512];
    pict.extend([0, 0]);
    pict.extend([0, 0, 0, 0, 0, 10, 0, 10]);
    pict.extend([0x00, 0x11, 0x02, 0xFF, 0x0C, 0x00]);
    pict.extend([-1i16, 0].iter().flat_map(|x| x.to_be_bytes()));
    pict.extend([0; 20]);
    // Valid records exceeding MAX_OPS budget (e.g. NOP opcodes 0x0000):
    pict.resize(pict.len() + 2 * (crate::pict::MAX_OPS + 1), 0);
    pict.extend([0x00, 0xFF]);

    let p = crate::parse(&pict).unwrap();
    assert!(p.truncated);
    assert_eq!(p.truncation_reason, Some(crate::TruncationReason::OpcodeLimitExceeded));
}

#[test]
fn pict_invalid_zero_height_bitmap_does_not_report_premature_eof() {
    let mut pict = vec![0xA5; 512];
    pict.extend([0, 0]);
    pict.extend([0, 0, 0, 0, 0, 10, 0, 10]);
    pict.extend([0x00, 0x11, 0x02, 0xFF, 0x0C, 0x00]);
    pict.extend([-1i16, 0].iter().flat_map(|x| x.to_be_bytes()));
    pict.extend([0; 20]);
    // Complete BitsRect (0x0090) with zero-height bounds, followed by EndPic (0x00FF):
    pict.extend([0x00, 0x90]);
    pict.extend([0x00, 0x02]); // row_bytes
    pict.extend([0, 0, 0, 0, 0, 0, 0, 10]); // bounds (top=0, left=0, bottom=0, right=10; height=0)
    pict.extend([0, 0, 0, 0, 0, 0, 0, 10]); // src
    pict.extend([0, 0, 0, 0, 0, 0, 0, 10]); // dst
    pict.extend([0x00, 0x00]); // mode
    pict.extend([0x00, 0xFF]); // EndPic

    let p = crate::parse(&pict).unwrap();
    assert!(p.truncated);
    assert_ne!(p.truncation_reason, Some(crate::TruncationReason::PrematureEof));
    assert_eq!(p.truncation_reason, None);
}

#[test]
fn pict_truncated_region_payload_stops_with_premature_eof() {
    let mut pict = vec![0xA5; 512];
    pict.extend([0, 0]);
    pict.extend([0, 0, 0, 0, 0, 10, 0, 10]);
    pict.extend([0x00, 0x11, 0x02, 0xFF, 0x0C, 0x00]);
    pict.extend([-1i16, 0].iter().flat_map(|x| x.to_be_bytes()));
    pict.extend([0; 20]);
    // Opcode 0x0080 (frameRgn) claiming 16 bytes for region, followed by EOF before body finishes:
    pict.extend([0x00, 0x80]);
    pict.extend([0x00, 0x10]);
    pict.extend([0x00, 0xFF]);

    let p = crate::parse(&pict).unwrap();
    assert!(p.truncated);
    assert_eq!(p.truncation_reason, Some(crate::TruncationReason::PrematureEof));
}

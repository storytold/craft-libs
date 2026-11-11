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
    let pic = crate::Picture { width: 10.0, height: 10.0, items: (0..8).map(|_| path()).collect(), skipped_records: 0 };
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

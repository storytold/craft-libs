//! `TiffBands` (#8169): decoding a TIFF a group of bands at a time gives exactly the image the
//! whole-image decoder does, for every layout the decoder reads, every orientation, and groups of
//! any size; and it opens images whose single buffer the whole-image decoder refuses.

use crate::common::tiffgen::*;
use craft_image_codecs::*;

/// A deterministic byte pattern.
fn noise(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761).max(1);
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8
        })
        .collect()
}

/// The image `TiffBands` decodes, assembled from its groups.
fn banded(bytes: &[u8], page: Option<usize>, limits: &Limits, upright: bool) -> Result<Image, CodecError> {
    let mut t = TiffBands::open(bytes, page, limits)?;
    if upright {
        t = t.upright();
    }
    let (w, h) = t.dimensions();
    let mut out = Image::new(w, h, t.layout(), t.sample_type())?;
    let bpp = t.layout().channels() * t.sample_type().bytes();
    let mut groups = 0;
    t.decode(|x, y, g: &Image| -> Result<(), CodecError> {
        groups += 1;
        assert_eq!((g.layout(), g.sample_type()), (out.layout(), out.sample_type()));
        let (gw, gh) = g.dimensions();
        assert!(x + gw <= w && y + gh <= h, "group {gw}x{gh} at {x},{y} outside {w}x{h}");
        let row = gw as usize * bpp;
        for r in 0..gh as usize {
            let dst = ((y as usize + r) * w as usize + x as usize) * bpp;
            out.data_mut()[dst..dst + row].copy_from_slice(&g.data()[r * row..(r + 1) * row]);
        }
        Ok(())
    })?;
    assert!(groups >= 1);
    out.icc = t.icc.clone();
    out.meta = t.meta.clone();
    out.warnings = t.warnings.clone();
    Ok(out)
}

/// The whole-image decode of the same page.
fn whole(bytes: &[u8], page: Option<usize>, upright: bool) -> Image {
    let opts = DecodeOptions { keep_orientation: !upright, ..Default::default() };
    match page {
        Some(p) => decode_tiff_page(bytes, p, &opts).unwrap(),
        None => decode_with(bytes, &opts).unwrap(),
    }
}

/// Limits whose `max_alloc` makes each group `bands` bands of `band_bytes`.
fn grouped(band_bytes: usize, bands: usize) -> Limits {
    Limits { max_alloc: (band_bytes * bands) as u64, ..Limits::banded() }
}

fn assert_same(bytes: &[u8], band_bytes: usize, what: &str) {
    for upright in [false, true] {
        let want = whole(bytes, None, upright);
        for limits in [Limits::banded(), grouped(band_bytes, 1), grouped(band_bytes, 2), grouped(band_bytes, 3)] {
            let got = banded(bytes, None, &limits, upright).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(got.dimensions(), want.dimensions(), "{what}");
            assert_eq!((got.layout(), got.sample_type()), (want.layout(), want.sample_type()), "{what}");
            assert!(got.data() == want.data(), "{what}: pixels differ (max_alloc {})", limits.max_alloc);
            assert_eq!(got.icc, want.icc, "{what}");
            assert_eq!(got.meta, want.meta, "{what}");
            assert_eq!(got.warnings, want.warnings, "{what}");
        }
    }
}

/// `n / 1` as a little-endian RATIONAL value.
fn rational(n: u64) -> u64 {
    n | 1 << 32
}

const W: u32 = 37;
const H: u32 = 29;

#[test]
fn every_layout_matches_the_whole_image_decoder() {
    let n = (W * H) as usize;
    let rows = 4u32;
    let row = |spp: usize, bps: usize| W as usize * spp * bps;
    let cases: Vec<(&str, Dir, usize)> = vec![
        ("rgb8 strips", strips(W, H, 8, 3, 2, rows, &noise(n * 3, 1)), row(3, 1) * rows as usize),
        ("gray8 WhiteIsZero", strips(W, H, 8, 1, 0, rows, &noise(n, 2)), row(1, 1) * rows as usize),
        ("gray16 big-endian", strips(W, H, 16, 1, 1, rows, &noise(n * 2, 3)), row(1, 2) * rows as usize),
        ("cmyk8", strips(W, H, 8, 4, 5, rows, &noise(n * 4, 4)), row(4, 1) * rows as usize),
        ("cielab8", strips(W, H, 8, 3, 8, rows, &noise(n * 3, 5)), row(3, 1) * rows as usize),
        ("rgba16 associated alpha", strips(W, H, 16, 4, 2, rows, &noise(n * 8, 6)).tag(338, SHORT, &[1]), row(4, 2) * rows as usize),
        (
            "gray f32",
            strips(W, H, 32, 1, 1, rows, &f32s(&noise(n, 7).iter().map(|&b| f32::from(b) / 255.0).collect::<Vec<_>>(), true)).tag(339, SHORT, &[3]),
            row(1, 4) * rows as usize,
        ),
        (
            "palette 4-bit",
            strips(W, H, 4, 1, 3, rows, &noise((W as usize).div_ceil(2) * H as usize, 8)).tag(
                320,
                SHORT,
                &(0..48u64).map(|i| i * 1365).collect::<Vec<_>>(),
            ),
            row(3, 1) * rows as usize,
        ),
        ("gray 1-bit", strips(W, H, 1, 1, 1, rows, &noise((W as usize).div_ceil(8) * H as usize, 9)), row(1, 1) * rows as usize),
    ];
    for (what, dir, band) in cases {
        // Both byte orders: the generator writes multi-byte samples as given, so the
        // big-endian files hold different values; each is compared against its own decode.
        for little in [true, false] {
            assert_same(&build(little, false, std::slice::from_ref(&dir), &[0]).bytes, band, what);
        }
    }
}

#[test]
fn tiles_and_planar_data_match() {
    let (w, h) = (W as usize, H as usize);
    let px = noise(w * h * 3 * 2, 11);
    let tiled = |planar: bool| {
        let mut d = Dir { entries: base(W, H, 16, 3, 2), chunks: tile_split(&px, w, h, 3, 2, 16, 16, planar), tiled: true }
            .tag(322, SHORT, &[16])
            .tag(323, SHORT, &[16]);
        if planar {
            d = d.tag(284, SHORT, &[2]);
        }
        d
    };
    let band = w * 3 * 2 * 16;
    assert_same(&build(true, false, &[tiled(false)], &[0]).bytes, band, "chunky tiles");
    assert_same(&build(true, true, &[tiled(true)], &[0]).bytes, band, "planar tiles, BigTIFF");
    let planar =
        Dir { entries: base(W, H, 16, 3, 2), chunks: planar_strips(&px, w, h, 3, 2, 5), tiled: false }.tag(278, LONG, &[5]).tag(284, SHORT, &[2]);
    assert_same(&build(true, false, &[planar], &[0]).bytes, w * 3 * 2 * 5, "planar strips");
}

#[test]
fn every_orientation_matches() {
    let n = (W * H) as usize;
    for o in 1..=8u64 {
        let dir = strips(W, H, 8, 3, 2, 3, &noise(n * 3, 20 + o as u32))
            .tag(274, SHORT, &[o])
            .tag(282, RATIONAL, &[rational(300)])
            .tag(283, RATIONAL, &[rational(150)])
            .tag(296, SHORT, &[2]);
        let bytes = build(true, false, &[dir], &[0]).bytes;
        assert_same(&bytes, W as usize * 3 * 3, &format!("orientation {o}"));
        let t = TiffBands::open(&bytes, None, &Limits::banded()).unwrap();
        assert_eq!(t.orientation(), o as u16);
        let up = t.upright();
        let swapped = o >= 5;
        assert_eq!(up.dimensions(), if swapped { (H, W) } else { (W, H) }, "orientation {o}");
        assert_eq!(up.meta.dpi, Some(if swapped { (150.0, 300.0) } else { (300.0, 150.0) }), "orientation {o}");
    }
}

#[test]
fn cut_off_data_matches_and_warns() {
    let n = (W * H) as usize;
    let built = build(true, false, &[strips(W, H, 8, 3, 2, 4, &noise(n * 3, 30))], &[0]);
    // The last four of eight strips point past the end of the file: their data is missing.
    let mut built = built;
    let past = built.bytes.len() as u64 + 4096;
    for i in 4..8 {
        patch_value(&mut built, true, false, 0, 273, i, past);
    }
    assert_same(&built.bytes, W as usize * 3 * 4, "cut off");
    let img = banded(&built.bytes, None, &Limits::banded(), true).unwrap();
    assert!(img.warnings.contains(&DecodeWarning::Truncated { format: Format::Tiff }));
}

#[test]
fn a_strip_cut_off_partway_decodes_as_zeros() {
    // An uncompressed strip read straight into its output rows, whose data starts in the file
    // and ends past it: the bytes read before the end must not survive (#8536 review).
    let n = (W * H) as usize;
    let mut built = build(true, false, &[strips(W, H, 8, 3, 2, 4, &noise(n * 3, 31))], &[0]);
    let strip = W as usize * 3 * 4;
    let near_end = built.bytes.len() as u64 - 100;
    patch_value(&mut built, true, false, 0, 273, 6, near_end);
    let img = banded(&built.bytes, None, &Limits::banded(), false).unwrap();
    assert!(img.warnings.contains(&DecodeWarning::Truncated { format: Format::Tiff }));
    assert!(img.data()[6 * strip..7 * strip].iter().all(|&v| v == 0), "partial strip bytes kept");
    assert!(img.data()[..6 * strip] == whole(&built.bytes, None, false).data()[..6 * strip]);
}

#[test]
fn other_pages_and_the_more_pages_warning_match() {
    let n = (W * H) as usize;
    let a = strips(W, H, 8, 1, 1, 5, &noise(n, 40));
    let b = strips(W, H, 8, 3, 2, 5, &noise(n * 3, 41));
    let bytes = build(true, false, &[a, b], &[0, 1]).bytes;
    for page in [None, Some(0), Some(1)] {
        let want = whole(&bytes, page, false);
        let got = banded(&bytes, page, &Limits::banded(), false).unwrap();
        assert!(got.data() == want.data(), "page {page:?}");
        assert_eq!(got.warnings, want.warnings, "page {page:?}");
    }
    assert!(matches!(TiffBands::open(&bytes, Some(9), &Limits::banded()), Err(CodecError::InvalidImage(_))));
}

#[test]
fn a_cut_off_page_of_several_warns_in_the_whole_image_order() {
    // Truncated before MorePages, as the whole-image decode reports them (#8536 review).
    let n = (W * H) as usize;
    let a = strips(W, H, 8, 3, 2, 4, &noise(n * 3, 42));
    let b = strips(W, H, 8, 1, 1, 5, &noise(n, 43));
    let mut built = build(true, false, &[a, b], &[0, 1]);
    let past = built.bytes.len() as u64 + 4096;
    for i in 4..8 {
        patch_value(&mut built, true, false, 0, 273, i, past);
    }
    let want = whole(&built.bytes, None, false);
    assert!(matches!(want.warnings[..], [DecodeWarning::Truncated { .. }, DecodeWarning::MorePages { .. }]), "{:?}", want.warnings);
    let got = banded(&built.bytes, None, &Limits::banded(), false).unwrap();
    assert_eq!(got.warnings, want.warnings);
}

#[test]
fn group_bytes_is_the_largest_group_handed_over() {
    let n = (W * H) as usize;
    // Eight strips of four rows, two to a group; one strip holding the whole image.
    let bytes = build(true, false, &[strips(W, H, 8, 3, 2, 4, &noise(n * 3, 44))], &[0]).bytes;
    let band = W as usize * 3 * 4;
    let mut t = TiffBands::open(&bytes, None, &grouped(band, 2)).unwrap();
    assert_eq!(t.group_bytes().unwrap(), 2 * band);
    let mut largest = 0;
    t.decode(|_, _, g: &Image| -> Result<(), CodecError> {
        largest = largest.max(g.data().len());
        Ok(())
    })
    .unwrap();
    assert_eq!(largest, 2 * band);
    let single = build(true, false, &[strips(W, H, 8, 3, 2, H, &noise(n * 3, 45))], &[0]).bytes;
    let t = TiffBands::open(&single, None, &Limits::banded()).unwrap();
    assert_eq!(t.group_bytes().unwrap(), n * 3, "a single strip is one group: the whole image");
}

#[test]
#[cfg(target_pointer_width = "64")]
fn opens_an_image_over_the_whole_image_budget() {
    // The whole-image decoder needs the image in one buffer; the banded one only a band (on
    // 64-bit targets: 32-bit ones still check the whole image).
    let n = (W * H) as usize;
    let bytes = build(true, false, &[strips(W, H, 8, 3, 2, 2, &noise(n * 3, 50))], &[0]).bytes;
    let band = W as u64 * 3 * 2;
    let tight = Limits { max_alloc: band * 3, ..Limits::banded() };
    let refused = decode_with(&bytes, &DecodeOptions { limits: tight, keep_orientation: false });
    assert!(matches!(refused, Err(CodecError::LimitExceeded(_))), "{refused:?}");
    let got = banded(&bytes, None, &tight, true).unwrap();
    assert!(got.data() == whole(&bytes, None, true).data());
}

#[test]
fn size_limits_still_apply() {
    let n = (W * H) as usize;
    let bytes = build(true, false, &[strips(W, H, 8, 3, 2, 4, &noise(n * 3, 60))], &[0]).bytes;
    let few_pixels = Limits { max_pixels: u64::from(W * H) - 1, ..Limits::banded() };
    assert!(matches!(TiffBands::open(&bytes, None, &few_pixels), Err(CodecError::LimitExceeded(_))));
    let narrow = Limits { max_width: W - 1, ..Limits::banded() };
    assert!(matches!(TiffBands::open(&bytes, None, &narrow), Err(CodecError::LimitExceeded(_))));
    // One strip is more than `max_alloc`: refused before decoding, with the image size.
    let tiny = Limits { max_alloc: 10, ..Limits::banded() };
    match TiffBands::open(&bytes, None, &tiny) {
        Err(CodecError::LimitExceeded(m)) => assert!(m.contains(&format!("{W}x{H}")), "{m}"),
        other => panic!("{:?}", other.map(|t| t.dimensions())),
    }
}

#[test]
fn banded_limits_allow_the_largest_document_size() {
    let l = Limits::banded();
    if cfg!(target_pointer_width = "64") {
        assert_eq!((l.max_width, l.max_height), (300_000, 300_000));
        assert!(l.max_pixels >= 300_000 * 300_000);
        // The banner from #8456 / #8169: 136 ft × 26 ft at 72 ppi.
        assert!(117_504u64 * 22_464 > Limits::default().max_pixels);
        assert!(117_504u64 * 22_464 <= l.max_pixels);
    } else {
        assert_eq!(l, Limits::default());
    }
    assert_eq!(l.max_alloc, Limits::default().max_alloc);
}

#[test]
fn a_hostile_header_is_refused_before_allocating() {
    // 300000² declared in a file of a few kilobytes: one 270 GB strip, which neither the
    // decompression-bomb guard nor the per-group budget lets through.
    let mut built = build(true, true, &[strips(1000, 4, 8, 3, 2, 4, &noise(12_000, 70))], &[0]);
    for tag in [256, 257, 278] {
        patch_value(&mut built, true, true, 0, tag, 0, 300_000);
    }
    assert!(matches!(TiffBands::open(&built.bytes, None, &Limits::banded()), Err(CodecError::Malformed { .. } | CodecError::LimitExceeded(_))));
    // Past the 300000 px a side documents allow.
    let mut wide = build(true, true, &[strips(1000, 4, 8, 3, 2, 4, &noise(12_000, 71))], &[0]);
    patch_value(&mut wide, true, true, 0, 256, 0, 300_001);
    assert!(matches!(TiffBands::open(&wide.bytes, None, &Limits::banded()), Err(CodecError::LimitExceeded(_))));
}

#[test]
fn padding_does_not_let_missing_chunks_past_the_bomb_guard() {
    // 20000² RGB declared as Deflate (1.2 GB of samples), every strip past the end of the file,
    // and 400 KB of padding: the file is large enough for that size at Deflate's ratio, but its
    // strips hold nothing. Refused before anything is allocated, by both decoders (#8536 review).
    let (w, h) = (20_000, 20_000);
    let mut dir = Dir { entries: base(w, h, 8, 3, 2), chunks: vec![vec![0]; 20], tiled: false }.tag(278, LONG, &[1000]);
    dir.entries.retain(|e| e.0 != 259);
    let dir = dir.tag(259, SHORT, &[8]).bytes(65_000, UNDEFINED, &vec![0; 400_000]);
    let mut built = build(true, false, &[dir], &[0]);
    let past = built.bytes.len() as u64 + 4096;
    for i in 0..20 {
        patch_value(&mut built, true, false, 0, 273, i, past);
    }
    match TiffBands::open(&built.bytes, None, &Limits::banded()) {
        Err(CodecError::Malformed { message, .. }) => assert!(message.contains("hold 0 bytes"), "{message}"),
        other => panic!("{:?}", other.map(|t| t.dimensions())),
    }
    assert!(matches!(decode(&built.bytes), Err(CodecError::Malformed { .. })));
}

/// An all-white CCITT Group 4 strip: one V0 code (a 1 bit) per row, then the EOFB.
fn blank_g4(h: u32) -> Vec<u8> {
    let mut bits = vec![true; h as usize];
    for _ in 0..2 {
        bits.extend([false; 11]);
        bits.push(true);
    }
    bits.chunks(8).map(|c| c.iter().enumerate().fold(0u8, |a, (i, &b)| a | (u8::from(b) << (7 - i)))).collect()
}

#[test]
fn a_blank_fax_page_wider_than_4096_px_passes_the_bomb_guard() {
    // A blank 600 dpi page (A4 landscape, A3) codes each row in one bit, past 4096:1: it must
    // open, with or without the metadata and other pages that used to raise the bound.
    for (w, h) in [(7016u32, 4961u32), (7016, 9921)] {
        let mut d = Dir { entries: base(w, h, 1, 1, 0), chunks: vec![blank_g4(h)], tiled: false }.tag(278, LONG, &[u64::from(h)]);
        d.entries.retain(|e| e.0 != 259);
        let d = d.tag(259, SHORT, &[4]);
        let pages: Vec<Dir> = (0..3).map(|_| d.clone()).collect();
        for (what, bytes) in
            [("one page", build(true, false, std::slice::from_ref(&d), &[0]).bytes), ("three pages", build(true, false, &pages, &[0, 1, 2]).bytes)]
        {
            let img = decode(&bytes).unwrap_or_else(|e| panic!("{w}x{h} {what}: {e}"));
            assert_eq!(img.dimensions(), (w, h), "{what}");
            assert!(img.data().iter().all(|&v| v == 255), "{w}x{h} {what}: not white");
        }
    }
    let mut d = Dir { entries: base(7016, 4961, 1, 1, 0), chunks: vec![blank_g4(4961)], tiled: false }.tag(278, LONG, &[4961]);
    d.entries.retain(|e| e.0 != 259);
    let bytes = build(true, false, &[d.tag(259, SHORT, &[4])], &[0]).bytes;
    assert!(banded(&bytes, None, &Limits::banded(), true).unwrap().data() == decode(&bytes).unwrap().data());
    // Still a bound: 100 bytes of Group 4 can't hold that page (4096:1 for Group 3, which is not decoded).
    for compression in [4, 3] {
        let mut d = Dir { entries: base(7016, 4961, 1, 1, 0), chunks: vec![vec![0x80; 100]], tiled: false }.tag(278, LONG, &[4961]);
        d.entries.retain(|e| e.0 != 259);
        let bytes = build(true, false, &[d.tag(259, SHORT, &[compression])], &[0]).bytes;
        match decode(&bytes) {
            Err(CodecError::Malformed { message, .. }) => assert!(message.contains("hold 100 bytes"), "{message}"),
            other => panic!("compression {compression}: {:?}", other.map(|i| i.dimensions())),
        }
    }
}

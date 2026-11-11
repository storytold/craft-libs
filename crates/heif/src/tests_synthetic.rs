//! Decoding lossless synthetic HEIF files ([`crate::testdata`]): every expected sample is known.

use crate::testdata::{Prop, Spec, build};
use crate::{Error, Options, decode, decode_thumbnail, probe};

/// A luma value unique enough to identify its position; chroma neutral.
fn luma(x: u32, y: u32) -> u16 {
    ((x * 7 + y * 13) % 220 + 20) as u16
}

fn grey(x: u32, y: u32) -> [u16; 3] {
    [luma(x, y), 128, 128]
}

const FULL_601: Prop = Prop::Nclx(1, 13, 6, true);

/// Decoded red channel at `(x, y)`, back in 8-bit code values.
fn at(d: &crate::Decoded, x: u32, y: u32) -> u16 {
    let ch = if d.has_alpha { 4 } else { 3 };
    d.data[((y * d.width + x) as usize) * ch] / 257
}

fn decoded(spec: &Spec) -> crate::Decoded {
    decode(&build(spec), &Options::default()).unwrap()
}

#[test]
fn a_grid_reassembles_exactly() {
    // 4 × 3 tiles of 32, cropped by the grid to 100 × 70.
    let d = decoded(&Spec { props: vec![FULL_601], ..Spec::new(100, 70, &grey) });
    assert_eq!((d.width, d.height, d.bit_depth, d.has_alpha), (100, 70, 8, false));
    for y in 0..70 {
        for x in 0..100 {
            let px = &d.data[((y * 100 + x) * 3) as usize..][..3];
            assert_eq!(px, [luma(x, y) * 257; 3], "({x}, {y})");
        }
    }
    let info = probe(&build(&Spec::new(100, 70, &grey))).unwrap();
    assert_eq!((info.width, info.height, info.coded_width, info.coded_height, info.bit_depth), (100, 70, 100, 70, 8));
}

#[test]
fn colour_follows_the_matrix_and_range() {
    // BT.601 full range, a saturated orange: within one code value of the exact conversion.
    let (y, cb, cr) = (150.0f32, 80.0, 190.0);
    let s = |_: u32, _: u32| [y as u16, cb as u16, cr as u16];
    let d = decoded(&Spec { props: vec![FULL_601], ..Spec::new(32, 32, &s) });
    let (u, v) = (cb - 128.0, cr - 128.0);
    let expected = [y + 1.402 * v, y - 0.344_136 * u - 0.714_136 * v, y + 1.772 * u].map(|c| c.clamp(0.0, 255.0));
    for (got, want) in d.data[..3].iter().zip(expected) {
        assert!((f32::from(*got) / 257.0 - want).abs() <= 1.0, "{:?} vs {expected:?}", &d.data[..3]);
    }
    assert_eq!(d.colour, crate::Colour { primaries: 1, transfer: 13, matrix: 6, full_range: true });

    // No nclx and no VUI: the H.265 defaults, limited range (16 → black, 235 → white).
    let ends = |x: u32, _: u32| [if x < 16 { 16 } else { 235 }, 128, 128];
    let d = decoded(&Spec::new(32, 32, &ends));
    assert!(!d.colour.full_range);
    assert_eq!((d.data[0], d.data[20 * 3]), (0, 65535));
}

#[test]
fn ten_bit_samples_keep_their_precision() {
    let s = |x: u32, y: u32| [(x * 31 + y) as u16 % 1024, 512, 512];
    let d = decoded(&Spec { bit_depth: 10, props: vec![FULL_601], ..Spec::new(64, 32, &s) });
    assert_eq!(d.bit_depth, 10);
    for x in [0, 1, 5, 33, 63] {
        let want = ((x * 31) % 1024) as f64 / 1023.0 * 65535.0;
        assert!((f64::from(d.data[(x * 3) as usize]) - want).abs() <= 1.0, "x {x}");
    }
}

#[test]
fn container_transforms_are_applied_like_libheif() {
    let (w, h) = (70u32, 40u32);
    let spec = |props: Vec<Prop>| Spec { props: [vec![FULL_601], props].concat(), ..Spec::new(w, h, &grey) };
    // irot: counter-clockwise quarter turns.
    let d = decoded(&spec(vec![Prop::Irot(1)]));
    assert_eq!((d.width, d.height), (h, w));
    assert_eq!(at(&d, 0, 0), luma(w - 1, 0)); // the right column becomes the top row
    assert_eq!(at(&d, h - 1, w - 1), luma(0, h - 1));
    let d = decoded(&spec(vec![Prop::Irot(2)]));
    assert_eq!(at(&d, 0, 0), luma(w - 1, h - 1));
    let d = decoded(&spec(vec![Prop::Irot(3)]));
    assert_eq!(at(&d, 0, 0), luma(0, h - 1));
    // imir: axis 0 exchanges top and bottom, axis 1 left and right (libheif's reading).
    let d = decoded(&spec(vec![Prop::Imir(0)]));
    assert_eq!((at(&d, 0, 0), at(&d, 5, 0)), (luma(0, h - 1), luma(5, h - 1)));
    let d = decoded(&spec(vec![Prop::Imir(1)]));
    assert_eq!((at(&d, 0, 0), at(&d, 0, 5)), (luma(w - 1, 0), luma(w - 1, 5)));
    // clap: centred crop moved by its offsets; a half-pixel corner goes left and down.
    let d = decoded(&spec(vec![Prop::Clap(31, 21, 3, -2)]));
    assert_eq!((d.width, d.height), (31, 21));
    // left = (70 - 31 + 6) / 2 = 22.5 → 22, top = (40 - 21 - 4) / 2 = 7.5 → 8
    assert_eq!(at(&d, 0, 0), luma(22, 8));
    // In association order: crop, then turn, then mirror.
    let d = decoded(&spec(vec![Prop::Clap(30, 20, 0, 0), Prop::Irot(1), Prop::Imir(1)]));
    assert_eq!((d.width, d.height), (20, 30));
    // The crop is (20, 10) to (49, 29); the turn brings its bottom-right corner to the top-right,
    // the left-right mirror to the top-left.
    assert_eq!(at(&d, 0, 0), luma(49, 29));
    let info = probe(&build(&spec(vec![Prop::Irot(1)]))).unwrap();
    assert_eq!((info.width, info.height, info.coded_width, info.coded_height), (h, w, w, h));
    // Transforms off: as coded.
    let d = decode(&build(&spec(vec![Prop::Irot(1)])), &Options { apply_transforms: false, ..Default::default() }).unwrap();
    assert_eq!((d.width, d.height, at(&d, 3, 2)), (w, h, luma(3, 2)));
}

#[test]
fn metadata_aux_images_and_thumbnails() {
    let icc = b"not parsed here: carried verbatim".to_vec();
    let exif = b"MM\0\x2a\0\0\0\x08\0\0".to_vec();
    let spec = Spec {
        props: vec![Prop::Icc(icc.clone()), FULL_601],
        exif: Some(exif.clone()),
        thumbnail: Some(&|_, _| [77, 128, 128]),
        ignored_aux: vec!["urn:com:apple:photo:2020:aux:hdrgainmap", "urn:mpeg:hevc:2015:auxid:2"],
        ..Spec::new(64, 64, &grey)
    };
    let bytes = build(&spec);
    let d = decode(&bytes, &Options::default()).unwrap();
    assert_eq!((d.width, d.height, d.has_alpha), (64, 64, false));
    assert_eq!(d.icc.as_deref(), Some(icc.as_slice()));
    assert_eq!(d.exif.as_deref(), Some(exif.as_slice()));
    let t = decode_thumbnail(&bytes, 32, &Options::default()).unwrap().unwrap();
    assert_eq!((t.width, t.height, t.data[0]), (32, 32, 77 * 257));
    assert_eq!(t.exif.as_deref(), Some(exif.as_slice()));
    assert_eq!(t.icc.as_deref(), Some(icc.as_slice()));
    assert!(decode_thumbnail(&bytes, 33, &Options::default()).unwrap().is_none());
}

#[test]
fn alpha_comes_from_its_auxiliary_image() {
    let alpha = |x: u32, _: u32| (x * 4) as u16;
    let d = decoded(&Spec { props: vec![FULL_601], alpha: Some(&alpha), ..Spec::new(64, 32, &grey) });
    assert!(d.has_alpha);
    assert_eq!(d.data.len(), 64 * 32 * 4);
    assert_eq!((d.data[3], d.data[10 * 4 + 3]), (0, 40 * 257));
    assert_eq!(d.data[..3], [luma(0, 0) * 257; 3]);
}

#[test]
fn limits_are_checked_before_decoding() {
    let bytes = build(&Spec::new(96, 64, &grey));
    let r = decode(&bytes, &Options { max_pixels: 96 * 64 - 1, ..Default::default() });
    assert!(matches!(r, Err(Error::Limit(_))), "{r:?}");
}

/// #1518: a grid's tiles together are held to the pixel budget before any is decoded. A
/// 33 × 33 canvas fits 1089 pixels, but its four 32 × 32 tiles hold 4096: over twice the limit.
#[test]
fn grid_tiles_are_held_to_the_pixel_budget_together() {
    let bytes = build(&Spec::new(33, 33, &grey));
    let r = decode(&bytes, &Options { max_pixels: 33 * 33, ..Default::default() });
    assert!(matches!(r, Err(Error::Limit(_))), "{r:?}");
    let d = decode(&bytes, &Options { max_pixels: 2 * 32 * 32, ..Default::default() }).unwrap();
    assert_eq!((d.width, d.height), (33, 33));
}

#[test]
fn truncated_and_corrupted_files_are_errors_not_panics() {
    let bytes = build(&Spec { props: vec![FULL_601, Prop::Irot(1)], thumbnail: Some(&grey), ..Spec::new(64, 64, &grey) });
    let full = decode(&bytes, &Options::default()).unwrap();
    for cut in (0..bytes.len()).step_by(7) {
        let b = &bytes[..cut];
        // Cut inside the photo: an error. Cut in the trailing thumbnail: the whole photo.
        if let (Ok(_), Ok(d)) = (probe(b), decode(b, &Options::default())) {
            assert_eq!(d.data, full.data, "cut at {cut} of {}", bytes.len());
        }
        let _ = decode_thumbnail(b, 1, &Options::default());
    }
    // Flip bits all over the file: any outcome but a panic.
    let mut state = 0x2545_f491_u32;
    for _ in 0..400 {
        let mut b = bytes.clone();
        for _ in 0..3 {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let i = state as usize % b.len();
            b[i] ^= 1 << (state % 8);
        }
        let _ = probe(&b);
        let _ = decode(&b, &Options::default());
        let _ = decode_thumbnail(&b, 1, &Options::default());
    }
}

/// CodeRabbit #44: heic-rs keeps every associated transform and each one re-allocates the
/// whole picture, so 13 of them are over the [`crate::MAX_TRANSFORMS`] cap and refused.
#[test]
fn too_many_transform_properties_are_refused() {
    let bytes = build(&Spec { props: std::iter::repeat_n(Prop::Irot(0), 13).collect(), ..Spec::new(32, 32, &grey) });
    let r = decode(&bytes, &Options::default());
    assert!(matches!(r, Err(Error::Limit(ref m)) if m.contains("transform")), "{r:?}");
    // At the cap they're not: 12 identity rotations decode normally.
    let bytes = build(&Spec { props: std::iter::repeat_n(Prop::Irot(0), 12).collect(), ..Spec::new(32, 32, &grey) });
    let d = decode(&bytes, &Options::default()).unwrap();
    assert_eq!((d.width, d.height), (32, 32));
}

//! The bounded DNG metadata reader (`dng_metadata`) on hand-built DNGs, checked against the tags
//! written and, for the colour tags, against the full reader.

use super::dng::{base_ifd, dng, dng_with};
use craft_raw::dng_lines::{self, Error};
use craft_raw::dng_metadata::{Metadata, Pattern};
use craft_tiff::classic::{Tiff, tags as ct};
use craft_tiff::{ByteOrder, IfdBuilder, ImageData, Value};

fn cfa_raw(edit: impl FnOnce(&mut IfdBuilder)) -> IfdBuilder {
    let mut raw = base_ifd(8, 4, 16, 1, true);
    raw.set(ct::COMPRESSION, Value::Short(vec![1]));
    raw.set_image(ImageData::Strips { rows_per_strip: 4, strips: vec![vec![0; 64]] });
    edit(&mut raw);
    raw
}

fn with_metadata<T>(bytes: &[u8], f: impl FnOnce(&Metadata) -> T) -> T {
    let (tiff, first) = Tiff::new(bytes).unwrap();
    let ifds = tiff.all_ifds(first).unwrap();
    let raw = dng_lines::find_raw_ifd(&tiff, &ifds).unwrap();
    f(&Metadata { tiff, main: &ifds[0], raw })
}

#[test]
fn processing_tags_are_read_from_the_raw_ifd() {
    let raw = cfa_raw(|r| {
        r.set(ct::ACTIVE_AREA, Value::Long(vec![1, 2, 3, 7]));
        r.set(ct::LINEARIZATION_TABLE, Value::Short(vec![0, 10, 20]));
        r.set(ct::BLACK_LEVEL_REPEAT_DIM, Value::Short(vec![2, 2]));
        r.set(ct::BLACK_LEVEL, Value::Rational(vec![(64, 1), (65, 1), (66, 1), (67, 1)]));
        r.set(ct::BLACK_LEVEL_DELTA_H, Value::SRational(vec![(1, 2); 8]));
        r.set(ct::BLACK_LEVEL_DELTA_V, Value::SRational(vec![(-1, 2); 4]));
        r.set(ct::WHITE_LEVEL, Value::Long(vec![4000]));
        r.set(ct::DEFAULT_CROP_ORIGIN, Value::Rational(vec![(1, 2), (1, 1)]));
        r.set(ct::DEFAULT_CROP_SIZE, Value::Long(vec![6, 2]));
    });
    for order in [ByteOrder::Little, ByteOrder::Big] {
        let p = with_metadata(&dng(raw.clone(), order), |m| m.params().unwrap());
        assert_eq!(p.active_area, Some([1, 2, 3, 7]));
        assert_eq!(p.linearization, Some(vec![0, 10, 20]));
        assert_eq!(p.black_repeat, (2, 2));
        assert_eq!(p.black, [64.0, 65.0, 66.0, 67.0]);
        assert_eq!(p.black_delta_h, [0.5; 8]);
        assert_eq!(p.black_delta_v, [-0.5; 4]);
        assert_eq!(p.white, [4000.0]);
        assert_eq!((p.crop_origin, p.crop_size), (Some((0.5, 1.0)), Some((6.0, 2.0))));
        // base_ifd writes CFAPattern 1 0 2 1 (GRBG)
        assert_eq!(p.cfa, Some(Pattern { rows: 2, cols: 2, colors: vec![1, 0, 2, 1] }));
    }
    // absent tags take the DNG defaults
    let p = with_metadata(&dng(cfa_raw(|_| {}), ByteOrder::Little), |m| m.params().unwrap());
    assert_eq!((p.active_area, p.linearization, p.black_repeat), (None, None, (1, 1)));
    assert!(p.black.is_empty() && p.white.is_empty() && p.crop_origin.is_none());
}

#[test]
fn cfa_patterns_map_plane_colours_and_refuse_odd_layouts() {
    let pattern = |edit: fn(&mut IfdBuilder)| with_metadata(&dng(cfa_raw(edit), ByteOrder::Little), |m| m.cfa());
    // CFAPlaneColor reorders the planes: file colour 2 is plane 0
    let swapped = pattern(|r| {
        r.set(ct::CFA_PLANE_COLOR, Value::Byte(vec![2, 1, 0]));
        r.set(ct::CFA_LAYOUT, Value::Short(vec![1]));
    });
    assert_eq!(swapped.unwrap(), Some(Pattern { rows: 2, cols: 2, colors: vec![1, 2, 0, 1] }));
    let unsupported = |r: Result<Option<Pattern>, Error>| matches!(r, Err(Error::Unsupported(_)));
    let malformed = |r: Result<Option<Pattern>, Error>| matches!(r, Err(Error::Malformed(_)));
    assert!(unsupported(pattern(|r| {
        r.set(ct::CFA_LAYOUT, Value::Short(vec![2]));
    })));
    assert!(unsupported(pattern(|r| {
        r.set(ct::CFA_PLANE_COLOR, Value::Byte(vec![0, 1, 3]));
    })));
    assert!(unsupported(pattern(|r| {
        r.set(ct::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![9, 2]));
    })));
    assert!(malformed(pattern(|r| {
        r.set(ct::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2]));
    })));
    assert!(malformed(pattern(|r| {
        r.set(ct::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![4, 4]));
    })));
    assert!(malformed(pattern(|r| {
        r.set(ct::CFA_PATTERN, Value::Byte(vec![0, 1, 1, 3]));
    })));
    // LinearRaw has no pattern
    let mut linear = base_ifd(4, 2, 16, 3, false);
    linear.set(ct::COMPRESSION, Value::Short(vec![1]));
    linear.set_image(ImageData::Strips { rows_per_strip: 2, strips: vec![vec![0; 48]] });
    assert_eq!(with_metadata(&dng(linear, ByteOrder::Big), |m| m.cfa()).unwrap(), None);
}

#[test]
fn calibration_matches_the_full_reader() {
    let bytes = dng_with(cfa_raw(|_| {}), ByteOrder::Little, |ifd0| {
        ifd0.set(ct::COLOR_MATRIX_2, Value::SRational(vec![(2, 1), (0, 1), (0, 1), (0, 1), (2, 1), (0, 1), (0, 1), (0, 1), (2, 1)]));
        ifd0.set(ct::CALIBRATION_ILLUMINANT_2, Value::Short(vec![17]));
        ifd0.set(ct::ANALOG_BALANCE, Value::Rational(vec![(1, 1); 3]));
        ifd0.set(ct::AS_SHOT_NEUTRAL, Value::Rational(vec![(1, 2), (1, 1), (3, 4)]));
        ifd0.set(ct::BASELINE_EXPOSURE, Value::SRational(vec![(1, 4)]));
        // not 3 x 3 (a four-colour camera): not a matrix here
        ifd0.set(ct::FORWARD_MATRIX_1, Value::SRational(vec![(1, 1); 12]));
    });
    let c = with_metadata(&bytes, |m| m.calibration().unwrap());
    let full = craft_raw::decode(&bytes).unwrap().color;
    assert_eq!(c.color_matrix, full.color_matrix);
    assert_eq!(c.illuminant, [Some(21), Some(17)]);
    assert_eq!(c.as_shot_neutral, full.as_shot_neutral);
    assert_eq!(c.analog_balance, Some([1.0; 3]));
    assert_eq!(c.as_shot_white_xy, Some((0.3127, 0.329)));
    assert_eq!(c.baseline_exposure, 0.25);
    assert_eq!(c.forward_matrix, [None, None]);
    assert_eq!(c.camera_calibration, [None, None]);
}

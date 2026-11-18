use super::*;
use craft_tiff::{Entry, IfdBuilder, ImageData, TiffWriter};

fn ifd(tags: Vec<(u16, Value)>) -> Ifd {
    let mut entries: Vec<_> = tags.into_iter().map(|(tag, value)| Entry { tag, value, offset: 0 }).collect();
    entries.sort_by_key(|entry| entry.tag);
    Ifd { entries, ..Default::default() }
}

#[test]
fn optional_crop_adjustments_are_explicit_and_valid_clipping_is_preserved() {
    let active = Rect::new(3, 2, 11, 8);
    let whole = Rect::new(0, 0, 11, 8);
    let clipped = ifd(vec![(t::DEFAULT_CROP_ORIGIN, Value::Long(vec![10, 0])), (t::DEFAULT_CROP_SIZE, Value::Long(vec![5, 4]))]);
    assert_eq!(default_crop(&clipped, active, 16, 12), (Rect::new(10, 0, 1, 4), true));
    assert_eq!(default_crop(&ifd(vec![(t::DEFAULT_CROP_ORIGIN, Value::Long(vec![1, 1]))]), active, 16, 12), (Rect::new(1, 1, 10, 7), true));
    for fields in [
        vec![(t::DEFAULT_CROP_SIZE, Value::Long(vec![4]))],
        vec![(t::DEFAULT_CROP_ORIGIN, Value::Long(vec![0, 0])), (t::DEFAULT_CROP_SIZE, Value::Long(vec![0, 4]))],
    ] {
        assert_eq!(default_crop(&ifd(fields), active, 16, 12), (whole, true));
    }
    assert_eq!(default_crop(&Ifd::default(), active, 16, 12), (whole, false));
    assert_eq!(default_crop(&ifd(vec![(t::DEFAULT_CROP_SIZE, Value::Long(vec![5, 4]))]), active, 16, 12), (Rect::new(0, 0, 5, 4), false));
    assert_eq!(
        default_crop(&ifd(vec![(t::DEFAULT_CROP_ORIGIN, Value::Long(vec![2, 1])), (t::DEFAULT_CROP_SIZE, Value::Long(vec![5, 4]))]), active, 16, 12),
        (Rect::new(2, 1, 5, 4), false)
    );
}

#[test]
fn hostile_crop_coordinates_fall_back_explicitly_before_saturating_casts() {
    for bad in [1e300, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        for tag in [t::DEFAULT_CROP_ORIGIN, t::DEFAULT_CROP_SIZE] {
            let raw = ifd(vec![(tag, Value::Double(vec![bad, 1.]))]);
            assert_eq!(default_crop(&raw, Rect::new(0, 0, 16, 12), 16, 12), (Rect::new(0, 0, 16, 12), true));
        }
    }
}

fn linear(extra: &[(u16, Value)]) -> Vec<u8> {
    let mut raw = IfdBuilder::new();
    for (tag, value) in [
        (t::IMAGE_WIDTH, Value::Long(vec![8])),
        (t::IMAGE_LENGTH, Value::Long(vec![6])),
        (t::BITS_PER_SAMPLE, Value::Short(vec![16])),
        (t::SAMPLES_PER_PIXEL, Value::Short(vec![1])),
        (t::PHOTOMETRIC, Value::Short(vec![photometric::LINEAR_RAW])),
        (t::COMPRESSION, Value::Short(vec![1])),
    ] {
        raw.set(tag, value);
    }
    for (tag, value) in extra {
        raw.set(*tag, value.clone());
    }
    raw.set_image(ImageData::Strips { rows_per_strip: 6, strips: vec![(0u16..48).flat_map(u16::to_le_bytes).collect()] });
    TiffWriter::new(ByteOrder::Little, false).write(&[raw]).unwrap()
}

#[test]
fn dng_and_generic_cfa_tiff_report_actual_level_sources_instead_of_unknown() {
    for format in [RawFormat::Dng, RawFormat::CfaTiff] {
        let missing = decode_as(&linear(&[]), Mode::Full, format, &Limits::default()).unwrap();
        assert_eq!(missing.level_sources, LevelSources { black: LevelSource::Fallback, white: LevelSource::Fallback });
        let declared = decode_as(
            &linear(&[(t::BLACK_LEVEL, Value::Short(vec![17])), (t::WHITE_LEVEL, Value::Short(vec![1000]))]),
            Mode::Full,
            format,
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(declared.level_sources, LevelSources { black: LevelSource::Metadata, white: LevelSource::Metadata });
        assert_eq!(declared.black.values, vec![17.]);
        assert_eq!(declared.white, vec![1000.]);
        let invalid = decode_as(
            &linear(&[(t::BLACK_LEVEL, Value::Double(vec![f64::NAN])), (t::WHITE_LEVEL, Value::Double(vec![f64::INFINITY]))]),
            Mode::Full,
            format,
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(invalid.level_sources, LevelSources { black: LevelSource::Fallback, white: LevelSource::Fallback });
    }
    let table = decode_as(&linear(&[(t::LINEARIZATION_TABLE, Value::Short(vec![0, 1000]))]), Mode::Full, RawFormat::Dng, &Limits::default()).unwrap();
    assert_eq!(table.white, vec![1000.]);
    assert_eq!(table.level_sources.white, LevelSource::Metadata);
}

#[test]
fn decoded_notices_record_actual_crop_gain_table_and_cfa_decisions() {
    let plain = decode_as(&linear(&[]), Mode::Full, RawFormat::Dng, &Limits::default()).unwrap();
    assert_eq!(plain.notices, RawNotices::default());
    let adjusted = decode_as(
        &linear(&[(t::DEFAULT_CROP_ORIGIN, Value::Long(vec![7, 0])), (t::DEFAULT_CROP_SIZE, Value::Long(vec![5, 4]))]),
        Mode::Full,
        RawFormat::Dng,
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(adjusted.crop, Rect::new(7, 0, 1, 4));
    assert!(adjusted.notices.default_crop_adjusted);
    let invalid_gain =
        decode_as(&linear(&[(t::PROFILE_GAIN_TABLE_MAP, Value::Undefined(vec![0; 16]))]), Mode::Full, RawFormat::Dng, &Limits::default()).unwrap();
    assert!(invalid_gain.notices.gain_table_omitted);
    assert!(invalid_gain.color.profile.gain_table_map.is_none());
    let assumed =
        decode_as(&linear(&[(t::PHOTOMETRIC, Value::Short(vec![photometric::CFA]))]), Mode::Full, RawFormat::CfaTiff, &Limits::default()).unwrap();
    assert!(assumed.notices.assumed_cfa);
    assert_eq!(assumed.cfa.as_ref().unwrap().pattern, vec![0, 1, 1, 2]);
    let tagged = decode_as(
        &linear(&[(t::PHOTOMETRIC, Value::Short(vec![photometric::CFA])), (t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 1, 2]))]),
        Mode::Full,
        RawFormat::Dng,
        &Limits::default(),
    )
    .unwrap();
    assert!(!tagged.notices.assumed_cfa);
    let limits = Limits { max_samples: 47, ..Default::default() };
    assert!(matches!(decode_as(&linear(&[]), Mode::Full, RawFormat::Dng, &limits), Err(RawError::Limit(_))));
}

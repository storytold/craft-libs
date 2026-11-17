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

/// A 16-bit CFA DNG of `width` x `height` whose strip holds every row in full but whose `StripByteCounts` says
/// `counted` bytes.
fn understated_strip_dng(width: u32, height: u32, counted: u32, active_area: Option<[u32; 4]>) -> Vec<u8> {
    let mut raw = IfdBuilder::new();
    for (tag, value) in [
        (t::IMAGE_WIDTH, Value::Long(vec![width])),
        (t::IMAGE_LENGTH, Value::Long(vec![height])),
        (t::BITS_PER_SAMPLE, Value::Short(vec![16])),
        (t::SAMPLES_PER_PIXEL, Value::Short(vec![1])),
        (t::PHOTOMETRIC, Value::Short(vec![photometric::CFA])),
        (t::COMPRESSION, Value::Short(vec![1])),
        (t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2])),
        (t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 1, 2])),
    ] {
        raw.set(tag, value);
    }
    if let Some(area) = active_area {
        raw.set(t::ACTIVE_AREA, Value::Long(area.to_vec()));
    }
    let samples = (width * height) as u16;
    raw.set_image(ImageData::Strips { rows_per_strip: height, strips: vec![(0..samples).flat_map(u16::to_le_bytes).collect()] });
    let mut bytes = TiffWriter::new(ByteOrder::Little, false).write(&[raw]).unwrap();
    // the writer counts the real bytes: understate that one count, a LONG in the IFD
    let count = (width * height * 2).to_le_bytes();
    let at = bytes.windows(4).position(|w| w == count).unwrap();
    bytes[at..at + 4].copy_from_slice(&counted.to_le_bytes());
    bytes
}

/// storytold/craft-libs#2040: the DJI Mavic 3 counts 5280 x 3956 x 2 bytes for a strip that holds 5376-wide rows
/// to the end of the file.
#[test]
fn a_strip_counted_only_active_area_wide_is_read_in_full() {
    let bytes = understated_strip_dng(8, 4, 6 * 4 * 2, Some([0, 0, 4, 6]));
    for mode in [Mode::Full, Mode::Header] {
        let img = decode_as(&bytes, mode, RawFormat::Dng, &Limits::default()).unwrap();
        assert_eq!((img.width, img.height), (8, 4));
    }
    let img = decode_as(&bytes, Mode::Full, RawFormat::Dng, &Limits::default()).unwrap();
    let RawData::U16(v) = &img.data else { panic!("integer samples") };
    assert_eq!(v, &(0..32).collect::<Vec<u16>>());
}

/// Any other short strip is still a corrupt file.
#[test]
fn other_short_strips_are_still_refused() {
    let refused = |bytes: Vec<u8>| matches!(decode_as(&bytes, Mode::Full, RawFormat::Dng, &Limits::default()), Err(RawError::Corrupt(_)));
    // no ActiveArea to explain the count
    assert!(refused(understated_strip_dng(8, 4, 6 * 4 * 2, None)));
    // the count is not exactly the area's bytes
    assert!(refused(understated_strip_dng(8, 4, 5 * 4 * 2, Some([0, 0, 4, 6]))));
    // the area does not start at column 0
    assert!(refused(understated_strip_dng(8, 4, 6 * 4 * 2, Some([0, 2, 4, 8]))));
    // the file does not hold the whole raster
    let mut cut = understated_strip_dng(8, 4, 6 * 4 * 2, Some([0, 0, 4, 6]));
    cut.truncate(cut.len() - 8);
    assert!(refused(cut));
}

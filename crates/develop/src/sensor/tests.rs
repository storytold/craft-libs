use super::*;

#[test]
fn a_picture_made_elsewhere_is_finished_like_develop_does() {
    let (w, h) = (12usize, 10usize);
    let raw = RawImage {
        format: RawFormat::Dng,
        width: w,
        height: h,
        cpp: 1,
        data: RawData::U16((0..w * h).map(|i| (100 + (i * 37) % 900) as u16).collect()),
        cfa: Cfa::bayer("RGGB"),
        bits: 12,
        black: BlackLevel::uniform(64.0),
        white: vec![1023.0],
        active_area: Rect::new(2, 2, 8, 6),
        crop: Rect::new(1, 1, 5, 4),
        orientation: Orientation::Normal,
        color: ColorData::default(),
        wb_multipliers: None,
        linearized: false,
        pixel_aspect: 1.0,
        opcodes: OpcodeLists::default(),
        camera: Camera::default(),
        cinema: Default::default(),
        level_sources: Default::default(),
        lens_metadata: Default::default(),
    };
    let direct = raw.develop(Method::Bilinear).unwrap();
    let by_hand = raw.finish_demosaiced(demosaic(&raw.normalized().unwrap(), Method::Bilinear));
    assert_eq!((direct.width, direct.height), (5, 4));
    assert_eq!(direct, by_hand);
    // the crop is what develop keeps; no usable crop keeps everything
    assert_eq!(raw.develop_crop(8, 6), Rect::new(1, 1, 5, 4));
    let mut whole = raw.clone();
    whole.crop = Rect::default();
    assert_eq!(whole.develop_crop(8, 6), Rect::new(0, 0, 8, 6));
    assert_eq!(whole.develop(Method::Bilinear).unwrap().width, 8);
    // a crop that pokes out is clipped to the picture
    whole.crop = Rect::new(6, 4, 10, 10);
    assert_eq!(whole.develop_crop(8, 6), Rect::new(6, 4, 2, 2));

    // tall pixels (D1X: 2.0) are stretched to square ones: the rows double, `developed_size` says so
    let mut tall = raw.clone();
    tall.pixel_aspect = 2.0;
    let d = tall.develop(Method::Bilinear).unwrap();
    assert_eq!((d.width, d.height), (5, 8));
    assert_eq!(tall.info().developed_size(), (5, 8));
    assert_eq!(raw.info().developed_size(), (5, 4));
    assert!(!tall.can_bin(2));
    assert_eq!(stretched_height(1324, 2.0), 2648);
    assert_eq!(stretched_height(1324, 1.0), 1324);
}

/// A maker-note crop (`PEF` AOC / `ORF` ImageProcessing) with an origin of `u64::MAX` used to
/// pass the bounds check (`x + width` wraps to 0) and then index out of bounds in
/// `normalized`. It must be a `Corrupt` error at validate time instead.
#[test]
fn an_active_area_that_wraps_past_the_image_is_corrupt() {
    let (w, h) = (12usize, 10usize);
    let mut raw = RawImage {
        format: RawFormat::Dng,
        width: w,
        height: h,
        cpp: 1,
        data: RawData::U16(vec![100; w * h]),
        cfa: Cfa::bayer("RGGB"),
        bits: 12,
        black: BlackLevel::uniform(64.0),
        white: vec![1023.0],
        active_area: Rect::new(usize::MAX, 0, 1, 1),
        crop: Rect::default(),
        orientation: Orientation::Normal,
        color: ColorData::default(),
        wb_multipliers: None,
        linearized: false,
        opcodes: OpcodeLists::default(),
        camera: Camera::default(),
        cinema: Default::default(),
        level_sources: Default::default(),
        lens_metadata: Default::default(),
        pixel_aspect: 1.0,
    };
    assert!(raw.validate().is_err(), "`x + width` wrapped: the area is outside the image");
    assert!(raw.normalized().is_err());
    raw.active_area = Rect::new(0, 0, w + 1, h);
    assert!(raw.validate().is_err(), "one past the last column is outside too");
}

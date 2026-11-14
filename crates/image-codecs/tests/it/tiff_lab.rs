//! CIE L*a*b* TIFFs: CIELab (PhotometricInterpretation 8) as Photoshop writes it, with a* and b*
//! signed, and ICCLab (9) with them offset, both read into the Lab layout (a* and b* as
//! `(a + 128) / 255`).

use craft_image_codecs::*;

/// One Lab pixel in the codec's normalized form.
fn lab(l: f32, a: f32, b: f32) -> [f32; 3] {
    [l / 100.0, (a + 128.0) / 255.0, (b + 128.0) / 255.0]
}

fn image(sample: SampleType) -> Image {
    let px: Vec<f32> = [lab(50.0, 50.0, -30.0), lab(100.0, 0.0, 0.0)].concat();
    Image::from_normalized(2, 1, ChannelLayout::Lab, sample, &px).unwrap()
}

fn uncompressed(img: &Image) -> Vec<u8> {
    encode(img, Format::Tiff, &EncodeOptions { tiff_compression: TiffCompression::None, ..Default::default() }).unwrap()
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[test]
fn eight_bit_cielab_holds_a_and_b_signed() {
    let file = uncompressed(&image(SampleType::U8));
    // L* 50 → 128, a* 50, b* −30 as a signed byte; then white with neutral a* and b*.
    assert!(find(&file, &[128, 50, (-30i8) as u8, 255, 0, 0]).is_some(), "CIELab samples");
    let back = decode(&file).unwrap();
    assert_eq!((back.layout(), back.sample_type()), (ChannelLayout::Lab, SampleType::U8));
    assert_eq!(back.data(), image(SampleType::U8).data());
}

#[test]
fn sixteen_bit_cielab_holds_256_a() {
    let file = uncompressed(&image(SampleType::U16));
    let le = &file[..2] == b"II";
    let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    // a* 50 → 12800, b* −30 → −7680, white's a* and b* 0.
    let want: Vec<u8> = [32768u16, 12800, (-7680i16) as u16, 65535, 0, 0].iter().flat_map(|v| u16b(*v)).collect();
    assert!(find(&file, &want).is_some(), "CIELab samples");
    let back = decode(&file).unwrap();
    assert_eq!((back.layout(), back.sample_type()), (ChannelLayout::Lab, SampleType::U16));
    let (a, b) = (image(SampleType::U16).to_normalized(), back.to_normalized());
    assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= 1.5 / 65535.0), "{a:?} {b:?}");
}

#[test]
fn icclab_reads_with_offset_a_and_b() {
    // The CIELab file turned into ICCLab: PhotometricInterpretation 9, a* and b* + 128.
    let mut file = uncompressed(&image(SampleType::U8));
    let at = find(&file, &[128, 50, (-30i8) as u8, 255, 0, 0]).unwrap();
    for i in [1, 2, 4, 5] {
        file[at + i] ^= 0x80;
    }
    let le = &file[..2] == b"II";
    let entry: Vec<u8> = if le { [262u16.to_le_bytes(), 3u16.to_le_bytes()].concat() } else { [262u16.to_be_bytes(), 3u16.to_be_bytes()].concat() };
    let e = find(&file, &entry).expect("PhotometricInterpretation entry");
    let nine = if le { 9u16.to_le_bytes() } else { 9u16.to_be_bytes() };
    file[e + 8..e + 10].copy_from_slice(&nine);
    let back = decode(&file).unwrap();
    assert_eq!(back.layout(), ChannelLayout::Lab);
    assert_eq!(back.data(), image(SampleType::U8).data());
}

#[test]
fn lab_converts_to_rgb_for_other_formats() {
    // White and a saturated orange-red; PNG has no Lab, so the encoder plan converts to RGB.
    let img = image(SampleType::U8);
    let png = decode(&encode(&img, Format::Png, &EncodeOptions::default()).unwrap()).unwrap();
    assert!(png.layout().is_rgb());
    let p = png.to_rgba8();
    assert_eq!(&p[4..7], &[255, 255, 255], "white");
    assert!(p[0] > 180 && p[1] < 120 && p[2] > 140, "L 50, a 50, b −30 is a magenta-ish purple: {:?}", &p[..3]);
}

#[test]
fn lab_keeps_extra_samples() {
    // A spot plate next to Lab: written as CIELab plus one unspecified extra sample, and read
    // back as Lab with the spot (not as LabA).
    for (layout, sample) in [(ChannelLayout::Lab, SampleType::U8), (ChannelLayout::LabA, SampleType::U16)] {
        let px: Vec<f32> =
            [lab(50.0, 50.0, -30.0), lab(100.0, 0.0, 0.0)].iter().flat_map(|p| p.iter().copied().chain(layout.has_alpha().then_some(0.5))).collect();
        let mut img = Image::from_normalized(2, 1, layout, sample, &px).unwrap();
        let spot: Vec<u8> = (0..2 * sample.bytes()).map(|i| (i * 37 + 5) as u8).collect();
        img.meta.extra_samples = vec![ExtraSample { kind: 0, data: spot.clone() }];
        let file = uncompressed(&img);
        if layout == ChannelLayout::Lab {
            assert!(find(&file, &[128, 50, (-30i8) as u8, spot[0], 255, 0, 0, spot[1]]).is_some(), "CIELab samples with the spot");
        }
        let back = decode(&file).unwrap();
        assert_eq!((back.layout(), back.sample_type()), (layout, sample));
        let (a, b) = (img.to_normalized(), back.to_normalized());
        assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() <= 1.5 / 65535.0), "{layout:?}: {a:?} {b:?}");
        assert_eq!(back.meta.extra_samples, vec![ExtraSample { kind: 0, data: spot }], "{layout:?}");
    }
}

#[test]
fn float_lab_writes_its_extra_samples_with_16_bits() {
    let px: Vec<f32> = [lab(50.0, 50.0, -30.0), lab(100.0, 0.0, 0.0)].concat();
    let mut img = Image::from_normalized(2, 1, ChannelLayout::Lab, SampleType::F32, &px).unwrap();
    img.meta.extra_samples = vec![ExtraSample { kind: 0, data: [0.25f32, 1.0].iter().flat_map(|v| v.to_ne_bytes()).collect() }];
    let back = decode(&uncompressed(&img)).unwrap();
    assert_eq!((back.layout(), back.sample_type()), (ChannelLayout::Lab, SampleType::U16));
    let spot: Vec<u8> = [16384u16, 65535].iter().flat_map(|v| v.to_ne_bytes()).collect();
    assert_eq!(back.meta.extra_samples, vec![ExtraSample { kind: 0, data: spot }]);
}

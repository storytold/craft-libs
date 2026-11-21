//! The compatibility facade must retain its policy while sharing the codec implementation.
use craft_raster_codecs::{ChannelLayout, CodecError, Image, Limits, SampleType, dds, fits, iff, pnm, xpm};

#[test]
fn facade_refuses_conversions_the_generic_encoder_can_plan() {
    for layout in [ChannelLayout::GrayA, ChannelLayout::Rgba, ChannelLayout::Cmyk, ChannelLayout::CmykA] {
        let image = Image::from_f32(1, 1, layout, &vec![0.5; layout.channels()]).unwrap();
        assert!(matches!(pnm::encode(&image), Err(CodecError::Encode { .. })));
        assert!(matches!(fits::encode(&image), Err(CodecError::Encode { .. })));
    }
    for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Cmyk, ChannelLayout::CmykA] {
        let image = Image::from_u8(1, 1, layout, vec![127; layout.channels()]).unwrap();
        for compression in [dds::DdsCompression::Bc1, dds::DdsCompression::Bc2, dds::DdsCompression::Bc3] {
            assert!(matches!(dds::encode(&image, dds::DdsOptions { compression, mipmaps: false }), Err(CodecError::Encode { .. })));
        }
    }
    for layout in [ChannelLayout::Rgb, ChannelLayout::Rgba] {
        let image = Image::from_u16(1, 1, layout, &vec![32768; layout.channels()]).unwrap();
        assert!(matches!(iff::encode(&image), Err(CodecError::Encode { .. })));
        assert!(matches!(dds::encode(&image, dds::DdsOptions::default()), Err(CodecError::Encode { .. })));
    }
}

#[test]
fn fits_facade_keeps_metadata_samples_and_explicit_limits() {
    let mut image = Image::from_u16(2, 1, ChannelLayout::Gray, &[0, 65535]).unwrap();
    image.text.push(("OBJECT".into(), "shared codec".into()));
    let bytes = fits::encode(&image).unwrap();
    let decoded = fits::decode(&bytes, &Limits::default()).unwrap();
    assert_eq!(decoded.sample_type(), SampleType::U16);
    assert_eq!(decoded.data(), image.data());
    assert!(decoded.text.contains(&("OBJECT".into(), "shared codec".into())));
    for limits in
        [Limits { max_width: 1, ..Limits::default() }, Limits { max_pixels: 1, ..Limits::default() }, Limits { max_alloc: 3, ..Limits::default() }]
    {
        assert!(matches!(fits::decode(&bytes, &limits), Err(CodecError::LimitExceeded(_))));
    }
}

#[test]
fn x11_table_lookup_does_not_adopt_xpm_aliases_or_normalization() {
    assert_eq!(xpm::x11_color_rgb("red"), Some([1.0, 0.0, 0.0]));
    for name in ["Red", "light blue", "light-blue", "light_blue", "none", "transparent", ""] {
        assert_eq!(xpm::x11_color_rgb(name), None, "{name}");
    }
}

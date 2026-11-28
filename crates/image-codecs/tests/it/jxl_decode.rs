//! JPEG XL decode through craft-jxl: lossless round trips, metadata, and hostile containers.
#![cfg(feature = "jxl")]

use craft_image_codecs::{ChannelLayout, CodecError, DecodeOptions, EncodeOptions, Format, Image, Limits, SampleType, decode, decode_with, encode};

fn rgba_8bit() -> Image {
    let data = vec![0, 255, 17, 127, 20, 40, 80, 255, 255, 0, 2, 0, 10, 20, 30, 64, 0, 0, 0, 1, 90, 80, 70, 128];
    Image::from_u8(3, 2, ChannelLayout::Rgba, data).unwrap()
}

#[test]
fn feature_config_an_eight_bit_export_decodes_back_to_the_same_pixels() {
    let input = rgba_8bit();
    let bytes = encode(&input, Format::Jxl, &EncodeOptions::default()).unwrap();
    let out = decode(&bytes).unwrap();
    assert_eq!((out.width(), out.height(), out.layout(), out.sample_type()), (3, 2, ChannelLayout::Rgba, SampleType::U8));
    assert_eq!(out.data(), input.data());
}

#[test]
fn feature_config_a_sixteen_bit_gray_export_decodes_back_to_the_same_samples() {
    let samples: Vec<u16> = (0..12).map(|i| i * 5461).collect();
    let input = Image::from_u16(4, 3, ChannelLayout::Gray, &samples).unwrap();
    let bytes = encode(&input, Format::Jxl, &EncodeOptions::default()).unwrap();
    let out = decode(&bytes).unwrap();
    assert_eq!((out.layout(), out.sample_type()), (ChannelLayout::Gray, SampleType::U16));
    assert_eq!(out.data(), input.data());
}

#[test]
fn feature_config_the_pixel_limit_is_an_error_not_an_allocation() {
    let bytes = encode(&rgba_8bit(), Format::Jxl, &EncodeOptions::default()).unwrap();
    let opts = DecodeOptions { limits: Limits { max_pixels: 4, ..Limits::default() }, ..DecodeOptions::default() };
    let err = decode_with(&bytes, &opts);
    assert!(matches!(err, Err(CodecError::LimitExceeded(_))), "{err:?}");
}

#[test]
fn feature_config_every_prefix_of_a_file_is_an_error_or_an_image_never_a_panic() {
    let bytes = encode(&rgba_8bit(), Format::Jxl, &EncodeOptions::default()).unwrap();
    for end in 0..bytes.len() {
        let _ = decode(&bytes[..end]);
    }
}

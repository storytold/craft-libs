//! Lossless JPEG XL export and honest export-only capability reporting.

use craft_image_codecs::{
    ASYMMETRIC_EXCEPTIONS, ChannelLayout, CodecError, EncodeOptions, Format, Image, SampleType, caps, decode, detect, encode, from_extension,
};

#[test]
fn jpeg_xl_is_export_only_and_detects_both_signatures() {
    let c = caps(Format::Jxl);
    assert!(c.write && !c.read);
    assert_eq!(c.depths, &[SampleType::U8, SampleType::U16]);
    assert_eq!(c.layouts, &[ChannelLayout::Gray, ChannelLayout::Rgb, ChannelLayout::Rgba]);
    assert!(c.alpha && !c.icc && !c.exif && !c.xmp && !c.dpi && !c.lossy);
    assert_eq!(Format::Jxl.mime_type(), "image/jxl");
    assert_eq!(from_extension("image.JXL"), Some(Format::Jxl));
    assert_eq!(detect(&[0xff, 0x0a, 0, 0]), Some(Format::Jxl));
    assert_eq!(detect(b"\0\0\0\x0cJXL \r\n\x87\n"), Some(Format::Jxl));
    assert!(ASYMMETRIC_EXCEPTIONS.iter().any(|(format, reason)| *format == Format::Jxl && !reason.is_empty()));
    let err = decode(&[0xff, 0x0a]);
    assert!(matches!(err, Err(CodecError::Unsupported { format: Format::Jxl, .. })), "{err:?}");
}

#[test]
fn jpeg_xl_encoding_is_deterministic_and_converts_float_to_integer() {
    let input =
        Image::from_u8(3, 2, ChannelLayout::Rgba, vec![0, 255, 17, 127, 20, 40, 80, 255, 255, 0, 2, 0, 10, 20, 30, 64, 0, 0, 0, 1, 90, 80, 70, 128])
            .unwrap();
    let opts = EncodeOptions::default();
    let one = encode(&input, Format::Jxl, &opts).unwrap();
    let two = encode(&input, Format::Jxl, &opts).unwrap();
    assert_eq!(one, two);
    assert_eq!(detect(&one), Some(Format::Jxl));
    let float = Image::from_f32(2, 2, ChannelLayout::Rgb, &[0.0, 0.5, 1.0, 0.25, 0.5, 0.75, 0.75, 0.5, 0.25, 1.0, 0.0, 0.5]).unwrap();
    let bytes = encode(&float, Format::Jxl, &opts).unwrap();
    assert_eq!(detect(&bytes), Some(Format::Jxl));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn jpeg_xl_decodes_losslessly_with_independent_decoder() {
    use jxl_oxide::JxlImage;

    for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        let channels = layout.channels();
        for sample in [SampleType::U8, SampleType::U16] {
            let (w, h) = (7, 5);
            let count = w as usize * h as usize * channels;
            let expected = match sample {
                SampleType::U8 => (0..count).map(|i| (((i * 37 + 19) % 256) as u16) * 257).collect::<Vec<_>>(),
                SampleType::U16 => (0..count).map(|i| ((i * 9973 + 27) % 65536) as u16).collect::<Vec<_>>(),
                _ => unreachable!(),
            };
            let input = match sample {
                SampleType::U8 => {
                    let data = expected.iter().map(|v| (v / 257) as u8).collect();
                    Image::from_u8(w, h, layout, data).unwrap()
                }
                SampleType::U16 => Image::from_u16(w, h, layout, &expected).unwrap(),
                _ => unreachable!(),
            };
            // GrayA is losslessly widened to RGBA. Keep the oracle comparison
            // against original gray and alpha samples, not only against a converted image.
            let (output_channels, expected_output) = if layout == ChannelLayout::GrayA {
                let expanded: Vec<u16> = expected.as_chunks::<2>().0.iter().flat_map(|[gray, alpha]| [*gray, *gray, *gray, *alpha]).collect();
                (4, expanded)
            } else {
                (channels, expected.clone())
            };
            let encoded = encode(&input, Format::Jxl, &EncodeOptions::default()).unwrap();
            let decoder = JxlImage::builder().read(&encoded[..]).unwrap();
            assert_eq!((decoder.width(), decoder.height()), (w, h));
            let render = decoder.render_frame(0).unwrap();
            let mut stream = render.stream();
            assert_eq!(stream.channels() as usize, output_channels, "{layout:?} {sample:?}");
            let mut output = vec![0u16; expected_output.len()];
            assert_eq!(stream.write_to_buffer(&mut output), expected_output.len());
            assert_eq!(output, expected_output, "{layout:?} {sample:?}");
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn jpeg_xl_preserves_sixteen_bit_alpha_across_dimension_header_sizes() {
    use jxl_oxide::JxlImage;

    // Exercise each size-header coding tier, and non-opaque alpha extremes.
    for width in [2u32, 513, 8193] {
        let height = 2u32;
        let mut samples = Vec::with_capacity(width as usize * height as usize * 4);
        for i in 0..(width as usize * height as usize) {
            samples.extend_from_slice(&[(i as u16).wrapping_mul(127), 65535, (i as u16).wrapping_mul(9973), [0u16, 1, 32768, 65535][i % 4]]);
        }
        let input = Image::from_u16(width, height, ChannelLayout::Rgba, &samples).unwrap();
        let encoded = encode(&input, Format::Jxl, &EncodeOptions::default()).unwrap();
        let decoder = JxlImage::builder().read(&encoded[..]).unwrap();
        let render = decoder.render_frame(0).unwrap();
        let mut stream = render.stream();
        assert_eq!(stream.channels(), 4, "width={width}");
        let mut output = vec![0u16; samples.len()];
        assert_eq!(stream.write_to_buffer(&mut output), samples.len());
        assert_eq!(output, samples, "width={width}");
    }
}

#[test]
fn jpeg_xl_rejects_one_pixel_images_without_panicking() {
    // zune-jpegxl currently requires at least two pixels on each axis.
    let image = Image::from_u8(1, 1, ChannelLayout::Rgb, vec![42, 100, 255]).unwrap();
    assert!(matches!(encode(&image, Format::Jxl, &EncodeOptions::default()), Err(CodecError::Encode { format: Format::Jxl, .. })));
}

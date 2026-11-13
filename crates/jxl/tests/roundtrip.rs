use craft_jxl::{DecodeLimits, EncodeOptions, ImageRef, Layout, Metadata, Samples, decode, encode};

#[test]
fn lossless_rgba_roundtrip_keeps_alpha_layout() {
    let samples = [255, 0, 0, 255, 0, 255, 0, 64];
    let image = ImageRef { width: 2, height: 1, layout: Layout::Rgba, samples: Samples::U8(&samples) };
    let encoded = encode(&image, EncodeOptions { lossless: true, ..EncodeOptions::default() }, &Metadata::default());
    assert!(encoded.is_ok(), "valid image should encode: {encoded:?}");
    let Ok(encoded) = encoded else { return };
    let decoded = decode(&encoded, DecodeLimits::default());
    assert!(decoded.is_ok(), "encoded image should decode: {decoded:?}");
    let Ok(decoded) = decoded else { return };

    assert_eq!((decoded.width, decoded.height), (2, 1));
    assert_eq!(decoded.layout, Layout::Rgba);
    assert_eq!(decoded.pixels.len(), 8);
    assert_eq!(decoded.pixels[3], 1.0);
    assert!((decoded.pixels[7] - 64.0 / 255.0).abs() < 0.01);
}

#[test]
fn decoder_enforces_input_and_pixel_limits() {
    let samples = [10_u8, 20, 30];
    let image = ImageRef { width: 1, height: 1, layout: Layout::Rgb, samples: Samples::U8(&samples) };
    let encoded = encode(&image, EncodeOptions { lossless: true, ..EncodeOptions::default() }, &Metadata::default());
    assert!(encoded.is_ok(), "valid image should encode: {encoded:?}");
    let Ok(encoded) = encoded else { return };

    let input_limit = DecodeLimits { max_input_bytes: encoded.len() - 1, ..DecodeLimits::default() };
    assert!(matches!(decode(&encoded, input_limit), Err(craft_jxl::Error::LimitExceeded(_))));
    let pixel_limit = DecodeLimits { max_pixels: 0, ..DecodeLimits::default() };
    assert!(matches!(decode(&encoded, pixel_limit), Err(craft_jxl::Error::LimitExceeded(_))));
}

#[test]
fn malformed_input_returns_an_error() {
    assert!(decode(b"not a JPEG XL file", DecodeLimits::default()).is_err());
}

#[test]
fn encoder_rejects_short_sample_buffers() {
    let image = ImageRef { width: 2, height: 1, layout: Layout::Rgb, samples: Samples::U8(&[1, 2, 3]) };
    assert!(matches!(encode(&image, EncodeOptions::default(), &Metadata::default()), Err(craft_jxl::Error::InvalidImage(_))));
}

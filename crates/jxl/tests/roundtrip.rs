use craft_jxl::{
    DecodeLimits, EncodeOptions, HDR_REFERENCE_WHITE_NITS, ImageRef, JxlColour, Layout, Metadata, OutputColor, Samples, decode, decode_with_color,
    encode,
};

fn raw_codestream(bytes: &[u8]) -> Vec<u8> {
    if bytes.starts_with(b"\xff\x0a") {
        return bytes.to_vec();
    }
    let mut raw = Vec::new();
    let mut at = 12_usize;
    while at.checked_add(8).is_some_and(|end| end <= bytes.len()) {
        let Some(size) = bytes.get(at..at + 4).and_then(|s| s.try_into().ok()).map(u32::from_be_bytes) else { break };
        let end = if size == 0 { bytes.len() } else { at.saturating_add(size as usize) };
        let Some(kind) = bytes.get(at + 4..at + 8) else { break };
        if end < at + 8 || end > bytes.len() {
            break;
        }
        let header = if kind == b"jxlp" { 12 } else { 8 };
        if (kind == b"jxlc" || kind == b"jxlp")
            && let Some(data) = bytes.get(at + header..end)
        {
            raw.extend_from_slice(data);
        }
        if end <= at {
            break;
        }
        at = end;
    }
    raw
}

fn box_bytes(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len() + 8).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
}

#[test]
fn lossless_rgba_roundtrip_keeps_alpha_layout() {
    let samples = [255, 0, 0, 255, 0, 255, 0, 64];
    let image = ImageRef { width: 2, height: 1, layout: Layout::Rgba, samples: Samples::U8(&samples) };
    let exif = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x01\0\0\0\0\0\0\0";
    let xmp = b"<xmp/>";
    let metadata = Metadata { exif: Some(exif.to_vec()), xmp: Some(xmp.to_vec()), ..Metadata::default() };
    let encoded = encode(&image, EncodeOptions { lossless: true, ..EncodeOptions::default() }, &metadata);
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
    assert_eq!(decoded.metadata.exif.as_deref(), Some(exif.as_slice()));
    assert_eq!(decoded.metadata.xmp.as_deref(), Some(xmp.as_slice()));
}

#[test]
fn exif_payload_starts_at_its_declared_tiff_header_offset() {
    let image = ImageRef { width: 2, height: 1, layout: Layout::Rgb, samples: Samples::U8(&[10, 20, 30, 40, 50, 60]) };
    let encoded = encode(&image, EncodeOptions { lossless: true, ..EncodeOptions::default() }, &Metadata::default());
    assert!(encoded.is_ok(), "valid image should encode: {encoded:?}");
    let Ok(encoded) = encoded else { return };
    let raw = raw_codestream(&encoded);
    let exif = b"II*\0\x08\0\0\0\0\0\0\0";

    for offset in [0_u32, 6, 64] {
        let mut exif_box = offset.to_be_bytes().to_vec();
        exif_box.resize(4 + offset as usize, 0xab);
        exif_box.extend_from_slice(exif);
        let mut container = b"\0\0\0\x0cJXL \x0d\x0a\x87\x0a".to_vec();
        box_bytes(&mut container, b"ftyp", b"jxl \0\0\0\0jxl ");
        box_bytes(&mut container, b"jxlc", &raw);
        box_bytes(&mut container, b"Exif", &exif_box);

        let decoded = decode(&container, DecodeLimits::default());
        assert!(decoded.is_ok(), "container with Exif offset {offset} should decode: {decoded:?}");
        let Ok(decoded) = decoded else { return };
        assert_eq!(decoded.metadata.exif.as_deref(), Some(exif.as_slice()), "TIFF header offset {offset}");
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn associated_alpha_is_unpremultiplied_before_color_conversion() {
    let straight = [[200_u8, 100, 50, 128], [30, 180, 240, 192], [100, 80, 60, 255], [220, 110, 40, 160]];
    let mut premultiplied = Vec::with_capacity(straight.len() * 4);
    for pixel in &straight {
        let alpha = f32::from(pixel[3]) / 255.0;
        premultiplied.extend(pixel[..3].iter().map(|channel| (f32::from(*channel) * alpha).round() as u8));
        premultiplied.push(pixel[3]);
    }
    let metadata = jixel::FlMeta { alpha_premultiplied: true, ..jixel::FlMeta::srgb() };
    let encoded = jixel::encode_fast_lossless(&premultiplied, 2, 2, jixel::ColorSpace::Rgb, true, &metadata);
    assert!(encoded.is_ok(), "associated-alpha image should encode: {encoded:?}");
    let Ok(encoded) = encoded else { return };

    let decoded = decode_with_color(&encoded, DecodeLimits::default(), OutputColor::LinearRec2020);
    assert!(decoded.is_ok(), "associated-alpha image should decode: {decoded:?}");
    let Ok(decoded) = decoded else { return };
    assert_eq!(decoded.layout, Layout::Rgba);
    assert!(!decoded.pixels_linear_rec2020, "the source encoding is needed for the caller's color transform");
    assert!(!decoded.pixels_premultiplied, "associated color samples must be divided before nonlinear conversion");

    for (i, source) in straight.iter().enumerate() {
        let alpha = f32::from(source[3]) / 255.0;
        assert!((decoded.pixels[i * 4 + 3] - alpha).abs() < 1e-6);
        for (channel, &value) in source[..3].iter().enumerate() {
            let expected = f32::from(value) / 255.0;
            assert!((decoded.pixels[i * 4 + channel] - expected).abs() < 0.01, "pixel {i}, channel {channel}");
        }
    }
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
    assert!(matches!(decode(&encoded, pixel_limit), Err(craft_jxl::Error::PixelLimitExceeded { width: 1, height: 1, max_pixels: 0 })));
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

#[test]
fn lossless_gray_input_is_expanded_to_rgb() {
    let samples = [0, 40, 128, 255];
    let image = ImageRef { width: 4, height: 1, layout: Layout::Gray, samples: Samples::U8(&samples) };
    let encoded = encode(&image, EncodeOptions { lossless: true, ..EncodeOptions::default() }, &Metadata::default());
    assert!(encoded.is_ok(), "valid grayscale image should encode: {encoded:?}");
    let Ok(encoded) = encoded else { return };
    let decoded = decode(&encoded, DecodeLimits::default());
    assert!(decoded.is_ok(), "grayscale image should decode: {decoded:?}");
    let Ok(decoded) = decoded else { return };
    assert_eq!(decoded.layout, Layout::Rgb);
    assert_eq!(decoded.pixels.len(), 4 * 3);
    let (pixels, remainder) = decoded.pixels.as_chunks::<3>();
    assert!(remainder.is_empty());
    for (pixel, gray) in pixels.iter().zip(samples) {
        let expected = f32::from(gray) / 255.0;
        assert!(pixel.iter().all(|channel| (channel - expected).abs() < 0.01));
    }
}

#[test]
fn lossless_u16_samples_keep_their_bit_depth_and_values() {
    let samples: Vec<u16> = (0..40 * 30 * 3).map(|i| (i as u32 * 2741 % 65536) as u16).collect();
    let image = ImageRef { width: 40, height: 30, layout: Layout::Rgb, samples: Samples::U16(&samples) };
    let encoded = encode(&image, EncodeOptions { lossless: true, ..EncodeOptions::default() }, &Metadata::default());
    assert!(encoded.is_ok(), "valid 16-bit image should encode: {encoded:?}");
    let Ok(encoded) = encoded else { return };
    let decoded = decode(&encoded, DecodeLimits::default());
    assert!(decoded.is_ok(), "16-bit image should decode: {decoded:?}");
    let Ok(decoded) = decoded else { return };
    assert_eq!(decoded.bit_depth, 16);
    assert_eq!(decoded.layout, Layout::Rgb);
    assert_eq!(decoded.pixels.len(), samples.len());
    for (got, sample) in decoded.pixels.iter().zip(samples) {
        let expected = f32::from(sample) / 65535.0;
        assert!((got - expected).abs() < 1e-5, "16-bit value differs: {got} vs {expected}");
    }
}

#[test]
fn enum_encoded_pixels_can_be_rendered_as_linear_rec2020() {
    let samples = [180, 40, 20, 10, 140, 60, 20, 80, 180];
    let image = ImageRef { width: 3, height: 1, layout: Layout::Rgb, samples: Samples::U8(&samples) };
    let encoded = encode(&image, EncodeOptions { lossless: true, colour: JxlColour::Rec2020, ..EncodeOptions::default() }, &Metadata::default());
    assert!(encoded.is_ok(), "valid image should encode: {encoded:?}");
    let Ok(encoded) = encoded else { return };
    let decoded = decode_with_color(&encoded, DecodeLimits::default(), OutputColor::LinearRec2020);
    assert!(decoded.is_ok(), "valid image should decode: {decoded:?}");
    let Ok(decoded) = decoded else { return };

    assert!(!decoded.xyb_encoded, "lossless enum encoding should use Modular coding");
    assert_eq!(decoded.layout, Layout::Rgb);
    assert_eq!((decoded.width, decoded.height), (3, 1));
    assert!(decoded.pixels.iter().all(|sample| sample.is_finite()));
    assert!(decoded.rendered_icc.len() > 128, "requested Rec. 2020 linear target has a rendered profile");
}

#[test]
fn xybcoded_pixels_use_the_linear_srgb_workaround_before_rec2020() {
    let samples = [
        250, 5, 5, 128, 10, 240, 10, 255, 5, 10, 250, 255, 190, 130, 40, 255, 120, 20, 170, 255, 20, 160, 180, 255, 240, 230, 10, 255, 80, 90, 100,
        255,
    ];
    let image = ImageRef { width: 4, height: 2, layout: Layout::Rgba, samples: Samples::U8(&samples) };
    let encoded = encode(&image, EncodeOptions { quality: 95, colour: JxlColour::Srgb, ..EncodeOptions::default() }, &Metadata::default());
    assert!(encoded.is_ok(), "valid image should encode: {encoded:?}");
    let Ok(encoded) = encoded else { return };
    let decoded = decode_with_color(&encoded, DecodeLimits::default(), OutputColor::LinearRec2020);
    assert!(decoded.is_ok(), "valid image should decode: {decoded:?}");
    let Ok(decoded) = decoded else { return };

    assert!(decoded.xyb_encoded, "lossy RGB JPEG XL should use XYB");
    assert_eq!(decoded.layout, Layout::Rgba);
    assert_eq!(decoded.pixels.len(), 4 * 2 * 4);
    let (decoded_pixels, decoded_remainder) = decoded.pixels.as_chunks::<4>();
    assert!(decoded_remainder.is_empty());
    assert!(decoded_pixels.iter().all(|pixel| (0.0..=1.0).contains(&pixel[3])));
    assert!(decoded.pixels.iter().all(|sample| sample.is_finite()));

    let reference = jxl_oxide::JxlImage::builder().read(encoded.as_slice());
    assert!(reference.is_ok(), "reference decoder should accept the encoded image: {reference:?}");
    let Ok(mut reference) = reference else { return };
    reference.request_color_encoding(jxl_oxide::color::EnumColourEncoding {
        colour_space: jxl_oxide::color::ColourSpace::Rgb,
        white_point: jxl_oxide::color::WhitePoint::D65,
        primaries: jxl_oxide::color::Primaries::Srgb,
        tf: jxl_oxide::color::TransferFunction::Linear,
        rendering_intent: jxl_oxide::color::RenderingIntent::Relative,
    });
    let reference_frame = reference.render_frame(0);
    assert!(reference_frame.is_ok(), "reference frame should render: {reference_frame:?}");
    let Ok(reference_frame) = reference_frame else { return };
    let reference = reference_frame.image_all_channels();
    assert_eq!(reference.channels(), 4);
    let (actual_pixels, actual_remainder) = decoded.pixels.as_chunks::<4>();
    let (reference_pixels, reference_remainder) = reference.buf().as_chunks::<4>();
    assert!(actual_remainder.is_empty());
    assert!(reference_remainder.is_empty());
    for (actual, linear_srgb) in actual_pixels.iter().zip(reference_pixels) {
        let [r, g, b] = [linear_srgb[0], linear_srgb[1], linear_srgb[2]];
        let expected = [
            0.627_403_9 * r + 0.329_283_04 * g + 0.043_313_067 * b,
            0.069_097_29 * r + 0.919_540_4 * g + 0.011_362_316 * b,
            0.016_391_44 * r + 0.088_013_31 * g + 0.895_595_25 * b,
        ];
        for (got, want) in actual[..3].iter().zip(expected) {
            assert!((got - want).abs() < 1e-6, "linear Rec. 2020 channel differs: {got} vs {want}");
        }
        assert_eq!(actual[3], linear_srgb[3], "color conversion leaves alpha unchanged");
    }
}

#[test]
fn icc_encoded_pixels_keep_the_embedded_profile_under_linear_target_request() {
    let samples = [180, 40, 20, 10, 140, 60, 20, 80, 180];
    let image = ImageRef { width: 3, height: 1, layout: Layout::Rgb, samples: Samples::U8(&samples) };
    let icc = craft_cms::Builtin::DisplayP3.profile().to_bytes();
    let metadata = Metadata { icc: Some(icc.as_ref().clone()), ..Metadata::default() };
    let encoded = encode(&image, EncodeOptions { lossless: true, colour: JxlColour::DisplayP3, ..EncodeOptions::default() }, &metadata);
    assert!(encoded.is_ok(), "valid image should encode: {encoded:?}");
    let Ok(encoded) = encoded else { return };
    let decoder_default = decode(&encoded, DecodeLimits::default());
    let linear_request = decode_with_color(&encoded, DecodeLimits::default(), OutputColor::LinearRec2020);
    assert!(decoder_default.is_ok(), "ICC image should decode: {decoder_default:?}");
    assert!(linear_request.is_ok(), "ICC image should decode: {linear_request:?}");
    let (Ok(decoder_default), Ok(linear_request)) = (decoder_default, linear_request) else { return };

    assert_eq!(decoder_default.metadata.icc.as_deref(), Some(icc.as_slice()));
    assert_eq!(linear_request.metadata.icc.as_deref(), Some(icc.as_slice()));
    assert_eq!(decoder_default.pixels, linear_request.pixels, "without a bundled CMS, the requested target preserves ICC-space samples");
}

#[test]
fn lossy_quality_effort_and_progressive_options_produce_decodable_images() {
    let (width, height) = (160_u32, 120_u32);
    let mut samples = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let fx = x as f32 / width as f32;
            let fy = y as f32 / height as f32;
            samples.extend_from_slice(&[(fx * 255.0) as u8, (fy * 255.0) as u8, ((1.0 - fx) * 200.0) as u8, (((fx + fy) * 0.5) * 255.0) as u8]);
        }
    }
    let image = ImageRef { width, height, layout: Layout::Rgba, samples: Samples::U8(&samples) };
    let encode_options = |quality, effort, progressive| EncodeOptions { quality, effort, progressive, ..EncodeOptions::default() };
    let hi = encode(&image, encode_options(95, craft_jxl::Effort::Normal, false), &Metadata::default());
    let lo = encode(&image, encode_options(40, craft_jxl::Effort::Normal, false), &Metadata::default());
    assert!(hi.is_ok(), "high-quality image should encode: {hi:?}");
    assert!(lo.is_ok(), "low-quality image should encode: {lo:?}");
    let (Ok(hi), Ok(lo)) = (hi, lo) else { return };
    assert!(lo.len() < hi.len(), "quality 40 output {} should be smaller than quality 95 output {}", lo.len(), hi.len());

    for bytes in [&hi, &lo] {
        let decoded = decode(bytes, DecodeLimits::default());
        assert!(decoded.is_ok(), "encoded image should decode: {decoded:?}");
        let Ok(decoded) = decoded else { return };
        assert_eq!((decoded.width, decoded.height), (width, height));
        assert_eq!(decoded.layout, Layout::Rgba);
        let threshold = if bytes.len() == hi.len() { 38.0 } else { 28.0 };
        let psnr = psnr8(&samples, &decoded.pixels, 4);
        assert!(psnr > threshold, "PSNR {psnr:.2} dB must exceed {threshold:.1} dB");
    }

    for effort in [craft_jxl::Effort::Fast, craft_jxl::Effort::Normal] {
        for progressive in [false, true] {
            let encoded = encode(&image, encode_options(90, effort, progressive), &Metadata::default());
            assert!(encoded.is_ok(), "{effort:?} progressive={progressive} should encode: {encoded:?}");
            let Ok(encoded) = encoded else { return };
            let decoded = decode(&encoded, DecodeLimits::default());
            assert!(decoded.is_ok(), "{effort:?} progressive={progressive} should decode: {decoded:?}");
            let Ok(decoded) = decoded else { return };
            let psnr = psnr8(&samples, &decoded.pixels, 4);
            assert!(psnr > 34.0, "{effort:?} progressive={progressive} PSNR {psnr:.2} dB");
        }
    }
}

fn psnr8(source: &[u8], decoded: &[f32], channels: usize) -> f64 {
    let mut squared_error = 0.0;
    for (source, decoded) in source.chunks_exact(channels).zip(decoded.chunks_exact(channels)) {
        for channel in 0..3 {
            let difference = f64::from(source[channel]) - f64::from(decoded[channel] * 255.0);
            squared_error += difference * difference;
        }
    }
    let samples = (source.len() / channels * 3) as f64;
    let mse = squared_error / samples;
    if mse == 0.0 { f64::INFINITY } else { 10.0 * (255.0 * 255.0 / mse).log10() }
}

#[test]
fn rec2020_pq_float_samples_are_written_as_hdr() {
    let (width, height) = (64_u32, 32_u32);
    let samples: Vec<f32> =
        (0..(width * height) as usize).flat_map(|i| [if i % (width as usize) < width as usize / 2 { 1.0 } else { 4.0 }; 3]).collect();
    let image = ImageRef { width, height, layout: Layout::Rgb, samples: Samples::F32(&samples) };
    for lossless in [false, true] {
        let encoded =
            encode(&image, EncodeOptions { lossless, quality: 95, colour: JxlColour::Rec2020Pq, ..EncodeOptions::default() }, &Metadata::default());
        assert!(encoded.is_ok(), "HDR image should encode: {encoded:?}");
        let Ok(encoded) = encoded else { return };
        let decoded = jxl_oxide::JxlImage::builder().read(encoded.as_slice());
        assert!(decoded.is_ok(), "reference decoder should read HDR output: {decoded:?}");
        let Ok(decoded) = decoded else { return };
        let metadata = &decoded.image_header().metadata;
        assert_eq!(metadata.tone_mapping.intensity_target, 10_000.0);
        assert!(format!("{:?}", metadata.colour_encoding).contains("Pq"), "{:?}", metadata.colour_encoding);
        let frame = decoded.render_frame(0);
        assert!(frame.is_ok(), "reference decoder should render HDR output: {frame:?}");
        let Ok(frame) = frame else { return };
        let buffer = frame.image_all_channels();
        let channels = buffer.channels();
        assert!(channels >= 3);
        let sample_at = |x: usize| buffer.buf()[((height as usize / 2) * width as usize + x) * channels + 1];
        for (x, nits) in [(width as usize / 4, HDR_REFERENCE_WHITE_NITS), (width as usize * 3 / 4, 4.0 * HDR_REFERENCE_WHITE_NITS)] {
            let encoded_pq = pq_encode(nits);
            let actual = sample_at(x);
            assert!((actual - encoded_pq).abs() < 0.005, "lossless={lossless}: {nits} nits encoded as {actual}, expected {encoded_pq}");
        }
    }
    assert!(matches!(encode(&image, EncodeOptions::default(), &Metadata::default()), Err(craft_jxl::Error::InvalidImage(_))));
}

fn pq_encode(nits: f32) -> f32 {
    const M1: f32 = 2610.0 / 16384.0;
    const M2: f32 = 2523.0 / 4096.0 * 128.0;
    const C1: f32 = 3424.0 / 4096.0;
    const C2: f32 = 2413.0 / 4096.0 * 32.0;
    const C3: f32 = 2392.0 / 4096.0 * 32.0;
    let y = (nits / 10_000.0).clamp(0.0, 1.0).powf(M1);
    ((C1 + C2 * y) / (1.0 + C3 * y)).powf(M2)
}

#[test]
fn bounded_feeds_keep_metadata_after_the_final_progressive_part() {
    let samples: Vec<u8> = (0..8 * 8 * 4).map(|i| (i * 13 % 256) as u8).collect();
    let image = ImageRef { width: 8, height: 8, layout: Layout::Rgba, samples: Samples::U8(&samples) };
    let encoded = encode(&image, EncodeOptions { lossless: true, progressive: true, ..EncodeOptions::default() }, &Metadata::default());
    assert!(encoded.is_ok(), "valid image should encode: {encoded:?}");
    let Ok(encoded) = encoded else { return };
    let raw = raw_codestream(&encoded);
    assert!(raw.starts_with(b"\xff\x0a"), "extract the JXL codestream from the encoder output");

    let mut bytes = b"\0\0\0\x0cJXL \x0d\x0a\x87\x0a".to_vec();
    box_bytes(&mut bytes, b"ftyp", b"jxl \0\0\0\0jxl ");
    let split = raw.len() / 2;
    let mut part = 0_u32.to_be_bytes().to_vec();
    part.extend_from_slice(&raw[..split]);
    box_bytes(&mut bytes, b"jxlp", &part);
    let padding = vec![0; 4096usize.saturating_sub(bytes.len() + 8)];
    box_bytes(&mut bytes, b"free", &padding);
    let mut part = 0x8000_0001_u32.to_be_bytes().to_vec();
    part.extend_from_slice(&raw[split..]);
    box_bytes(&mut bytes, b"jxlp", &part);
    let exif = b"II*\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x01\0\0\0\0\0\0\0";
    let mut exif_box = vec![0; 4];
    exif_box.extend_from_slice(exif);
    box_bytes(&mut bytes, b"Exif", &exif_box);
    let xmp = b"<xmp/>";
    box_bytes(&mut bytes, b"xml ", xmp);
    assert!(bytes.len() < 8192, "metadata fixture should fit in two decoder feeds");

    let reference = jxl_oxide::JxlImage::builder().read(bytes.as_slice());
    assert!(reference.is_ok(), "reference decoder should read the fixture: {reference:?}");
    let Ok(reference) = reference else { return };
    let reference_exif = match reference.aux_boxes().first_exif() {
        Ok(jxl_oxide::AuxBoxData::Data(data)) => Some(data.payload().to_vec()),
        _ => None,
    };
    let reference_xmp = match reference.aux_boxes().first_xml() {
        jxl_oxide::AuxBoxData::Data(data) => Some(data.to_vec()),
        _ => None,
    };
    assert!(reference_exif.is_some(), "reference decoder should preserve the Exif box");
    assert!(reference_xmp.is_some(), "reference decoder should preserve the XMP box");
    let (Some(reference_exif), Some(reference_xmp)) = (reference_exif, reference_xmp) else { return };
    let decoded = decode(&bytes, DecodeLimits::default());
    assert!(decoded.is_ok(), "shared decoder should read the fixture: {decoded:?}");
    let Ok(decoded) = decoded else { return };
    assert_eq!(decoded.metadata.exif.as_deref(), Some(reference_exif.as_slice()));
    assert_eq!(decoded.metadata.xmp.as_deref(), Some(reference_xmp.as_slice()));
}

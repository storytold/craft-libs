//! Format-specific behaviour: Adam7 PNG, JPEG colour models, oracle
//! cross-checks against the `image` crate, and the optional PngSuite corpus.

use crate::common;
use common::*;
use craft_image_codecs::*;

// ---------------------------------------------------------------------------
// PNG Adam7
// ---------------------------------------------------------------------------

fn interlaced(img: &Image) -> Vec<u8> {
    encode(img, Format::Png, &EncodeOptions { png_interlaced: true, ..Default::default() }).unwrap()
}

fn check_adam7(w: u32, h: u32, layout: ChannelLayout, sample: SampleType) {
    let img = synth(w, h, layout, sample, (w * 31 + h) as u64, 0.5);
    let a = interlaced(&img);
    let b = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
    assert_eq!(a[28], 1, "IHDR interlace flag");
    assert_eq!(b[28], 0);
    let da = decode(&a).unwrap();
    let db = decode(&b).unwrap();
    assert_eq!(da, db, "{w}x{h} {layout:?} {sample:?}: interlaced != progressive");
    assert_eq!(da.data(), img.data());
    // Oracle: the image crate agrees.
    let o = image::load_from_memory_with_format(&a, image::ImageFormat::Png).unwrap();
    let ours = da.convert(ChannelLayout::Rgba, SampleType::U16).to_u16_samples().unwrap();
    assert_eq!(o.into_rgba16().into_raw(), ours);
}

#[test]
fn adam7_small_sizes_all_layouts() {
    for (w, h) in [(1, 1), (2, 2), (3, 3), (5, 1), (1, 9), (7, 5), (8, 8), (9, 17), (33, 31)] {
        for l in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
            check_adam7(w, h, l, SampleType::U8);
        }
    }
}

#[test]
fn adam7_16bit() {
    for (w, h) in [(1, 1), (4, 3), (13, 11), (64, 40)] {
        for l in [ChannelLayout::Gray, ChannelLayout::Rgba] {
            check_adam7(w, h, l, SampleType::U16);
        }
    }
}

#[test]
fn adam7_with_metadata() {
    let mut img = test_image(ChannelLayout::Rgb, SampleType::U8).with_icc(Some(sample_icc(400)));
    img.meta.text.push(("Comment".into(), "interlaced".into()));
    let back = decode(&interlaced(&img)).unwrap();
    assert_eq!(back.icc, img.icc);
    assert_eq!(back.meta.text, img.meta.text);
}

#[test]
fn adam7_compressions() {
    let img = test_image(ChannelLayout::Rgba, SampleType::U8);
    for c in [PngCompression::None, PngCompression::Fast, PngCompression::Best] {
        let b = encode(&img, Format::Png, &EncodeOptions { png_interlaced: true, png_compression: c, ..Default::default() }).unwrap();
        assert_eq!(decode(&b).unwrap().data(), img.data());
    }
}

#[test]
fn png_from_borrowed_rgba8_matches_encoding_an_image() {
    let img = synth(37, 23, ChannelLayout::Rgba, SampleType::U8, 9042, 0.5);
    for interlace in [false, true] {
        let opts = EncodeOptions { png_interlaced: interlace, ..Default::default() };
        let borrowed = encode_png_rgba8(img.width(), img.height(), img.data(), &opts).unwrap();
        assert_eq!(borrowed, encode(&img, Format::Png, &opts).unwrap(), "interlaced {interlace}");
        assert_eq!(decode(&borrowed).unwrap().data(), img.data());
    }
}

#[test]
fn png_from_borrowed_rgba8_rejects_a_buffer_of_the_wrong_size() {
    let pixels = [0u8; 4 * 4 * 4];
    let opts = EncodeOptions::default();
    for (w, h, data) in [(4, 4, &pixels[..pixels.len() - 1]), (4, 3, &pixels[..]), (0, 4, &[][..])] {
        let result = encode_png_rgba8(w, h, data, &opts);
        assert!(matches!(result, Err(CodecError::InvalidImage(_))), "{w}x{h} with {} bytes: {result:?}", data.len());
    }
}

#[test]
fn png_palette_and_low_bit_depth_expand() {
    // Build a 2-bit gray PNG and a palette PNG with tRNS using the png crate.
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, 4, 1);
        e.set_color(png::ColorType::Grayscale);
        e.set_depth(png::BitDepth::Two);
        let mut w = e.write_header().unwrap();
        w.write_image_data(&[0b00_01_10_11]).unwrap();
    }
    let img = decode(&out).unwrap();
    assert_eq!((img.layout(), img.data()), (ChannelLayout::Gray, &[0u8, 85, 170, 255][..]));

    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, 2, 1);
        e.set_color(png::ColorType::Indexed);
        e.set_depth(png::BitDepth::Eight);
        e.set_palette(vec![255, 0, 0, 0, 0, 255]);
        e.set_trns(vec![128]);
        let mut w = e.write_header().unwrap();
        w.write_image_data(&[0, 1]).unwrap();
    }
    let img = decode(&out).unwrap();
    assert_eq!((img.layout(), img.data()), (ChannelLayout::Rgba, &[255u8, 0, 0, 128, 0, 0, 255, 255][..]));
}

// ---------------------------------------------------------------------------
// JPEG colour models
// ---------------------------------------------------------------------------

#[test]
fn jpeg_grayscale_decodes_as_gray() {
    let img = synth(40, 30, ChannelLayout::Gray, SampleType::U8, 2, 0.02);
    let back = decode(&encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap()).unwrap();
    assert_eq!(back.layout(), ChannelLayout::Gray);
    assert!(psnr(&img, &back) > 35.0);
}

#[test]
fn jpeg_roundtrip_preserves_inch_density_and_single_exif_prefix() {
    let mut img = synth(40, 30, ChannelLayout::Rgb, SampleType::U8, 2, 0.02);
    img.meta.dpi = Some((300.0, 240.0));
    img.meta.exif = Some(b"II*\0\x08\0\0\0\0\0\0\0\0\0\0".to_vec());
    let bytes = encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
    assert_eq!(bytes.windows(6).filter(|w| *w == b"Exif\0\0").count(), 1);
    let back = decode(&bytes).unwrap();
    assert_eq!(back.meta.dpi, Some((300.0, 240.0)));
    assert_eq!(back.meta.exif.as_deref(), img.meta.exif.as_deref());
}

#[test]
fn jpeg_cmyk_roundtrip_with_adobe_marker() {
    let img = synth(40, 30, ChannelLayout::Cmyk, SampleType::U8, 3, 0.02);
    let bytes = encode(&img, Format::Jpeg, &EncodeOptions { jpeg_quality: 95, ..Default::default() }).unwrap();
    assert!(bytes.windows(5).any(|w| w == b"Adobe"), "APP14 Adobe marker");
    let back = decode(&bytes).unwrap();
    assert_eq!(back.layout(), ChannelLayout::Cmyk);
    assert!(psnr(&img, &back) > 35.0, "{}", psnr(&img, &back));
}

#[test]
fn jpeg_cmyk_without_adobe_marker_keeps_samples_as_inks() {
    let img = Image::from_u8(16, 16, ChannelLayout::Cmyk, [10u8, 100, 200, 50].repeat(256)).unwrap();
    let marked = encode(&img, Format::Jpeg, &EncodeOptions { jpeg_quality: 95, ..Default::default() }).unwrap();
    let signature = b"\xFF\xEE\x00\x0EAdobe";
    let start = marked.windows(signature.len()).position(|part| part == signature).expect("Adobe APP14 marker");
    // APP14's two marker bytes and two-byte length precede its 12-byte payload.
    let mut unmarked = Vec::with_capacity(marked.len() - 16);
    unmarked.extend_from_slice(&marked[..start]);
    unmarked.extend_from_slice(&marked[start + 16..]);

    let adobe_pixels = decode(&marked).unwrap().data().to_vec();
    let unmarked_pixels = decode(&unmarked).unwrap().data().to_vec();
    for (adobe, raw) in adobe_pixels.as_chunks::<4>().0.iter().zip(unmarked_pixels.as_chunks::<4>().0) {
        for (&ink, &stored) in adobe.iter().zip(raw) {
            assert_eq!(stored, 255 - ink, "unmarked CMYK samples are direct inks");
        }
    }
}

#[test]
fn jpeg_ycck_decodes_to_cmyk() {
    let img = synth(40, 32, ChannelLayout::Cmyk, SampleType::U8, 4, 0.02);
    let mut bytes = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut bytes, 95);
    enc.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_4_4);
    enc.encode(img.data(), 40, 32, jpeg_encoder::ColorType::CmykAsYcck).unwrap();
    let back = decode(&bytes).unwrap();
    assert_eq!(back.layout(), ChannelLayout::Cmyk);
    assert!(psnr(&img, &back) > 32.0, "{}", psnr(&img, &back));
}

#[test]
fn jpeg_cmyk_flat_colour_exact_ish() {
    let img = Image::from_u8(16, 16, ChannelLayout::Cmyk, [10u8, 100, 200, 50].repeat(256)).unwrap();
    let back = decode(&encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap()).unwrap();
    for px in back.data().chunks(4) {
        for (a, b) in px.iter().zip([10u8, 100, 200, 50]) {
            assert!((*a as i32 - b as i32).abs() <= 2);
        }
    }
}

#[test]
fn jpeg_rgb_matches_oracle_decoder() {
    let img = test_image(ChannelLayout::Rgb, SampleType::U8);
    let bytes = encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
    let ours = decode(&bytes).unwrap();
    let theirs = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).unwrap().into_rgb8();
    let max = ours.data().iter().zip(theirs.as_raw()).map(|(a, b)| (*a as i32 - *b as i32).abs()).max().unwrap();
    assert!(max <= 1, "max diff {max}");
}

#[test]
fn jpeg_progressive_from_other_encoder_decodes() {
    let img = test_image(ChannelLayout::Rgb, SampleType::U8);
    let mut bytes = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut bytes, 90);
    enc.set_progressive(true);
    enc.encode(img.data(), img.width() as u16, img.height() as u16, jpeg_encoder::ColorType::Rgb).unwrap();
    let back = decode(&bytes).unwrap();
    assert!(psnr(&img, &back) > 25.0);
}

// ---------------------------------------------------------------------------
// Oracle: files we write are readable by the image crate
// ---------------------------------------------------------------------------

#[test]
fn png_output_matches_oracle_all_layouts() {
    for l in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        for s in [SampleType::U8, SampleType::U16] {
            let img = test_image(l, s);
            let b = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
            let o = image::load_from_memory(&b).unwrap().into_rgba16().into_raw();
            assert_eq!(o, img.to_rgba16(), "{l:?} {s:?}");
        }
    }
}

#[test]
fn tiff_output_matches_oracle() {
    for l in [ChannelLayout::Gray, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        for s in [SampleType::U8, SampleType::U16] {
            let img = test_image(l, s);
            let b = encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap();
            let o = image::load_from_memory(&b).unwrap().into_rgba16().into_raw();
            assert_eq!(o, img.to_rgba16(), "{l:?} {s:?}");
        }
    }
}

#[test]
fn tiff_f32_output_matches_oracle() {
    let img = test_image(ChannelLayout::Rgba, SampleType::F32);
    let b = encode(&img, Format::Tiff, &EncodeOptions { tiff_compression: TiffCompression::None, ..Default::default() }).unwrap();
    let o = image::load_from_memory(&b).unwrap().into_rgba32f().into_raw();
    assert_eq!(o, img.to_f32_samples().unwrap());
}

// ---------------------------------------------------------------------------
// PNG decoded in steps (PngDecodeRows)
// ---------------------------------------------------------------------------

/// Decodes `bytes` with [`PngDecodeRows`], going on for `per_call` rows per call (0: unlimited);
/// returns the image and how many calls it took.
fn decode_in_steps(bytes: &[u8], per_call: usize) -> Result<(Image, usize), CodecError> {
    let mut rows = PngDecodeRows::new(bytes, &Limits::default())?;
    let mut calls = 0;
    loop {
        calls += 1;
        let mut left = per_call;
        let mut more = || {
            left = left.wrapping_sub(1);
            per_call == 0 || left > 0
        };
        if let Some(img) = rows.step(&mut more)? {
            return Ok((img, calls));
        }
        assert!(calls < 100_000, "the decoder never finished");
    }
}

fn assert_steps_equal_decode(bytes: &[u8], what: &str) {
    let whole = decode(bytes);
    for per_call in [1, 7, 0] {
        // `decode` also takes the resolution from EXIF or XMP and turns the image upright;
        // `finish_png_decode` does that for a stepped decode, as the stepped import does.
        let stepped = decode_in_steps(bytes, per_call).and_then(|(img, calls)| Ok((finish_png_decode(img, &DecodeOptions::default())?, calls)));
        match (&whole, stepped) {
            (Ok(a), Ok((b, _))) => assert_eq!(a, &b, "{what}, {per_call} rows per call"),
            (Err(_), Err(_)) => {}
            (a, b) => panic!("{what}, {per_call} rows per call: decode {:?}, steps {:?}", a.as_ref().map(|_| ()), b.map(|_| ())),
        }
    }
}

#[test]
fn png_rows_equal_decode_in_every_layout_and_depth() {
    for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        for sample in [SampleType::U8, SampleType::U16] {
            let mut img = synth(41, 29, layout, sample, 77, 0.3).with_icc(Some(sample_icc(400)));
            img.meta.text.push(("Comment".into(), "stepped".into()));
            img.meta.dpi = Some((300.0, 300.0));
            for interlaced in [false, true] {
                let bytes = encode(&img, Format::Png, &EncodeOptions { png_interlaced: interlaced, ..Default::default() }).unwrap();
                assert_steps_equal_decode(&bytes, &format!("{layout:?} {sample:?} interlaced={interlaced}"));
            }
        }
    }
}

#[test]
fn png_rows_equal_decode_for_palette_and_transparency() {
    let pal = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [9, 9, 9]];
    let idx: Vec<u8> = (0..50 * 20).map(|i| (i % 4) as u8).collect();
    let bytes = encode_png_indexed(50, 20, &idx, &pal, Some(3), None, None).unwrap();
    assert_steps_equal_decode(&bytes, "palette with a transparent entry");
}

#[test]
fn png_rows_take_one_call_per_row_and_report_progress() {
    let bytes = encode(&synth(16, 40, ChannelLayout::Rgb, SampleType::U8, 5, 0.2), Format::Png, &EncodeOptions::default()).unwrap();
    let mut rows = PngDecodeRows::new(bytes.as_slice(), &Limits::default()).unwrap();
    assert_eq!(rows.size(), (16, 40));
    assert_eq!(rows.progress(), 0.0);
    let mut seen = Vec::new();
    let img = loop {
        if let Some(img) = rows.step(&mut || false).unwrap() {
            break img;
        }
        seen.push(rows.progress());
    };
    assert_eq!(seen.len(), 39, "one call per row but the last, which also finishes the file");
    assert!(seen.windows(2).all(|w| w[0] < w[1]));
    assert_eq!(rows.progress(), 1.0);
    assert_eq!(img, decode(&bytes).unwrap());
    // A decoder that has finished says so instead of decoding again.
    assert!(rows.step(&mut || true).is_err());
}

#[test]
fn png_rows_share_their_bytes_without_copying_them() {
    let bytes =
        std::sync::Arc::new(encode(&synth(20, 20, ChannelLayout::Rgba, SampleType::U8, 3, 0.2), Format::Png, &EncodeOptions::default()).unwrap());
    let mut rows = PngDecodeRows::new(SharedBytes(bytes.clone()), &Limits::default()).unwrap();
    let img = rows.step(&mut || true).unwrap().unwrap();
    assert_eq!(img, decode(&bytes).unwrap());
    drop(rows);
    assert_eq!(std::sync::Arc::strong_count(&bytes), 1, "the decoder let go of the bytes");
}

/// A CRC-valid PNG chunk.
fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut crc = flate2::Crc::new();
    crc.update(kind);
    crc.update(data);
    let mut out = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
    out
}

fn zlib(text: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(text).unwrap();
    encoder.finish().unwrap()
}

/// A `zTXt` chunk (`deflated`) or a compressed `iTXt` one (`!deflated`) with `text` as its value.
fn png_text_chunk(kind: &[u8; 4], keyword: &[u8], text: &[u8]) -> Vec<u8> {
    let mut data = keyword.to_vec();
    data.push(0);
    if kind == b"zTXt" {
        data.push(0);
    } else {
        data.extend_from_slice(&[1, 0, 0, 0]);
    }
    data.extend_from_slice(&zlib(text));
    png_chunk(kind, &data)
}

/// `png` with `chunks` between the image data and `IEND` (the text-after-IDAT case).
fn with_chunks_before_iend(png: &[u8], chunks: &[Vec<u8>]) -> Vec<u8> {
    let mut at = 8;
    loop {
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        if &png[at + 4..at + 8] == b"IEND" {
            let mut out = png[..at].to_vec();
            for c in chunks {
                out.extend_from_slice(c);
            }
            out.extend_from_slice(&png[at..]);
            return out;
        }
        at += len + 12;
    }
}

#[test]
fn png_rows_read_compressed_text_in_bounded_batches() {
    let img = synth(19, 23, ChannelLayout::Rgba, SampleType::U8, 9, 0.4);
    let base = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
    let text = "A".repeat(150_000);
    for kind in [b"zTXt", b"iTXt"] {
        let bytes = with_chunks_before_iend(&base, &[png_text_chunk(kind, b"Comment", text.as_bytes())]);
        let mut rows = PngDecodeRows::new(bytes.as_slice(), &Limits::default()).unwrap();
        let mut calls = 0;
        // An empty budget: every step reads one bounded batch and stops.
        let stepped = loop {
            calls += 1;
            if let Some(img) = rows.step(&mut || false).unwrap() {
                break img;
            }
            assert!(calls < 100_000, "the decoder never finished");
        };
        assert_eq!(stepped, decode(&bytes).unwrap(), "{kind:?}");
        assert_eq!(stepped.meta.text, vec![("Comment".into(), text.clone())], "{kind:?}: the whole value is kept");
        assert!(calls > 20, "{kind:?}: {calls} bounded calls, not one blocking pass");
        assert_steps_equal_decode(&bytes, &format!("compressed {kind:?} text"));
    }
}

/// An Adam7 image whose compressed text needs many metadata yields: the pixels are decoded
/// once, before the first yield, and later steps only read text.
#[test]
fn png_rows_finish_adam7_text_in_bounded_batches() {
    let img = synth(19, 23, ChannelLayout::Rgba, SampleType::U8, 9, 0.4);
    let base = encode(&img, Format::Png, &EncodeOptions { png_interlaced: true, ..Default::default() }).unwrap();
    let text = "A".repeat(150_000);
    for kind in [b"zTXt", b"iTXt"] {
        let bytes = with_chunks_before_iend(&base, &[png_text_chunk(kind, b"Comment", text.as_bytes())]);
        let mut rows = PngDecodeRows::new(bytes.as_slice(), &Limits::default()).unwrap();
        let mut calls = 0;
        let stepped = loop {
            calls += 1;
            if let Some(img) = rows.step(&mut || false).unwrap() {
                break img;
            }
            assert!(calls < 100_000, "the decoder never finished");
        };
        assert_eq!(stepped, decode(&bytes).unwrap(), "{kind:?}");
        assert_eq!(stepped.meta.text, vec![("Comment".into(), text.clone())], "{kind:?}: the whole value is kept");
        assert!(calls > 20, "{kind:?}: {calls} bounded calls after the one Adam7 pass");
    }
}

/// An uncompressed `iTXt` chunk (`value` is its text, so its CRC covers the whole value).
fn png_itxt_uncompressed(keyword: &[u8], value: &[u8]) -> Vec<u8> {
    let mut data = keyword.to_vec();
    data.push(0);
    data.extend_from_slice(&[0, 0, 0, 0]);
    data.extend_from_slice(value);
    png_chunk(b"iTXt", &data)
}

/// The CRC of a large text chunk takes bounded batches too, before any of its value is read.
#[test]
fn png_rows_read_a_large_text_crc_in_bounded_batches() {
    let img = synth(5, 5, ChannelLayout::Rgb, SampleType::U8, 3, 0.2);
    let base = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
    let text: String = (0..300_000u32).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
    let bytes = with_chunks_before_iend(&base, &[png_itxt_uncompressed(b"Comment", text.as_bytes())]);
    let mut rows = PngDecodeRows::new(bytes.as_slice(), &Limits::default()).unwrap();
    let mut calls = 0;
    let stepped = loop {
        calls += 1;
        if let Some(img) = rows.step(&mut || false).unwrap() {
            break img;
        }
        assert!(calls < 100_000, "the decoder never finished");
    };
    assert_eq!(stepped.meta.text, vec![("Comment".into(), text)]);
    assert!(calls > 60, "{calls} calls: the CRC and the value each take bounded batches");
}

/// A text chunk whose CRC does not match is still dropped after its bounded CRC batches.
#[test]
fn png_rows_drop_a_corrupt_text_crc() {
    let img = synth(5, 5, ChannelLayout::Rgb, SampleType::U8, 3, 0.2);
    let base = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
    let mut chunk = png_itxt_uncompressed(b"Comment", b"not accepted");
    let at = chunk.len() - 5;
    chunk[at] ^= 0xFF;
    let bytes = with_chunks_before_iend(&base, &[chunk]);
    let stepped = decode_in_steps(&bytes, 0).unwrap().0;
    assert!(stepped.meta.text.is_empty(), "a broken CRC drops the text");
    assert_eq!(stepped, decode(&bytes).unwrap(), "the one-pass decode drops it too");
}

/// An uncompressed iTXt value that is not UTF-8 is dropped without charging its bytes against
/// the budget, even when it is larger than all of it: the previous reader checked UTF-8 first.
#[test]
fn png_rows_drop_invalid_uncompressed_text_before_charging_the_budget() {
    let img = synth(4, 4, ChannelLayout::Rgb, SampleType::U8, 3, 0.2);
    let base = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
    let mut value = vec![b'A'; 12_000];
    value[11_999] = 0xFF;
    let bytes = with_chunks_before_iend(&base, &[png_itxt_uncompressed(b"Comment", &value)]);
    let opts = DecodeOptions { limits: Limits { max_alloc: 200, ..Default::default() }, ..Default::default() };
    let mut rows = PngDecodeRows::new(bytes.as_slice(), &opts.limits).unwrap();
    let stepped = rows.step(&mut || true).unwrap().unwrap();
    let stepped = finish_png_decode(stepped, &opts).unwrap();
    assert!(stepped.meta.text.is_empty(), "invalid UTF-8 is skipped, not a budget failure");
    let whole = decode_with(&bytes, &opts).unwrap();
    assert!(whole.meta.text.is_empty(), "the one-pass decode skips it too");
}

#[test]
fn png_rows_fail_like_decode_on_damaged_files() {
    let img = synth(64, 64, ChannelLayout::Rgb, SampleType::U8, 11, 1.0);
    for interlaced in [false, true] {
        let bytes = encode(&img, Format::Png, &EncodeOptions { png_interlaced: interlaced, ..Default::default() }).unwrap();
        // Cuts inside the signature, the IHDR chunk or the next chunk header leave no complete image.
        for cut in [8, 20, 40] {
            let truncated = bytes.get(..cut).unwrap();
            assert!(decode(truncated).is_err(), "cut at {cut} interlaced={interlaced}");
            assert!(decode_in_steps(truncated, 1).is_err(), "cut at {cut} interlaced={interlaced}");
        }
        for cut in [bytes.len() / 3, bytes.len() / 2, bytes.len() - 20] {
            assert_steps_equal_decode(bytes.get(..cut).unwrap(), &format!("cut at {cut} interlaced={interlaced}"));
        }
        // A flipped byte in the middle of the compressed data.
        let mut bad = bytes.clone();
        let mid = bad.len() / 2;
        bad[mid] ^= 0xFF;
        assert_steps_equal_decode(&bad, "a damaged byte");
    }
    assert!(decode_in_steps(&[], 1).is_err());
    assert!(decode_in_steps(b"not a png at all, just some text bytes", 1).is_err());
}

// ---------------------------------------------------------------------------
// PngSuite corpus (repo-root corpus/pngsuite/; feature `corpus`). The tests read the PNGs that
// tests/it/common/pngsuite.tsv lists, each checked against its SHA-256 first; a missing or changed
// listed file fails the test, and an unlisted file is not read.
// ---------------------------------------------------------------------------

#[cfg(feature = "corpus")]
#[test]
fn pngsuite_corpus_matches_oracle() {
    let mut checked = 0;
    for (name, bytes) in common::pngsuite::pngs() {
        let oracle = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png);
        let ours = decode(&bytes);
        match (ours, oracle) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a.to_rgba16(), b.into_rgba16().into_raw(), "{name}");
                checked += 1;
            }
            (Err(e), Ok(_)) => panic!("{name}: we failed but oracle decoded: {e}"),
            _ => {}
        }
    }
    eprintln!("pngsuite: {checked} files compared");
    assert!(checked > 100, "only {checked} PNGs compared");
}

/// The pinned list names the 178 files of the 2017 archive and `LICENSE`, a copy of the archive's
/// `PngSuite.LICENSE` with the same SHA-256: 179 names, 176 of them PNGs, every name once.
#[test]
fn pngsuite_manifest_lists_the_whole_suite() {
    use common::pngsuite::{MANIFEST, parse};
    let rows = parse(MANIFEST).unwrap();
    assert_eq!(rows.len(), 179);
    assert_eq!(rows.keys().filter(|n| n.ends_with(".png")).count(), 176);
    assert_eq!(rows["LICENSE"], rows["PngSuite.LICENSE"]);
    for name in ["PngSuite.README", "PngSuite.png", "basn0g01.png", "xs7n0g01.png", "z09n2c08.png"] {
        assert!(rows.contains_key(name), "{name}");
    }
    assert_eq!(MANIFEST.lines().filter(|l| l.starts_with('#')).count(), 7);
}

/// A row with a bad digest, a path, a hidden name or a repeated name is refused, and so is a list
/// with no rows. Extra columns, blank lines and comments are accepted.
#[test]
fn pngsuite_manifest_refuses_bad_rows() {
    use common::pngsuite::parse;
    let d = "a".repeat(64);
    assert_eq!(parse(&format!("# note\n\n{d}\tx.png\tCC0-1.0\thttps://example.com/x.png\n")).unwrap().len(), 1);
    for bad in [
        format!("{d}\tx.png\n{d}\tx.png\n"),
        format!("{d}\t../x.png\n"),
        format!("{d}\tdir/x.png\n"),
        format!("{d}\tdir\\x.png\n"),
        format!("{d}\t.x.png\n"),
        format!("{d}\t\n"),
        format!("{d}  x.png\n"),
        format!("{}\tx.png\n", "a".repeat(63)),
        format!("{}\tx.png\n", "A".repeat(64)),
        format!("{}\tx.png\n", "g".repeat(64)),
        "# only a comment\n".to_string(),
        String::new(),
    ] {
        assert!(parse(&bad).is_err(), "{bad:?}");
    }
}

/// The corpus reader returns every listed PNG with its bytes and nothing else: an unlisted PNG in
/// the folder is not read, and a listed PNG that is missing, changed or not a regular file fails.
#[test]
fn pngsuite_reader_takes_only_listed_files_that_match() {
    use common::pngsuite::{pinned_pngs, sha256_hex};
    let dir = std::env::temp_dir().join(format!("craft-image-codecs-pngsuite-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let a = encode(&synth(5, 4, ChannelLayout::Rgb, SampleType::U8, 1, 0.5), Format::Png, &EncodeOptions::default()).unwrap();
    let b = encode(&synth(3, 7, ChannelLayout::Gray, SampleType::U16, 2, 0.5), Format::Png, &EncodeOptions::default()).unwrap();
    std::fs::write(dir.join("a.png"), &a).unwrap();
    std::fs::write(dir.join("b.png"), &b).unwrap();
    std::fs::write(dir.join("unlisted.png"), &a).unwrap();
    let manifest = format!("# test\n{}\ta.png\n{}\tb.png\n{}\tnotes.txt\n", sha256_hex(&a), sha256_hex(&b), "0".repeat(64));

    let got = pinned_pngs(&dir, &manifest).unwrap();
    assert_eq!(got, vec![("a.png".to_string(), a.clone()), ("b.png".to_string(), b.clone())]);

    let mut changed = b.clone();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    std::fs::write(dir.join("b.png"), &changed).unwrap();
    let err = pinned_pngs(&dir, &manifest).unwrap_err();
    assert!(err.contains("b.png") && err.contains("does not match"), "{err}");
    std::fs::write(dir.join("b.png"), &b).unwrap();

    std::fs::remove_file(dir.join("a.png")).unwrap();
    assert!(pinned_pngs(&dir, &manifest).unwrap_err().contains("a.png"));
    std::fs::create_dir(dir.join("a.png")).unwrap();
    assert!(pinned_pngs(&dir, &manifest).unwrap_err().contains("regular file"));

    std::fs::remove_dir_all(&dir).unwrap();
}

/// The preflight charges the decoded layout's bytes per pixel: a 1000×1000 Gray8 PNG (1 MB) fits a
/// 1.5 MB budget, which the old worst-case RGBA16 preflight (8 MB) refused.
#[test]
fn png_preflight_uses_the_output_layout() {
    let img = synth(1000, 1000, ChannelLayout::Gray, SampleType::U8, 7, 0.5);
    let bytes = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
    let opts = DecodeOptions { limits: Limits { max_alloc: 1_500_000, ..Limits::none() }, ..Default::default() };
    let got = decode_with(&bytes, &opts).expect("a Gray8 image is charged 1 byte per pixel");
    assert_eq!(got.data(), img.data());
    let tight = DecodeOptions { limits: Limits { max_alloc: 900_000, ..Limits::none() }, ..Default::default() };
    assert!(decode_with(&bytes, &tight).is_err(), "the actual pixel budget still applies");
}

#[cfg(feature = "corpus")]
#[test]
fn png_rows_equal_decode_on_the_pngsuite_corpus() {
    let mut checked = 0;
    for (name, bytes) in common::pngsuite::pngs() {
        assert_steps_equal_decode(&bytes, &name);
        checked += 1;
    }
    assert!(checked > 100, "only {checked} PNGs compared");
}

/// Row-by-row decoding gives `decode`'s pixels and metadata for every PngSuite file it takes (all
/// but the interlaced ones), and fails where `decode` fails.
#[cfg(feature = "corpus")]
#[test]
fn pngsuite_corpus_rows_match_the_whole_decode() {
    let opts = DecodeOptions { keep_orientation: true, ..Default::default() };
    let (mut streamed, mut refused, mut skipped) = (0, 0, 0);
    for (name, bytes) in common::pngsuite::pngs() {
        // The importer only streams files `detect` takes for PNG.
        if detect(&bytes) != Some(Format::Png) {
            skipped += 1;
            continue;
        }
        let whole = decode_with(&bytes, &opts);
        let (mut data, mut header) = (Vec::new(), None);
        let rows = decode_png_rows::<CodecError>(
            &bytes,
            &opts,
            256,
            &mut |h| {
                header = Some(*h);
                Ok(true)
            },
            &mut |_, r| {
                data.extend_from_slice(r);
                Ok(())
            },
        );
        match (whole, rows) {
            (Ok(img), Ok(Some(x))) => {
                let h = header.unwrap();
                assert_eq!((img.width(), img.height(), img.layout(), img.sample_type()), (h.width, h.height, h.layout, h.sample), "{name}");
                assert!(img.data() == data, "{name}: pixels");
                assert_eq!(
                    (&img.icc, &img.meta.exif, &img.meta.text, &img.meta.xmp, &img.warnings),
                    (&x.icc, &x.meta.exif, &x.meta.text, &x.meta.xmp, &x.warnings),
                    "{name}"
                );
                assert_eq!(img.meta.dpi, x.meta.dpi, "{name}");
                streamed += 1;
            }
            (Ok(_), Ok(None)) => refused += 1,
            (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string(), "{name}"),
            (a, b) => panic!("{name}: whole {:?}, rows {:?}", a.map(|_| ()), b.map(|r| r.map(|_| ()))),
        }
    }
    eprintln!("pngsuite: {streamed} streamed, {refused} refused (interlaced), {skipped} skipped (not detected as PNG)");
    assert!(streamed > 100, "only {streamed} PNGs streamed");
}

/// Streaming a PNG's rows a group at a time gives the pixels and metadata `decode` gives, holds
/// only a group at a time, and stops between rows when asked.
#[test]
fn step_stream_gives_the_pixels_and_extras_of_decode() {
    for (layout, sample) in [(ChannelLayout::Gray, SampleType::U8), (ChannelLayout::Rgb, SampleType::U16), (ChannelLayout::Rgba, SampleType::U8)] {
        let (w, h) = (23u32, 19u32);
        let len = w as usize * h as usize * layout.channels() * sample.bytes();
        let mut img = Image::from_raw(w, h, layout, sample, (0..len).map(|i| (i.wrapping_mul(2_654_435_761) >> 11) as u8).collect()).unwrap();
        img.meta.dpi = Some((144.0, 144.0));
        img.meta.text.push(("Comment".into(), "streamed".into()));
        img.icc = Some(vec![7; 40]);
        let bytes = encode(&img, Format::Png, &EncodeOptions::default()).unwrap();
        let want = decode(&bytes).unwrap();
        for group in [0u32, 1, 4, 19, 64] {
            let mut dec = PngDecodeRows::new_streaming(bytes.as_slice(), &Limits::default()).unwrap();
            assert!(dec.streamable());
            assert_eq!(dec.header(), PngHeader { width: w, height: h, layout, sample });
            let (mut data, mut next, mut calls) = (Vec::new(), 0u32, 0);
            let extras = loop {
                calls += 1;
                let mut sink = |y0: u32, rows: &[u8]| -> Result<(), CodecError> {
                    assert_eq!(y0, next, "{layout:?} group {group}");
                    assert!(rows.len() <= group.max(1) as usize * dec_line(w, layout, sample), "a group at most");
                    next += (rows.len() / dec_line(w, layout, sample)) as u32;
                    data.extend_from_slice(rows);
                    Ok(())
                };
                if let Some(extras) = dec.step_stream(group, &mut || false, &mut sink).unwrap() {
                    break extras;
                }
                assert!(calls < 10_000);
            };
            assert!(calls > 1, "{layout:?}: stopped between rows");
            assert_eq!(data, want.data(), "{layout:?} group {group}: pixels");
            assert_eq!((extras.icc.as_deref(), extras.meta.dpi, &extras.meta.text), (want.icc.as_deref(), want.meta.dpi, &want.meta.text));
            assert_eq!(extras.orientation, 1);
            assert!(dec.step_stream::<CodecError>(group, &mut || true, &mut |_, _| Ok(())).is_err(), "spent");
        }
    }
    // An interlaced file doesn't stream.
    let img = Image::from_raw(9, 9, ChannelLayout::Gray, SampleType::U8, vec![1; 81]).unwrap();
    let bytes = encode(&img, Format::Png, &EncodeOptions { png_interlaced: true, ..Default::default() }).unwrap();
    assert!(!PngDecodeRows::new_streaming(bytes.as_slice(), &Limits::default()).unwrap().streamable());
}

/// Bytes per row of `w` pixels.
fn dec_line(w: u32, layout: ChannelLayout, sample: SampleType) -> usize {
    w as usize * layout.channels() * sample.bytes()
}

#[test]
fn bounded_png_stream_does_not_charge_unretained_full_frame() {
    let image = synth(256, 1024, ChannelLayout::Rgba, SampleType::U16, 77, 0.5);
    let bytes = encode(&image, Format::Png, &EncodeOptions::default()).unwrap();
    let limits = Limits { max_alloc: 512 * 1024, ..Limits::default() };
    assert!(image.data().len() > limits.max_alloc as usize);
    assert!(PngDecodeRows::new(bytes.as_slice(), &limits).is_err());
    assert!(PngDecodeRows::new_streaming(bytes.as_slice(), &limits).is_err());
    let mut decoder = PngDecodeRows::new_bounded_streaming(bytes.as_slice(), &limits).unwrap();
    assert!(decoder.streamable());
    // Opening a stream must not become a way to bypass whole-image allocation guards.
    assert!(decoder.reserve_output().is_err());
    assert!(decoder.step(&mut || true).is_err());
    let mut next = 0;
    let mut sink = |first: u32, rows: &[u8]| -> Result<(), CodecError> {
        assert_eq!(first as usize, next);
        assert!(rows.len() <= 3 * 256 * 8);
        let start = next * 256 * 8;
        assert_eq!(rows, &image.data()[start..start + rows.len()]);
        next += rows.len() / (256 * 8);
        Ok(())
    };
    while decoder.step_stream(3, &mut || false, &mut sink).unwrap().is_none() {}
    assert_eq!(next, 1024);
}

#[test]
fn bounded_png_stream_keeps_group_dimension_and_fallback_limits() {
    let image = synth(256, 1024, ChannelLayout::Rgba, SampleType::U16, 81, 0.5);
    let bytes = encode(&image, Format::Png, &EncodeOptions::default()).unwrap();
    let limits = Limits { max_alloc: 512 * 1024, ..Limits::default() };
    let mut decoder = PngDecodeRows::new_bounded_streaming(bytes.as_slice(), &limits).unwrap();
    assert!(decoder.step_stream::<CodecError>(1024, &mut || true, &mut |_, _| panic!("oversized group reached sink")).is_err());
    for strict in [
        Limits { max_width: 255, ..limits },
        Limits { max_height: 1023, ..limits },
        Limits { max_pixels: 256 * 1024 - 1, ..limits },
        Limits { max_alloc: 1024, ..limits },
    ] {
        assert!(PngDecodeRows::new_bounded_streaming(bytes.as_slice(), &strict).is_err());
    }
    let adam7 = interlaced(&image);
    assert!(PngDecodeRows::new_bounded_streaming(adam7.as_slice(), &limits).is_err());
}

#[test]
fn bounded_png_stream_rejects_shrinking_below_buffered_rows() {
    let image = synth(16, 8, ChannelLayout::Rgba, SampleType::U16, 83, 0.5);
    let bytes = encode(&image, Format::Png, &EncodeOptions::default()).unwrap();
    let mut decoder = PngDecodeRows::new_bounded_streaming(bytes.as_slice(), &Limits::default()).unwrap();
    for _ in 0..3 {
        assert!(decoder.step_stream::<CodecError>(4, &mut || false, &mut |_, _| panic!("group is incomplete")).unwrap().is_none());
    }
    assert!(decoder.step_stream::<CodecError>(1, &mut || true, &mut |_, _| panic!("changed group reached sink")).is_err());
    let mut output = Vec::new();
    decoder
        .step_stream::<CodecError>(4, &mut || true, &mut |_, rows| {
            output.extend_from_slice(rows);
            Ok(())
        })
        .unwrap();
    assert_eq!(output, image.data());
}

#[test]
fn bounded_png_stream_preserves_late_exif_across_metadata_yields() {
    let image = synth(16, 8, ChannelLayout::Rgba, SampleType::U16, 89, 0.5);
    let bytes = encode(&image, Format::Png, &EncodeOptions::default()).unwrap();
    let mut exif = sample_exif();
    exif[18..22].copy_from_slice(&6u32.to_le_bytes());
    let mut chunks = vec![png_chunk(b"eXIf", &exif)];
    for i in 0..150 {
        chunks.push(png_chunk(b"tEXt", format!("key{i}\0value").as_bytes()));
    }
    let bytes = with_chunks_before_iend(&bytes, &chunks);
    let mut decoder = PngDecodeRows::new_bounded_streaming(bytes.as_slice(), &Limits::default()).unwrap();
    assert!(decoder.streamable());
    let mut output = Vec::new();
    let extras = loop {
        if let Some(extras) = decoder
            .step_stream::<CodecError>(2, &mut || false, &mut |_, rows| {
                output.extend_from_slice(rows);
                Ok(())
            })
            .unwrap()
        {
            break extras;
        }
    };
    assert_eq!(output, image.data());
    assert_eq!(extras.orientation, 6);
    assert_eq!(extras.meta.text.len(), 150);
}

/// zune-jpeg 0.5 asserts on some subsampled CMYK frames whose first component isn't the largest
/// (jpeg-encoder writes C, M and Y at 1×1 and K larger); craft-jpeg reads those instead, and they
/// decode to the inks they were written with. The others of these files read as before.
#[test]
fn cmyk_frames_zune_jpeg_panics_on_decode() {
    use jpeg_encoder::{ColorType, Encoder, SamplingFactor as S};
    let mut seen = 0;
    for (s, progressive) in
        [(S::F_2_2, true), (S::R_4_2_0, true), (S::F_4_1, false), (S::F_4_2, false), (S::F_2_4, false), (S::R_4_1_1, false), (S::R_4_1_0, false)]
    {
        for (w, h) in [(1u16, 1u16), (9, 1), (1, 9), (17, 13), (40, 40)] {
            let inks: Vec<u8> = (0..usize::from(w) * usize::from(h)).flat_map(|_| [30u8, 90, 150, 210]).collect();
            let mut out = Vec::new();
            let mut e = Encoder::new(&mut out, 100);
            e.set_sampling_factor(s);
            e.set_progressive(progressive);
            e.encode(&inks, w, h, ColorType::Cmyk).unwrap();
            let zune = std::panic::catch_unwind(|| {
                let mut d = zune_jpeg::JpegDecoder::new(zune_core::bytestream::ZCursor::new(&out[..]));
                d.decode().is_ok()
            });
            if zune.is_ok() {
                // Read or refused by zune-jpeg as before; no panic either way.
                let _ = decode(&out);
                continue;
            }
            seen += 1;
            let img = decode(&out).unwrap_or_else(|e| panic!("{s:?} {w}x{h}: {e}"));
            assert_eq!((img.dimensions(), img.layout()), ((u32::from(w), u32::from(h)), ChannelLayout::Cmyk));
            assert!(img.data().iter().zip(&inks).all(|(a, b)| a.abs_diff(*b) <= 3), "{s:?} progressive {progressive} {w}x{h}");
        }
    }
    assert!(seen >= 10, "zune-jpeg panicked on {seen} of them");
}

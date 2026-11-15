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
// PngSuite corpus (repo-root corpus/pngsuite/*.png; feature `corpus`, fetched by
// `cargo xtask corpus --all`; missing = failure)
// ---------------------------------------------------------------------------

#[cfg(feature = "corpus")]
#[test]
fn pngsuite_corpus_matches_oracle() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/pngsuite");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        panic!("{} is missing: run `cargo xtask corpus --all`", dir.display());
    };
    let mut checked = 0;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
        let oracle = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png);
        let ours = decode(&bytes);
        match (ours, oracle) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a.to_rgba16(), b.into_rgba16().into_raw(), "{}", p.display());
                checked += 1;
            }
            (Err(e), Ok(_)) => panic!("{}: we failed but oracle decoded: {e}", p.display()),
            _ => {}
        }
    }
    eprintln!("pngsuite: {checked} files compared");
    assert!(checked > 100, "{}: only {checked} PNGs compared: run `cargo xtask corpus --all`", dir.display());
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
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/pngsuite");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        panic!("{} is missing: run `cargo xtask corpus --all`", dir.display());
    };
    let mut checked = 0;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
        assert_steps_equal_decode(&bytes, &p.display().to_string());
        checked += 1;
    }
    assert!(checked > 100, "{}: only {checked} PNGs compared: run `cargo xtask corpus --all`", dir.display());
}

/// Row-by-row decoding gives `decode`'s pixels and metadata for every PngSuite file it takes (all
/// but the interlaced ones), and fails where `decode` fails.
#[cfg(feature = "corpus")]
#[test]
fn pngsuite_corpus_rows_match_the_whole_decode() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/pngsuite");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        panic!("{} is missing: run `cargo xtask corpus --all`", dir.display());
    };
    let opts = DecodeOptions { keep_orientation: true, ..Default::default() };
    let (mut streamed, mut refused, mut skipped) = (0, 0, 0);
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("png") {
            continue;
        }
        let bytes = std::fs::read(&p).unwrap();
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
                assert_eq!((img.width(), img.height(), img.layout(), img.sample_type()), (h.width, h.height, h.layout, h.sample), "{}", p.display());
                assert!(img.data() == data, "{}: pixels", p.display());
                assert_eq!(
                    (&img.icc, &img.meta.exif, &img.meta.text, &img.meta.xmp, &img.warnings),
                    (&x.icc, &x.meta.exif, &x.meta.text, &x.meta.xmp, &x.warnings),
                    "{}",
                    p.display()
                );
                assert_eq!(img.meta.dpi, x.meta.dpi, "{}", p.display());
                streamed += 1;
            }
            (Ok(_), Ok(None)) => refused += 1,
            (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string(), "{}", p.display()),
            (a, b) => panic!("{}: whole {:?}, rows {:?}", p.display(), a.map(|_| ()), b.map(|r| r.map(|_| ()))),
        }
    }
    eprintln!("pngsuite: {streamed} streamed, {refused} refused (interlaced), {skipped} skipped (not detected as PNG)");
    assert!(streamed > 100, "only {streamed} PNGs streamed: run `cargo xtask corpus --all`");
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

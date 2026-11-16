//! `jpeg_inks`: four-component JPEGs read as Adobe-inverted inks, as `zune-jpeg`-based readers
//! kept them, beside `decode`'s own reading.

use craft_image_codecs::jpeg_inks::{decode_jpeg_inks, jpeg_components, jpeg_dimensions, jpeg_icc_profile};
use craft_image_codecs::*;

/// A `w` × `h` four-component JPEG of `samples` at quality 100, written by `jpeg-encoder` as
/// `ty`, with ICC profile `icc` when given.
fn four(w: u16, h: u16, samples: &[u8], ty: jpeg_encoder::ColorType, icc: Option<&[u8]>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, 100);
    enc.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_4_4);
    if let Some(icc) = icc {
        enc.add_icc_profile(icc).unwrap();
    }
    enc.encode(samples, w, h, ty).unwrap();
    out
}

/// Flat 8×8 blocks of ink, so the coding keeps each block's value.
fn blocks(w: usize, h: usize) -> Vec<u8> {
    (0..w * h)
        .flat_map(|i| {
            let b = ((i % w) / 8 + (i / w) / 8 * 7) as u8;
            [b.wrapping_mul(37), b.wrapping_mul(91), 255 - b.wrapping_mul(13), b.wrapping_mul(53)]
        })
        .collect()
}

/// The segments of `bytes` up to the scan without Adobe's APP14 marker.
fn without_adobe(bytes: &[u8]) -> Vec<u8> {
    let mut out = bytes[..2].to_vec();
    let mut at = 2;
    loop {
        let marker = bytes[at + 1];
        if marker == 0xDA {
            out.extend_from_slice(&bytes[at..]);
            return out;
        }
        let len = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        if !(marker == 0xEE && &bytes[at + 4..at + 9] == b"Adobe") {
            out.extend_from_slice(&bytes[at..at + 2 + len]);
        }
        at += 2 + len;
    }
}

/// Adobe CMYK reads to the inks it was written with, as `decode` reads it; without the marker
/// the samples are still taken as inverted (where `decode` keeps them as they are), and YCCK
/// turns into the same inks closely, truncating where `decode` rounds.
#[test]
fn four_component_jpegs_read_as_inverted_inks() {
    let (w, h) = (32u16, 16u16);
    let inks = blocks(usize::from(w), usize::from(h));
    let cmyk = four(w, h, &inks, jpeg_encoder::ColorType::Cmyk, None);
    let read = decode_jpeg_inks(&cmyk, &Limits::default()).unwrap();
    assert_eq!((read.dimensions(), read.layout()), ((32, 16), ChannelLayout::Cmyk));
    assert!(read.data().iter().zip(&inks).all(|(a, b)| a.abs_diff(*b) <= 2), "Adobe CMYK keeps its inks");
    assert_eq!(read.data(), decode(&cmyk).unwrap().data(), "as decode reads Adobe CMYK");
    // No marker: inverted all the same, where decode keeps the stored samples.
    let bare = without_adobe(&cmyk);
    assert_ne!(bare, cmyk);
    let unmarked = decode_jpeg_inks(&bare, &Limits::default()).unwrap();
    assert_eq!(unmarked.data(), read.data());
    let kept = decode(&bare).unwrap();
    assert!(kept.data().iter().zip(read.data()).all(|(k, r)| *k == 255 - *r), "decode keeps them as stored");
    // YCCK: libjpeg's conversion, truncated.
    let ycck = four(w, h, &inks, jpeg_encoder::ColorType::CmykAsYcck, None);
    let read = decode_jpeg_inks(&ycck, &Limits::default()).unwrap();
    assert!(read.data().iter().zip(&inks).all(|(a, b)| a.abs_diff(*b) <= 3), "YCCK keeps its inks closely");
    let rounded = decode(&ycck).unwrap();
    let differ = read.data().iter().zip(rounded.data()).filter(|(a, b)| a != b).count();
    assert!(read.data().iter().zip(rounded.data()).all(|(a, b)| a.abs_diff(*b) <= 1), "within a step of decode's rounding");
    assert!(differ > 0, "truncation differs from rounding somewhere");
}

/// The headers alone give the component count of any JPEG and its ICC profile, put together from
/// its numbered segments.
#[test]
fn headers_give_components_and_icc() {
    let icc: Vec<u8> = (0..200_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let cmyk = four(8, 8, &[9; 256], jpeg_encoder::ColorType::Cmyk, Some(&icc));
    assert_eq!((jpeg_components(&cmyk), jpeg_dimensions(&cmyk)), (Some(4), Some((8, 8))));
    assert_eq!(jpeg_icc_profile(&cmyk).as_deref(), Some(icc.as_slice()), "three APP2 segments");
    let gray = encode(&Image::from_u8(8, 8, ChannelLayout::Gray, vec![50; 64]).unwrap(), Format::Jpeg, &EncodeOptions::default()).unwrap();
    assert_eq!((jpeg_components(&gray), jpeg_icc_profile(&gray)), (Some(1), None));
    for not in [&b""[..], b"\xFF\xD8", b"\x89PNG\r\n\x1a\n"] {
        assert_eq!((jpeg_components(not), jpeg_icc_profile(not), jpeg_dimensions(not)), (None, None, None));
    }
}

/// A frame header past 16384 pixels a side, `zune-jpeg`'s default, isn't read.
#[test]
fn sides_past_zunes_default_are_not_read() {
    let mut gray = encode(&Image::from_u8(8, 8, ChannelLayout::Gray, vec![50; 64]).unwrap(), Format::Jpeg, &EncodeOptions::default()).unwrap();
    let sof = gray.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    gray[sof + 5..sof + 9].copy_from_slice(&[0x40, 0x00, 0x40, 0x00]);
    assert_eq!(jpeg_dimensions(&gray), Some((16384, 16384)));
    gray[sof + 7..sof + 9].copy_from_slice(&[0x40, 0x01]);
    assert_eq!((jpeg_dimensions(&gray), jpeg_components(&gray)), (None, None));
}

/// What isn't a four-component image `zune-jpeg` decodes is refused: other component counts, an
/// arithmetic-coded or 12-bit frame (`decode` hands those to craft-jpeg), a size past the limits,
/// and data that isn't a JPEG.
#[test]
fn other_jpegs_are_refused() {
    let gray = encode(&Image::from_u8(8, 8, ChannelLayout::Gray, vec![50; 64]).unwrap(), Format::Jpeg, &EncodeOptions::default()).unwrap();
    assert!(matches!(decode_jpeg_inks(&gray, &Limits::default()), Err(CodecError::Unsupported { .. })));
    let cmyk = four(16, 8, &[40; 16 * 8 * 4], jpeg_encoder::ColorType::Cmyk, None);
    let sof = cmyk.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    for (marker, precision) in [(0xC9u8, 8u8), (0xC1, 12)] {
        let mut other = cmyk.clone();
        other[sof + 1] = marker;
        other[sof + 4] = precision;
        assert!(decode_jpeg_inks(&other, &Limits::default()).is_err(), "SOF {marker:#x}, {precision} bits");
    }
    // A tiny file declaring a large frame is refused before zune-jpeg allocates its output.
    let mut huge = cmyk.clone();
    huge[sof + 5..sof + 9].copy_from_slice(&[0x20, 0x00, 0x20, 0x00]);
    assert!(matches!(decode_jpeg_inks(&huge, &Limits::default()), Err(CodecError::InvalidImage(m)) if m.contains("too short")));
    let tight = Limits { max_pixels: 100, ..Limits::default() };
    assert!(matches!(decode_jpeg_inks(&cmyk, &tight), Err(CodecError::LimitExceeded(_))));
    assert!(matches!(decode_jpeg_inks(b"not a jpeg", &Limits::default()), Err(CodecError::InvalidImage(_))));
    assert!(decode_jpeg_inks(&cmyk[..12], &Limits::default()).is_err(), "headers cut short");
}

/// The ink reader four-component JPEGs were read with before (vectorcraft's `jpeg_inks`, on
/// `zune-jpeg` as this crate uses it), kept as the reference the readers here match.
fn reference_inks(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    use zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};
    let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), DecoderOptions::default());
    d.decode_headers().ok()?;
    let space = d.input_colorspace()?;
    if d.info()?.components != 4 || !matches!(space, ColorSpace::CMYK | ColorSpace::YCCK) {
        return None;
    }
    let (w, h) = d.dimensions()?;
    d.set_options(DecoderOptions::default().jpeg_set_out_colorspace(space));
    let mut data = d.decode().ok()?;
    for p in data.as_chunks_mut::<4>().0 {
        if space == ColorSpace::YCCK {
            let [y, cb, cr] = [p[0], p[1], p[2]].map(f32::from);
            p[0] = (434.456 - y - 1.402 * cr).clamp(0.0, 255.0) as u8;
            p[1] = (119.541 - y + 0.344 * cb + 0.714 * cr).clamp(0.0, 255.0) as u8;
            p[2] = (481.816 - y - 1.772 * cb).clamp(0.0, 255.0) as u8;
        }
        for v in p.iter_mut() {
            *v = 255 - *v;
        }
    }
    Some((w, h, data))
}

/// Over CMYK and YCCK JPEGs of many sizes, samplings, qualities and pictures, with and without
/// Adobe's marker, baseline and progressive, the inks and component counts read the same as the
/// reference reader.
#[test]
fn matches_the_reference_reader() {
    use jpeg_encoder::{ColorType, Encoder, SamplingFactor};
    let (mut seen, mut refused) = (0, 0);
    for (i, (w, h)) in [(1u16, 1u16), (7, 3), (8, 8), (17, 9), (33, 31), (64, 5), (100, 77)].into_iter().enumerate() {
        let (wu, n) = (usize::from(w), usize::from(w) * usize::from(h));
        let noise: Vec<u8> = (0..n * 4).map(|k| ((k as u64).wrapping_mul(2_654_435_761).wrapping_add(i as u64 * 97) >> 7) as u8).collect();
        let ramp: Vec<u8> = (0..n * 4).map(|k| (k / 4 % wu * 255 / wu.max(2)) as u8).collect();
        for samples in [&noise, &ramp] {
            for ty in [ColorType::Cmyk, ColorType::CmykAsYcck] {
                for (q, sampling, progressive) in
                    [(100, SamplingFactor::R_4_4_4, false), (75, SamplingFactor::R_4_2_0, false), (40, SamplingFactor::R_4_2_2, true)]
                {
                    let mut out = Vec::new();
                    let mut enc = Encoder::new(&mut out, q);
                    enc.set_sampling_factor(sampling);
                    enc.set_progressive(progressive);
                    enc.encode(samples, w, h, ty).unwrap();
                    for file in [without_adobe(&out), out] {
                        let read = decode_jpeg_inks(&file, &Limits::default());
                        assert_eq!(jpeg_components(&file), Some(4));
                        let (dw, dh) = jpeg_dimensions(&file).unwrap();
                        assert_eq!((usize::from(w), usize::from(h)), (dw as usize, dh as usize));
                        let Some((rw, rh, inks)) = reference_inks(&file) else {
                            eprintln!("refused: {w}x{h} {ty:?} q{q} progressive {progressive}");
                            assert!(read.is_err(), "refused by both: {w}x{h} {ty:?} q{q}");
                            refused += 1;
                            continue;
                        };
                        let read = read.unwrap();
                        assert_eq!(read.dimensions(), (rw as u32, rh as u32));
                        assert_eq!(read.data(), inks.as_slice(), "{w}x{h} {ty:?} q{q}");
                        seen += 1;
                    }
                }
            }
        }
    }
    eprintln!("read {seen}, refused {refused}");
    assert_eq!(seen + refused, 7 * 2 * 2 * 3 * 2);
    assert!(seen > refused);
}

/// zune-jpeg 0.5 panics on some subsampled CMYK frames whose first component isn't the largest
/// (here `jpeg-encoder`'s 2×2 K with C, M and Y at 1×1, progressive): an error, not a panic.
#[test]
fn frames_zune_panics_on_are_errors() {
    for (w, h) in [(1u16, 1u16), (3, 3)] {
        let mut out = Vec::new();
        let mut enc = jpeg_encoder::Encoder::new(&mut out, 70);
        enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
        enc.set_progressive(true);
        enc.encode(&vec![9; usize::from(w * h) * 4], w, h, jpeg_encoder::ColorType::Cmyk).unwrap();
        assert!(std::panic::catch_unwind(|| reference_inks(&out)).is_err(), "the reference reader panics on {w}x{h}");
        assert!(matches!(decode_jpeg_inks(&out, &Limits::default()), Err(CodecError::Unsupported { .. })), "{w}x{h}");
        assert_eq!((jpeg_components(&out), jpeg_dimensions(&out)), (Some(4), Some((u32::from(w), u32::from(h)))));
    }
}

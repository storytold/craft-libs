//! Decoding files built in the tests: PNG with the `png` crate, TIFF and Radiance HDR byte by byte
//! from their published layouts, and OpenEXR with `craft-exr`'s writer.
#![cfg(any(feature = "png", feature = "tiff", feature = "hdr", feature = "exr"))]
#![allow(clippy::unwrap_used, clippy::panic)]

#[cfg(any(feature = "png", feature = "tiff"))]
use std::collections::HashSet;

use craft_texture::decode::{Alpha, DecodeError, Transfer};
use craft_texture::{Kind, Limits};

/// How many different values the floats of `d` hold.
#[cfg(any(feature = "png", feature = "tiff"))]
fn distinct(d: &craft_texture::decode::Decoded) -> usize {
    d.image.data().iter().map(|v| v.to_bits()).collect::<HashSet<_>>().len()
}

/// 512 16-bit codes from 0 to 65535, all different.
#[cfg(any(feature = "png", feature = "tiff"))]
fn ramp16() -> Vec<u16> {
    (0..512u32).map(|x| (x * 65535 / 511) as u16).collect()
}

#[cfg(feature = "png")]
mod png {
    use super::*;
    use craft_texture::ImageError;
    use png_encoder::{BitDepth, ColorType, Encoder};

    pub(super) fn encode(w: u32, h: u32, color: ColorType, depth: BitDepth, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(depth);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(data).unwrap();
        writer.finish().unwrap();
        out
    }

    #[test]
    fn sixteen_bit_gray_ramp_keeps_every_level() {
        let codes = ramp16();
        let bytes: Vec<u8> = codes.iter().flat_map(|v| v.to_be_bytes()).collect();
        let file = encode(512, 1, ColorType::Grayscale, BitDepth::Sixteen, &bytes);
        let d = craft_texture::decode::png(&file, &Limits::default()).unwrap();
        assert_eq!((d.image.width(), d.image.height(), d.image.kind()), (512, 1, Kind::Gray));
        assert!(distinct(&d) > 256, "{} levels", distinct(&d));
        for (v, &c) in d.image.data().iter().zip(&codes) {
            assert_eq!(*v, f32::from(c) / 65535.0);
        }
        assert_eq!((d.image.data()[0], d.image.data()[511]), (0.0, 1.0));
        assert_eq!((d.transfer, d.alpha), (Transfer::Encoded, Alpha::Straight));
    }

    #[test]
    fn sixteen_bit_rgba_keeps_every_level_in_every_channel() {
        let codes = ramp16();
        let bytes: Vec<u8> = codes.iter().flat_map(|&v| [v, 65535 - v, v, v].into_iter().flat_map(u16::to_be_bytes)).collect();
        let file = encode(512, 1, ColorType::Rgba, BitDepth::Sixteen, &bytes);
        let d = craft_texture::decode::png(&file, &Limits::default()).unwrap();
        assert_eq!(d.image.kind(), Kind::Color);
        for c in 0..4 {
            let levels: HashSet<u32> = d.image.data().as_chunks::<4>().0.iter().map(|p| p[c].to_bits()).collect();
            assert_eq!(levels.len(), 512, "channel {c}");
        }
        assert_eq!(d.image.get(1, 0, false), [128.0 / 65535.0, 65407.0 / 65535.0, 128.0 / 65535.0, 128.0 / 65535.0]);
    }

    #[test]
    fn eight_bit_layouts_map_to_gray_or_rgba() {
        let ga = encode(2, 1, ColorType::GrayscaleAlpha, BitDepth::Eight, &[51, 0, 255, 102]);
        let d = craft_texture::decode::png(&ga, &Limits::default()).unwrap();
        assert_eq!(d.image.kind(), Kind::Color);
        assert_eq!(d.image.get(0, 0, false), [0.2, 0.2, 0.2, 0.0]);
        assert_eq!(d.image.get(1, 0, false), [1.0, 1.0, 1.0, 0.4]);
        let rgb = encode(1, 1, ColorType::Rgb, BitDepth::Eight, &[255, 0, 51]);
        let d = craft_texture::decode::png(&rgb, &Limits::default()).unwrap();
        assert_eq!(d.image.get(0, 0, false), [1.0, 0.0, 0.2, 1.0]);
    }

    #[test]
    fn the_limit_counts_the_rgba_plane_made_from_rgb() {
        // 4 × 4 RGB is 48 bytes in the file's own depth and 256 bytes as an RGBA float plane.
        let file = encode(4, 4, ColorType::Rgb, BitDepth::Eight, &[7; 48]);
        let mut limits = Limits::default();
        limits.max_bytes = 255;
        assert_eq!(craft_texture::decode::png(&file, &limits), Err(DecodeError::Image(ImageError::TooLarge(4, 4))));
        limits.max_bytes = 256;
        assert!(craft_texture::decode::png(&file, &limits).is_ok());
        limits.max_side = 3;
        assert!(craft_texture::decode::png(&file, &limits).is_err());
    }

    #[test]
    fn empty_wrong_and_cut_short_bytes_are_errors() {
        let l = Limits::default();
        assert_eq!(craft_texture::decode::png(&[], &l), Err(DecodeError::WrongFormat("PNG")));
        assert_eq!(craft_texture::decode::png(b"#?RADIANCE\n", &l), Err(DecodeError::WrongFormat("PNG")));
        let codes = ramp16();
        let bytes: Vec<u8> = codes.iter().flat_map(|v| v.to_be_bytes()).collect();
        let file = encode(512, 1, ColorType::Grayscale, BitDepth::Sixteen, &bytes);
        for cut in [9, 40, file.len() / 2, file.len() - 13] {
            assert!(craft_texture::decode::png(&file[..cut], &l).is_err(), "cut at {cut}");
        }
    }
}

#[cfg(feature = "tiff")]
mod tiff {
    use super::*;

    /// A little-endian, uncompressed, single-strip TIFF (TIFF 6.0, section 2 and the baseline
    /// fields of sections 3 to 8) of one row.
    fn build(w: u32, photometric: u16, bits: &[u16], sample_format: u16, extra: Option<u16>, pixels: &[u8]) -> Vec<u8> {
        build_strips(w, 1, photometric, bits, sample_format, extra, pixels)
    }

    /// [`build`] with `rows` rows, one strip a row. `pixels` holds the first rows; the strips of
    /// the rows it does not hold point past the end of the file.
    fn build_strips(w: u32, rows: u32, photometric: u16, bits: &[u16], sample_format: u16, extra: Option<u16>, pixels: &[u8]) -> Vec<u8> {
        let spp = bits.len() as u16;
        let row_bytes = w * bits.iter().map(|&b| u32::from(b)).sum::<u32>() / 8;
        let shorts = |v: &[u16]| v.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>();
        let longs = |v: &[u32]| v.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>();
        let offsets: Vec<u32> = (0..rows).map(|r| if (r + 1) * row_bytes <= pixels.len() as u32 { 8 + r * row_bytes } else { 0x7fff_0000 }).collect();
        let mut out = b"II*\0".to_vec();
        let ifd_at = (8 + pixels.len() as u32).next_multiple_of(2);
        out.extend(ifd_at.to_le_bytes());
        out.extend(pixels);
        out.resize(ifd_at as usize, 0);
        let mut entries: Vec<(u16, u16, u32, Vec<u8>)> = vec![
            (256, 4, 1, w.to_le_bytes().to_vec()),
            (257, 4, 1, rows.to_le_bytes().to_vec()),
            (258, 3, u32::from(spp), shorts(bits)),
            (259, 3, 1, shorts(&[1])),
            (262, 3, 1, shorts(&[photometric])),
            (273, 4, rows, longs(&offsets)),
            (277, 3, 1, shorts(&[spp])),
            (278, 4, 1, 1u32.to_le_bytes().to_vec()),
            (279, 4, rows, longs(&vec![row_bytes; rows as usize])),
        ];
        if let Some(e) = extra {
            entries.push((338, 3, 1, shorts(&[e])));
        }
        entries.push((339, 3, u32::from(spp), shorts(&vec![sample_format; usize::from(spp)])));
        let mut spill_at = ifd_at + 2 + entries.len() as u32 * 12 + 4;
        let mut spill = Vec::new();
        out.extend((entries.len() as u16).to_le_bytes());
        for (tag, kind, count, mut value) in entries {
            out.extend(tag.to_le_bytes());
            out.extend(kind.to_le_bytes());
            out.extend(count.to_le_bytes());
            if value.len() <= 4 {
                value.resize(4, 0);
                out.extend(value);
            } else {
                out.extend(spill_at.to_le_bytes());
                spill_at += value.len() as u32;
                spill.extend(value);
            }
        }
        out.extend(0u32.to_le_bytes());
        out.extend(spill);
        out
    }

    fn le16(v: &[u16]) -> Vec<u8> {
        v.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn sixteen_bit_gray_ramp_keeps_every_level() {
        let codes = ramp16();
        let file = build(512, 1, &[16], 1, None, &le16(&codes));
        let d = craft_texture::decode::tiff(&file, &Limits::default()).unwrap();
        assert_eq!(d.image.kind(), Kind::Gray);
        assert!(distinct(&d) > 256, "{} levels", distinct(&d));
        for (v, &c) in d.image.data().iter().zip(&codes) {
            assert_eq!(*v, f32::from(c) / 65535.0);
        }
        assert_eq!((d.transfer, d.alpha), (Transfer::Encoded, Alpha::Straight));
    }

    #[test]
    fn sixteen_bit_rgba_with_unassociated_alpha_keeps_every_level() {
        let codes = ramp16();
        let px: Vec<u16> = codes.iter().flat_map(|&v| [v, v, 65535 - v, v]).collect();
        let file = build(512, 2, &[16; 4], 1, Some(2), &le16(&px));
        let d = craft_texture::decode::tiff(&file, &Limits::default()).unwrap();
        assert_eq!(d.image.kind(), Kind::Color);
        let alpha: HashSet<u32> = d.image.data().as_chunks::<4>().0.iter().map(|p| p[3].to_bits()).collect();
        assert_eq!(alpha.len(), 512);
        assert_eq!(d.image.get(511, 0, false), [1.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn associated_alpha_becomes_straight() {
        // Color 0.25 premultiplied by alpha 0.5 is straight color 0.5.
        let file = build(1, 2, &[16; 4], 1, Some(1), &le16(&[16384, 16384, 16384, 32768]));
        let d = craft_texture::decode::tiff(&file, &Limits::default()).unwrap();
        let p = d.image.get(0, 0, false);
        assert!(p[..3].iter().all(|c| (c - 0.5).abs() < 1e-3), "{p:?}");
        assert_eq!(p[3], 32768.0 / 65535.0);
        assert_eq!(d.alpha, Alpha::Straight);
    }

    #[test]
    fn float_samples_keep_values_outside_zero_to_one() {
        let px: Vec<u8> = [3.5f32, -0.25].iter().flat_map(|v| v.to_le_bytes()).collect();
        let file = build(2, 1, &[32], 3, None, &px);
        let d = craft_texture::decode::tiff(&file, &Limits::default()).unwrap();
        assert_eq!(d.image.data(), &[3.5, -0.25]);
        assert_eq!(d.transfer, Transfer::Unknown);
    }

    #[test]
    fn cmyk_is_unsupported() {
        let file = build(1, 5, &[8; 4], 1, None, &[0, 64, 128, 255]);
        assert!(matches!(craft_texture::decode::tiff(&file, &Limits::default()), Err(DecodeError::Unsupported { .. })));
    }

    #[test]
    fn a_strip_missing_rows_is_an_error() {
        let codes = ramp16();
        // Two rows, one strip each; the second strip points past the end of the file.
        let file = build_strips(512, 2, 1, &[16], 1, None, &le16(&codes));
        assert!(matches!(craft_texture::decode::tiff(&file, &Limits::default()), Err(DecodeError::Malformed { .. })));
    }

    #[test]
    fn empty_and_wrong_bytes_are_errors() {
        let l = Limits::default();
        assert_eq!(craft_texture::decode::tiff(&[], &l), Err(DecodeError::WrongFormat("TIFF")));
        assert_eq!(craft_texture::decode::tiff(b"II*", &l), Err(DecodeError::WrongFormat("TIFF")));
        let file = build(512, 1, &[16], 1, None, &le16(&ramp16()));
        assert!(craft_texture::decode::tiff(&file[..12], &l).is_err());
    }
}

#[cfg(feature = "hdr")]
mod hdr {
    use super::*;
    use craft_texture::ImageError;

    /// A flat Radiance picture of `pixels` RGBE quadruples in one row (`-Y 1 +X n`).
    fn build(pixels: &[[u8; 4]]) -> Vec<u8> {
        let mut out = format!("#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X {}\n", pixels.len()).into_bytes();
        out.extend(pixels.iter().flatten());
        out
    }

    #[test]
    fn values_above_one_are_kept() {
        // Mantissa × 2^(exponent − 136): 128 × 2^−6 is 2, and 128 × 2^4 is 2048.
        let file = build(&[[128, 128, 128, 130], [128, 64, 32, 129], [128, 0, 0, 140]]);
        let d = craft_texture::decode::hdr(&file, &Limits::default()).unwrap();
        assert_eq!(d.image.kind(), Kind::Color);
        assert_eq!(d.image.data(), &[2.0, 2.0, 2.0, 1.0, 1.0, 0.5, 0.25, 1.0, 2048.0, 0.0, 0.0, 1.0]);
        assert_eq!((d.transfer, d.alpha), (Transfer::Linear, Alpha::Straight));
    }

    #[test]
    fn short_wrong_and_oversized_pictures_are_errors() {
        let l = Limits::default();
        assert_eq!(craft_texture::decode::hdr(&[], &l), Err(DecodeError::WrongFormat("Radiance HDR")));
        let file = build(&[[128, 128, 128, 130]; 4]);
        assert!(matches!(craft_texture::decode::hdr(&file[..file.len() - 2], &l), Err(DecodeError::Malformed { .. })));
        let mut narrow = Limits::default();
        narrow.max_side = 3;
        assert_eq!(craft_texture::decode::hdr(&file, &narrow), Err(DecodeError::Image(ImageError::BadSize(4, 1))));
    }

    #[test]
    fn a_side_over_the_limit_is_refused_from_the_header() {
        // No pixel data follows, so only the header can give the size error.
        let mut narrow = Limits::default();
        narrow.max_side = 50;
        let wide = b"#?RADIANCE\r\nFORMAT=32-bit_rle_rgbe\r\n\r\n-Y 2 +X 100\n";
        assert_eq!(craft_texture::decode::hdr(wide, &narrow), Err(DecodeError::Image(ImageError::BadSize(100, 2))));
        let tall = b"#?RADIANCE\n\n+X 3 -Y 60\n";
        assert_eq!(craft_texture::decode::hdr(tall, &narrow), Err(DecodeError::Image(ImageError::BadSize(3, 60))));
        // The decoder treats a line of carriage returns alone as the empty line that ends the header.
        let crs = b"#?RADIANCE\r\r\n\r\r\n-Y 2 +X 100\n";
        assert_eq!(craft_texture::decode::hdr(crs, &narrow), Err(DecodeError::Image(ImageError::BadSize(100, 2))));
        // A byte that is not UTF-8 after the four tokens does not hide the sizes.
        let trailing = b"#?RADIANCE\n\n-Y 2 +X 100 \xff\n";
        assert_eq!(craft_texture::decode::hdr(trailing, &narrow), Err(DecodeError::Image(ImageError::BadSize(100, 2))));
        // A resolution line longer than the 64 KiB header limit is not read for sizes; the
        // decoder refuses it as a damaged header.
        let mut long = b"#?RADIANCE\n\n-Y 2 +X 100 ".to_vec();
        long.extend(std::iter::repeat_n(0xff, 70_000));
        long.push(b'\n');
        assert!(matches!(craft_texture::decode::hdr(&long, &narrow), Err(DecodeError::Malformed { .. })));
    }
}

#[cfg(feature = "exr")]
mod exr {
    use super::*;
    use craft_exr::decode::{Channel, Compression, Samples, f16, write};
    use craft_texture::ImageError;

    fn half(v: &[f32]) -> Samples {
        Samples::F16(v.iter().map(|&x| f16::from_f32(x)).collect())
    }

    #[test]
    fn half_and_float_values_are_kept_as_stored() {
        let channels = [
            Channel { name: "R".into(), samples: half(&[4.0, -0.5, 1.0]) },
            Channel { name: "G".into(), samples: Samples::F32(vec![f32::NAN, 1.0e6, 0.0]) },
            Channel { name: "B".into(), samples: Samples::F32(vec![f32::INFINITY, 0.125, 0.0]) },
            Channel { name: "A".into(), samples: half(&[1.0, 0.5, 0.0]) },
        ];
        let file = write(3, 1, &channels, Compression::Zip16).unwrap();
        let d = craft_texture::decode::exr(&file, &Limits::default()).unwrap();
        assert_eq!((d.image.width(), d.image.height(), d.image.kind()), (3, 1, Kind::Color));
        let p = d.image.data();
        assert_eq!(p[0], 4.0);
        assert!(p[1].is_nan());
        assert_eq!((p[2], p[3]), (f32::INFINITY, 1.0));
        assert_eq!(&p[4..], &[-0.5, 1.0e6, 0.125, 0.5, 1.0, 0.0, 0.0, 0.0]);
        assert_eq!((d.transfer, d.alpha), (Transfer::Linear, Alpha::Premultiplied));
    }

    #[test]
    fn luminance_gives_a_gray_plane_and_a_lone_alpha_is_unsupported() {
        let y = write(2, 1, &[Channel { name: "Y".into(), samples: half(&[3.0, 0.25]) }], Compression::None).unwrap();
        let d = craft_texture::decode::exr(&y, &Limits::default()).unwrap();
        assert_eq!((d.image.kind(), d.image.data()), (Kind::Gray, &[3.0f32, 0.25][..]));
        // A file of one channel is gray whatever the channel's name, such as a height map in Z.
        let z = write(2, 1, &[Channel { name: "Z".into(), samples: Samples::F32(vec![-2.0, 7.5]) }], Compression::None).unwrap();
        let d = craft_texture::decode::exr(&z, &Limits::default()).unwrap();
        assert_eq!((d.image.kind(), d.image.data()), (Kind::Gray, &[-2.0f32, 7.5][..]));
        let za = [Channel { name: "Z".into(), samples: half(&[1.0]) }, Channel { name: "A".into(), samples: half(&[1.0]) }];
        let za = write(1, 1, &za, Compression::None).unwrap();
        assert!(matches!(craft_texture::decode::exr(&za, &Limits::default()), Err(DecodeError::Unsupported { .. })));
    }

    #[test]
    fn channels_outside_the_color_group_count_toward_max_bytes() {
        // 11 channels of 4 × 4 declare 704 bytes; the RGBA plane made from R, G and B is 256.
        let names = ["R", "G", "B", "N.X", "N.Y", "N.Z", "P.X", "P.Y", "P.Z", "Z", "id"];
        let channels: Vec<Channel> = names.iter().map(|n| Channel { name: (*n).into(), samples: Samples::F32(vec![1.5; 16]) }).collect();
        let file = write(4, 4, &channels, Compression::Zip16).unwrap();
        let mut limits = Limits::default();
        limits.max_bytes = 703;
        assert!(matches!(craft_texture::decode::exr(&file, &limits), Err(DecodeError::LimitExceeded { .. })));
        limits.max_bytes = 704;
        let d = craft_texture::decode::exr(&file, &limits).unwrap();
        assert_eq!(d.image.get(3, 3, false), [1.5, 1.5, 1.5, 1.0]);
    }

    #[test]
    fn the_plane_is_checked_before_decoding_and_damage_is_an_error() {
        let rgb: Vec<Channel> = ["R", "G", "B"].iter().map(|n| Channel { name: (*n).into(), samples: half(&[2.0; 16]) }).collect();
        let file = write(4, 4, &rgb, Compression::Zip16).unwrap();
        let mut limits = Limits::default();
        // 4 × 4 RGB as an RGBA float plane is 256 bytes.
        limits.max_bytes = 255;
        assert_eq!(craft_texture::decode::exr(&file, &limits), Err(DecodeError::Image(ImageError::TooLarge(4, 4))));
        limits.max_bytes = 256;
        assert!(craft_texture::decode::exr(&file, &limits).is_ok());
        let l = Limits::default();
        assert_eq!(craft_texture::decode::exr(&[], &l), Err(DecodeError::WrongFormat("OpenEXR")));
        assert!(craft_texture::decode::exr(&file[..file.len() - 8], &l).is_err());
    }

    /// Appends a header attribute: its name, its type, the payload's size and the payload.
    fn attr(out: &mut Vec<u8>, name: &str, ty: &str, payload: &[u8]) {
        for s in [name, ty] {
            out.extend_from_slice(s.as_bytes());
            out.push(0);
        }
        out.extend_from_slice(&i32::try_from(payload.len()).unwrap().to_le_bytes());
        out.extend_from_slice(payload);
    }

    /// A multi-part file of one pixel: a flat scan-line part with float B, G and R channels of
    /// 0.25, then a deep scan-line part with one float A sample of 0.5.
    fn flat_and_deep() -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0x0131_2f76u32.to_le_bytes());
        out.extend_from_slice(&(2u32 | 0x800 | 0x1000).to_le_bytes());
        for deep in [false, true] {
            let names: &[&str] = if deep { &["A"] } else { &["B", "G", "R"] };
            let mut chlist = Vec::new();
            for n in names {
                chlist.extend_from_slice(n.as_bytes());
                chlist.push(0);
                chlist.extend_from_slice(&2i32.to_le_bytes()); // float
                chlist.extend_from_slice(&[0; 4]); // pLinear and reserved
                chlist.extend_from_slice(&1i32.to_le_bytes());
                chlist.extend_from_slice(&1i32.to_le_bytes());
            }
            chlist.push(0);
            let window: Vec<u8> = [0i32; 4].iter().flat_map(|v| v.to_le_bytes()).collect();
            attr(&mut out, "channels", "chlist", &chlist);
            attr(&mut out, "chunkCount", "int", &1i32.to_le_bytes());
            attr(&mut out, "compression", "compression", &[0]);
            attr(&mut out, "dataWindow", "box2i", &window);
            attr(&mut out, "displayWindow", "box2i", &window);
            attr(&mut out, "lineOrder", "lineOrder", &[0]);
            if deep {
                attr(&mut out, "maxSamplesPerPixel", "int", &1i32.to_le_bytes());
            }
            attr(&mut out, "name", "string", if deep { b"deep" } else { b"flat" });
            attr(&mut out, "pixelAspectRatio", "float", &1f32.to_le_bytes());
            attr(&mut out, "screenWindowCenter", "v2f", &[0; 8]);
            attr(&mut out, "screenWindowWidth", "float", &1f32.to_le_bytes());
            attr(&mut out, "type", "string", if deep { b"deepscanline" } else { b"scanlineimage" });
            if deep {
                attr(&mut out, "version", "int", &1i32.to_le_bytes());
            }
            out.push(0); // end of this header
        }
        out.push(0); // end of the header list
        // Each chunk starts with its part number and its scan line.
        let mut flat = [0i32.to_le_bytes(), 0i32.to_le_bytes(), 12i32.to_le_bytes()].concat();
        (0..3).for_each(|_| flat.extend_from_slice(&0.25f32.to_le_bytes()));
        // The deep chunk holds the sizes of the count table and of the packed and unpacked
        // samples, then the table (the running sample count) and the one sample.
        let mut deep = [1i32.to_le_bytes(), 0i32.to_le_bytes()].concat();
        (0..3).for_each(|_| deep.extend_from_slice(&4u64.to_le_bytes()));
        deep.extend_from_slice(&1u32.to_le_bytes());
        deep.extend_from_slice(&0.5f32.to_le_bytes());
        let first = out.len() + 16;
        out.extend_from_slice(&(first as u64).to_le_bytes());
        out.extend_from_slice(&((first + flat.len()) as u64).to_le_bytes());
        out.extend_from_slice(&flat);
        out.extend_from_slice(&deep);
        out
    }

    #[test]
    fn a_deep_part_beside_a_flat_color_part_is_unsupported() {
        let file = flat_and_deep();
        let parts = craft_exr::decode::parts(&file, &craft_exr::decode::Limits::default()).unwrap();
        assert_eq!(parts.iter().map(|p| p.deep).collect::<Vec<_>>(), [false, true]);
        // The flat part alone decodes, so only the deep part can refuse the file.
        let flat = craft_exr::decode::decode_part(&file, 0, &|_| true, &craft_exr::decode::Limits::default()).unwrap();
        assert_eq!(flat.channel("R").unwrap().samples, Samples::F32(vec![0.25]));
        let refused = craft_texture::decode::exr(&file, &Limits::default());
        assert_eq!(refused, Err(DecodeError::Unsupported { format: "OpenEXR", reason: "deep data".into() }));
    }
}

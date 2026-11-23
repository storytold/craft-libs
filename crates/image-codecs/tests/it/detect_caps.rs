//! Magic-number detection, extensions, and the capability table
//! (including the symmetric read/write guarantee).

use crate::common;
use common::*;
use craft_image_codecs::*;

// ---------------------------------------------------------------------------
// Symmetric guarantee
// ---------------------------------------------------------------------------

#[test]
fn every_enabled_format_is_symmetric_or_whitelisted() {
    for f in Format::ALL {
        let c = caps(f);
        if !(c.read || c.write) {
            continue;
        }
        if c.read && c.write {
            continue;
        }
        let exc = ASYMMETRIC_EXCEPTIONS.iter().find(|(g, _)| *g == f);
        let (_, reason) = exc.unwrap_or_else(|| panic!("{f:?} is enabled but not symmetric (read={}, write={})", c.read, c.write));
        assert!(reason.len() > 20, "{f:?} exception needs a real reason");
    }
}

#[test]
fn exceptions_are_documented_and_unique() {
    for (i, (f, reason)) in ASYMMETRIC_EXCEPTIONS.iter().enumerate() {
        assert!(!reason.is_empty());
        assert!(ASYMMETRIC_EXCEPTIONS[i + 1..].iter().all(|(g, _)| g != f));
    }
}

#[test]
fn never_write_only_without_exception() {
    // Stronger form: no format may be writable-but-unreadable unless listed.
    for f in writable_formats() {
        if !caps(f).read {
            assert!(ASYMMETRIC_EXCEPTIONS.iter().any(|(g, _)| *g == f), "{f:?}");
        }
    }
}

#[test]
fn default_build_formats_all_symmetric() {
    let expected = [
        Format::Png,
        Format::Jpeg,
        Format::Tiff,
        Format::WebP,
        Format::Gif,
        Format::Bmp,
        Format::Tga,
        Format::Ico,
        Format::Pnm,
        Format::Qoi,
        Format::OpenExr,
        Format::Hdr,
        Format::Fits,
        Format::Dpx,
        Format::Cineon,
        Format::Iff,
        #[cfg(feature = "sunraster")]
        Format::SunRaster,
        #[cfg(feature = "sgi")]
        Format::Sgi,
        Format::JpegLs,
    ];
    for f in expected {
        assert!(caps(f).read && caps(f).write, "{f:?}");
    }
}

#[test]
fn avif_is_readable_only_with_the_decode_feature() {
    assert_eq!(caps(Format::Avif).read, cfg!(all(feature = "avif-decode", not(target_arch = "wasm32"))));
    assert_eq!(caps(Format::Avif).write, cfg!(feature = "avif"));
    let header = decode_as(Format::Avif, b"\0\0\0\x1cftypavif");
    if caps(Format::Avif).read {
        assert!(matches!(header, Err(CodecError::Malformed { .. })), "{header:?}");
    } else {
        assert!(matches!(header, Err(CodecError::Unsupported { .. })), "{header:?}");
    }
}

#[test]
fn heif_is_detected_always_and_readable_only_with_the_feature() {
    let header = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic";
    assert_eq!(detect(header), Some(Format::Heif));
    assert_eq!(from_extension("IMG_0001.HEIC"), Some(Format::Heif));
    assert_eq!(caps(Format::Heif).read, cfg!(feature = "heif"));
    assert!(!caps(Format::Heif).write);
    assert!(ASYMMETRIC_EXCEPTIONS.iter().any(|(f, _)| *f == Format::Heif));
    // Either way a bare header is an error, never a panic.
    assert!(decode_as(Format::Heif, header).is_err());
}

#[test]
fn detect_heif_brands() {
    assert_eq!(detect(b"\0\0\0\x18ftypheic\0\0\0\0mif1heic"), Some(Format::Heif));
    assert_eq!(detect(b"\0\0\0\x18ftypmif1\0\0\0\0mif1heic"), Some(Format::Heif), "generic major brand");
    assert_eq!(detect(b"\0\0\0\x18ftypheix\0\0\0\0mif1heix"), Some(Format::Heif), "10-bit");
    assert_eq!(detect(b"\0\0\0\x18ftypmsf1\0\0\0\0msf1hevc"), Some(Format::Heif), "image sequence");
    assert_eq!(detect(b"\0\0\0\x18ftypmif1\0\0\0\0mif1avif"), Some(Format::Avif), "AVIF wins over the generic brand");
    assert_eq!(detect(b"\0\0\0\x18ftypisom\0\0\0\0isommp41"), None, "mp4 is not HEIF");
    assert_eq!(detect(b"\0\0\0\x14ftypqt  \0\0\0\0qt  "), None, "QuickTime is not HEIF");
}

#[test]
fn ftyp_brands_respect_prefix_and_declared_bounds() {
    let mut prefix = *b"\0\0\0\x1cftypisomavifavismif1xx";
    for end in 0..12 {
        assert!(ftyp_brands(&prefix[..end]).next().is_none(), "prefix length {end}");
    }
    assert!(ftyp_brands(b"\0\0\0\x0cmdatheic").next().is_none());
    assert!(ftyp_brands(&prefix).eq([b"isom", b"avis", b"mif1"]));
    assert!(ftyp_brands(&prefix[..18]).eq([b"isom"]));

    prefix[..4].copy_from_slice(&20u32.to_be_bytes());
    assert!(ftyp_brands(&prefix).eq([b"isom", b"avis"]));
    prefix[..4].copy_from_slice(&8u32.to_be_bytes());
    assert!(ftyp_brands(&prefix).eq([b"isom"]));
}

#[test]
fn detect_ftyp_keeps_its_short_brand_scan() {
    let mut prefix = [0; 68];
    prefix[..12].copy_from_slice(b"\0\0\0\x44ftypheic");
    prefix[64..68].copy_from_slice(b"avif");
    assert!(ftyp_brands(&prefix).any(|brand| brand == b"avif"));
    assert_eq!(detect(&prefix), Some(Format::Heif));

    prefix[60..64].copy_from_slice(b"avif");
    assert_eq!(detect(&prefix), Some(Format::Avif));
}

#[test]
fn caps_are_internally_consistent() {
    for f in Format::ALL {
        let c = caps(f);
        assert!(!c.depths.is_empty() && !c.layouts.is_empty(), "{f:?}");
        assert_eq!(c.alpha, c.layouts.iter().any(|l| l.has_alpha()), "{f:?} alpha flag vs layouts");
        // DICOM's registered media type is `application/dicom`; every other format is an image.
        let mime_ok = f.mime_type().starts_with("image/") || (f == Format::Dicom && f.mime_type() == "application/dicom");
        assert!(!f.extensions().is_empty() && !f.name().is_empty() && mime_ok, "{f:?}");
    }
}

#[test]
fn caps_function_matches_method() {
    for f in Format::ALL {
        assert_eq!(caps(f), f.caps());
    }
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

#[test]
fn detect_encoded_output_of_every_format() {
    for f in writable_formats() {
        let img = test_image(ChannelLayout::Rgba, SampleType::U8);
        let bytes = encode(&img, f, &EncodeOptions::default()).unwrap();
        assert_eq!(detect(&bytes), Some(f));
    }
}

#[test]
fn detect_magic_png() {
    assert_eq!(detect(b"\x89PNG\r\n\x1a\n\0\0"), Some(Format::Png));
}
#[test]
fn detect_magic_jpeg() {
    assert_eq!(detect(&[0xFF, 0xD8, 0xFF, 0xE0]), Some(Format::Jpeg));
    assert_eq!(detect(&[0xFF, 0xD8]), None);
}
#[test]
fn detect_magic_tiff_both_endians_and_bigtiff() {
    assert_eq!(detect(b"II*\0\x08\0\0\0"), Some(Format::Tiff));
    assert_eq!(detect(b"MM\0*\0\0\0\x08"), Some(Format::Tiff));
    assert_eq!(detect(b"II+\0\x08\0\0\0"), Some(Format::Tiff));
    assert_eq!(detect(b"MM\0+\0\x08\0\0"), Some(Format::Tiff));
}
#[test]
fn detect_magic_webp() {
    assert_eq!(detect(b"RIFF\x10\0\0\0WEBPVP8L"), Some(Format::WebP));
    assert_eq!(detect(b"RIFF\x10\0\0\0WAVEfmt "), None);
}
#[test]
fn detect_magic_gif() {
    assert_eq!(detect(b"GIF87a\x01\0"), Some(Format::Gif));
    assert_eq!(detect(b"GIF89a\x01\0"), Some(Format::Gif));
    assert_eq!(detect(b"GIF90a\x01\0"), None);
}
#[test]
fn detect_magic_bmp() {
    let mut b = b"BM".to_vec();
    b.resize(54, 0);
    assert_eq!(detect(&b), Some(Format::Bmp));
    assert_eq!(detect(b"BM"), None);
}
#[test]
fn detect_magic_ico() {
    assert_eq!(detect(&[0, 0, 1, 0, 1, 0, 16, 16]), Some(Format::Ico));
    assert_eq!(detect(&[0, 0, 1, 0, 0, 0]), None, "zero images");
}
#[test]
fn detect_magic_pnm_family() {
    for m in [&b"P1\n"[..], b"P2 ", b"P3\t", b"P4\n", b"P5\r", b"P6\n", b"P7\n", b"PF\n", b"Pf\n"] {
        assert_eq!(detect(m), Some(Format::Pnm), "{:?}", std::str::from_utf8(m));
    }
    assert_eq!(detect(b"P8\n"), None);
    assert_eq!(detect(b"P6x"), None);
}
#[test]
fn detect_magic_qoi() {
    assert_eq!(detect(b"qoif\0\0\0\x01\0\0\0\x01\x03\0"), Some(Format::Qoi));
}
#[test]
fn detect_magic_exr() {
    assert_eq!(detect(&[0x76, 0x2F, 0x31, 0x01, 2, 0, 0, 0]), Some(Format::OpenExr));
}
#[test]
fn detect_magic_hdr() {
    assert_eq!(detect(b"#?RADIANCE\n"), Some(Format::Hdr));
    assert_eq!(detect(b"#?RGBE\n"), Some(Format::Hdr));
}
#[test]
fn detect_magic_cineon() {
    let mut b = vec![0u8; 208];
    b[0..4].copy_from_slice(&[0x80, 0x2A, 0x5F, 0xD7]);
    assert_eq!(detect(&b), Some(Format::Cineon));
    assert_eq!(from_extension("scan.CIN"), Some(Format::Cineon));
    assert!(caps(Format::Cineon).read && caps(Format::Cineon).write, "Cineon is read and written");
    assert!(!ASYMMETRIC_EXCEPTIONS.iter().any(|(f, _)| *f == Format::Cineon));
    // A bare magic is an error, never a panic.
    assert!(decode_as(Format::Cineon, &b).is_err());
}

#[test]
fn detect_magic_iff() {
    // FORM + a four-byte length + the bitmap type.
    assert_eq!(detect(b"FORM\0\0\0\x14ILBM"), Some(Format::Iff));
    assert_eq!(detect(b"FORM\0\0\0\x14PBM "), Some(Format::Iff));
    assert_eq!(from_extension("art.LBM"), Some(Format::Iff));
    assert_eq!(from_extension("art.ilbm"), Some(Format::Iff));
    assert!(caps(Format::Iff).read && caps(Format::Iff).write, "IFF is read and written");
    assert!(caps(Format::Iff).alpha);
    assert!(caps(Format::Iff).layouts.contains(&craft_image_codecs::ChannelLayout::Rgba));
    assert!(!ASYMMETRIC_EXCEPTIONS.iter().any(|(f, _)| *f == Format::Iff));
    assert_eq!(Format::Iff.max_dimensions(), Some((65535, 65535)));
    // A bare FORM without a bitmap type is not IFF.
    assert_eq!(detect(b"FORM\0\0\0\x14AIFF"), None);
    assert!(decode_as(Format::Iff, b"FORM\0\0\0\x14ILBM").is_err());
}
#[cfg(feature = "sgi")]
#[test]
fn detect_magic_sgi() {
    use craft_image_codecs::{ChannelLayout, EncodeOptions, Image, encode};
    let img = Image::from_u8(5, 3, ChannelLayout::Rgba, (0..60).collect()).unwrap();
    let bytes = encode(&img, Format::Sgi, &EncodeOptions::default()).unwrap();
    assert_eq!(detect(&bytes), Some(Format::Sgi));
    assert_eq!(from_extension("scan.RGB"), Some(Format::Sgi));
    assert_eq!(from_extension("scan.bw"), Some(Format::Sgi));
    assert_eq!(decode(&bytes).unwrap().data(), img.data());
    assert!(!ASYMMETRIC_EXCEPTIONS.iter().any(|(f, _)| *f == Format::Sgi));
    assert_eq!(Format::Sgi.max_dimensions(), Some((65535, 65535)));
    // The magic number alone, without a full header, is not SGI.
    assert_eq!(detect(&[0x01, 0xda, 0, 1, 0, 2]), None);
    assert!(decode_as(Format::Sgi, &[0x01, 0xda]).is_err());
}

#[test]
fn detect_magic_jpeg_ls() {
    use craft_image_codecs::{ChannelLayout, EncodeOptions, Image, encode};
    let img = Image::from_u8(5, 3, ChannelLayout::Gray, (0..15).collect()).unwrap();
    let bytes = encode(&img, Format::JpegLs, &EncodeOptions::default()).unwrap();
    assert_eq!(detect(&bytes), Some(Format::JpegLs));
    assert_eq!(from_extension("scan.JLS"), Some(Format::JpegLs));
    assert_eq!(decode(&bytes).unwrap().data(), img.data());
    assert!(!ASYMMETRIC_EXCEPTIONS.iter().any(|(f, _)| *f == Format::JpegLs));
    assert_eq!(Format::JpegLs.max_dimensions(), Some((65535, 65535)));
    // Plain JPEG still is JPEG: SOI, then a baseline frame header.
    assert_eq!(detect(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 11]), Some(Format::Jpeg));
    assert!(decode_as(Format::JpegLs, &[0xFF, 0xD8, 0xFF, 0xF7]).is_err());
}

/// `file` with `count` marker segments of `body` bytes each inserted right after its SOI.
fn with_segments(file: &[u8], marker: u8, count: usize, body: usize) -> Vec<u8> {
    let mut out = file[..2].to_vec();
    for i in 0..count {
        out.extend_from_slice(&[0xFF, marker]);
        out.extend_from_slice(&u16::try_from(body + 2).unwrap().to_be_bytes());
        out.extend((0..body).map(|j| (i + j) as u8));
    }
    out.extend_from_slice(&file[2..]);
    out
}

fn jpeg_ls_file() -> (Image, Vec<u8>) {
    let img = Image::from_u8(5, 3, ChannelLayout::Gray, (0..15).collect()).unwrap();
    let bytes = encode(&img, Format::JpegLs, &EncodeOptions::default()).unwrap();
    (img, bytes)
}

/// Grows a prefix of `bytes` as a [`PrefixDetector`] asks, starting from `first` bytes, and checks
/// each answer against [`detect_prefix`].
fn detect_by_prefix(bytes: &[u8], first: usize) -> Detection {
    let mut detector = PrefixDetector::new();
    let mut len = first.min(bytes.len());
    loop {
        let answer = detector.detect(&bytes[..len]);
        assert_eq!(answer, detect_prefix(&bytes[..len]), "after {len} bytes");
        match answer {
            Detection::NeedBytes(more) if len < bytes.len() => {
                assert!(more > len, "{more} bytes asked for after {len}");
                len = more.min(bytes.len());
            }
            answer => return answer,
        }
    }
}

#[test]
fn jpeg_ls_after_a_long_comment_needs_the_frame_header() {
    let (img, plain) = jpeg_ls_file();
    let bytes = with_segments(&plain, 0xFE, 1, 100);
    assert_eq!(detect(&bytes), Some(Format::JpegLs));
    assert_eq!(decode(&bytes).unwrap().data(), img.data());
    // The first 64 bytes end inside the comment: `detect` says JPEG, `detect_prefix` asks for
    // the bytes up to the next marker.
    assert_eq!(detect(&bytes[..64]), Some(Format::Jpeg));
    assert_eq!(detect_prefix(&bytes[..64]), Detection::NeedBytes(2 + 2 + 102 + 2));
    assert_eq!(detect_by_prefix(&bytes, 64), Detection::Format(Format::JpegLs));
    assert_eq!(detect_prefix(&bytes), Detection::Format(Format::JpegLs));
}

#[test]
fn jpeg_ls_after_many_segments_is_still_jpeg_ls() {
    // An ICC profile can span 255 APP2 segments.
    let (img, plain) = jpeg_ls_file();
    let bytes = with_segments(&plain, 0xE2, 255, 14);
    assert_eq!(detect(&bytes), Some(Format::JpegLs));
    assert_eq!(decode(&bytes).unwrap().data(), img.data());
    assert_eq!(detect_by_prefix(&bytes, 64), Detection::Format(Format::JpegLs));
    // A plain JPEG with as many segments before its baseline frame header stays JPEG.
    let jpeg = with_segments(&[0xFF, 0xD8, 0xFF, 0xC0, 0, 11, 8, 0, 1, 0, 1, 1, 1, 0x11, 0], 0xE2, 255, 14);
    assert_eq!(detect(&jpeg), Some(Format::Jpeg));
    assert_eq!(detect_by_prefix(&jpeg, 64), Detection::Format(Format::Jpeg));
}

#[test]
fn a_cut_off_jpeg_asks_for_more_bytes() {
    let (_, plain) = jpeg_ls_file();
    let bytes = with_segments(&plain, 0xFE, 1, 100);
    // Cut inside the comment, inside its length field, and right after the SOI.
    assert_eq!(detect_prefix(&bytes[..50]), Detection::NeedBytes(108));
    assert_eq!(detect_prefix(&bytes[..5]), Detection::NeedBytes(6));
    assert_eq!(detect_prefix(&bytes[..3]), Detection::NeedBytes(4));
    assert_eq!(detect_prefix(&[0xFF, 0xD8]), Detection::NeedBytes(4));
    assert_eq!(detect(&bytes[..50]), Some(Format::Jpeg));
    assert!(decode_as(Format::JpegLs, &bytes[..50]).is_err());
    // A byte that is not a marker where one should be ends the walk: no frame header follows.
    assert_eq!(detect_prefix(&[0xFF, 0xD8, 0xFF, 0xFE, 0, 2, 0x12, 0x34]), Detection::Format(Format::Jpeg));
    // Formats other than JPEG answer as `detect` does.
    assert_eq!(detect_prefix(b"\x89PNG\r\n\x1a\n\0\0"), Detection::Format(Format::Png));
    assert_eq!(detect_prefix(&[0u8; 64]), Detection::Unknown);
    assert_eq!(detect_prefix(&[]), Detection::Unknown);
}

#[test]
fn a_restart_interval_before_the_frame_header_is_jpeg_ls() {
    // A DRI segment may come before the frame header. An interval of 0 turns restarts off.
    let (img, plain) = jpeg_ls_file();
    let mut bytes = plain[..2].to_vec();
    bytes.extend_from_slice(&[0xFF, 0xDD, 0, 4, 0, 0]);
    bytes.extend_from_slice(&plain[2..]);
    assert_eq!(detect(&bytes), Some(Format::JpegLs));
    assert_eq!(detect_by_prefix(&bytes, 4), Detection::Format(Format::JpegLs));
    assert_eq!(decode(&bytes).unwrap().data(), img.data());
    assert_eq!(decode_as(Format::JpegLs, &bytes).unwrap().data(), img.data());
    // A file that sets a restart interval is JPEG-LS, and its restart markers are not read.
    bytes[7] = 1;
    assert_eq!(detect(&bytes), Some(Format::JpegLs));
    assert!(matches!(decode(&bytes), Err(CodecError::Unsupported { .. })));
    // A 3- or 4-byte interval of 0 also turns restarts off. A segment with fewer than 2 interval
    // bytes or more than 4 is malformed.
    for (size, ok) in [(0, false), (1, false), (2, true), (3, true), (4, true), (5, false)] {
        let mut bytes = plain[..2].to_vec();
        bytes.extend_from_slice(&[0xFF, 0xDD, 0, 2 + size as u8]);
        bytes.extend(std::iter::repeat_n(0, size));
        bytes.extend_from_slice(&plain[2..]);
        assert_eq!(detect(&bytes), Some(Format::JpegLs));
        assert_eq!(decode(&bytes).is_ok(), ok, "{size} interval bytes");
    }
}

#[test]
fn the_restart_interval_in_effect_at_the_scan_decides_decoding() {
    // The 5x3 file's frame header ends at byte 15, where its scan header starts.
    let (img, plain) = jpeg_ls_file();
    assert_eq!(plain[2..4], [0xFF, 0xF7]);
    assert_eq!(plain[15..17], [0xFF, 0xDA]);
    let with_dri = |intervals: &[u16]| {
        let mut bytes = plain[..15].to_vec();
        for interval in intervals {
            bytes.extend_from_slice(&[0xFF, 0xDD, 0, 4]);
            bytes.extend_from_slice(&interval.to_be_bytes());
        }
        bytes.extend_from_slice(&plain[15..]);
        bytes
    };
    // A later interval of 0 turns restarts off again.
    assert_eq!(decode(&with_dri(&[1, 0])).unwrap().data(), img.data());
    // An interval of at least the scan's 3 lines puts the whole scan in one interval, so the scan
    // has no restart marker.
    assert_eq!(decode(&with_dri(&[3])).unwrap().data(), img.data());
    assert_eq!(decode(&with_dri(&[u16::MAX])).unwrap().data(), img.data());
    // A shorter interval needs restart markers in the scan.
    assert!(matches!(decode(&with_dri(&[2])), Err(CodecError::Unsupported { .. })));
    assert!(matches!(decode(&with_dri(&[0, 2])), Err(CodecError::Unsupported { .. })));
}

#[test]
fn a_restart_marker_after_a_scan_of_one_interval_is_an_error() {
    // With an interval of 3 lines the 5x3 scan is one interval, which no restart marker follows.
    let (img, plain) = jpeg_ls_file();
    assert_eq!(plain[plain.len() - 2..], [0xFF, 0xD9]);
    let mut bytes = plain[..15].to_vec();
    bytes.extend_from_slice(&[0xFF, 0xDD, 0, 4, 0, 3]);
    bytes.extend_from_slice(&plain[15..]);
    assert_eq!(decode(&bytes).unwrap().data(), img.data());
    let eoi = bytes.len() - 2;
    let mut marked = bytes.clone();
    marked.splice(eoi..eoi, [0xFF, 0xD0]);
    assert!(decode(&marked).is_err());
    // Fill bytes before the restart marker do not hide it.
    bytes.splice(eoi..eoi, [0xFF, 0xFF, 0xFF, 0xD3]);
    assert!(decode(&bytes).is_err());
}

#[test]
fn a_stuffed_zero_where_a_marker_should_be_is_jpeg() {
    // `FF 00` is not a marker, so the walk ends there without asking for a length field.
    assert_eq!(detect_prefix(&[0xFF, 0xD8, 0xFF, 0x00]), Detection::Format(Format::Jpeg));
    assert_eq!(detect_prefix(&[0xFF, 0xD8, 0xFF, 0xFE, 0, 2, 0xFF, 0x00]), Detection::Format(Format::Jpeg));
}

#[test]
fn a_detector_that_found_a_jpeg_frame_keeps_answering_jpeg() {
    // SOI, a comment with two bytes of text, then a baseline frame header.
    let file = [0xFF, 0xD8, 0xFF, 0xFE, 0, 4, 1, 2, 0xFF, 0xC0, 0, 11];
    let mut detector = PrefixDetector::new();
    assert_eq!(detector.detect(&file[..10]), Detection::Format(Format::Jpeg));
    assert_eq!(detector.detect(&file), detect_prefix(&file));
    assert_eq!(detector.detect(&file), Detection::Format(Format::Jpeg));
}

#[test]
fn a_detector_keeps_its_place_between_calls() {
    // SOI, a comment with two bytes of text, then an SOF55 frame header.
    let file = [0xFF, 0xD8, 0xFF, 0xFE, 0, 4, 1, 2, 0xFF, 0xF7, 0, 11];
    let mut detector = PrefixDetector::new();
    assert_eq!(detector.detect(&file[..5]), Detection::NeedBytes(6));
    assert_eq!(detector.detect(&file[..9]), Detection::NeedBytes(10));
    // Each call extends the previous prefix, and the answer matches a walk from the SOI.
    assert_eq!(detector.detect(&file), detect_prefix(&file));
    assert_eq!(detect_prefix(&file), Detection::Format(Format::JpegLs));
}

#[cfg(feature = "sunraster")]
#[test]
fn detect_magic_sun_raster() {
    use craft_image_codecs::{ChannelLayout, EncodeOptions, Image, encode};
    let img = Image::from_u8(5, 3, ChannelLayout::Rgb, (0..45).collect()).unwrap();
    let bytes = encode(&img, Format::SunRaster, &EncodeOptions::default()).unwrap();
    assert_eq!(detect(&bytes), Some(Format::SunRaster));
    assert_eq!(from_extension("scan.RAS"), Some(Format::SunRaster));
    assert_eq!(decode(&bytes).unwrap().data(), img.data());
    assert!(!ASYMMETRIC_EXCEPTIONS.iter().any(|(f, _)| *f == Format::SunRaster));
    assert_eq!(detect(&[0x59, 0xa6, 0x6a, 0x95]), None);
    assert!(decode_as(Format::SunRaster, &[0x59, 0xa6, 0x6a, 0x95]).is_err());
}

#[test]
fn detect_magic_avif() {
    assert_eq!(detect(b"\0\0\0\x1cftypavif\0\0\0\0avifmif1miaf"), Some(Format::Avif));
    assert_eq!(detect(b"\0\0\0\x1cftypavis\0\0\0\0avismsf1miaf"), Some(Format::Avif));
    assert_eq!(detect(b"\0\0\0\x18ftypmif1\0\0\0\0mif1avif"), Some(Format::Avif), "compatible brand");
    assert_eq!(detect(b"\0\0\0\x18ftypisom\0\0\0\0isommp41"), None, "mp4 is not avif");
}
#[test]
fn detect_magic_dicom() {
    let mut b = vec![0u8; 128];
    b.extend_from_slice(b"DICM");
    assert_eq!(detect(&b), Some(Format::Dicom));
    assert_eq!(from_extension("scan.DCM"), Some(Format::Dicom));
    assert_eq!(from_extension("scan.dc3"), Some(Format::Dicom));
    assert!(caps(Format::Dicom).read && !caps(Format::Dicom).write);
    assert!(ASYMMETRIC_EXCEPTIONS.iter().any(|(f, _)| *f == Format::Dicom));
    // A bare magic is an error, never a panic.
    assert!(decode_as(Format::Dicom, &b).is_err());
}

#[test]
fn detect_tga_heuristic() {
    // Uncompressed true-colour 2x2, 24 bpp.
    let mut b = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 2, 0, 24, 0];
    b.extend_from_slice(&[0; 12]);
    assert_eq!(detect(&b), Some(Format::Tga));
    let mut bad = b.clone();
    bad[2] = 7; // invalid image type
    assert_eq!(detect(&bad), None);
    let mut bad = b.clone();
    bad[16] = 13; // invalid depth
    assert_eq!(detect(&bad), None);
}
#[test]
fn detect_negatives() {
    assert_eq!(detect(b""), None);
    assert_eq!(detect(b"x"), None);
    assert_eq!(detect(b"hello world, this is not an image"), None);
    assert_eq!(detect(b"%PDF-1.7\n"), None);
    assert_eq!(detect(b"PK\x03\x04"), None);
    assert_eq!(detect(&[0u8; 64]), None);
}
#[test]
fn detect_random_bytes_rarely_matches_tga() {
    let mut rng = Rng::new(5);
    let hits = (0..2000).filter(|_| detect(&rng.bytes(64)) == Some(Format::Tga)).count();
    assert!(hits < 10, "{hits} false TGA positives");
}
#[test]
fn decode_unknown_is_error() {
    assert!(matches!(decode(b"garbage garbage"), Err(CodecError::UnknownFormat)));
}

// ---------------------------------------------------------------------------
// Extensions
// ---------------------------------------------------------------------------

#[test]
fn extensions_map_back() {
    for f in Format::ALL {
        for e in f.extensions() {
            assert_eq!(from_extension(e), Some(f), "{e}");
            assert_eq!(from_extension(&e.to_uppercase()), Some(f));
            assert_eq!(from_extension(&format!(".{e}")), Some(f));
            assert_eq!(from_extension(&format!("dir/file.name.{e}")), Some(f));
        }
    }
}
#[test]
fn extension_specific_cases() {
    assert_eq!(from_extension("JPG"), Some(Format::Jpeg));
    assert_eq!(from_extension("tif"), Some(Format::Tiff));
    assert_eq!(from_extension("pfm"), Some(Format::Pnm));
    assert_eq!(from_extension("pam"), Some(Format::Pnm));
    assert_eq!(from_extension("FIT"), Some(Format::Fits));
    assert_eq!(from_extension("m31.fits"), Some(Format::Fits));
    assert_eq!(from_extension("exr"), Some(Format::OpenExr));
    assert_eq!(from_extension("psd"), None);
    assert_eq!(from_extension(""), None);
    assert_eq!(from_extension("png.bak"), None);
}
#[test]
fn extensions_unique_across_formats() {
    let mut all: Vec<&str> = Format::ALL.iter().flat_map(|f| f.extensions().iter().copied()).collect();
    let n = all.len();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), n);
}

#[cfg(not(feature = "sgi"))]
#[test]
fn sgi_requires_explicit_feature() {
    use craft_image_codecs::{ChannelLayout, EncodeOptions, Image, encode};
    assert!(!caps(Format::Sgi).read && !caps(Format::Sgi).write);
    assert!(decode_as(Format::Sgi, &[]).is_err());
    let image = Image::from_u8(1, 1, ChannelLayout::Gray, vec![0]).unwrap();
    assert!(encode(&image, Format::Sgi, &EncodeOptions::default()).is_err());
    let mut header = vec![0; 512];
    header[..12].copy_from_slice(&[1, 218, 0, 1, 0, 2, 0, 1, 0, 1, 0, 1]);
    assert_eq!(detect(&header), None);
}

#[cfg(not(feature = "sunraster"))]
#[test]
fn sun_raster_disabled_by_default() {
    use craft_image_codecs::{ChannelLayout, EncodeOptions, Image, encode};
    let bytes = [0x59, 0xa6, 0x6a, 0x95, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 8, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 42, 0];
    assert_ne!(detect(&bytes), Some(Format::SunRaster));
    assert!(!caps(Format::SunRaster).read && !caps(Format::SunRaster).write);
    assert!(decode_as(Format::SunRaster, &bytes).is_err());
    let image = Image::from_u8(1, 1, ChannelLayout::Gray, vec![42]).unwrap();
    assert!(encode(&image, Format::SunRaster, &EncodeOptions::default()).is_err());
}

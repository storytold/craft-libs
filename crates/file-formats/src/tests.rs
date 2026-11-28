use super::{Availability, Format};

#[test]
fn every_registered_family_has_consistent_metadata_and_extension_lookup() {
    for format in Format::ALL {
        let extensions = format.extensions();
        assert_eq!(format.preferred_extension(), extensions.first().copied());
        assert!(format.mime_type().is_some());
        let _capabilities = format.capabilities();
        #[cfg(feature = "raster-codecs")]
        let _codec = format.raster_codec();

        for extension in extensions {
            assert_eq!(super::from_extension(extension), Some(format), "{format:?}: {extension}");
            assert_eq!(super::from_extension(&extension.to_ascii_uppercase()), Some(format), "{format:?}: {extension}");
        }
    }
}

#[test]
fn known_formats_separate_build_disabled_and_unsupported_operations() {
    let jpeg = Format::Jpeg.capabilities();
    assert!(!jpeg.multiple_frames && !jpeg.file_sequence);
    #[cfg(feature = "raster-codecs")]
    assert_eq!(jpeg.read, Availability::Available);
    #[cfg(not(feature = "raster-codecs"))]
    assert_eq!(jpeg.read, Availability::BuildDisabled);

    // Whether this build reads JPEG XL is craft-image-codecs' `jxl` feature to decide, and this
    // crate cannot see that through `cfg!`: a workspace build turns it on without this crate
    // selecting anything, which is how a plain equality here failed every `--all-features` gate.
    // The codec's own answer is the one to check against, so a wrong read status is still caught.
    #[cfg(feature = "raster-codecs")]
    {
        let codec = Format::Jxl.raster_codec();
        assert!(codec.is_some(), "JXL is handled by craft-image-codecs and must map to one of its formats");
        let expected = if codec.is_some_and(|codec| codec.caps().read) { Availability::Available } else { Availability::Unsupported };
        assert_eq!(Format::Jxl.capabilities().read, expected);
    }
    #[cfg(not(feature = "raster-codecs"))]
    assert_eq!(Format::Jxl.capabilities().read, Availability::Unsupported);
    #[cfg(not(feature = "raster-codecs"))]
    assert_eq!(Format::Jxl.capabilities().write, Availability::BuildDisabled);
    assert!(!Format::Psd.capabilities().file_sequence);
    assert!(Format::Raw.capabilities().file_sequence);
    assert!(Format::Raw.capabilities().multiple_frames);
}

#[test]
fn bounded_probes_prefer_signatures_and_resolve_extensions() {
    assert_eq!(super::detect("misnamed.png", b"\x76\x2f\x31\x01rest"), Some(Format::OpenExr));
    assert_eq!(super::detect("misnamed.exr", b"\x89PNG\r\n\x1a\n"), Some(Format::Png));
    assert_eq!(super::detect("native.psdt", b"8BPS\0\0"), Some(Format::Psd));
    assert_eq!(super::detect("art.svg", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), Some(Format::Svg));
    assert_eq!(super::detect("photo.EXR", b""), Some(Format::OpenExr));
    assert_eq!(super::detect("notes.txt", b""), Some(Format::PlainText));
    assert_eq!(super::detect("unknown", b""), None);
}

#[test]
fn jpeg_ls_is_told_from_jpeg_and_resolves_its_extension() {
    assert_eq!(super::from_extension("JLS"), Some(Format::JpegLs));
    assert_eq!(Format::JpegLs.mime_type(), Some("image/jls"));
    assert_eq!(super::detect("scan.jls", b""), Some(Format::JpegLs));
    // SOI followed by an SOF55 frame header.
    assert_eq!(super::detect("scan.jpg", b"\xff\xd8\xff\xf7\0\x0b\x08\0\x01\0\x01\x01\x01\x11\0"), Some(Format::JpegLs));
    assert_eq!(super::detect("photo.jls", b"\xff\xd8\xff\xe0\0\x10JFIF\0\x01\x01\0\0\x01\0\x01\0\0\xff\xc0"), Some(Format::Jpeg));
    // An APP0 segment, a fill byte and a DRI segment before the SOF55 frame header.
    let with_segments = b"\xff\xd8\xff\xe0\0\x04\0\0\xff\xff\xdd\0\x04\0\0\xff\xf7\0\x0b\x08\0\x01\0\x01\x01\x01\x11\0";
    assert_eq!(super::detect("scan.jpg", with_segments), Some(Format::JpegLs));
    assert_eq!(super::detect("", with_segments), Some(Format::JpegLs));
    // Bytes that end before any frame header: the name decides.
    assert_eq!(super::detect("scan.jls", b"\xff\xd8\xff\xe0\0\x10"), Some(Format::JpegLs));
    assert_eq!(super::detect("photo.jpg", b"\xff\xd8\xff\xe0\0\x10"), Some(Format::Jpeg));
    assert_eq!(super::detect("", b"\xff\xd8\xff\xe0\0\x10"), Some(Format::Jpeg));
}

#[test]
fn jpeg_marker_walk_reads_only_the_first_mebibyte() {
    let mut bytes = vec![0xFF; (1 << 20) + 64];
    bytes[1] = 0xD8;
    let len = bytes.len();
    bytes[len - 2] = 0xF7;
    assert_eq!(super::detect("scan.jpg", &bytes), Some(Format::Jpeg));
    assert_eq!(super::detect("scan.jls", &bytes), Some(Format::JpegLs));
    bytes[1 << 19] = 0xF7;
    assert_eq!(super::detect("scan.jpg", &bytes), Some(Format::JpegLs));
}

#[cfg(feature = "raster-codecs")]
#[test]
fn jpeg_marker_walk_matches_the_codec_detection() {
    let cases: [&[u8]; 6] = [
        b"\xff\xd8\xff\xf7\0\x0b\x08\0\x01\0\x01\x01\x01\x11\0",
        b"\xff\xd8\xff\xe0\0\x04\0\0\xff\xff\xdd\0\x04\0\0\xff\xf7\0\x0b\x08\0\x01\0\x01\x01\x01\x11\0",
        b"\xff\xd8\xff\xe0\0\x10JFIF\0\x01\x01\0\0\x01\0\x01\0\0\xff\xc0",
        b"\xff\xd8\xff\xd0\xff\x01\xff\xf7\0\x0b",
        b"\xff\xd8\xff\xe0\0\x04\0\0\x12\x34",
        b"\xff\xd8\xff\xda\0\x08",
    ];
    for bytes in cases {
        let expected = match craft_image_codecs::detect(bytes) {
            Some(craft_image_codecs::Format::JpegLs) => Format::JpegLs,
            _ => Format::Jpeg,
        };
        assert_eq!(super::detect("", bytes), Some(expected), "{bytes:02x?}");
    }
}

#[test]
fn jpeg_ls_capabilities_follow_the_codec() {
    #[cfg(feature = "raster-codecs")]
    {
        assert_eq!(Format::JpegLs.raster_codec(), Some(craft_image_codecs::Format::JpegLs));
        assert_eq!(Format::JpegLs.extensions(), craft_image_codecs::Format::JpegLs.extensions());
        assert_eq!(Format::JpegLs.capabilities().read, Availability::Available);
        assert_eq!(Format::JpegLs.capabilities().write, Availability::Available);
    }
    #[cfg(not(feature = "raster-codecs"))]
    {
        assert_eq!(Format::JpegLs.capabilities().read, Availability::BuildDisabled);
        assert_eq!(Format::JpegLs.capabilities().write, Availability::BuildDisabled);
    }
}

#[test]
fn registered_mime_and_preferred_extensions_remain_distinct() {
    assert_eq!(Format::Svg.mime_type(), Format::Svgz.mime_type());
    assert_eq!(Format::Svg.preferred_extension(), Some("svg"));
    assert_eq!(Format::Svgz.preferred_extension(), Some("svgz"));
    assert_eq!(Format::DesignCraftJson.mime_type(), Some("application/json"));
    assert_eq!(Format::DesignCraftJson.preferred_extension(), None);
    assert_eq!(Format::TaggedText.preferred_extension(), None);
    assert_eq!(super::from_extension("not-a-format"), None);
}

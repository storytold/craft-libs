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
fn registered_mime_and_preferred_extensions_remain_distinct() {
    assert_eq!(Format::Svg.mime_type(), Format::Svgz.mime_type());
    assert_eq!(Format::Svg.preferred_extension(), Some("svg"));
    assert_eq!(Format::Svgz.preferred_extension(), Some("svgz"));
    assert_eq!(Format::DesignCraftJson.mime_type(), Some("application/json"));
    assert_eq!(Format::DesignCraftJson.preferred_extension(), None);
    assert_eq!(Format::TaggedText.preferred_extension(), None);
    assert_eq!(super::from_extension("not-a-format"), None);
}

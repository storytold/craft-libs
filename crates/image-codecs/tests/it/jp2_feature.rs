//! JPEG 2000 decode is opt-in: without the `jp2` feature a JP2 file is detected and refused with a
//! named reason rather than decoded, so a crate that only reads SVG keeps no JPEG 2000 decoder.

use craft_image_codecs::*;

/// JP2 signature box and file type box, and nothing else.
const JP2_HEAD: &[u8] = b"\0\0\0\x0cjP  \r\n\x87\n\0\0\0\x14ftypjp2 \0\0\0\0jp2 ";

#[test]
fn a_jp2_file_is_detected_without_the_feature() {
    assert_eq!(detect(JP2_HEAD), Some(Format::Jp2));
}

#[test]
fn a_jp2_file_is_refused_with_the_feature_named() {
    let err = decode_with(JP2_HEAD, &DecodeOptions::default()).expect_err("no image in a bare header");
    if cfg!(feature = "jp2") {
        // With the feature the bare header reaches the decoder and fails as undecodable.
        assert!(matches!(err, CodecError::Malformed { .. } | CodecError::InvalidImage(_)), "decodes as far as the decoder: {err:?}");
    } else {
        match err {
            CodecError::Unsupported { format, reason } => {
                assert_eq!(format, Format::Jp2);
                assert!(reason.contains("jp2 feature"), "reason names the feature: {reason}");
            }
            other => panic!("expected an unsupported format error, got {other:?}"),
        }
    }
}

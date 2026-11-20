//! Unit tests of the track decoder: codec selection, configuration checks and the frames kept
//! from a failed call. Decoding real streams is in `tests/track`.

use super::*;

fn params(codec: CodecId, extradata: &[u8]) -> CodecParams {
    CodecParams::new(codec).with_extradata(extradata.to_vec())
}

fn new_err(codec: CodecId, extradata: &[u8]) -> VideoError {
    match TrackDecoder::new(&params(codec, extradata)) {
        Ok(_) => panic!("{codec:?} with extradata {extradata:02x?} was accepted"),
        Err(e) => e,
    }
}

#[test]
fn a_codec_without_a_decoder_is_unsupported() {
    let off = [
        CodecId::Theora,
        CodecId::Aac,
        CodecId::Unknown,
        #[cfg(not(feature = "prores"))]
        CodecId::ProRes,
        #[cfg(not(feature = "dnx"))]
        CodecId::Dnxhd,
        #[cfg(not(feature = "apv-decode"))]
        CodecId::Apv,
        #[cfg(not(feature = "ffv1"))]
        CodecId::Ffv1,
        #[cfg(not(feature = "vp8-decode"))]
        CodecId::Vp8,
        #[cfg(not(feature = "h264-decode"))]
        CodecId::H264,
        #[cfg(not(feature = "hevc-decode"))]
        CodecId::Hevc,
        #[cfg(not(feature = "vp9-decode"))]
        CodecId::Vp9,
        #[cfg(not(feature = "av1-decode"))]
        CodecId::Av1,
        #[cfg(not(feature = "mpeg2-decode"))]
        CodecId::Mpeg2Video,
    ];
    for codec in off {
        let err = new_err(codec, &[]);
        assert!(matches!(err, VideoError::Unsupported(_)), "{codec:?}: {err}");
        assert!(err.to_string().contains(codec.name()), "{err}");
    }
}

#[test]
fn decoders_other_than_ffv1_start_without_configuration() {
    let on = [
        #[cfg(feature = "prores")]
        CodecId::ProRes,
        #[cfg(feature = "dnx")]
        CodecId::Dnxhd,
        #[cfg(feature = "apv-decode")]
        CodecId::Apv,
        #[cfg(feature = "vp8-decode")]
        CodecId::Vp8,
        #[cfg(feature = "h264-decode")]
        CodecId::H264,
        #[cfg(feature = "hevc-decode")]
        CodecId::Hevc,
        #[cfg(feature = "vp9-decode")]
        CodecId::Vp9,
        #[cfg(feature = "av1-decode")]
        CodecId::Av1,
        #[cfg(feature = "mpeg2-decode")]
        CodecId::Mpeg2Video,
    ];
    for codec in on {
        let mut dec = TrackDecoder::new(&params(codec, &[])).unwrap();
        assert_eq!(dec.codec(), codec);
        assert!(dec.flush().unwrap().is_empty(), "{codec:?}");
        dec.reset().unwrap();
        assert!(dec.decode(&[], 0).unwrap_or_default().is_empty(), "{codec:?}");
    }
}

#[test]
fn track_decoder_is_send() {
    fn send<T: Send>() {}
    send::<TrackDecoder>();
}

/// An Annex B configuration that holds a slice would start a picture: it is refused before any
/// NAL unit reaches the decoder (which would report the slice's missing parameter set instead).
#[cfg(feature = "h264-decode")]
#[test]
fn h264_annex_b_configuration_with_a_slice_is_refused() {
    let err = new_err(CodecId::H264, &[0, 0, 0, 1, 0x65, 0x88, 0x84, 0x00]);
    assert!(matches!(err, VideoError::Malformed(_)), "{err}");
    assert!(err.to_string().contains("NAL unit type 5"), "{err}");
}

#[cfg(feature = "hevc-decode")]
#[test]
fn hevc_annex_b_configuration_with_a_slice_is_refused() {
    // IDR_W_RADL (19): (19 << 1) = 0x26
    let err = new_err(CodecId::Hevc, &[0, 0, 1, 0x26, 0x01, 0xaf, 0x00]);
    assert!(matches!(err, VideoError::Malformed(_)), "{err}");
    assert!(err.to_string().contains("NAL unit type 19"), "{err}");
}

/// A picture after a valid sequence header is refused, not decoded and then dropped.
#[cfg(feature = "mpeg2-decode")]
#[test]
fn mpeg2_configuration_with_a_picture_is_refused() {
    // 32x32, 4:3, 25 fps
    let header = [0, 0, 1, 0xb3, 0x02, 0x00, 0x20, 0x23, 0xff, 0xff, 0xe0, 0x18];
    assert!(TrackDecoder::new(&params(CodecId::Mpeg2Video, &header)).is_ok());
    // a picture header (I picture) and the start of its first slice
    let picture = [0, 0, 1, 0x00, 0x00, 0x0f, 0xff, 0xf8, 0, 0, 1, 0x01, 0x12, 0x34];
    let err = new_err(CodecId::Mpeg2Video, &[&header[..], &picture].concat());
    assert!(matches!(err, VideoError::Malformed(_)), "{err}");
    assert!(err.to_string().contains("start code 0x00"), "{err}");
}

/// A record that is neither Annex B nor version 1 is the codec's own error.
#[cfg(feature = "h264-decode")]
#[test]
fn h264_configuration_that_is_not_avcc_is_refused() {
    let err = new_err(CodecId::H264, &[2, 0x42, 0, 0x1e, 0xff, 0xe0, 0]);
    assert!(matches!(err, VideoError::Malformed(_)), "{err}");
    assert!(err.to_string().contains("avcC"), "{err}");
}

#[cfg(feature = "av1-decode")]
mod av1c {
    use super::*;

    const HEADER: [u8; 4] = [0x81, 0x00, 0x0c, 0x00];

    fn with_obus(obus: &[u8]) -> Vec<u8> {
        [&HEADER[..], obus].concat()
    }

    #[test]
    fn empty_and_header_only_records_hold_no_obus() {
        assert_eq!(av1c_config_obus(&[]).unwrap(), &[] as &[u8]);
        assert_eq!(av1c_config_obus(&HEADER).unwrap(), &[] as &[u8]);
    }

    #[test]
    fn sequence_headers_and_metadata_pass() {
        // a sequence header (size 2), one with an extension byte (size 0), metadata (size 1), and
        // a sequence header without a size field that runs to the end
        let obus = [0x0a, 2, 0xaa, 0xbb, 0x0e, 0x00, 0, 0x2a, 1, 0xcc, 0x08, 0xdd, 0xee];
        assert_eq!(av1c_config_obus(&with_obus(&obus)).unwrap(), &obus);
    }

    #[test]
    fn a_record_that_is_not_av1c_version_1_is_refused() {
        for bad in [&[0x81, 0, 0][..], &[0x80, 0, 0, 0], &[0x82, 0, 0, 0], &[0x0a, 0, 0, 0, 0]] {
            let err = av1c_config_obus(bad).unwrap_err();
            assert!(err.to_string().contains("not an av1C record"), "{bad:02x?}: {err}");
        }
    }

    #[test]
    fn a_frame_or_temporal_delimiter_is_refused() {
        // temporal delimiter (2), frame header (3), tile group (4), frame (6), after a valid OBU
        for t in [2u8, 3, 4, 6] {
            let obus = [0x0a, 0, (t << 3) | 2, 0];
            let err = av1c_config_obus(&with_obus(&obus)).unwrap_err();
            assert!(err.to_string().contains(&format!("OBU type {t}")), "{t}: {err}");
        }
        assert!(matches!(new_err(CodecId::Av1, &with_obus(&[0x32, 0])), VideoError::Malformed(_)));
    }

    #[test]
    fn truncated_obus_are_refused() {
        for obus in [&[0x0a, 5, 1, 2][..], &[0x0a, 0x80], &[0x0a, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff], &[0x0e]] {
            let err = av1c_config_obus(&with_obus(obus)).unwrap_err();
            assert!(matches!(err, VideoError::Malformed(_)), "{obus:02x?}: {err}");
        }
    }

    #[test]
    fn hostile_records_do_not_panic() {
        let seed = with_obus(&[0x0a, 2, 0xaa, 0xbb, 0x2a, 1, 0xcc]);
        craft_media_testkit::hostile::hammer("av1C config OBUs", &seed, 2000, |data| {
            let _ = av1c_config_obus(data);
        });
    }
}

/// Configuration bytes of every codec, damaged, never panic the decoder that applies them.
#[test]
fn hostile_configuration_does_not_panic() {
    let seeds: &[(CodecId, &[u8])] = &[
        #[cfg(feature = "h264-decode")]
        (CodecId::H264, &[1, 0x42, 0xc0, 0x1e, 0xff, 0xe1, 0x00, 0x04, 0x67, 0x42, 0xc0, 0x1e, 0x01, 0x00, 0x02, 0x68, 0xce]),
        #[cfg(feature = "h264-decode")]
        (CodecId::H264, &[0, 0, 0, 1, 0x67, 0x42, 0xc0, 0x1e, 0, 0, 1, 0x68, 0xce]),
        #[cfg(feature = "hevc-decode")]
        (CodecId::Hevc, &[0, 0, 0, 1, 0x40, 0x01, 0x0c, 0, 0, 1, 0x42, 0x01, 0x01, 0, 0, 1, 0x44, 0x01, 0xc1]),
        #[cfg(feature = "av1-decode")]
        (CodecId::Av1, &[0x81, 0x00, 0x0c, 0x00, 0x0a, 0x0b, 0x00, 0x00, 0x00, 0x24, 0xc4, 0xff, 0xdf, 0x00, 0x68, 0x02, 0x10]),
        #[cfg(feature = "mpeg2-decode")]
        (CodecId::Mpeg2Video, &[0, 0, 1, 0xb3, 0x02, 0x00, 0x20, 0x13, 0xff, 0xff, 0xe0, 0x18, 0, 0, 1, 0xb5, 0x14, 0x8a, 0x00, 0x01, 0x00, 0x00]),
    ];
    for &(codec, seed) in seeds {
        craft_media_testkit::hostile::hammer(codec.name(), seed, 300, move |data| {
            if let Ok(mut dec) = TrackDecoder::new(&params(codec, data)) {
                let _ = dec.reset();
            }
        });
    }
}

#[cfg(feature = "h264-decode")]
mod held {
    use super::*;

    /// A 4x2 picture with timestamp `pts`; `y_len` other than 8 makes it fail to convert.
    fn picture(y_len: usize, pts: i64) -> h264::decode::Picture {
        h264::decode::Picture {
            width: 4,
            height: 2,
            chroma_width: 2,
            chroma_height: 1,
            y: vec![16; y_len],
            u: vec![128; 2],
            v: vec![128; 2],
            y_stride: 4,
            uv_stride: 2,
            pts,
            poc: 0,
            key: true,
            color: h264::decode::ColorInfo { full_range: false, primaries: 1, transfer: 1, matrix: 1 },
            sar: (0, 0),
            draft: false,
        }
    }

    fn decoder() -> TrackDecoder {
        TrackDecoder::new(&params(CodecId::H264, &[])).unwrap()
    }

    fn pts(frames: &[VideoFrame]) -> Vec<Option<i64>> {
        frames.iter().map(|f| f.pts).collect()
    }

    /// A picture that does not convert fails the call; the others come with the next call, ahead
    /// of its own frames and in order.
    #[test]
    fn frames_around_a_picture_that_does_not_convert_come_next() {
        let mut dec = decoder();
        let err = dec.output(vec![picture(8, 1), picture(3, 2), picture(8, 3)], None).unwrap_err();
        assert!(matches!(err, VideoError::Frame(FrameError::Plane(_))), "{err}");
        assert_eq!(pts(&dec.output(vec![picture(8, 4)], None).unwrap()), [Some(1), Some(3), Some(4)]);
        assert!(dec.output(Vec::<h264::decode::Picture>::new(), None).unwrap().is_empty());
    }

    /// An error reported with the pictures (a flush whose decoding job failed) keeps them too.
    #[test]
    fn frames_finished_with_an_error_come_next() {
        let mut dec = decoder();
        let err = dec.output(vec![picture(8, 1)], Some(VideoError::Malformed("job".into()))).unwrap_err();
        assert!(matches!(err, VideoError::Malformed(_)));
        assert_eq!(pts(&dec.flush().unwrap()), [Some(1)]);
    }

    /// An error from the codec itself leaves the kept frames for the call after it.
    #[test]
    fn a_failed_packet_does_not_drop_kept_frames() {
        let mut dec = decoder();
        assert!(dec.output(vec![picture(8, 1), picture(0, 2)], None).is_err());
        // an IDR slice without parameter sets
        assert!(dec.decode(&[0, 0, 1, 0x65, 0x88, 0x84], 5).is_err());
        assert_eq!(pts(&dec.decode(&[], 6).unwrap()), [Some(1)]);
    }

    #[test]
    fn reset_drops_kept_frames() {
        let mut dec = decoder();
        assert!(dec.output(vec![picture(8, 1), picture(0, 2)], None).is_err());
        dec.reset().unwrap();
        assert!(dec.flush().unwrap().is_empty());
    }
}

#[cfg(any(feature = "prores", feature = "dnx", feature = "apv", feature = "ffv1", feature = "vp8"))]
fn check_packet(params: CodecParams, packet: &[u8], mut expected: VideoFrame) {
    let mut dec = TrackDecoder::new(&params).unwrap();
    assert_eq!(dec.codec(), params.codec);
    for pts in [-17, 91] {
        expected.pts = Some(pts);
        assert_eq!(dec.decode(packet, pts).unwrap(), [expected.clone()]);
        assert!(dec.flush().unwrap().is_empty());
        assert!(dec.flush().unwrap().is_empty());
        assert!(matches!(dec.decode(&[], 0), Err(VideoError::Malformed(_))));
        dec.reset().unwrap();
        assert!(dec.flush().unwrap().is_empty());
    }
}

#[cfg(feature = "prores")]
#[test]
fn prores_packet_timestamps_and_reset() {
    use crate::prores::*;
    let frame = Frame::new(32, 32, ChromaFormat::Yuv422, 10, false).unwrap();
    let mut enc = Encoder::new(Profile::Proxy, 32, 32);
    let packet = enc.encode(&frame).unwrap();
    check_packet(enc.codec_params(), &packet, decode_frame(&packet).unwrap().try_into().unwrap());
}

#[cfg(feature = "dnx")]
#[test]
fn dnx_packet_timestamps_and_reset() {
    use crate::dnx::*;
    let frame = Frame::new(64, 64, ChromaFormat::Yuv422, 8, false);
    let mut enc = Encoder::new(Profile::Lb, 64, 64).unwrap();
    let packet = enc.encode(&frame).unwrap();
    check_packet(enc.codec_params(), &packet, decode_frame(&packet).unwrap().try_into().unwrap());
}

#[cfg(feature = "apv")]
#[test]
fn apv_packet_timestamps_and_reset() {
    use crate::apv::*;
    let frame = Frame::new(32, 32, ChromaFormat::Yuv422, 10, false);
    let mut enc = Encoder::new(Profile::P422_10, 32, 32).unwrap();
    let packet = enc.encode(&frame).unwrap();
    check_packet(params(CodecId::Apv, &enc.decoder_config_record()), &packet, decode_frame(&packet).unwrap().try_into().unwrap());
}

#[cfg(feature = "ffv1")]
#[test]
fn ffv1_configuration_dimensions_timestamps_and_reset() {
    use crate::ffv1::*;
    let frame = Frame {
        width: 16,
        height: 16,
        colorspace: Colorspace::Rgb,
        bit_depth: 10,
        chroma_shift: (0, 0),
        planes: vec![vec![128; 256], vec![512; 256], vec![768; 256], vec![1023; 256]],
    };
    let enc = Encoder::new(&frame, EncoderConfig::default()).unwrap();
    let config = enc.config_record();
    let packet = enc.encode(&frame).unwrap();
    let p = params(CodecId::Ffv1, &config).with_size(16, 16);
    check_packet(p.clone(), &packet, frame.try_into().unwrap());
    for (width, height) in [(None, Some(16)), (Some(16), None), (Some(0), Some(16)), (Some(16), Some(0))] {
        let mut bad = p.clone();
        bad.width = width;
        bad.height = height;
        assert!(matches!(TrackDecoder::new(&bad), Err(VideoError::Malformed(_))));
    }
    for config in [&[][..], &config[..config.len() - 1]] {
        assert!(matches!(TrackDecoder::new(&params(CodecId::Ffv1, config).with_size(16, 16)), Err(VideoError::Malformed(_))));
    }
}

#[cfg(feature = "vp8")]
#[test]
fn vp8_hidden_frames_timestamps_and_reset() {
    use crate::vp8::*;
    let packet = encode_rgba(&vec![128; 32 * 32 * 4], 32, 32, &Vp8Params::default()).unwrap().data;
    let expected = Decoder::new().decode(&packet, 0).unwrap().unwrap().try_into().unwrap();
    check_packet(params(CodecId::Vp8, &[]), &packet, expected);
    let mut dec = TrackDecoder::new(&params(CodecId::Vp8, &[])).unwrap();
    let mut hidden = packet.clone();
    hidden[0] &= !0x10; // show_frame = 0: updates references, produces no displayed picture.
    assert!(dec.decode(&hidden, 33).unwrap().is_empty());
    assert!(dec.flush().unwrap().is_empty());
    dec.reset().unwrap();
    // An inter-frame tag with one byte of first partition must fail for missing references
    // before its payload is parsed. Keeping the previous key frame would give a syntax error.
    let err = dec.decode(&[0x31, 0, 0, 0], 34).unwrap_err();
    assert!(err.to_string().contains("missing reference frame"), "{err}");
    assert_eq!(dec.decode(&packet, 35).unwrap()[0].pts, Some(35));
}

#[test]
fn new_codec_errors_keep_their_category() {
    #[cfg(feature = "prores")]
    assert!(matches!(VideoError::from(crate::prores::Error::Unsupported("profile")), VideoError::Unsupported(_)));
    #[cfg(feature = "dnx")]
    assert!(matches!(VideoError::from(crate::dnx::Error::UnknownCid(0)), VideoError::Unsupported(_)));
    #[cfg(feature = "apv-decode")]
    assert!(matches!(VideoError::from(crate::apv::Error::Truncated), VideoError::Malformed(_)));
    #[cfg(feature = "ffv1")]
    assert!(matches!(VideoError::from(crate::ffv1::Error::Unsupported("inter frames")), VideoError::Unsupported(_)));
    #[cfg(feature = "vp8-decode")]
    assert!(matches!(VideoError::from(crate::vp8::Error::Invalid("header".into())), VideoError::Malformed(_)));
}

#[test]
#[allow(irrefutable_let_patterns, clippy::cognitive_complexity)]
fn track_decoder_options_and_draft_persist_across_reset() {
    #[allow(unused_variables)]
    let opts = TrackDecoderOptions { threads: Some(0), draft: true };
    #[cfg(feature = "h264-decode")]
    {
        let mut dec = TrackDecoder::with_options(&params(CodecId::H264, &[]), opts).unwrap();
        assert_eq!(dec.threads(), Some(1));
        assert!(dec.draft());
        if let Inner::H264 { dec: inner, .. } = &dec.inner {
            assert_eq!(inner.threads(), 1);
            assert!(inner.draft());
        } else {
            panic!("expected H264 inner");
        }
        dec.reset().unwrap();
        assert!(dec.draft());
        if let Inner::H264 { dec: inner, .. } = &dec.inner {
            assert_eq!(inner.threads(), 1);
            assert!(inner.draft());
        }
        dec.set_draft(false);
        assert!(!dec.draft());
        if let Inner::H264 { dec: inner, .. } = &dec.inner {
            assert!(!inner.draft());
        }
        let dec2 = TrackDecoder::with_threads(&params(CodecId::H264, &[]), 2).unwrap();
        assert_eq!(dec2.threads(), Some(2));
        assert!(!dec2.draft());
        if let Inner::H264 { dec: inner, .. } = &dec2.inner {
            let expected = if cfg!(all(feature = "threads", not(target_arch = "wasm32"))) { 2 } else { 1 };
            assert_eq!(inner.threads(), expected);
            assert!(!inner.draft());
        }
    }
    #[cfg(feature = "hevc-decode")]
    {
        let mut dec = TrackDecoder::with_options(&params(CodecId::Hevc, &[]), opts).unwrap();
        assert_eq!(dec.threads(), Some(1));
        assert!(dec.draft());
        if let Inner::Hevc { dec: inner, .. } = &dec.inner {
            assert!(inner.draft());
        }
        dec.reset().unwrap();
        assert!(dec.draft());
        if let Inner::Hevc { dec: inner, .. } = &dec.inner {
            assert!(inner.draft());
        }
    }
    #[cfg(feature = "av1-decode")]
    {
        let mut dec = TrackDecoder::with_options(&params(CodecId::Av1, &[]), opts).unwrap();
        assert_eq!(dec.threads(), Some(1));
        assert!(dec.draft());
        if let Inner::Av1 { dec: inner, .. } = &dec.inner {
            assert!(inner.draft());
        }
        dec.reset().unwrap();
        assert!(dec.draft());
        if let Inner::Av1 { dec: inner, .. } = &dec.inner {
            assert!(inner.draft());
        }
    }
    #[cfg(feature = "vp9-decode")]
    {
        let mut dec = TrackDecoder::with_options(&params(CodecId::Vp9, &[]), opts).unwrap();
        assert_eq!(dec.threads(), Some(1));
        assert!(dec.draft());
        if let Inner::Vp9(inner) = &dec.inner {
            assert_eq!(inner.threads(), 1);
            assert!(inner.draft());
        }
        dec.reset().unwrap();
        assert!(dec.draft());
        if let Inner::Vp9(inner) = &dec.inner {
            assert_eq!(inner.threads(), 1);
            assert!(inner.draft());
        }
    }
}

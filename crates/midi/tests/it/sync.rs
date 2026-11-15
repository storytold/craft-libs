use craft_midi::sync::{CLOCK, CONTINUE, MAX_POSITION, Rate, SONG_POSITION, START, STOP, SyncMessage, full_frame, quarter_frames, song_position};

#[test]
fn transport_prefix_compatibility() {
    for (status, expected) in [(CLOCK, SyncMessage::Clock), (START, SyncMessage::Start), (CONTINUE, SyncMessage::Continue), (STOP, SyncMessage::Stop)]
    {
        assert_eq!(SyncMessage::parse(&[status]), Some(expected));
        assert_eq!(SyncMessage::parse(&[status, 0, 0xff]), Some(expected));
    }
    for invalid in [&[][..], &[0x90, 60, 127], &[SONG_POSITION], &[SONG_POSITION, 0], &[SONG_POSITION, 0x80, 0], &[SONG_POSITION, 0, 0x80]] {
        assert_eq!(SyncMessage::parse(invalid), None);
    }
}

#[test]
fn song_position_preserves_clamping_and_seven_bit_order() {
    for value in 0..=u16::MAX {
        let bytes = song_position(value);
        assert!(bytes[1] < 128 && bytes[2] < 128);
        assert_eq!(SyncMessage::parse(&bytes), Some(SyncMessage::Position(value.min(MAX_POSITION))));
    }
    assert_eq!(song_position(128), [0xf2, 0, 1]);
    assert_eq!(SyncMessage::parse(&[0xf2, 1, 2, 0xff]), Some(SyncMessage::Position(257)));
}

#[test]
fn full_frame_known_vectors_and_legacy_conversion() {
    for (rate, bits) in [(Rate::Fps24, 0), (Rate::Fps25, 0x20), (Rate::Fps30, 0x60)] {
        assert_eq!(full_frame(3723.0, rate), [0xf0, 0x7f, 0x7f, 1, 1, bits | 1, 2, 3, 0, 0xf7]);
        assert_eq!(full_frame(86400.0, rate), full_frame(0.0, rate));
        assert_eq!(full_frame(-1.0, rate), full_frame(0.0, rate));
        assert_eq!(full_frame(f64::NAN, rate), full_frame(0.0, rate));
        assert_eq!(full_frame(1.0 / f64::from(rate.fps()), rate)[8], 1);
    }
    assert_eq!(Rate::of(24), Rate::Fps24);
    assert_eq!(Rate::of(30), Rate::Fps30);
    assert_eq!(Rate::of(29), Rate::Fps25);
}

#[test]
fn quarter_frames_known_vector() {
    assert_eq!(
        quarter_frames(3723.04, Rate::Fps25),
        [[0xf1, 1], [0xf1, 0x10], [0xf1, 0x23], [0xf1, 0x30], [0xf1, 0x42], [0xf1, 0x50], [0xf1, 0x61], [0xf1, 0x72]]
    );
    for rate in [Rate::Fps24, Rate::Fps25, Rate::Fps30] {
        for secs in [0.0, 59.99, 3599.99, 86399.99, f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            for bytes in quarter_frames(secs, rate) {
                assert_eq!(bytes[0], 0xf1);
                assert!(bytes[1] < 128);
            }
        }
    }
}

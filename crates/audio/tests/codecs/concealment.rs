//! A packet that fails to decode is replaced by silence of its length, so the audio after it keeps
//! its timing; a stream where nothing decodes is still an error. The streams are made with
//! craft-audio's own AAC encoder (no ffmpeg needed).

use craft_audio::aac::{AdtsHeader, Encoder, EncoderConfig};
use craft_audio::{AudioError, decode};

/// One second of a 440 Hz stereo tone as ADTS AAC, and the offset and length of each frame.
fn adts_stream() -> (Vec<u8>, Vec<(usize, usize)>) {
    let rate = 48_000;
    let tone: Vec<f32> = (0..rate).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.5).collect();
    let mut enc = Encoder::new(EncoderConfig::cbr(rate as u32, 2, 128_000).with_adts(true)).unwrap();
    let mut units = enc.encode(&[&tone, &tone]);
    units.extend(enc.flush());
    let mut frames = Vec::new();
    let mut bytes = Vec::new();
    for u in units {
        frames.push((bytes.len(), u.len()));
        bytes.extend(u);
    }
    (bytes, frames)
}

#[test]
fn garbled_frames_keep_the_timeline() {
    let (clean, frames) = adts_stream();
    let (_, reference) = decode(&clean, Some("aac")).unwrap();
    let mut garbled = clean.clone();
    // Overwrite the payload (not the 7-byte header) of ten frames in the middle.
    for &(at, len) in &frames[10..20] {
        let header = AdtsHeader::parse(&garbled[at..]).unwrap().header_length;
        for b in &mut garbled[at + header..at + len] {
            *b = 0xFF;
        }
    }
    let (_, damaged) = decode(&garbled, Some("aac")).unwrap();
    assert_eq!(damaged.frames(), reference.frames(), "a garbled frame must not shorten the stream");
    // Audio after the damage stays where it was.
    let at = reference.frames() - 4800;
    assert_eq!(&damaged.channels[0][at..], &reference.channels[0][at..]);
}

#[test]
fn a_stream_where_nothing_decodes_is_an_error() {
    let (mut bytes, frames) = adts_stream();
    for &(at, len) in &frames {
        let header = AdtsHeader::parse(&bytes[at..]).unwrap().header_length;
        for b in &mut bytes[at + header..at + len] {
            *b = 0xFF;
        }
    }
    match decode(&bytes, Some("aac")) {
        Err(AudioError::Malformed(_) | AudioError::Unsupported(_)) => {}
        other => panic!("expected an error, not silence: {:?}", other.map(|(i, b)| (i.frames, b.frames()))),
    }
}

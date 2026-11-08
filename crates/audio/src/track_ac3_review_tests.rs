// Test fixture construction and expected-success operations intentionally unwrap.
#![allow(clippy::unwrap_used)]

use super::*;
use crate::ac3::encoder::{Codec, CodingMode, Config, Encoder};

fn excessive_dependent_frames() -> Vec<u8> {
    // Structural-only E-AC-3 dependent frames: no primary audio is allocated or decoded.
    [0x0b, 0x77, 0x40, 3, 0x34, 0x80, 0, 0].repeat(4097)
}

#[test]
fn ac3_dependent_packet_decode_has_a_work_bound() {
    let mut dec = TrackDecoder::new(&CodecParams::new(CodecId::Eac3), &[]).unwrap();
    assert!(matches!(dec.decode(&excessive_dependent_frames()), Err(AudioError::TooLarge(_))));
}

#[test]
fn ac3_dependent_packet_concealment_has_a_work_bound() {
    let mut dec = TrackDecoder::new(&CodecParams::new(CodecId::Eac3), &[]).unwrap();
    assert!(matches!(dec.conceal(Some(&excessive_dependent_frames())), Err(AudioError::TooLarge(_))));
}

#[test]
fn ac3_dependent_packet_lookahead_has_a_work_bound() {
    assert!(matches!(TrackDecoder::new(&CodecParams::new(CodecId::Eac3), &[&excessive_dependent_frames()]), Err(AudioError::TooLarge(_))));
}

#[test]
fn ac3_truncated_primary_does_not_establish_track_format() {
    let wrong = packets(Codec::Eac3, CodingMode::Mono, 32_000);
    let good = packets(Codec::Eac3, CodingMode::Stereo, 48_000);
    let mut dec = TrackDecoder::new(&CodecParams::new(CodecId::Eac3), &[&wrong[0][..8], &good[0]]).unwrap();
    assert_eq!((dec.sample_rate(), dec.channels()), (48_000, 2));
    assert!(dec.decode(&good[0]).is_ok());
}

fn packets(codec: Codec, mode: CodingMode, rate: u32) -> Vec<Vec<u8>> {
    let config = Config { codec, mode, lfe: false, sample_rate: rate, bitrate_kbps: 192 };
    let mut encoder = Encoder::new(config).unwrap();
    let signal: Vec<f32> = (0..4096).map(|i| 0.3 * (i as f32 * 0.17).sin()).collect();
    let planes: Vec<&[f32]> = (0..config.channels()).map(|_| signal.as_slice()).collect();
    let mut packets = encoder.push(&planes).unwrap();
    packets.extend(encoder.finish().unwrap().packets);
    packets.into_iter().map(|p| p.data).collect()
}

#[test]
fn bounded_ac3_append_rejects_before_mutating_output() {
    let mut out = vec![vec![1.0, 2.0]; 2];
    let before = out.clone();
    assert!(matches!(append_ac3(&mut out, vec![vec![3.0]; 2], 5), Err(AudioError::TooLarge(_))));
    assert_eq!(out, before);
    append_ac3(&mut out, vec![vec![3.0]; 2], 6).unwrap();
    assert_eq!(out, vec![vec![1.0, 2.0, 3.0]; 2]);
    let mut empty = Vec::new();
    assert!(matches!(append_ac3(&mut empty, vec![vec![0.0; 3]; 2], 5), Err(AudioError::TooLarge(_))));
    assert!(empty.is_empty());
}

#[test]
fn ac3_packet_errors_are_not_partial_success_or_state_changes() {
    let frames = packets(Codec::Ac3, CodingMode::Stereo, 48_000);
    let params = CodecParams::new(CodecId::Ac3);
    let mut got = TrackDecoder::new(&params, &[&frames[0]]).unwrap();
    let mut reference = TrackDecoder::new(&params, &[&frames[0]]).unwrap();
    got.decode(&frames[0]).unwrap();
    reference.decode(&frames[0]).unwrap();
    let broken = [frames[1].as_slice(), &[0; 8]].concat();
    assert!(got.decode(&broken).is_err(), "a later failed syncframe must fail the entire packet");
    assert_eq!(got.decode(&frames[1]).unwrap().channels, reference.decode(&frames[1]).unwrap().channels);
    let mono = packets(Codec::Ac3, CodingMode::Mono, 48_000);
    assert!(got.decode(&[frames[2].as_slice(), mono[0].as_slice()].concat()).is_err());
    assert_eq!(got.decode(&frames[2]).unwrap().channels, reference.decode(&frames[2]).unwrap().channels);
}

#[test]
fn ac3_track_uses_first_valid_metadata_and_rejects_later_changes() {
    let frames = packets(Codec::Eac3, CodingMode::Stereo, 32_000);
    let params = CodecParams::new(CodecId::Ac3).with_sample_rate(48_000);
    let mut dec = TrackDecoder::new(&params, &[&[0; 8], &frames[0]]).unwrap();
    assert_eq!((dec.codec(), dec.sample_rate()), (CodecId::Eac3, 32_000));
    dec.decode(&frames[0]).unwrap();
    let changed = packets(Codec::Eac3, CodingMode::Stereo, 48_000);
    assert!(dec.decode(&changed[0]).is_err());
    assert_eq!(dec.sample_rate(), 32_000);
    assert_eq!(dec.frames_per_packet(), None, "AC3-family packet duration is not inherently fixed");
}

#[test]
fn ac3_concealment_sums_primaries_and_preserves_dependent_only_zero() {
    let frames = packets(Codec::Eac3, CodingMode::Stereo, 48_000);
    let mut dec = TrackDecoder::new(&CodecParams::new(CodecId::Eac3), &[&frames[0]]).unwrap();
    dec.decode(&frames[0]).unwrap();
    let packet = [frames[0].as_slice(), frames[1].as_slice()].concat();
    assert_eq!(dec.conceal(Some(&packet)).unwrap().frames(), 3072);
    let mut dependent = frames[0].clone();
    dependent[2] = (dependent[2] & 0x3f) | 0x40;
    dependent.truncate(8);
    assert_eq!(dec.conceal(Some(&dependent)).unwrap().frames(), 0);
}

#[test]
fn ac3_packet_timing_distinguishes_short_blocks_and_programmes() {
    let packets = packets(Codec::Eac3, CodingMode::Stereo, 48_000);
    let mut short = packets[0].clone();
    short[4] &= !0x30; // numblkscod=0: one 256-sample block, header-only timing test.
    let mut dependent = packets[0].clone();
    dependent[2] = (dependent[2] & 0x3f) | 0x40;
    let mut other_programme = packets[0].clone();
    other_programme[2] = (other_programme[2] & !0x38) | 0x08;
    let aggregate = [short.as_slice(), dependent.as_slice(), other_programme.as_slice(), packets[1].as_slice()].concat();
    assert_eq!(ac3_packet_frames(&aggregate).unwrap(), Some(256 + 1536));
    assert_eq!(ac3_packet_frames(&short[..8]).unwrap(), Some(256));
    assert_eq!(ac3_packet_frames(&[]).unwrap(), None);
    assert_eq!(ac3_packet_frames(&[short.as_slice(), &[0; 8]].concat()).unwrap(), None);
}

#[test]
fn ac3_first_success_refines_unconfigured_metadata_atomically() {
    let frames = packets(Codec::Eac3, CodingMode::Stereo, 32_000);
    let params = CodecParams::new(CodecId::Ac3).with_sample_rate(48_000);
    let mut dec = TrackDecoder::new(&params, &[]).unwrap();
    assert!(dec.decode(&[frames[0].as_slice(), &[0; 8]].concat()).is_err());
    assert_eq!((dec.codec(), dec.sample_rate()), (CodecId::Ac3, 48_000));
    dec.decode(&frames[0]).unwrap();
    assert_eq!((dec.codec(), dec.sample_rate()), (CodecId::Eac3, 32_000));
    // Equal channel counts do not make dual mono and stereo the same speaker assignment.
    let dual = packets(Codec::Eac3, CodingMode::DualMono, 32_000);
    assert!(dec.decode(&dual[0]).is_err());
}

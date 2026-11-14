//! `TrackDecoder` on streams an app's own demuxer hands over without codec configuration:
//! ADTS-framed AAC (MPEG-TS) and Opus without an `OpusHead` (RTP).

use craft_audio::aac::{Encoder as AacEncoder, EncoderConfig};
use craft_audio::opus::Encoder as OpusEncoder;
use craft_audio::{CodecId, CodecParams, TrackDecoder};

fn tone(n: usize) -> Vec<f32> {
    (0..n).map(|i| (i as f32 * 0.03).sin() * 0.4).collect()
}

#[test]
fn adts_aac_without_extradata() {
    let left = tone(48_000);
    let right = tone(48_000);
    let mut enc = AacEncoder::new(EncoderConfig::cbr(48_000, 2, 128_000).with_adts(true)).unwrap();
    let mut units = enc.encode(&[left.as_slice(), right.as_slice()]);
    units.extend(enc.flush());

    // No AudioSpecificConfig: the first ADTS header gives it, and every header is stripped.
    let lookahead: Vec<&[u8]> = units.iter().take(craft_audio::LOOKAHEAD_PACKETS).map(Vec::as_slice).collect();
    let mut dec = TrackDecoder::new(&CodecParams::new(CodecId::Aac), &lookahead).unwrap();
    assert_eq!((dec.codec(), dec.sample_rate(), dec.channels()), (CodecId::Aac, 48_000, 2));
    let frames: usize = units.iter().map(|u| dec.decode(u).unwrap().frames()).sum();
    assert_eq!(frames, units.len() * 1024);

    // Two ADTS frames in one packet, as a PES payload often holds, decode as both.
    let mut dec = TrackDecoder::new(&CodecParams::new(CodecId::Aac), &lookahead).unwrap();
    let pair = [units[0].as_slice(), units[1].as_slice()].concat();
    assert_eq!(dec.decode(&pair).unwrap().frames(), 2048);

    // Raw access units still need their configuration.
    let raw = &units[0][7..];
    assert!(TrackDecoder::new(&CodecParams::new(CodecId::Aac), &[raw]).is_err());
}

#[test]
fn opus_without_opus_head() {
    let mut enc = OpusEncoder::new(1, 64_000).unwrap();
    let pcm = tone(OpusEncoder::FRAME_SIZE * 10);
    let packets: Vec<Vec<u8>> = pcm.chunks(OpusEncoder::FRAME_SIZE).map(|f| enc.encode_float(f).unwrap()).collect();

    let params = CodecParams::new(CodecId::Opus).with_channels(1).with_sample_rate(48_000);
    let mut dec = TrackDecoder::new(&params, &[]).unwrap();
    assert_eq!((dec.sample_rate(), dec.channels()), (48_000, 1));
    for p in &packets {
        assert_eq!(dec.decode(p).unwrap().frames(), OpusEncoder::FRAME_SIZE);
    }

    // More than two channels need the mapping an OpusHead carries.
    assert!(TrackDecoder::new(&CodecParams::new(CodecId::Opus).with_channels(6), &[]).is_err());
}

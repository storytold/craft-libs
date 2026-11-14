//! `PacketEncoder`: every codec round-trips through `TrackDecoder` from the description the
//! encoder gives, at the length that was encoded and with each channel on its speaker.

use std::io::Cursor;

use craft_audio::{
    AudioError, BitDepth, ChannelLayout, CodecId, EncodeFormat, EncodeOptions, EncodedPacket, FileEncoder, PacketEncoder, TrackDecoder,
};

/// `channels` channels of `frames` frames, each a tone of its own frequency low enough for an LFE.
fn tones(sample_rate: u32, channels: usize, frames: usize) -> Vec<Vec<f32>> {
    (0..channels)
        .map(|c| {
            let step = (40.0 + 9.0 * c as f32) * std::f32::consts::TAU / sample_rate as f32;
            (0..frames).map(|i| ((i as f32 * step).sin() * 0.4 * 32768.0).round() / 32768.0).collect()
        })
        .collect()
}

/// Normalised correlation of two signals over the samples both have.
fn correlation(a: &[f32], b: &[f32]) -> f64 {
    let (mut ab, mut aa, mut bb) = (0f64, 0f64, 0f64);
    for (&x, &y) in a.iter().zip(b) {
        let (x, y) = (f64::from(x), f64::from(y));
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    ab / (aa * bb).sqrt().max(1e-30)
}

/// Encodes `input` in uneven blocks and returns the packets.
fn encode(enc: &mut PacketEncoder, input: &[Vec<f32>]) -> Vec<EncodedPacket> {
    let frames = input[0].len();
    let mut packets = Vec::new();
    let mut at = 0;
    for n in [1, 999, 4096, 10_000, 7, usize::MAX].into_iter().cycle() {
        let n = n.min(frames - at);
        let block: Vec<&[f32]> = input.iter().map(|c| &c[at..at + n]).collect();
        packets.extend(enc.encode(&block, n).unwrap());
        at += n;
        if at == frames {
            break;
        }
    }
    packets.extend(enc.finish().unwrap());
    packets
}

/// Encodes, checks the timing the packets carry, decodes them from the encoder's description, and
/// returns the decoded audio from the first input frame on, cut to the input's length, and the
/// layout the decoder reports.
fn decode_back(codec: CodecId, opts: &EncodeOptions, sample_rate: u32, input: &[Vec<f32>]) -> (PacketEncoder, Vec<Vec<f32>>, ChannelLayout) {
    let frames = input[0].len();
    let mut enc = PacketEncoder::new(codec, opts, sample_rate, input.len() as u16).unwrap();
    let packets = encode(&mut enc, input);
    assert_eq!(enc.frames(), frames as u64);
    // The packets follow one another, and add up to the priming, the audio and the padding.
    let mut pts = 0;
    for p in &packets {
        assert_eq!(p.pts, pts, "{codec:?}");
        pts += u64::from(p.duration);
    }
    assert_eq!(pts, u64::from(enc.priming()) + frames as u64 + enc.padding(), "{codec:?}");
    let last = packets.last().map_or(0, |p| u64::from(p.duration));
    assert!(enc.padding() < last, "{codec:?}: {} samples of padding after a packet of {last}", enc.padding());

    let lookahead: Vec<&[u8]> = packets.iter().take(craft_audio::LOOKAHEAD_PACKETS).map(|p| p.data.as_slice()).collect();
    let mut dec = TrackDecoder::new(&enc.codec_params(), &lookahead).unwrap();
    assert_eq!((dec.codec(), dec.sample_rate()), (codec, sample_rate), "{codec:?}");
    let mut out = vec![Vec::new(); usize::from(dec.channels())];
    for p in &packets {
        let block = dec.decode(&p.data).unwrap();
        assert_eq!(block.frames(), p.duration as usize, "{codec:?}: a packet's duration is what it decodes to");
        for (o, c) in out.iter_mut().zip(block.channels) {
            o.extend(c);
        }
    }
    let skip = enc.priming() as usize;
    let out = out.into_iter().map(|c| c[skip..skip + frames].to_vec()).collect();
    (enc, out, dec.layout())
}

/// [`decode_back`] for a codec whose decoder returns every channel: on the speakers encoded.
fn round_trip(codec: CodecId, opts: &EncodeOptions, sample_rate: u32, input: &[Vec<f32>]) -> (PacketEncoder, Vec<Vec<f32>>) {
    let (enc, out, layout) = decode_back(codec, opts, sample_rate, input);
    assert_eq!(layout, enc.layout(), "{codec:?}");
    (enc, out)
}

/// Each decoded channel is its input channel, in place: it matches that channel and no other. The
/// AC-3 encoders low-pass the LFE, which shifts its phase, so the match is by magnitude.
fn assert_in_place(codec: CodecId, input: &[Vec<f32>], output: &[Vec<f32>], least: f64) {
    for (c, got) in output.iter().enumerate() {
        let scores: Vec<f64> = input.iter().map(|want| correlation(want, got).abs()).collect();
        let least = if matches!(codec, CodecId::Ac3 | CodecId::Eac3) && c == 3 { 0.5 } else { least };
        assert!(scores[c] > least, "{codec:?} channel {c}: {scores:?}");
        for (other, &score) in scores.iter().enumerate().filter(|&(o, _)| o != c) {
            assert!(score < scores[c] - 0.3, "{codec:?} channel {c} matches channel {other}: {scores:?}");
        }
    }
}

#[test]
fn aac_round_trips_in_place() {
    let input = tones(48_000, 6, 30_001);
    let opts = EncodeOptions::new(EncodeFormat::Aac).with_bitrate(384_000);
    let (enc, out) = round_trip(CodecId::Aac, &opts, 48_000, &input);
    assert_eq!((enc.priming(), enc.layout(), enc.bitrate()), (1024, ChannelLayout::SURROUND_5_1_SIDE, Some(384_000)));
    assert_eq!(enc.extradata().len(), 2, "an AudioSpecificConfig for channel configuration 6");
    assert_in_place(CodecId::Aac, &input, &out, 0.95);
}

/// 16-bit ALAC is lossless: the input comes back bit for bit.
#[test]
fn alac_round_trips_exactly() {
    let input = tones(44_100, 6, 10_000);
    let opts = EncodeOptions::new(EncodeFormat::Alac).with_bit_depth(BitDepth::Int16).with_layout(ChannelLayout::SURROUND_5_1);
    let (enc, out) = round_trip(CodecId::Alac, &opts, 44_100, &input);
    assert_eq!((enc.priming(), enc.padding(), enc.bit_depth(), enc.layout()), (0, 0, Some(16), ChannelLayout::SURROUND_5_1));
    assert_eq!(out, input);
}

#[test]
fn mp3_round_trips_in_place() {
    for (rate, frame) in [(48_000, 1152), (44_100, 1152), (24_000, 576)] {
        let input = tones(rate, 2, 20_000);
        let opts = EncodeOptions::new(EncodeFormat::Mp3).with_bitrate(192_000);
        let mut enc = PacketEncoder::new(CodecId::Mp3, &opts, rate, 2).unwrap();
        let packets = encode(&mut enc, &input);
        assert!(packets.iter().all(|p| p.duration == frame && p.data[0] == 0xFF), "{rate} Hz");
        let (enc, out) = round_trip(CodecId::Mp3, &opts, rate, &input);
        assert_eq!((enc.priming(), enc.extradata()), (1057, vec![]));
        assert_in_place(CodecId::Mp3, &input, &out, 0.95);
    }
}

#[test]
fn ac3_round_trips_in_place() {
    let input = tones(48_000, 6, 20_000);
    let (enc, out) = round_trip(CodecId::Ac3, &EncodeOptions::new(EncodeFormat::Ac3), 48_000, &input);
    assert_eq!((enc.priming(), enc.layout(), enc.bitrate()), (256, ChannelLayout::SURROUND_5_1_SIDE, Some(448_000)));
    assert_in_place(CodecId::Ac3, &input, &out, 0.9);
}

#[test]
fn eac3_round_trips_in_place() {
    let input = tones(48_000, 6, 20_000);
    let (enc, out) = round_trip(CodecId::Eac3, &EncodeOptions::new(EncodeFormat::Eac3), 48_000, &input);
    assert_eq!((enc.priming(), enc.layout()), (256, ChannelLayout::SURROUND_5_1_SIDE));
    assert_in_place(CodecId::Eac3, &input, &out, 0.9);
}

/// 7.1 E-AC-3 is encoded as 7.1; `TrackDecoder` plays its 5.1 independent substream (see the
/// README), at the same length.
#[test]
fn eac3_7_1_decodes_as_its_5_1_substream() {
    let input = tones(48_000, 8, 20_000);
    let opts = EncodeOptions::new(EncodeFormat::Eac3).with_layout(ChannelLayout::SURROUND_7_1);
    let (enc, out, layout) = decode_back(CodecId::Eac3, &opts, 48_000, &input);
    assert_eq!((enc.layout(), enc.channels(), layout), (ChannelLayout::SURROUND_7_1, 8, ChannelLayout::SURROUND_5_1_SIDE));
    assert!(out.iter().all(|c| c.len() == 20_000));
}

#[test]
fn opus_round_trips_in_place() {
    for channels in [1, 2] {
        let input = tones(48_000, channels, 20_000);
        let opts = EncodeOptions::default().with_bitrate(128_000);
        let (enc, out) = round_trip(CodecId::Opus, &opts, 48_000, &input);
        let head = enc.extradata();
        assert_eq!((&head[..8], head[9], u32::from(u16::from_le_bytes([head[10], head[11]]))), (&b"OpusHead"[..], channels as u8, enc.priming()));
        assert_in_place(CodecId::Opus, &input, &out, 0.9);
    }
}

/// The raw streams `FileEncoder` writes are the packets in a row.
#[test]
fn file_encoder_writes_the_packets() {
    for (format, layout) in [(EncodeFormat::Mp3, ChannelLayout::STEREO), (EncodeFormat::Ac3, ChannelLayout::SURROUND_5_1)] {
        let input = tones(48_000, usize::from(layout.channels()), 12_345);
        let opts = EncodeOptions::new(format).with_layout(layout);
        let mut file = FileEncoder::new(Cursor::new(Vec::new()), &opts, 48_000, layout.channels()).unwrap();
        file.write(&input, 12_345).unwrap();
        let file = file.finish().unwrap().into_inner();
        let mut enc = PacketEncoder::new(format.codec(), &opts, 48_000, layout.channels()).unwrap();
        let packets: Vec<u8> = encode(&mut enc, &input).into_iter().flat_map(|p| p.data).collect();
        assert_eq!(file, packets, "{format:?}");
    }
}

/// What a codec cannot take is refused, and as `EncodeOptions::check` refuses it for the format.
#[test]
fn unsupported_settings_are_refused() {
    let unsupported =
        |codec, opts: &EncodeOptions, rate, channels| matches!(PacketEncoder::new(codec, opts, rate, channels), Err(AudioError::Unsupported(_)));
    let opts = EncodeOptions::default();
    assert!(unsupported(CodecId::Vorbis, &opts, 48_000, 2));
    assert!(unsupported(CodecId::Flac, &opts, 48_000, 2));
    assert!(unsupported(CodecId::Opus, &opts, 44_100, 2));
    assert!(unsupported(CodecId::Opus, &opts, 48_000, 3));
    assert!(unsupported(CodecId::Opus, &opts.clone().with_quality(0.5), 48_000, 2));
    for (codec, format, rate, channels) in [
        (CodecId::Mp3, EncodeFormat::Mp3, 96_000, 2),
        (CodecId::Mp3, EncodeFormat::Mp3, 48_000, 0),
        (CodecId::Mp3, EncodeFormat::Mp3, 48_000, 6),
        (CodecId::Ac3, EncodeFormat::Ac3, 48_000, 8),
        (CodecId::Aac, EncodeFormat::Aac, 44_000, 2),
    ] {
        let opts = EncodeOptions::new(format);
        let (packets, file) = (PacketEncoder::new(codec, &opts, rate, channels).map(drop), opts.check(rate, channels));
        assert_eq!(format!("{packets:?}"), format!("{file:?}"), "{codec:?} {rate} {channels}");
        assert!(packets.is_err());
    }
    assert!(unsupported(CodecId::Alac, &opts.clone().with_bit_depth(BitDepth::Float32), 48_000, 2));
    assert!(unsupported(CodecId::Ac3, &opts.clone().with_layout(ChannelLayout::unordered(2)), 48_000, 2));
    // A layout for another channel count is the caller's mistake.
    assert!(matches!(PacketEncoder::new(CodecId::Aac, &opts.with_layout(ChannelLayout::STEREO), 48_000, 1), Err(AudioError::Encode(_))));
}

/// Blocks that do not fit, and calls after `finish`, are errors that leave the stream as it was.
#[test]
fn misuse_is_an_error() {
    let mut enc = PacketEncoder::new(CodecId::Aac, &EncodeOptions::new(EncodeFormat::Aac), 48_000, 2).unwrap();
    let short = [vec![0.0f32; 10], vec![0.0; 5]];
    assert!(matches!(enc.encode(&short, 10), Err(AudioError::Encode(_))));
    assert!(matches!(enc.encode(&short[..1], 5), Err(AudioError::Encode(_))));
    assert_eq!(enc.frames(), 0);
    assert!(enc.encode(&short, 5).unwrap().is_empty());
    assert_eq!(enc.finish().unwrap().len(), 2, "5 frames after 1024 of priming fit in two packets");
    assert!(matches!(enc.finish(), Err(AudioError::Encode(_))));
    assert!(matches!(enc.encode(&short, 5), Err(AudioError::Encode(_))));
    // An encoder given nothing returns nothing: no packets, no frames and no padding. `priming`
    // is the codec's delay either way, so it equals that of an encoder that was given input.
    for codec in [CodecId::Aac, CodecId::Alac, CodecId::Mp3, CodecId::Ac3, CodecId::Eac3, CodecId::Opus] {
        let mut enc = PacketEncoder::new(codec, &EncodeOptions::default().with_bit_depth(BitDepth::Int16), 48_000, 2).unwrap();
        assert!(enc.finish().unwrap().is_empty(), "{codec:?}");
        assert_eq!((enc.frames(), enc.padding()), (0, 0), "{codec:?}");
        let mut fed = PacketEncoder::new(codec, &EncodeOptions::default().with_bit_depth(BitDepth::Int16), 48_000, 2).unwrap();
        fed.encode(&short, 5).unwrap();
        fed.finish().unwrap();
        assert_eq!(enc.priming(), fed.priming(), "{codec:?}");
    }
}

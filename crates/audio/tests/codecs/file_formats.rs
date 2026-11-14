//! The coded formats of `encode` and `FileEncoder` (AAC and ALAC in M4A; MP3, AC-3 and E-AC-3
//! streams), read back with this crate's decoder and, where ffmpeg is installed, with ffmpeg.

use std::io::{Cursor, Seek, SeekFrom, Write};
use std::process::Command;

use craft_audio::{
    AudioBuffer, AudioError, BitDepth, ChannelLayout, CodecId, Container, ENCODE_EXTENSIONS, EncodeFormat, EncodeOptions, FileEncoder, RateControl,
    decode, encode, encode_format_for,
};

const CODED: [EncodeFormat; 5] = [EncodeFormat::Aac, EncodeFormat::Alac, EncodeFormat::Mp3, EncodeFormat::Ac3, EncodeFormat::Eac3];

/// `channels` channels of `frames` frames, each channel a tone of its own frequency, on the
/// 16-bit grid (so a lossless 16-bit format returns it exactly).
fn tones(sample_rate: u32, channels: usize, frames: usize) -> AudioBuffer {
    let channel = |c: usize| {
        let step = (60.0 + 17.0 * c as f32) * std::f32::consts::TAU / sample_rate as f32;
        (0..frames).map(|i| ((i as f32 * step).sin() * 0.4 * 32768.0).round() / 32768.0).collect()
    };
    AudioBuffer { sample_rate, channels: (0..channels).map(channel).collect() }
}

/// Signal-to-noise ratio in dB of `got` against `want`, over the frames both have.
fn snr(want: &[f32], got: &[f32]) -> f64 {
    let (signal, noise) =
        want.iter().zip(got).fold((0f64, 0f64), |(s, n), (&w, &g)| (s + f64::from(w).powi(2), n + (f64::from(w) - f64::from(g)).powi(2)));
    10.0 * (signal / noise.max(1e-30)).log10()
}

fn is_unsupported<T: std::fmt::Debug>(r: Result<T, AudioError>) -> bool {
    matches!(r, Err(AudioError::Unsupported(_)))
}

/// An AAC file decodes to the frames that were written, in place: the edit list skips the
/// encoder's priming and cuts the padding of the last packet.
#[test]
fn aac_decodes_to_the_frames_written() {
    for frames in [1, 1000, 1024, 1025, 5000, 48_017] {
        let buf = tones(48_000, 2, frames);
        let file = encode(&buf, &EncodeOptions::new(EncodeFormat::Aac)).unwrap();
        let (info, out) = decode(&file, Some("m4a")).unwrap();
        assert_eq!((info.container, info.codec, info.sample_rate, info.damaged), (Container::Mp4, CodecId::Aac, 48_000, 0));
        assert_eq!((info.frames, out.frames(), info.layout), (frames as u64, frames, ChannelLayout::STEREO), "{frames} frames");
        if frames >= 5000 {
            for (want, got) in buf.channels.iter().zip(&out.channels) {
                assert!(snr(want, got) > 30.0, "{frames} frames: {} dB", snr(want, got));
            }
        }
    }
}

/// The M4A track's durations add up to the priming and the audio, and its one edit skips the
/// priming and lasts as long as the audio, in a movie timescale that is the sample rate.
#[test]
fn aac_timing_is_in_the_sample_table_and_the_edit_list() {
    use craft_audio::craft_media::isobmff;
    let file = encode(&tones(44_100, 1, 3000), &EncodeOptions::new(EncodeFormat::Aac)).unwrap();
    let mp4 = isobmff::open(file.as_slice()).unwrap();
    let track = &mp4.tracks[0];
    assert_eq!((mp4.timescale, track.timescale), (44_100, 44_100));
    assert_eq!(track.samples.iter().map(|s| u64::from(s.duration)).sum::<u64>(), 1024 + 3000);
    assert_eq!(track.edits.len(), 1);
    assert_eq!((track.edits[0].media_time, track.edits[0].segment_duration), (1024, 3000));
    // Every packet is in a `roll` group of distance -1, which Apple's decoders need to apply the edit.
    let groups = &track.sample_groups;
    assert_eq!((groups.len(), groups[0].grouping_type), (1, isobmff::FourCc(*b"roll")));
    assert_eq!((groups[0].runs.as_slice(), &groups[0].descriptions), ([(track.samples.len() as u32, 1)].as_slice(), &vec![vec![0xFF, 0xFF]]));
}

/// The sample entry is completed when the file is: AAC's average bit rate, and the largest packet
/// and average bit rate in ALAC's cookie.
#[test]
fn the_sample_entry_describes_the_finished_stream() {
    use craft_audio::craft_media::isobmff::{self, CodecConfig};
    let buf = tones(48_000, 2, 48_000);
    let file = encode(&buf, &EncodeOptions::new(EncodeFormat::Aac).with_bitrate(96_000)).unwrap();
    let mp4 = isobmff::open(file.as_slice()).unwrap();
    let CodecConfig::Aac(aac) = &mp4.tracks[0].entries[0].codec else { panic!("not AAC") };
    assert!((90_000..100_000).contains(&aac.avg_bitrate), "{} bit/s", aac.avg_bitrate);

    let file = encode(&buf, &EncodeOptions::new(EncodeFormat::Alac).with_bit_depth(BitDepth::Int16)).unwrap();
    let mp4 = isobmff::open(file.as_slice()).unwrap();
    let CodecConfig::Alac { cookie } = &mp4.tracks[0].entries[0].codec else { panic!("not ALAC") };
    let config = craft_audio::alac::SpecificConfig::parse(cookie).unwrap();
    let largest = mp4.tracks[0].samples.iter().map(|s| s.size).max().unwrap();
    assert_eq!((config.max_frame_bytes, config.bit_depth, config.frame_length), (largest, 16, 4096));
    assert!(config.avg_bit_rate > 0);
}

/// Dither reaches ALAC's quantizer: a level below one 16-bit step is silence without it.
#[test]
fn alac_dithers_when_asked() {
    let buf = AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.3 / 32768.0; 4096]] };
    let opts = EncodeOptions::new(EncodeFormat::Alac).with_bit_depth(BitDepth::Int16);
    let (_, plain) = decode(&encode(&buf, &opts).unwrap(), Some("m4a")).unwrap();
    let (_, dithered) = decode(&encode(&buf, &opts.with_dither(true)).unwrap(), Some("m4a")).unwrap();
    assert!(plain.channels[0].iter().all(|&s| s == 0.0));
    assert!(dithered.channels[0].iter().any(|&s| s != 0.0));
}

/// ALAC returns the quantized samples exactly, for every channel count, with each channel where
/// its speaker is.
#[test]
fn alac_is_lossless_in_every_layout() {
    for channels in 1..=8u8 {
        let layout = ChannelLayout::from_mask(craft_audio::alac::channel_mask(channels));
        let buf = tones(44_100, usize::from(channels), 10_000);
        let opts = EncodeOptions::new(EncodeFormat::Alac).with_bit_depth(BitDepth::Int16).with_layout(layout);
        let file = encode(&buf, &opts).unwrap();
        let (info, out) = decode(&file, Some("m4a")).unwrap();
        assert_eq!((info.container, info.codec, info.frames, info.layout), (Container::Mp4, CodecId::Alac, 10_000, layout), "{channels} channels");
        assert_eq!(out, buf, "{channels} channels");
    }
    // 24 bits, and a length that is no whole number of packets.
    let buf = tones(96_000, 2, 4096 * 2 + 1);
    let (_, out) = decode(&encode(&buf, &EncodeOptions::new(EncodeFormat::Alac)).unwrap(), Some("m4a")).unwrap();
    assert_eq!(out, buf);
}

/// Channels go to the encoder by speaker. ALAC's 6.1 has the back centre last and AAC's has it
/// before the surround pair; 5.1 with side surrounds is the format's one 5.1.
#[test]
fn channels_reach_their_speakers() {
    // WAV 6.1 with a back pair (L R C LFE Lb Rb Cb), the usual 7-channel layout, into ALAC: the same.
    let buf = tones(48_000, 7, 8000);
    let (info, out) = decode(&encode(&buf, &EncodeOptions::new(EncodeFormat::Alac).with_bit_depth(BitDepth::Int16)).unwrap(), Some("m4a")).unwrap();
    assert_eq!(info.layout, ChannelLayout::from_mask(0x13F));
    assert_eq!(out, buf);
    // 5.1 with side surrounds decodes from ALAC as 5.1 with back surrounds, channel for channel.
    let buf = tones(48_000, 6, 8000);
    let opts = EncodeOptions::new(EncodeFormat::Alac).with_bit_depth(BitDepth::Int16).with_layout(ChannelLayout::SURROUND_5_1_SIDE);
    assert_eq!(decode(&encode(&buf, &opts).unwrap(), Some("m4a")).unwrap().1, buf);

    // AAC: the decoded channel each input channel comes back as. 5.1 decodes in WAV order; the
    // 6.1 and 7.1 streams have no speaker assignment and decode in AAC order (C L R Ls Rs Cs LFE
    // and C Lc Rc L R Ls Rs LFE).
    let wide = ChannelLayout::from_mask(0xFF);
    let cases: [(usize, Option<ChannelLayout>, &[usize]); 3] =
        [(6, None, &[0, 1, 2, 3, 4, 5]), (7, None, &[2, 0, 1, 4, 5, 6, 3]), (8, Some(wide), &[2, 6, 7, 0, 1, 4, 5, 3])];
    for (channels, layout, decoded_from) in cases {
        let buf = tones(48_000, channels, 48_000);
        let mut opts = EncodeOptions::new(EncodeFormat::Aac);
        opts.layout = layout;
        let (info, out) = decode(&encode(&buf, &opts).unwrap(), Some("m4a")).unwrap();
        assert_eq!((usize::from(info.channels), out.frames()), (channels, 48_000));
        for (decoded, &input) in decoded_from.iter().enumerate() {
            let db = snr(&buf.channels[input], &out.channels[decoded]);
            assert!(db > 20.0, "{channels} channels: decoded {decoded} is not input {input} ({db} dB)");
        }
    }
}

/// MP3, AC-3 and E-AC-3 are raw streams: the audio comes back behind the codec's delay.
#[test]
fn raw_streams_decode_behind_the_codec_delay() {
    let buf = tones(48_000, 2, 48_000);
    for (format, codec, container, delay, floor) in [
        (EncodeFormat::Mp3, CodecId::Mp3, Container::MpegAudio, craft_audio::mp3::DELAY as usize, 25.0),
        (EncodeFormat::Ac3, CodecId::Ac3, Container::Ac3, 256, 15.0),
        (EncodeFormat::Eac3, CodecId::Eac3, Container::Ac3, 256, 15.0),
    ] {
        let file = encode(&buf, &EncodeOptions::new(format)).unwrap();
        let (info, out) = decode(&file, Some(format.extension())).unwrap();
        assert_eq!((info.codec, info.container, info.sample_rate, info.layout, info.damaged), (codec, container, 48_000, ChannelLayout::STEREO, 0));
        assert_eq!((format.codec(), format.container()), (codec, container));
        assert!(out.frames() >= 48_000 + delay, "{format:?}: {} frames", out.frames());
        for (want, got) in buf.channels.iter().zip(&out.channels) {
            let db = snr(want, &got[delay..]);
            assert!(db > floor, "{format:?}: {db} dB");
        }
    }
    // 5.1 with back surrounds is AC-3's 3/2 mode with an LFE, channel for channel.
    let buf = tones(48_000, 6, 48_000);
    let (info, out) = decode(&encode(&buf, &EncodeOptions::new(EncodeFormat::Ac3)).unwrap(), Some("ac3")).unwrap();
    assert_eq!(info.layout, ChannelLayout::SURROUND_5_1_SIDE);
    for (c, (want, got)) in buf.channels.iter().zip(&out.channels).enumerate() {
        // The encoder low-pass filters the LFE, which shifts its phase: it is compared by level.
        if c == 3 {
            let rms = |x: &[f32]| (x.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
            let level = rms(&got[4800..]) / rms(want);
            assert!((0.5..1.5).contains(&level), "LFE at {level} of its level");
        } else {
            assert!(snr(want, &got[256..]) > 12.0, "channel {c}: {} dB", snr(want, &got[256..]));
        }
    }
    // 7.1 is an E-AC-3 programme.
    let buf = tones(48_000, 8, 4800);
    assert!(is_unsupported(encode(&buf, &EncodeOptions::new(EncodeFormat::Ac3))));
    // (Read back by ffmpeg in `ffmpeg_reads_the_files`: this crate's file reader plays the 5.1
    // independent substream of a raw 7.1 stream.)
    assert!(!encode(&buf, &EncodeOptions::new(EncodeFormat::Eac3)).unwrap().is_empty());
}

/// Blocks of any sizes give the bytes `encode` gives.
#[test]
fn streaming_writes_the_bytes_of_encode() {
    let buf = tones(44_100, 2, 30_000);
    for format in CODED {
        let opts = EncodeOptions::new(format);
        let whole = encode(&buf, &opts).unwrap();
        let mut enc = FileEncoder::new(Cursor::new(Vec::new()), &opts, 44_100, 2).unwrap();
        assert!(format!("{enc:?}").contains(&format!("{format:?}")));
        let mut at = 0;
        for n in [1, 4095, 4097, 0, 9000, 12_807] {
            let block: Vec<&[f32]> = buf.channels.iter().map(|c| &c[at..]).collect();
            enc.check_room(n as u64).unwrap();
            enc.write(&block, n).unwrap();
            at += n;
        }
        assert_eq!(at, buf.frames());
        assert!(enc.finish().unwrap().into_inner() == whole, "{format:?}");
    }
}

/// A file starts at the sink's position.
#[test]
fn a_file_starts_at_the_sinks_position() {
    let buf = tones(48_000, 1, 6000);
    for format in CODED {
        let opts = EncodeOptions::new(format);
        let mut sink = Cursor::new(b"junk".to_vec());
        sink.seek(SeekFrom::End(0)).unwrap();
        let mut enc = FileEncoder::new(sink, &opts, 48_000, 1).unwrap();
        enc.write(&buf.channels, 6000).unwrap();
        let mut sink = enc.finish().unwrap();
        let end = sink.stream_position().unwrap();
        let bytes = sink.into_inner();
        assert_eq!(end, bytes.len() as u64, "{format:?}");
        assert!(bytes[4..] == encode(&buf, &opts).unwrap(), "{format:?}");
    }
}

/// A file with no audio is still a file of its format.
#[test]
fn no_audio_gives_an_empty_file() {
    let buf = AudioBuffer::new(48_000, 2, 0);
    for format in CODED {
        let file = encode(&buf, &EncodeOptions::new(format)).unwrap();
        if format.container() == Container::Mp4 {
            let mp4 = craft_audio::craft_media::isobmff::open(file.as_slice()).unwrap();
            assert_eq!((mp4.tracks.len(), mp4.tracks[0].samples.len(), mp4.tracks[0].edits.len()), (1, 0, 0), "{format:?}");
        } else {
            assert!(file.is_empty(), "{format:?}");
        }
    }
}

/// The bit rate asked for is the one written, moved to the nearest the format has.
#[test]
fn the_bit_rate_is_followed() {
    let buf = tones(48_000, 2, 48_000);
    let size = |opts: &EncodeOptions| encode(&buf, opts).unwrap().len();
    // MP3 at 48 kHz: 24 bytes per frame and kbit/s, 43 frames for a second and the delay.
    assert_eq!(size(&EncodeOptions::new(EncodeFormat::Mp3)), 43 * 192 * 3);
    assert_eq!(size(&EncodeOptions::new(EncodeFormat::Mp3).with_bitrate(130_000)), 43 * 128 * 3);
    assert_eq!(size(&EncodeOptions::new(EncodeFormat::Mp3).with_bitrate(1)), 43 * 32 * 3);
    assert_eq!(size(&EncodeOptions::new(EncodeFormat::Mp3).with_bitrate(u32::MAX)), 43 * 320 * 3);
    // AC-3: 32 packets of 1536 frames, 4 bytes per packet and kbit/s.
    assert_eq!(size(&EncodeOptions::new(EncodeFormat::Ac3)), 32 * 192 * 4);
    assert_eq!(size(&EncodeOptions::new(EncodeFormat::Ac3).with_bitrate(100_000)), 32 * 96 * 4);
    assert_eq!(size(&EncodeOptions::new(EncodeFormat::Eac3).with_bitrate(100_000)), 32 * 100 * 4);
    // AAC: a second of audio at about the rate asked for, around the 128 kbit/s default.
    let (low, default, high) = (
        size(&EncodeOptions::new(EncodeFormat::Aac).with_bitrate(64_000)),
        size(&EncodeOptions::new(EncodeFormat::Aac)),
        size(&EncodeOptions::new(EncodeFormat::Aac).with_bitrate(256_000)),
    );
    assert!((7000..10_000).contains(&low) && (15_000..18_500).contains(&default) && (30_000..35_000).contains(&high), "{low} {default} {high}");
    // A rate no stream can carry is moved into range, not refused.
    encode(&buf, &EncodeOptions::new(EncodeFormat::Aac).with_bitrate(u32::MAX)).unwrap();
    // The lowest rate still codes the audio: the encoder accepts rates that leave only silence
    // (128 for 128 kbit/s is one), and the writer does not go that low.
    for (sample_rate, channels) in [(8000, 1), (16_000, 2), (44_100, 2), (48_000, 6), (96_000, 2)] {
        let buf = tones(sample_rate, channels, sample_rate as usize);
        let (_, out) = decode(&encode(&buf, &EncodeOptions::new(EncodeFormat::Aac).with_bitrate(128)).unwrap(), Some("m4a")).unwrap();
        let rms = |x: &[f32]| (x.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
        let level = rms(&out.channels[0]) / rms(&buf.channels[0]);
        assert!(level > 0.5, "{sample_rate} Hz, {channels} channels: {level} of the level");
    }
    // Quality is AAC's; ALAC has no rate to control.
    let (small, large) =
        (size(&EncodeOptions::new(EncodeFormat::Aac).with_quality(0.0)), size(&EncodeOptions::new(EncodeFormat::Aac).with_quality(1.0)));
    assert!(small < large, "{small} {large}");
    assert!(matches!(encode(&buf, &EncodeOptions::new(EncodeFormat::Aac).with_quality(f32::NAN)), Err(AudioError::Encode(_))));
    for format in [EncodeFormat::Mp3, EncodeFormat::Ac3, EncodeFormat::Eac3] {
        assert!(is_unsupported(encode(&buf, &EncodeOptions::new(format).with_quality(0.5))), "{format:?}");
    }
    let alac = EncodeOptions::new(EncodeFormat::Alac);
    assert_eq!(size(&alac), size(&alac.clone().with_bitrate(64_000)));
    assert_eq!(alac.rate_control, None);
    assert_eq!(alac.with_quality(0.5).rate_control, Some(RateControl::Quality(0.5)));
}

/// What a format cannot hold is refused before anything is written.
#[test]
fn what_a_format_cannot_hold_is_refused() {
    let refused = |format, sample_rate, channels: u16, layout: Option<ChannelLayout>| {
        let mut opts = EncodeOptions::new(format);
        opts.layout = layout;
        let mut sink = Cursor::new(Vec::new());
        let r = FileEncoder::new(&mut sink, &opts, sample_rate, channels).map(drop);
        assert!(r.is_ok() || sink.into_inner().is_empty(), "{format:?}: bytes written for a refused file");
        assert_eq!(r.is_err(), encode(&AudioBuffer::new(sample_rate, channels, 100), &opts).is_err());
        r
    };
    // Quad has no centre, which AAC's and ALAC's four channels have.
    assert!(is_unsupported(refused(EncodeFormat::Aac, 48_000, 4, None)));
    assert!(is_unsupported(refused(EncodeFormat::Alac, 48_000, 4, None)));
    refused(EncodeFormat::Aac, 48_000, 4, Some(ChannelLayout::from_mask(0x107))).unwrap();
    // WAV 7.1 (backs and sides) is not the 7.1 of AAC and ALAC (a front pair left and right of centre).
    assert!(is_unsupported(refused(EncodeFormat::Aac, 48_000, 8, None)));
    assert!(is_unsupported(refused(EncodeFormat::Alac, 48_000, 8, Some(ChannelLayout::SURROUND_7_1))));
    assert!(is_unsupported(refused(EncodeFormat::Aac, 48_000, 2, Some(ChannelLayout::unordered(2)))));
    for format in CODED {
        assert!(is_unsupported(refused(format, 48_000, 9, None)), "{format:?}");
        assert!(matches!(refused(format, 48_000, 2, Some(ChannelLayout::MONO)), Err(AudioError::Encode(_))), "{format:?}");
        assert!(matches!(refused(format, 0, 2, None), Err(AudioError::Encode(_))), "{format:?}");
        assert!(matches!(refused(format, 48_000, 0, None), Err(AudioError::Encode(_))), "{format:?}");
    }
    assert!(is_unsupported(refused(EncodeFormat::Mp3, 48_000, 3, None)));
    assert!(is_unsupported(refused(EncodeFormat::Ac3, 48_000, 7, None)));
    // Sample rates the codec has not.
    for format in [EncodeFormat::Aac, EncodeFormat::Mp3, EncodeFormat::Ac3, EncodeFormat::Eac3] {
        assert!(matches!(refused(format, 44_000, 2, None), Err(AudioError::Encode(_))), "{format:?}");
    }
    refused(EncodeFormat::Alac, 44_000, 2, None).unwrap();
    let float = EncodeOptions::new(EncodeFormat::Alac).with_bit_depth(BitDepth::Float32);
    assert!(is_unsupported(encode(&AudioBuffer::new(48_000, 2, 100), &float)));
}

/// Samples no encoder should see: NaN is silence, and values past full scale do not fail a
/// write (AC-3, whose encoder refuses them, gets them clamped).
#[test]
fn non_finite_and_loud_samples_are_written() {
    let mut buf = tones(48_000, 2, 6000);
    buf.channels[0][100] = f32::NAN;
    buf.channels[0][200] = f32::INFINITY;
    buf.channels[1][300] = -7.5;
    for format in CODED {
        let file = encode(&buf, &EncodeOptions::new(format)).unwrap();
        let (_, out) = decode(&file, Some(format.extension())).unwrap();
        assert!(out.channels.iter().flatten().all(|s| s.is_finite()), "{format:?}");
    }
}

/// A sink that fails once `room` bytes are written.
struct Full {
    inner: Cursor<Vec<u8>>,
    room: usize,
}

impl Write for Full {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.inner.get_ref().len() + data.len() > self.room {
            return Err(std::io::Error::other("disk full"));
        }
        self.inner.write(data)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Seek for Full {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

/// After a failed write the encoder is unusable: later writes and `finish` fail too.
#[test]
fn a_failed_write_ends_the_encoder() {
    let buf = tones(48_000, 2, 48_000);
    for format in CODED {
        let mut enc = FileEncoder::new(Full { inner: Cursor::new(Vec::new()), room: 2000 }, &EncodeOptions::new(format), 48_000, 2).unwrap();
        assert!(matches!(enc.write(&buf.channels, 48_000), Err(AudioError::Io(_))), "{format:?}");
        assert!(matches!(enc.write(&buf.channels, 1), Err(AudioError::Encode(_))), "{format:?}");
        assert!(matches!(enc.finish(), Err(AudioError::Encode(_))), "{format:?}");
    }
}

#[test]
fn the_formats_are_named() {
    assert_eq!(encode_format_for("m4a"), Some(EncodeFormat::Aac));
    assert_eq!(encode_format_for(".ALAC"), Some(EncodeFormat::Alac));
    assert_eq!(encode_format_for("mp3"), Some(EncodeFormat::Mp3));
    assert_eq!(encode_format_for("ac3"), Some(EncodeFormat::Ac3));
    assert_eq!(encode_format_for("eac3"), Some(EncodeFormat::Eac3));
    assert_eq!(encode_format_for("ec3"), Some(EncodeFormat::Eac3));
    for &format in EncodeFormat::ALL {
        assert!(ENCODE_EXTENSIONS.contains(&format.extension()), "{format:?}");
        let named = encode_format_for(format.extension()).unwrap();
        assert_eq!((named.extension(), named.mime_type()), (format.extension(), format.mime_type()));
        assert_eq!(format.is_lossy(), !matches!(format.codec(), CodecId::Pcm | CodecId::Flac | CodecId::Alac));
    }
    for name in ENCODE_EXTENSIONS {
        assert!(encode_format_for(name).is_some(), "{name}");
    }
    assert_eq!(ENCODE_EXTENSIONS.len(), 12);
    for format in CODED {
        assert!(EncodeFormat::ALL.contains(&format) && format.is_built());
    }
    let features: Vec<_> = EncodeFormat::ALL.iter().map(|f| f.feature()).collect();
    assert_eq!(features, ["pcm-encode", "pcm-encode", "flac-encode", "alac-encode", "aac-encode", "mp3-encode", "ac3-encode", "eac3-encode"]);
}

/// ffmpeg's decode of `file` as interleaved `f32`, split into `channels` channels.
fn ffmpeg_decode(ff: &std::path::Path, file: &[u8], name: &str, channels: usize) -> Vec<Vec<f32>> {
    let path = crate::testkit::fixtures_dir("file_formats").join(name);
    std::fs::write(&path, file).unwrap();
    let out = Command::new(ff).args(["-v", "error", "-i"]).arg(&path).args(["-f", "f32le", "-"]).output().unwrap();
    assert!(out.status.success() && out.stderr.is_empty(), "{name}: {}", String::from_utf8_lossy(&out.stderr));
    let mut planar = vec![Vec::new(); channels];
    for (i, s) in out.stdout.as_chunks::<4>().0.iter().enumerate() {
        planar[i % channels].push(f32::from_le_bytes(*s));
    }
    planar
}

/// ffmpeg reads every format: the M4A files without their priming (it applies the edit list) and,
/// from ffmpeg 9, at the length written; ALAC exactly; the raw streams behind their delay.
#[test]
fn ffmpeg_reads_the_files() {
    let ff = require_ffmpeg!();
    for (channels, layout) in [(1, None), (2, None), (6, None), (8, Some(ChannelLayout::from_mask(0xFF)))] {
        let buf = tones(48_000, channels, 20_000);
        let mut opts = EncodeOptions::new(EncodeFormat::Aac);
        opts.layout = layout;
        let out = ffmpeg_decode(&ff, &encode(&buf, &opts).unwrap(), &format!("aac_{channels}.m4a"), channels);
        // The priming is skipped by every ffmpeg (the channels below line up). The padding of the
        // last packet is cut by ffmpeg 9; ffmpeg 8 plays it, to the end of the 20th packet.
        assert!([20_000, 20_480].contains(&out[0].len()), "AAC, {channels} channels: {} frames", out[0].len());
        if channels <= 6 {
            // ffmpeg also returns 5.1 in WAV order.
            for (c, (want, got)) in buf.channels.iter().zip(&out).enumerate() {
                assert!(snr(want, got) > 20.0, "AAC, {channels} channels, channel {c}: {} dB", snr(want, got));
            }
        }
        let opts = opts.with_format(EncodeFormat::Alac).with_bit_depth(BitDepth::Int16);
        let opts = if channels == 6 { opts.with_layout(ChannelLayout::SURROUND_5_1) } else { opts };
        let out = ffmpeg_decode(&ff, &encode(&buf, &opts).unwrap(), &format!("alac_{channels}.m4a"), channels);
        assert!(out == buf.channels, "ALAC, {channels} channels");
    }
    let buf = tones(44_100, 2, 20_000);
    for (format, delay, floor) in
        [(EncodeFormat::Mp3, craft_audio::mp3::DELAY as usize, 25.0), (EncodeFormat::Ac3, 256, 15.0), (EncodeFormat::Eac3, 256, 15.0)]
    {
        let name = format!("stream.{}", format.extension());
        let out = ffmpeg_decode(&ff, &encode(&buf, &EncodeOptions::new(format)).unwrap(), &name, 2);
        for (want, got) in buf.channels.iter().zip(&out) {
            assert!(
                got.len() >= 20_000 + delay && snr(want, &got[delay..]) > floor,
                "{format:?}: {} frames, {} dB",
                got.len(),
                snr(want, &got[delay..])
            );
        }
    }
    // E-AC-3 7.1: the front channels, which the 5.1 downmix leaves as they are.
    let buf = tones(48_000, 8, 20_000);
    let out = ffmpeg_decode(&ff, &encode(&buf, &EncodeOptions::new(EncodeFormat::Eac3)).unwrap(), "stream_7_1.eac3", 8);
    assert!(out[7].len() >= 20_256);
    for c in 0..3 {
        assert!(snr(&buf.channels[c], &out[c][256..]) > 12.0, "E-AC-3 7.1, channel {c}: {} dB", snr(&buf.channels[c], &out[c][256..]));
    }
}

/// Apple's decoder (macOS `afinfo`; skipped elsewhere) takes the priming and the length of an AAC
/// file from the edit list, which it does only for a track with a `roll` sample group.
#[test]
fn apple_reads_the_aac_priming_and_length() {
    let Some(afinfo) = crate::testkit::oracle::find_tool("afinfo") else { return };
    for (frames, remainder) in [(20_000, 480), (1, 1023), (4096, 0)] {
        let path = crate::testkit::fixtures_dir("file_formats").join(format!("apple_{frames}.m4a"));
        std::fs::write(&path, encode(&tones(48_000, 2, frames), &EncodeOptions::new(EncodeFormat::Aac)).unwrap()).unwrap();
        let out = Command::new(&afinfo).arg(&path).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains(&format!("{frames} valid frames + 1024 priming + {remainder} remainder")), "{frames} frames: {text}");
    }
}

/// What `EncodeFormat` says a format takes is what the writer accepts: for every format, a file
/// can be begun exactly when the sample rate, the channel count and the layout are listed.
#[test]
fn the_format_queries_agree_with_the_writer() {
    // Every rate a format lists, each with its neighbours, and some that none lists.
    let mut rates: Vec<u32> = EncodeFormat::ALL.iter().filter_map(|f| f.sample_rates()).flatten().flat_map(|&r| [r - 1, r, r + 1]).collect();
    rates.extend([44_000, 176_400, 192_000]);
    rates.sort_unstable();
    rates.dedup();
    let masks = [
        0x4, 0x1, 0x3, 0xC, 0x600, 0x7, 0xB, 0x103, 0x33, 0x107, 0x603, 0xF, 0x10B, 0x37, 0x607, 0x10F, 0x60B, 0x3F, 0x60F, 0x13F, 0x70F, 0x63F, 0xFF,
    ];
    let layouts = masks.iter().map(|&mask| ChannelLayout::from_mask(mask)).chain((1..=9).map(ChannelLayout::unordered));
    for layout in layouts {
        for &format in EncodeFormat::ALL {
            // 24 bits is a depth every format has.
            let opts = EncodeOptions::new(format).with_layout(layout);
            for &sample_rate in &rates {
                let listed = format.sample_rates().is_none_or(|r| r.contains(&sample_rate))
                    && layout.channels() <= format.max_channels()
                    && format.supports_layout(&layout);
                let checked = opts.check(sample_rate, layout.channels());
                assert_eq!(checked.is_ok(), listed, "{format:?}, {sample_rate} Hz, {layout:?}: {checked:?}");
                let begun = FileEncoder::new(Cursor::new(Vec::new()), &opts, sample_rate, layout.channels());
                assert_eq!(begun.is_ok(), listed, "{format:?}, {sample_rate} Hz, {layout:?}");
            }
            if let Some(fixed) = format.fixed_layouts(layout.channels()) {
                assert!(fixed.iter().all(|l| format.supports_layout(l) && l.channels() == layout.channels()));
                assert_eq!(format.supports_layout(&layout), fixed.iter().any(|l| same_speakers(l, &layout)), "{format:?}, {layout:?}");
            }
        }
    }
    for &format in EncodeFormat::ALL {
        assert!(!format.supports_layout(&ChannelLayout::unordered(0)));
        assert_eq!(format.supports_quality(), EncodeOptions::new(format).with_quality(0.5).check(48_000, 2).is_ok() && format.is_lossy());
        assert_eq!(format.fixed_layouts(2).is_none(), matches!(format, EncodeFormat::Wav | EncodeFormat::Aiff | EncodeFormat::Flac));
    }
    // The most channels is the most: one more is refused, by the layouts of a format that has
    // a fixed set and by the writer of one that has not.
    let most: Vec<u16> = EncodeFormat::ALL.iter().map(|f| f.max_channels()).collect();
    assert_eq!(most, [32_767, 32_767, 8, 8, 8, 2, 6, 8]);
    for &format in EncodeFormat::ALL {
        let most = format.max_channels();
        match format.fixed_layouts(most) {
            Some(layouts) => assert!(!layouts.is_empty() && format.fixed_layouts(most + 1).unwrap().is_empty(), "{format:?}"),
            None => {
                let opts = EncodeOptions::new(format).with_bit_depth(BitDepth::Int16);
                assert!(opts.check(48_000, most).is_ok() && opts.check(48_000, most + 1).is_err(), "{format:?}");
                assert!(format.supports_layout(&ChannelLayout::unordered(most)) && !format.supports_layout(&ChannelLayout::unordered(most + 1)));
            }
        }
    }
    assert_eq!(EncodeFormat::Ac3.fixed_layouts(3).unwrap().len(), 3);
    assert!(EncodeFormat::Aac.fixed_layouts(9).unwrap().is_empty());
}

/// Whether two layouts name the same speakers, a surround pair being either sides or backs.
fn same_speakers(a: &ChannelLayout, b: &ChannelLayout) -> bool {
    let one_pair = |mask: u32| if mask & 0x630 == 0x30 { mask & !0x30 | 0x600 } else { mask };
    matches!((a.mask(), b.mask()), (Some(a), Some(b)) if one_pair(a) == one_pair(b))
}

/// `bitrate` is the rate the file is written at, for the options and for the encoder they give.
#[test]
fn the_bit_rate_used_can_be_asked_for() {
    let rate = |opts: EncodeOptions, sample_rate, channels| {
        let asked = opts.bitrate(sample_rate, channels).unwrap();
        assert_eq!(FileEncoder::new(Cursor::new(Vec::new()), &opts, sample_rate, channels).unwrap().bitrate(), asked);
        asked
    };
    assert_eq!(rate(EncodeOptions::new(EncodeFormat::Mp3), 48_000, 2), Some(192_000));
    assert_eq!(rate(EncodeOptions::new(EncodeFormat::Mp3).with_bitrate(130_000), 48_000, 2), Some(128_000));
    assert_eq!(rate(EncodeOptions::new(EncodeFormat::Mp3).with_bitrate(u32::MAX), 22_050, 1), Some(160_000));
    assert_eq!(rate(EncodeOptions::new(EncodeFormat::Ac3).with_bitrate(191_999), 48_000, 2), Some(160_000));
    assert_eq!(rate(EncodeOptions::new(EncodeFormat::Ac3), 48_000, 6), Some(448_000));
    assert_eq!(rate(EncodeOptions::new(EncodeFormat::Eac3).with_bitrate(100_999), 48_000, 2), Some(100_000));
    assert_eq!(rate(EncodeOptions::new(EncodeFormat::Aac), 48_000, 2), Some(128_000));
    assert_eq!(rate(EncodeOptions::new(EncodeFormat::Aac).with_bitrate(128), 48_000, 2), Some(24_000));
    assert_eq!(rate(EncodeOptions::new(EncodeFormat::Aac).with_bitrate(u32::MAX), 8000, 1), Some(48_000));
    assert_eq!(rate(EncodeOptions::new(EncodeFormat::Aac).with_quality(0.5), 48_000, 2), None);
    for format in [EncodeFormat::Wav, EncodeFormat::Aiff, EncodeFormat::Flac, EncodeFormat::Alac] {
        assert_eq!(rate(EncodeOptions::new(format).with_bitrate(96_000), 48_000, 2), None, "{format:?}");
    }
    // The size of a second of MP3 is the rate reported.
    let buf = tones(48_000, 2, 48_000);
    let opts = EncodeOptions::new(EncodeFormat::Mp3).with_bitrate(130_000);
    assert_eq!(encode(&buf, &opts).unwrap().len() as u32, 43 * 1152 / 8 * opts.bitrate(48_000, 2).unwrap().unwrap() / 48_000);
    assert!(is_unsupported(EncodeOptions::new(EncodeFormat::Mp3).bitrate(48_000, 6)));
}

#![allow(clippy::unwrap_used)]
use super::*;
use crate::ac3::encoder::{Codec, CodingMode, Config, Encoder};
use craft_media::isobmff::{Brand, CodecConfig, FourCc, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};

#[test]
fn negative_matroska_padding_is_not_silently_treated_as_zero() {
    // BlockGroup containing signed DiscardPadding=-1 ns. Beginning-trim support is not
    // implemented by the native packet reader, so it must not claim an exact timeline.
    let bytes = [0xa0, 0x84, 0x75, 0xa2, 0x81, 0xff];
    assert!(matches!(block_padding(&Src::Slice(&bytes), 0), Err(AudioError::Unsupported(_))));
}

fn file(codec: Codec) -> Vec<u8> {
    mp4_file(codec, &[0..1, 1..3, 3..4, 4..6, 6..8], 0, 0)
}

fn mp4_file(codec: Codec, ranges: &[std::ops::Range<usize>], trim: u32, damaged: usize) -> Vec<u8> {
    mp4_file_with_duration(codec, ranges, trim, damaged, None)
}

fn mp4_file_with_duration(codec: Codec, ranges: &[std::ops::Range<usize>], trim: u32, damaged: usize, declared: Option<u32>) -> Vec<u8> {
    let config = Config { codec, sample_rate: 48_000, bitrate_kbps: 192, mode: CodingMode::Stereo, lfe: false };
    let mut encoder = Encoder::new(config).unwrap();
    let signal: Vec<f32> = (0..1536 * 16).map(|i| ((i as f32 * 0.071).sin()) * 0.2).collect();
    let mut packets = encoder.push(&[&signal, &signal]).unwrap();
    packets.extend(encoder.finish().unwrap().packets);
    let entry = match codec {
        Codec::Ac3 => SampleEntry::audio(FourCc(*b"ac-3"), CodecConfig::Ac3 { dac3: encoder.codec_configuration() }, 2, 48_000.0, 16),
        Codec::Eac3 => SampleEntry::audio(FourCc(*b"ec-3"), CodecConfig::Eac3 { dec3: encoder.codec_configuration() }, 2, 48_000.0, 16),
    };
    let mut writer = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
    let track = writer.add_track(TrackConfig::new(entry, 48_000)).unwrap();
    // Alternate one and two complete syncframes per MP4 sample.
    for (i, range) in ranges.iter().enumerate() {
        let duration = declared.unwrap_or(range.len() as u32 * 1536 - if i + 1 == ranges.len() { trim } else { 0 });
        let mut data: Vec<u8> = packets[range.clone()].iter().flat_map(|p| p.data.iter().copied()).collect();
        if i < damaged {
            data.fill(0);
        }
        writer.write_sample(track, WriteSample { data: &data, duration, composition_offset: 0, is_sync: true }).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn final_multiframe_mp4_packet_keeps_its_declared_trim() {
    for codec in [Codec::Ac3, Codec::Eac3] {
        let bytes = mp4_file(codec, &[0..1, 1..3], 123, 0);
        let (info, full) = crate::decode(&bytes, Some("mp4")).unwrap();
        assert_eq!(info.frames, 3 * 1536 - 123);
        assert_eq!(full.frames(), 3 * 1536 - 123);
    }
}

#[test]
fn native_ac3_metadata_must_be_known_before_timeline_is_cached() {
    let ranges: Vec<_> = (0..10).map(|i| i..i + 1).collect();
    let bytes = mp4_file(Codec::Eac3, &ranges, 0, crate::track::LOOKAHEAD_PACKETS);
    assert!(matches!(super::super::open_mp4(&Src::Slice(&bytes), crate::Container::Mp4, None), Some(Err(AudioError::Malformed(_)))));
    let bytes = mp4_file(Codec::Eac3, &ranges, 0, 1);
    assert!(super::super::open_mp4(&Src::Slice(&bytes), crate::Container::Mp4, None).unwrap().is_ok());
}

fn stripped_matroska(laced: bool, padded: bool) -> Vec<u8> {
    stripped_matroska_with_final_duration(laced, padded, None)
}

fn stripped_matroska_with_final_duration(laced: bool, padded: bool, final_duration: Option<u64>) -> Vec<u8> {
    use craft_media::matroska::{ebml, element_ids::*};
    fn el(id: u32, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        ebml::write_id(&mut out, id);
        ebml::write_size(&mut out, body.len() as u64, 1);
        out.extend_from_slice(body);
        out
    }
    let config = Config { codec: Codec::Eac3, sample_rate: 48_000, bitrate_kbps: 192, mode: CodingMode::Stereo, lfe: false };
    let mut encoder = Encoder::new(config).unwrap();
    let signal: Vec<f32> = (0..1536 * 12).map(|i| ((i as f32 * 0.071).sin()) * 0.2).collect();
    let packets = encoder.push(&[&signal, &signal]).unwrap();
    let mut bytes = el(EBML, &el(DOC_TYPE, b"matroska"));
    ebml::write_id(&mut bytes, SEGMENT);
    ebml::write_unknown_size(&mut bytes, 8);
    bytes.extend(el(INFO, &el(TIMESTAMP_SCALE, &1_000_000u32.to_be_bytes())));
    let compression = el(CONTENT_COMPRESSION, &[el(CONTENT_COMP_ALGO, &[3]), el(CONTENT_COMP_SETTINGS, &[0x0b, 0x77])].concat());
    let entry = [
        el(TRACK_NUMBER, &[1]),
        el(TRACK_UID, &[1]),
        el(TRACK_TYPE, &[2]),
        el(CODEC_ID, b"A_EAC3"),
        el(CODEC_DELAY, &5_333_333u64.to_be_bytes()),
        el(AUDIO, &[el(SAMPLING_FREQUENCY, &48_000f64.to_be_bytes()), el(CHANNELS, &[2])].concat()),
        el(CONTENT_ENCODINGS, &el(CONTENT_ENCODING, &compression)),
    ]
    .concat();
    bytes.extend(el(TRACKS, &el(TRACK_ENTRY, &entry)));
    let mut elapsed = 0u64;
    for (i, group_packets) in packets[..11].chunks(if laced { 3 } else { 1 }).enumerate() {
        let mut block = vec![0x81, 0, 0, if group_packets.len() > 1 { 0x84 } else { 0x80 }];
        if group_packets.len() > 1 {
            block.push((group_packets.len() - 1) as u8);
        }
        for packet in group_packets {
            block.extend_from_slice(&packet.data[2..]);
        }
        let mut group = el(BLOCK, &block);
        if i + 1 == 11usize.div_ceil(if laced { 3 } else { 1 })
            && let Some(duration) = final_duration
        {
            group.extend(el(BLOCK_DURATION, &duration.to_be_bytes()));
        }
        if padded && i == 2 {
            group.extend(el(DISCARD_PADDING, &5_333_333i64.to_be_bytes()));
        }
        let cluster = [el(TIMESTAMP, &(elapsed * 1000 / 48_000).to_be_bytes()), el(BLOCK_GROUP, &group)].concat();
        bytes.extend(el(CLUSTER, &cluster));
        elapsed += group_packets.len() as u64 * 1536 - if padded && i == 2 { 256 } else { 0 };
    }
    bytes
}

#[test]
fn matroska_header_stripping_and_discard_padding_keep_exact_seek() {
    let bytes = stripped_matroska(false, true);
    let (_, full) = crate::decode(&bytes, Some("mkv")).unwrap();
    assert_eq!(full.frames(), 11 * 1536 - 256 - 256);
    let mut reader = super::super::open_mkv(&Src::Slice(&bytes), crate::Container::Matroska, None).unwrap().unwrap();
    reader.seek(11_000).unwrap();
    let block = reader.next_block().unwrap().unwrap();
    for (channel, expected) in block.channels.iter().zip(&full.channels) {
        assert!(channel.iter().zip(&expected[11_000..]).all(|(a, b)| (a - b).abs() < 1e-4));
    }
}

#[test]
fn identical_lace_timestamps_do_not_overshoot_exact_seek() {
    let bytes = stripped_matroska(true, false);
    let (_, full) = crate::decode(&bytes, Some("mkv")).unwrap();
    let mut reader = super::super::open_mkv(&Src::Slice(&bytes), crate::Container::Matroska, None).unwrap().unwrap();
    reader.seek(7000).unwrap();
    let block = reader.next_block().unwrap().unwrap();
    for (channel, expected) in block.channels.iter().zip(&full.channels) {
        assert!(!channel.is_empty());
        assert!(channel.iter().zip(&expected[7000..]).all(|(a, b)| (a - b).abs() < 1e-4));
    }
    let mut demux = craft_media::matroska::Demuxer::from_slice(&bytes).unwrap();
    for i in [0, 1, 2, 5, 7, 10] {
        let expected = demux.read_sample(0, i).unwrap();
        assert_eq!(demux.seek_sample(0, i).unwrap().sample, Some(i));
        assert_eq!(demux.next_packet().unwrap().unwrap().data, expected);
    }
    assert!(demux.seek_sample(0, usize::MAX).is_err());
    assert!(demux.seek_sample(usize::MAX, 0).is_err());
    let unsupported = stripped_matroska(true, true);
    assert!(matches!(super::super::open_mkv(&Src::Slice(&unsupported), crate::Container::Matroska, None), Some(Err(AudioError::Unsupported(_)))));
}

#[test]
fn variable_syncframe_counts_keep_matroska_seek_aligned() {
    use craft_media::matroska::{LacingMode, MkvWriter, MuxOptions, TrackKind, TrackSpec};
    for codec in [Codec::Ac3, Codec::Eac3] {
        for rate in [32_000, 44_100, 48_000] {
            let config = Config { codec, sample_rate: rate, bitrate_kbps: 192, mode: CodingMode::Stereo, lfe: false };
            let mut encoder = Encoder::new(config).unwrap();
            let signal: Vec<f32> = (0..1536 * 12).map(|i| ((i as f32 * 0.071).sin()) * 0.2).collect();
            let packets = encoder.push(&[&signal, &signal]).unwrap();
            let codec_id = if codec == Codec::Ac3 { "A_AC3" } else { "A_EAC3" };
            let mut spec = TrackSpec::new(TrackKind::Audio, codec_id);
            spec.audio = Some((f64::from(rate), 2, None));
            let options = MuxOptions { lacing: LacingMode::None, ..MuxOptions::default() };
            let mut writer = MkvWriter::new(std::io::Cursor::new(Vec::new()), vec![spec], options).unwrap();
            // Millisecond container ticks are customary; packet sizes alternate rather
            // than assuming a fixed number of decoded frames per container block.
            for range in [0..1, 1..3, 3..4, 4..6, 6..8, 8..9, 9..11] {
                let pts = range.start as i64 * 1536 * 1_000_000_000 / i64::from(rate);
                let duration = range.len() as u64 * 1536 * 1_000_000_000 / u64::from(rate);
                let data: Vec<u8> = packets[range].iter().flat_map(|p| p.data.iter().copied()).collect();
                writer.write_frame(0, pts, true, &data, Some(duration)).unwrap();
            }
            let bytes = writer.finish().unwrap().into_inner();
            let (_, full) = crate::decode(&bytes, Some("mkv")).unwrap();
            assert_eq!(full.frames(), 11 * 1536, "{codec:?} at {rate} Hz must not be trimmed by rounded block duration");
            let mut reader = super::super::open_mkv(&Src::Slice(&bytes), crate::Container::Matroska, None).unwrap().unwrap();
            for at in [7000, 11_000, 16_000] {
                reader.seek(at).unwrap();
                let block = reader.next_block().unwrap().unwrap();
                for (channel, expected) in block.channels.iter().zip(&full.channels) {
                    assert!(!channel.is_empty());
                    let error = channel.iter().zip(&expected[at as usize..]).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
                    assert!(error < 1e-4, "{codec:?}, {rate} Hz, seek {at}: max error {error}");
                }
            }
        }
    }
}

#[test]
fn variable_syncframe_counts_keep_full_mp4_duration_and_seek() {
    for codec in [Codec::Ac3, Codec::Eac3] {
        let bytes = file(codec);
        let (info, full) = crate::decode(&bytes, Some("mp4")).unwrap();
        assert_eq!(info.frames, 1536 * 8);
        assert_eq!(full.frames(), 1536 * 8);
        let src = Src::Slice(&bytes);
        let mut reader = super::super::open_mp4(&src, crate::Container::Mp4, None).unwrap().unwrap();
        let at = 7000;
        reader.seek(at).unwrap();
        let block = reader.next_block().unwrap().unwrap();
        for (channel, expected) in block.channels.iter().zip(&full.channels) {
            assert!(!channel.is_empty());
            assert!(channel.iter().zip(&expected[at as usize..]).all(|(a, b)| (a - b).abs() < 1e-4));
        }
    }
}

#[test]
fn underreported_mp4_durations_are_not_mistaken_for_a_final_trim() {
    for codec in [Codec::Ac3, Codec::Eac3] {
        let bytes = mp4_file_with_duration(codec, &[0..1, 1..3, 3..5, 5..7], 0, 0, Some(1536));
        let (info, full) = crate::decode(&bytes, Some("mp4")).unwrap();
        assert_eq!(info.frames, 7 * 1536);
        assert_eq!(full.frames(), 7 * 1536);
    }
}

#[test]
fn final_matroska_block_duration_retains_a_meaningful_trim() {
    use craft_media::matroska::{LacingMode, MkvWriter, MuxOptions, TrackKind, TrackSpec};
    for codec in [Codec::Ac3, Codec::Eac3] {
        for rate in [32_000, 44_100, 48_000] {
            let mut encoder = Encoder::new(Config { codec, sample_rate: rate, bitrate_kbps: 192, mode: CodingMode::Stereo, lfe: false }).unwrap();
            let signal: Vec<f32> = (0..1536 * 4).map(|i| (i as f32 * 0.071).sin() * 0.2).collect();
            let packets = encoder.push(&[&signal, &signal]).unwrap();
            let mut spec = TrackSpec::new(TrackKind::Audio, if codec == Codec::Ac3 { "A_AC3" } else { "A_EAC3" });
            spec.audio = Some((f64::from(rate), 2, None));
            // Fine container ticks distinguish a real 512-sample trim from rounding.
            let options = MuxOptions { timestamp_scale: 1, lacing: LacingMode::None, ..MuxOptions::default() };
            let mut writer = MkvWriter::new(std::io::Cursor::new(Vec::new()), vec![spec], options).unwrap();
            for (i, packet) in packets.iter().take(3).enumerate() {
                let pts = i as i64 * 1536 * 1_000_000_000 / i64::from(rate);
                let frames = if i == 2 { 1024 } else { 1536 };
                let duration = frames * 1_000_000_000 / u64::from(rate);
                writer.write_frame(0, pts, true, &packet.data, Some(duration)).unwrap();
            }
            let bytes = writer.finish().unwrap().into_inner();
            let (info, full) = crate::decode(&bytes, Some("mkv")).unwrap();
            assert_eq!(info.frames, 2 * 1536 + 1024, "{codec:?} at {rate} Hz");
            assert_eq!(full.frames(), 2 * 1536 + 1024, "{codec:?} at {rate} Hz");
            let mut reader = super::super::open_mkv(&Src::Slice(&bytes), crate::Container::Matroska, None).unwrap().unwrap();
            reader.seek(4000).unwrap();
            let end = reader.next_block().unwrap().unwrap();
            assert_eq!(end.channels[0].len(), 96);
            assert!(reader.next_block().unwrap().is_none());
        }
    }
}

#[test]
fn damaged_headers_cannot_bypass_later_matroska_padding_validation() {
    for laced in [false, true] {
        let mut bytes = stripped_matroska(laced, true);
        let file = craft_media::matroska::open(Src::Slice(&bytes)).unwrap();
        let first = file.tracks[0].samples[0];
        bytes[first.offset as usize..first.offset as usize + first.size as usize].fill(0xff);
        if !laced {
            let padding = bytes.windows(3).position(|w| w == [0x75, 0xa2, 0x88]).unwrap();
            bytes[padding + 3..padding + 11].copy_from_slice(&(-1i64).to_be_bytes());
        }
        assert!(
            matches!(super::super::open_mkv(&Src::Slice(&bytes), crate::Container::Matroska, None), Some(Err(AudioError::Unsupported(_)))),
            "damaged earlier header must not hide {} padding",
            if laced { "laced" } else { "negative" }
        );
    }
}

#[test]
fn final_matroska_trim_keeps_interior_padding_and_codec_delay() {
    // 21 millisecond ticks declare exactly 1008 samples at 48 kHz. Earlier 256-sample
    // padding and 256-sample CodecDelay must each still be removed once, not forgotten
    // or subtracted twice when the final block is shortened.
    let bytes = stripped_matroska_with_final_duration(false, true, Some(21));
    let (info, full) = crate::decode(&bytes, Some("mkv")).unwrap();
    let expected = 10 * 1536 + 1008 - 256 - 256;
    assert_eq!(info.frames, expected);
    assert_eq!(full.frames() as u64, expected);
    let mut reader = super::super::open_mkv(&Src::Slice(&bytes), crate::Container::Matroska, None).unwrap().unwrap();
    reader.seek(expected - 100).unwrap();
    let end = reader.next_block().unwrap().unwrap();
    assert_eq!(end.channels[0].len(), 100);
    assert!(reader.next_block().unwrap().is_none());
}

#[test]
fn damaged_header_still_allows_supported_matroska_padding_fallback() {
    let mut bytes = stripped_matroska(false, true);
    let file = craft_media::matroska::open(Src::Slice(&bytes)).unwrap();
    let first = file.tracks[0].samples[0];
    bytes[first.offset as usize..first.offset as usize + first.size as usize].fill(0xff);
    let src = Src::Slice(&bytes);
    let file = craft_media::matroska::open(src.clone()).unwrap();
    assert!(mkv_index(&src, &file.tracks[0], 48_000).unwrap().is_none());
    assert!(super::super::open_mkv(&src, crate::Container::Matroska, None).unwrap().is_ok());
}

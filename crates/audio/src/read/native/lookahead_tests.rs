//! Original AC-3 fixtures with source faults isolated to packet lookahead reads.
#![allow(clippy::unwrap_used)]

use super::*;
use crate::ac3::encoder::{Codec, CodingMode, Config, Encoder};
use craft_media::ByteSource;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn fixture(codec: Codec, container: Container) -> (Vec<u8>, Vec<(u64, usize)>) {
    let config = Config { codec, sample_rate: 48_000, bitrate_kbps: 192, mode: CodingMode::Stereo, lfe: false };
    let mut encoder = Encoder::new(config).unwrap();
    let signal = vec![0.0; 1536 * 3];
    let packets = encoder.push(&[&signal, &signal]).unwrap();
    let bytes = if container == Container::Mp4 {
        use isobmff::{Brand, CodecConfig, FourCc, Mp4Writer, SampleEntry, TrackConfig, WriteSample, WriterOptions};
        let entry = match codec {
            Codec::Ac3 => SampleEntry::audio(FourCc(*b"ac-3"), CodecConfig::Ac3 { dac3: encoder.codec_configuration() }, 2, 48_000.0, 16),
            Codec::Eac3 => SampleEntry::audio(FourCc(*b"ec-3"), CodecConfig::Eac3 { dec3: encoder.codec_configuration() }, 2, 48_000.0, 16),
        };
        let mut writer = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4)).unwrap();
        let track = writer.add_track(TrackConfig::new(entry, 48_000)).unwrap();
        for packet in &packets {
            writer.write_sample(track, WriteSample { data: &packet.data, duration: 1536, composition_offset: 0, is_sync: true }).unwrap();
        }
        writer.finish().unwrap().into_inner()
    } else {
        use matroska::{LacingMode, MkvWriter, MuxOptions, TrackKind, TrackSpec};
        let mut track = TrackSpec::new(TrackKind::Audio, if codec == Codec::Ac3 { "A_AC3" } else { "A_EAC3" });
        track.audio = Some((48_000.0, 2, None));
        let options = MuxOptions { lacing: LacingMode::None, ..MuxOptions::default() };
        let mut writer = MkvWriter::new(std::io::Cursor::new(Vec::new()), vec![track], options).unwrap();
        for (i, packet) in packets.iter().enumerate() {
            writer.write_frame(0, i as i64 * 32_000_000, true, &packet.data, Some(32_000_000)).unwrap();
        }
        writer.finish().unwrap().into_inner()
    };
    let locations = if container == Container::Mp4 {
        isobmff::open(bytes.as_slice()).unwrap().tracks[0].samples.iter().map(|s| (s.offset, s.size as usize)).collect()
    } else {
        matroska::Demuxer::from_slice(&bytes).unwrap().file().tracks[0].samples.iter().map(|s| (s.offset, s.size as usize)).collect()
    };
    (bytes, locations)
}

struct FailingPackets {
    bytes: Vec<u8>,
    locations: Vec<(u64, usize)>,
    failures: Arc<AtomicUsize>,
}

impl ByteSource for FailingPackets {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<()> {
        if self.locations.contains(&(offset, buffer.len())) {
            self.failures.fetch_add(1, Ordering::Relaxed);
            return Err(std::io::Error::other("packet lookahead source failure"));
        }
        self.bytes.as_slice().read_at(offset, buffer)
    }
}

fn open(src: &Src<'static>, container: Container) -> Option<Result<Native<'static>>> {
    if container == Container::Mp4 { open_mp4(src, container, None) } else { open_mkv(src, container, None) }
}

fn assert_failed_lookahead_preserves_io(container: Container) {
    for codec in [Codec::Ac3, Codec::Eac3] {
        let (bytes, locations) = fixture(codec, container);
        let expected_reads = locations.len();
        let failures = Arc::new(AtomicUsize::new(0));
        let source = Src::Shared(Arc::new(FailingPackets { bytes, locations, failures: failures.clone() }));
        let result = open(&source, container);
        assert_eq!(failures.load(Ordering::Relaxed), expected_reads, "{codec:?} {container:?}: lookahead must run");
        assert!(matches!(result, Some(Err(AudioError::Io(_)))), "{codec:?} {container:?}: {:?}", result.map(|r| r.err()));
    }
}

#[test]
fn failed_ac3_mp4_lookahead_preserves_source_io_error() {
    assert_failed_lookahead_preserves_io(Container::Mp4);
}

#[test]
fn failed_ac3_matroska_lookahead_preserves_source_io_error() {
    assert_failed_lookahead_preserves_io(Container::Matroska);
}

#[test]
fn later_ac3_primary_can_establish_format_after_a_failed_lookahead_read() {
    for codec in [Codec::Ac3, Codec::Eac3] {
        for container in [Container::Mp4, Container::Matroska] {
            let (bytes, mut locations) = fixture(codec, container);
            locations.truncate(1);
            let failures = Arc::new(AtomicUsize::new(0));
            let source = Src::Shared(Arc::new(FailingPackets { bytes, locations, failures: failures.clone() }));
            let result = open(&source, container).unwrap();
            assert!(failures.load(Ordering::Relaxed) > 0, "fault must actually run");
            assert!(result.is_ok(), "{codec:?} {container:?}: {:?}", result.err());
        }
    }
}

#[test]
fn ac3_probe_resource_errors_are_preserved_without_changing_other_codec_fallback() {
    let dependents = [0x0b, 0x77, 0x40, 3, 0x34, 0x80, 0, 0].repeat(crate::track::MAX_AC3_PACKET_SYNCFRAMES + 1);
    let result = track_lookahead(&CodecParams::new(CodecId::Eac3), std::iter::once(Ok(dependents)));
    assert!(matches!(result, Some(Err(AudioError::TooLarge(_)))));
    let io_error = AudioError::Io(std::io::Error::other("unavailable non-native packet"));
    assert!(track_lookahead(&CodecParams::new(CodecId::Flac), std::iter::once(Err(io_error))).is_none());
}

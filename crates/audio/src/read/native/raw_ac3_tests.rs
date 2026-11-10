//! Raw AC-3 readers must establish metadata before exposing cached information or seeking.
#![allow(clippy::unwrap_used)]

use super::*;
use crate::ac3::encoder::{Codec, CodingMode, Config, Encoder};
use crate::{DecodeOptions, Decoder};
use craft_media::ByteSource;

fn packets(codec: Codec) -> Vec<Vec<u8>> {
    let mut encoder = Encoder::new(Config { codec, sample_rate: 48_000, bitrate_kbps: 192, mode: CodingMode::Stereo, lfe: false }).unwrap();
    let signal: Vec<f32> = (0..1536 * 12).map(|i| 0.2 * (i as f32 * 0.071).sin()).collect();
    encoder.push(&[&signal, &signal]).unwrap().into_iter().map(|p| p.data).collect()
}

fn damage_crc(packet: &mut [u8]) {
    let end = packet.len();
    packet[end - 1] ^= 1;
}

#[test]
fn raw_ac3_later_valid_frame_establishes_info_decode_and_seek() {
    for codec in [Codec::Ac3, Codec::Eac3] {
        let mut packets = packets(codec);
        damage_crc(&mut packets[0]);
        let count = packets.len();
        let bytes = packets.concat();
        let mut reader = Decoder::from_bytes(&bytes, Some("ac3"), &DecodeOptions::default()).unwrap();
        assert_eq!((reader.info().sample_rate, reader.info().channels), (48_000, 2), "{codec:?}");
        assert!(reader.length_known());
        assert_eq!(reader.info().frames, count as u64 * 1536);
        let (info, full) = crate::decode(&bytes, Some("ac3")).unwrap();
        assert_eq!((info.sample_rate, info.channels, full.frames()), (48_000, 2, count * 1536));
        assert_eq!(reader.seek(7000).unwrap(), 7000);
        let block = reader.next_block().unwrap().unwrap();
        assert_eq!(block.sample_rate, 48_000);
        for (channel, expected) in block.channels.iter().zip(&full.channels) {
            assert!(!channel.is_empty());
            assert!(channel.iter().zip(&expected[7000..]).all(|(a, b)| (a - b).abs() < 1e-4));
        }
    }
}

#[test]
fn raw_ac3_requires_a_valid_primary_within_bounded_lookahead() {
    for codec in [Codec::Ac3, Codec::Eac3] {
        let mut packets = packets(codec);
        for packet in packets.iter_mut().take(crate::track::LOOKAHEAD_PACKETS) {
            damage_crc(packet);
        }
        let bytes = packets.concat();
        let result = Decoder::from_bytes(&bytes, Some("ac3"), &DecodeOptions::default());
        assert!(matches!(result, Err(AudioError::Malformed(_))), "{codec:?}: {:?}", result);
    }
}

#[test]
fn raw_ac3_expanded_lookahead_preserves_source_io_errors() {
    struct Failing {
        bytes: Vec<u8>,
        second_offset: u64,
        packet_size: usize,
    }
    impl ByteSource for Failing {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }
        fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<()> {
            if offset == self.second_offset && buffer.len() == self.packet_size {
                return Err(std::io::Error::other("raw lookahead source failure"));
            }
            self.bytes.as_slice().read_at(offset, buffer)
        }
    }
    for codec in [Codec::Ac3, Codec::Eac3] {
        let packets = packets(codec);
        let source = Failing { bytes: packets.concat(), second_offset: packets[0].len() as u64, packet_size: packets[1].len() };
        let result = Decoder::open(source, Some("ac3"), &DecodeOptions::default());
        assert!(matches!(result, Err(AudioError::Io(_))), "{codec:?}: {:?}", result);
    }
}

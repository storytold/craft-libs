//! Source failures while looking past metadata must not become successful format probes.

#![allow(clippy::unwrap_used)]

use crate::ac3::encoder::{Codec, CodingMode, Config, Encoder};
use crate::{AudioError, DecodeOptions, Decoder};
use craft_media::ByteSource;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn tagged_ac3() -> (Vec<u8>, u64) {
    let mut encoder =
        Encoder::new(Config { codec: Codec::Ac3, sample_rate: 48_000, bitrate_kbps: 192, mode: CodingMode::Stereo, lfe: false }).unwrap();
    let silence = vec![0.0; 1536];
    let mut packets = encoder.push(&[&silence, &silence]).unwrap();
    packets.extend(encoder.finish().unwrap().packets);
    let size = 100_000u32;
    let mut bytes = b"ID3\x03\x00\x00".to_vec();
    bytes.extend([size >> 21, size >> 14, size >> 7, size].map(|v| (v & 0x7f) as u8));
    bytes.resize(10 + size as usize, 0);
    let start = bytes.len() as u64;
    // Keep the post-tag format probe at its full length rather than EOF-shortening it.
    bytes.extend(packets[0].data.repeat(128));
    (bytes, start)
}

struct FailProbe {
    bytes: Vec<u8>,
    offset: u64,
    length: usize,
    failed: Arc<AtomicBool>,
}

impl ByteSource for FailProbe {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> std::io::Result<()> {
        if offset == self.offset && buffer.len() == self.length && !self.failed.swap(true, Ordering::Relaxed) {
            return Err(std::io::Error::other("ID3 probe source failure"));
        }
        self.bytes.as_slice().read_at(offset, buffer)
    }
}

fn assert_probe_io_failure(length: usize) {
    let (bytes, offset) = tagged_ac3();
    let (info, audio) = crate::decode(&bytes, Some("ac3")).unwrap();
    assert_eq!(info.sample_rate, 48_000);
    assert_eq!(audio.frames(), 128 * 1536);
    let failed = Arc::new(AtomicBool::new(false));
    let source = FailProbe { bytes, offset, length, failed: failed.clone() };
    let result = Decoder::open(source, Some("ac3"), &DecodeOptions::default());
    assert!(failed.load(Ordering::Relaxed), "the {length}-byte probe must actually run");
    assert!(matches!(result, Err(AudioError::Io(_))), "{length}-byte probe must preserve I/O errors: {:?}", result.err());
}

#[test]
fn ac3_id3_header_probe_propagates_io_failure() {
    assert_probe_io_failure(10);
}

#[test]
fn ac3_id3_post_tag_probe_propagates_io_failure() {
    assert_probe_io_failure(64 * 1024);
}

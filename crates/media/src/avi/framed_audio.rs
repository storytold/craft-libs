//! AVI audio chunk boundaries need not match MPEG audio or AC-3 frame boundaries.

use std::io;

use super::{ByteSource, Chunk, Error, Result};
use crate::mpegts::{Codec, FrameInfo, frame_bytes, frame_info};

// Indexed chunks can refer to the same file bytes repeatedly. Bound even
// the concatenated length, before making any reads through that index.
const MAX_SCAN_BYTES: u64 = 256 << 20;
const WINDOW: usize = 16 << 10;
const HEADER: usize = 8;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Position {
    chunk: usize,
    offset: u32,
}

impl Position {
    fn take(&mut self, chunks: &[Chunk], count: usize) -> Option<(u64, usize)> {
        while let Some(chunk) = chunks.get(self.chunk) {
            if self.offset < chunk.size {
                let start = chunk.offset.checked_add(u64::from(self.offset))?;
                let len = count.min((chunk.size - self.offset) as usize);
                self.offset += len as u32;
                return Some((start, len));
            }
            self.chunk += 1;
            self.offset = 0;
        }
        None
    }

    fn skip(&mut self, chunks: &[Chunk], mut count: usize) -> Result<()> {
        while count != 0 {
            let (_, len) = self.take(chunks, count).ok_or_else(short_range)?;
            count -= len;
        }
        Ok(())
    }

    fn read(mut self, source: &(impl ByteSource + ?Sized), chunks: &[Chunk], mut dst: &mut [u8]) -> Result<()> {
        while !dst.is_empty() {
            let (offset, len) = self.take(chunks, dst.len()).ok_or_else(short_range)?;
            let (current, rest) = dst.split_at_mut(len);
            source.read_at(offset, current)?;
            dst = rest;
        }
        Ok(())
    }
}

fn short_range() -> Error {
    Error::Io(io::ErrorKind::UnexpectedEof.into())
}

/// A complete compressed audio packet in the logical concatenation of an AVI stream's chunks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramedAudioPacket {
    start: Position,
    len: usize,
    /// Sample information from the frame header, when it was readable.
    pub info: Option<FrameInfo>,
}

impl FramedAudioPacket {
    /// Read exactly the packet bytes using the same chunk table used to find the packet.
    pub fn read(self, source: &(impl ByteSource + ?Sized), chunks: &[Chunk]) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(self.len).map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
        bytes.resize(self.len, 0);
        self.start.read(source, chunks, &mut bytes)?;
        Ok(bytes)
    }
}

/// Walk MPEG audio or AC-3 packets across the chunks of one AVI audio stream.
///
/// The iterator uses a fixed-size window and leaves packet index storage to
/// the caller. It skips unrecognized bytes, omits incomplete frames and
/// stops after reporting a source read error. The scan admits at most four
/// times the source size in logical chunks, with a 256 MiB ceiling.
pub fn framed_audio_packets<'a>(
    source: &'a (impl ByteSource + ?Sized),
    chunks: &'a [Chunk],
    codec: &'a Codec,
) -> Result<impl Iterator<Item = Result<FramedAudioPacket>> + 'a> {
    if !matches!(codec, Codec::MpegAudio | Codec::Ac3 | Codec::Eac3) {
        return Err(Error::Invalid("unsupported framed AVI audio codec".into()));
    }
    let budget = source.len().saturating_mul(4).min(MAX_SCAN_BYTES) as usize;
    let total = chunks
        .iter()
        .try_fold(0usize, |sum, chunk| sum.checked_add(chunk.size as usize).filter(|&n| n <= budget))
        .ok_or_else(|| Error::Invalid("AVI framed audio exceeds the scan budget".into()))?;
    let mut window = [0; WINDOW];
    let (mut base, mut len, mut pos) = (0usize, 0usize, 0usize);
    let mut cursor = Position::default();

    Ok(std::iter::from_fn(move || {
        while pos < total {
            let want = HEADER.min(total - pos);
            let have = (base + len).saturating_sub(pos);
            if have < want {
                // Retain a partial header before loading the next window, so
                // no chunk bytes are read twice during the scan.
                window.copy_within(len - have..len, 0);
                base = pos;
                len = WINDOW.min(total - pos);
                let mut reader = cursor;
                if let Err(error) = reader.skip(chunks, have).and_then(|()| reader.read(source, chunks, &mut window[have..len])) {
                    pos = total;
                    return Some(Err(error));
                }
            }
            let head = &window[pos - base..pos - base + want];
            let packet = frame_bytes(codec, head).filter(|&size| size > 0 && size <= total - pos).map(|size| FramedAudioPacket {
                start: cursor,
                len: size,
                info: frame_info(codec, head),
            });
            let advance = packet.map_or(1, |packet| packet.len);
            if let Err(error) = cursor.skip(chunks, advance) {
                pos = total;
                return Some(Err(error));
            }
            pos += advance;
            if let Some(packet) = packet {
                return Some(Ok(packet));
            }
        }
        None
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Source {
        bytes: Vec<u8>,
        read: Cell<usize>,
    }

    impl ByteSource for Source {
        fn len(&self) -> u64 {
            self.bytes.len() as u64
        }

        fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
            self.read.set(self.read.get() + out.len());
            self.bytes.read_at(offset, out)
        }
    }

    fn interleave_padding(logical: &[u8], lengths: &[usize]) -> (Source, Vec<Chunk>) {
        let mut chunks = Vec::new();
        let mut bytes = Vec::new();
        let mut pos = 0;
        for length in lengths.iter().copied().chain(std::iter::once(logical.len())) {
            let count = length.min(logical.len() - pos);
            chunks.push(Chunk { offset: bytes.len() as u64, size: count as u32, key: true });
            bytes.extend_from_slice(&logical[pos..pos + count]);
            bytes.push(0xff); // Bytes between the AVI chunks must not enter the stream.
            pos += count;
        }
        (Source { bytes, read: Cell::new(0) }, chunks)
    }

    #[test]
    fn split_mpeg_header_and_packet_match_the_original_bytes() {
        // Original frame framing, with 128 kb/s Layer III at 48 kHz.
        let mut packet = vec![0x12; 384];
        packet[..4].copy_from_slice(&[0xff, 0xfb, 0x94, 0xc0]);
        let mut logical = vec![0x22; WINDOW - 3];
        logical.extend_from_slice(&packet);
        logical.extend_from_slice(&packet);
        logical.extend_from_slice(&packet[..20]); // Incomplete last frame.
        let (source, chunks) = interleave_padding(&logical, &[WINDOW - 2, 0, 1, 19, 128]);
        let found = framed_audio_packets(&source, &chunks, &Codec::MpegAudio).unwrap().collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(source.read.get(), logical.len(), "the small window never rereads bytes");
        assert_eq!(found.len(), 2);
        for frame in found {
            assert_eq!(frame.info.unwrap().samples, 1152);
            assert_eq!(frame.read(&source, &chunks).unwrap(), packet);
        }
    }

    #[test]
    fn ac3_frame_spans_separate_chunks() {
        // Original AC-3 test header for a 128-byte, 48 kHz stereo frame.
        let mut packet = vec![0; 128];
        packet[..8].copy_from_slice(&[0x0b, 0x77, 0, 0, 0, 0x40, 0x40, 0]);
        let (source, chunks) = interleave_padding(&packet, &[1, 6, 2]);
        let mut scan = framed_audio_packets(&source, &chunks, &Codec::Ac3).unwrap();
        let frame = scan.next().unwrap().unwrap();
        assert_eq!(frame.info.map(|info| (info.sample_rate, info.channels, info.samples)), Some((48_000, 2, 1536)));
        assert_eq!(frame.read(&source, &chunks).unwrap(), packet);
        assert!(scan.next().is_none());
    }

    #[test]
    fn repeated_index_ranges_cannot_expand_the_scan_budget() {
        let source = Source { bytes: vec![0; 64], read: Cell::new(0) };
        let chunks = [Chunk { offset: 0, size: 64, key: true }; 5];
        assert!(matches!(framed_audio_packets(&source, &chunks, &Codec::MpegAudio), Err(Error::Invalid(_))));
        assert_eq!(source.read.get(), 0);
    }
}

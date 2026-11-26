//! The Info frame at the start of an MP3 file: a Layer III frame that carries no audio, whose
//! Xing / Info header gives the frame count, the byte count and a seek table, and whose LAME tag
//! gives the encoder delay and the padding at the end, so a reader can return exactly the samples
//! that were encoded. `InfoTag` (feature `mp3-decode`) reads it back.
//!
//! The layout followed is the one decoders read: the frame header and empty side information,
//! `Info` (the name for a constant bit rate), flags 15 (frames, bytes, table of contents,
//! quality), the four fields, then the 36-byte LAME tag. Its encoder string begins with `LAME`,
//! which is what readers (this crate's, ffmpeg's) look for before they use the delay and padding;
//! the rest of it names this encoder. The tag ends with a CRC-16 of the frame up to it, and the
//! one before it is a CRC-16 of the audio.

use std::io::{self, Seek, SeekFrom, Write};

use super::encoder::DELAY;
use super::tables;

/// The 9-byte encoder string of the LAME tag.
const ENCODER: &[u8; 9] = b"LAMEcraft";
/// What the LAME tag's delay leaves out: a decoder's own delay (528 + 1 samples).
const DECODER_DELAY: u32 = 529;
/// Bytes of the Xing / Info part: `Info`, the flags, frames, bytes, the table of contents and
/// the quality.
const XING_LEN: usize = 4 + 4 + 4 + 4 + 100 + 4;
/// Bytes of the LAME tag.
const LAME_LEN: usize = 36;

/// CRC-16 with the polynomial 0x8005, bits reflected, starting at 0 (the LAME tag's CRCs).
pub(crate) fn crc16(mut crc: u16, bytes: &[u8]) -> u16 {
    for &b in bytes {
        crc ^= u16::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xa001 } else { crc >> 1 };
        }
    }
    crc
}

/// What an Info frame describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Stream {
    pub sample_rate: u32,
    pub channels: usize,
    /// The audio's bit rate, kbit/s.
    pub kbps: u32,
    /// Audio frames after the Info frame.
    pub frames: u64,
    /// Bytes of those frames.
    pub bytes: u64,
    /// Samples per channel that were encoded.
    pub samples: u64,
    /// [`crc16`] of those frames' bytes.
    pub crc: u16,
}

/// The frame layout for a stream: MPEG-2 for the lower sample rates, the sample-rate index, the
/// side information's length and the samples per frame.
fn layout(sample_rate: u32, channels: usize) -> Option<(bool, u32, usize, u64)> {
    let (lsf, index) = match tables::RATES_MPEG1.iter().position(|&r| r == sample_rate) {
        Some(i) => (false, i),
        None => (true, tables::RATES_LSF.iter().position(|&r| r == sample_rate)?),
    };
    let side = match (lsf, channels) {
        (false, 1) => 17,
        (false, 2) => 32,
        (true, 1) => 9,
        (true, 2) => 17,
        _ => return None,
    };
    Some((lsf, index as u32, side, if lsf { 576 } else { 1152 }))
}

/// The bit rate of the Info frame and its length: the audio's rate when the tag fits in a frame
/// of it, else the lowest rate whose frame it fits in.
fn frame_rate(sample_rate: u32, channels: usize, kbps: u32) -> Option<(u32, usize)> {
    let (lsf, _, side, _) = layout(sample_rate, channels)?;
    let need = 4 + side + XING_LEN + LAME_LEN;
    let rates: &[u32] = if lsf { &tables::BITRATES_LSF } else { &tables::BITRATES_MPEG1 };
    let len = |r: u32| (u64::from(if lsf { 72_000u32 } else { 144_000 }) * u64::from(r) / u64::from(sample_rate)) as usize;
    let index = rates.iter().position(|&r| r == kbps && len(r) >= need).or_else(|| rates.iter().position(|&r| len(r) >= need))?;
    Some((index as u32 + 1, len(rates[index])))
}

/// Bytes the Info frame of a stream at `sample_rate`, `channels` and `kbps` takes, or `None` for
/// a stream the encoder can't write.
pub(crate) fn frame_len(sample_rate: u32, channels: usize, kbps: u32) -> Option<usize> {
    frame_rate(sample_rate, channels, kbps).map(|(_, len)| len)
}

/// The Info frame for `stream`, [`frame_len`] bytes long.
pub(crate) fn frame(stream: &Stream) -> Option<Vec<u8>> {
    let (lsf, sr_index, side, per_frame) = layout(stream.sample_rate, stream.channels)?;
    let (br_index, len) = frame_rate(stream.sample_rate, stream.channels, stream.kbps)?;
    // The header: sync, MPEG-1 (11) or MPEG-2 (10), Layer III (01), no CRC; no padding; the
    // channel mode (mono 11, joint stereo 01); not copyrighted, an original. Then empty side
    // information.
    let mode = if stream.channels == 1 { 3 } else { 1 };
    let mut f = vec![0xff, if lsf { 0xf3 } else { 0xfb }, ((br_index << 4) | (sr_index << 2)) as u8, (mode << 6) | 0x04];
    f.resize(4 + side, 0);
    let total = (len as u64).saturating_add(stream.bytes);
    let be32 = |n: u64| u32::try_from(n).unwrap_or(u32::MAX).to_be_bytes();
    // A file too big for the byte count leaves it out: a wrong one would make readers distrust
    // the frame count, and with it the length.
    let bytes = u32::try_from(total).ok();
    f.extend_from_slice(b"Info");
    f.extend_from_slice(&(if bytes.is_some() { 15u32 } else { 13 }).to_be_bytes());
    f.extend_from_slice(&be32(stream.frames));
    if let Some(bytes) = bytes {
        f.extend_from_slice(&bytes.to_be_bytes());
    }
    // The table of contents: where in the file each percent of the audio starts, in 256ths. At a
    // constant bit rate that is in proportion to the bytes.
    f.extend((0..100u64).map(|i| ((len as u64 + stream.bytes * i / 100) * 256 / total.max(1)).min(255) as u8));
    f.extend_from_slice(&0u32.to_be_bytes()); // quality: not given
    f.extend_from_slice(ENCODER);
    f.push(0x01); // tag revision 0, constant bit rate
    f.extend_from_slice(&[0; 9]); // lowpass (1), peak (4), two replay gains (2 + 2): not given
    f.extend_from_slice(&[0, stream.kbps.min(255) as u8]); // flags; the bit rate
    // The delay counts the encoder's part of DELAY; the padding is what the last frames hold past
    // the audio, the decoder's delay included. 12 bits each.
    let delay = DELAY - DECODER_DELAY;
    let decoded = stream.frames.saturating_mul(per_frame);
    let padding = decoded.saturating_sub(stream.samples).saturating_sub(u64::from(delay));
    let padding = u32::try_from(padding).ok().filter(|p| *p < 4096)?;
    f.extend_from_slice(&[(delay >> 4) as u8, (((delay & 15) << 4) | (padding >> 8)) as u8, padding as u8]);
    f.extend_from_slice(&[0; 4]); // misc, gain, preset and surround: not given
    f.extend_from_slice(&be32(total)); // the music length, this frame included
    f.extend_from_slice(&stream.crc.to_be_bytes());
    let crc = crc16(0, &f);
    f.extend_from_slice(&crc.to_be_bytes());
    debug_assert_eq!(f.len(), 4 + side + XING_LEN + LAME_LEN - if bytes.is_some() { 0 } else { 4 });
    f.resize(len, 0);
    Some(f)
}

/// The frames in `data`, a run of whole Layer III frames of one stream, by their headers.
fn frames_in(data: &[u8]) -> u64 {
    let mut n = 0;
    let mut at = 0;
    while let Some(&[0xff, b1, b2, _]) = data.get(at..at + 4) {
        let lsf = b1 & 0x18 == 0x10;
        let (rates, bitrates): (&[u32], &[u32]) =
            if lsf { (&tables::RATES_LSF, &tables::BITRATES_LSF) } else { (&tables::RATES_MPEG1, &tables::BITRATES_MPEG1) };
        let (Some(&kbps), Some(&rate)) = (bitrates.get(usize::from(b2 >> 4).wrapping_sub(1)), rates.get(usize::from((b2 >> 2) & 3))) else { break };
        let len = (u64::from(if lsf { 72_000u32 } else { 144_000 }) * u64::from(kbps) / u64::from(rate)) as usize + usize::from((b2 >> 1) & 1);
        at += len;
        n += 1;
    }
    n
}

/// Puts an Info frame in front of the frames written through it: a provisional one before the
/// first (no frames yet), the real one at [`Writer::finish`]. A stream with no audio gets none.
pub(crate) struct Writer {
    stream: Stream,
    /// Where the Info frame starts, once there is audio.
    at: Option<u64>,
}

impl Writer {
    /// For a stream at `sample_rate`, `channels` and `kbps`; `None` for one the encoder can't write.
    pub(crate) fn new(sample_rate: u32, channels: usize, kbps: u32) -> Option<Writer> {
        frame_len(sample_rate, channels, kbps)?;
        Some(Writer { stream: Stream { sample_rate, channels, kbps, frames: 0, bytes: 0, samples: 0, crc: 0 }, at: None })
    }

    /// Writes `data`, whole frames, after the Info frame.
    pub(crate) fn write<W: Write + Seek>(&mut self, out: &mut W, data: &[u8]) -> io::Result<()> {
        if self.at.is_none() {
            let start = out.stream_position()?;
            out.write_all(&frame(&self.stream).ok_or_else(|| io::Error::other("MP3: no Info frame for this stream"))?)?;
            self.at = Some(start);
        }
        out.write_all(data)?;
        self.stream.frames = self.stream.frames.saturating_add(frames_in(data));
        self.stream.bytes = self.stream.bytes.saturating_add(data.len() as u64);
        self.stream.crc = crc16(self.stream.crc, data);
        Ok(())
    }

    /// Writes the Info frame for the frames written, `samples` long, and returns to the end.
    pub(crate) fn finish<W: Write + Seek>(&self, out: &mut W, samples: u64) -> io::Result<()> {
        let Some(at) = self.at else { return Ok(()) };
        let info = frame(&Stream { samples, ..self.stream }).ok_or_else(|| io::Error::other("MP3: the Info frame can't describe this stream"))?;
        let end = out.stream_position()?;
        out.seek(SeekFrom::Start(at))?;
        out.write_all(&info)?;
        out.seek(SeekFrom::Start(end))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_counted_by_their_headers() {
        let mut enc = crate::mp3::Encoder::new(crate::mp3::Config::cbr(44_100, 2, 128)).unwrap();
        let tone: Vec<f32> = (0..44_100).map(|i| (i as f32 * 0.03).sin() * 0.3).collect();
        let mut data = enc.encode(&[&tone, &tone]);
        data.extend(enc.flush());
        assert_eq!(frames_in(&data), enc.frames(), "44.1 kHz frames, some with the padding slot");
        assert_eq!(frames_in(&data[..data.len() - 1]), enc.frames(), "a cut last frame still counts");
        assert_eq!(frames_in(&[0; 8]), 0);
    }

    #[test]
    fn a_stream_too_big_for_the_byte_count_leaves_it_out() {
        let stream =
            Stream { sample_rate: 48_000, channels: 2, kbps: 320, frames: 4_000_000, bytes: 5 << 30, samples: 4_000_000 * 1152 - 2000, crc: 0 };
        let f = frame(&stream).unwrap();
        assert_eq!(&f[36..44], &[b'I', b'n', b'f', b'o', 0, 0, 0, 13]);
        assert_eq!(&f[48 + 100 + 4..48 + 100 + 8], b"LAME", "the table follows the frame count");
    }

    #[test]
    fn the_crc_is_crc16_arc() {
        assert_eq!(crc16(0, b"123456789"), 0xbb3d);
        assert_eq!(crc16(crc16(0, b"1234"), b"56789"), 0xbb3d, "it continues across calls");
    }

    #[test]
    fn the_frame_is_one_of_the_streams_frames_when_the_tag_fits() {
        // 192 kbit/s at 48 kHz: 576 bytes, the audio's own rate.
        assert_eq!(frame_len(48_000, 2, 192), Some(576));
        // 32 kbit/s at 48 kHz is 96 bytes, too few for the 192 the tag takes in stereo: 64 kbit/s.
        assert_eq!(frame_len(48_000, 2, 32), Some(192));
        // MPEG-2: 8 kbit/s at 24 kHz is 24 bytes; mono needs 4 + 9 + 120 + 36 = 169, so 64 kbit/s.
        assert_eq!(frame_len(24_000, 1, 8), Some(192));
        assert_eq!(frame_len(44_100, 3, 128), None);
        assert_eq!(frame_len(8000, 1, 32), None);
    }
}

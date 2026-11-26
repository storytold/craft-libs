//! SMPTE ST 302M: AES3 audio carried in MPEG-2 transport stream packets (feature `s302m`): a
//! packet decoder ([`decode`]) and encoder ([`Encoder`]) for the payload of one PES packet.
//!
//! A packet is a 4-byte header and the audio. The header is the size in bytes of the audio (16 bits,
//! big-endian), then 16 bits: the channel count code (2 bits; 0 to 3 for 2, 4, 6 or 8 channels), the
//! identification of the first channel (8 bits), the sample size code (2 bits; 0, 1 and 2 for 16, 20
//! and 24 bits) and 4 alignment bits. The audio is, for every frame, one 40-, 48- or 56-bit group
//! per channel pair, the pairs in order: each pair is two AES3 subframes, the left channel then the
//! right one, each of the sample (least significant bit first) and four flag bits (V, U, C and a
//! frame-start flag set on the left subframe of each channel pair in the first frame of every block of 192 frames). The
//! bits go out in transmission order, so a byte holds them most significant bit first.
//!
//! Samples are signed `i32` in the stream's own range (16, 20 or 24 bits, not scaled). The
//! decoder reads the flags and ignores them; the encoder writes the validity, user and channel
//! status bits as zero and sets the frame-start flag.
//!
//! Written from the standard's description of the packet and the AES3 subframe. ffmpeg is only run
//! as a black-box oracle in the tests.

/// The most audio bytes one packet can hold (the size field is 16 bits).
pub const MAX_AUDIO_BYTES: usize = 0xFFFF;

/// What went wrong decoding or encoding.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The packet is damaged, truncated, or uses a reserved code.
    #[error("malformed SMPTE 302M packet: {0}")]
    Malformed(&'static str),
    /// The settings or samples given to the encoder cannot be written.
    #[error("SMPTE 302M cannot encode: {0}")]
    InvalidInput(&'static str),
}

/// `Result` of this module.
pub type Result<T> = std::result::Result<T, Error>;

/// The header of a packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// 2, 4, 6 or 8.
    pub channels: usize,
    /// The identification of the first channel (as the stream gives it).
    pub channel_id: u8,
    /// 16, 20 or 24.
    pub bits: u32,
    /// The alignment bits as written.
    pub alignment: u8,
    /// Bytes of audio after the header.
    pub audio_bytes: usize,
}

impl Header {
    /// Bytes of one frame (one sample of every channel), or 0 if `channels` or `bits` are unsupported or overflow.
    pub fn frame_bytes(&self) -> usize {
        let pair_bytes = match self.bits {
            16 => 5,
            20 => 6,
            24 => 7,
            _ => return 0,
        };
        (self.channels / 2).checked_mul(pair_bytes).unwrap_or(0)
    }
}

/// Parse the 4-byte header at the start of `packet`.
pub fn parse_header(packet: &[u8]) -> Result<Header> {
    let [a, b, c, d, ..] = *packet else { return Err(Error::Malformed("shorter than the header")) };
    let size = usize::from(u16::from_be_bytes([a, b]));
    let word = u16::from_be_bytes([c, d]);
    let channels = 2 * (usize::from(word >> 14) + 1);
    let bits = match (word >> 4) & 3 {
        0 => 16,
        1 => 20,
        2 => 24,
        _ => return Err(Error::Malformed("reserved sample size code")),
    };
    Ok(Header { channels, channel_id: ((word >> 6) & 0xFF) as u8, bits, alignment: (word & 0xF) as u8, audio_bytes: size })
}

/// One decoded packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The packet header.
    pub header: Header,
    /// Interleaved samples, `header.channels` per frame.
    pub samples: Vec<i32>,
}

/// Bit `i` of a byte string in transmission order (the first bit sent is the least significant bit of the sample, held in the most significant bit of the byte).
fn bit(bytes: &[u8], i: usize) -> u32 {
    bytes.get(i / 8).map_or(0, |b| u32::from((b >> (7 - i % 8)) & 1))
}

/// Decode the payload of one PES packet.
pub fn decode(packet: &[u8]) -> Result<Decoded> {
    let header = parse_header(packet)?;
    let audio = packet.get(4..).unwrap_or(&[]);
    if audio.len() < header.audio_bytes {
        return Err(Error::Malformed("the audio is shorter than the header says"));
    }
    let audio = audio.get(..header.audio_bytes).unwrap_or(&[]);
    let frame_bytes = header.frame_bytes();
    if frame_bytes == 0 || audio.len() % frame_bytes != 0 {
        return Err(Error::Malformed("the audio is not a whole number of frames"));
    }
    let n = header.bits as usize;
    let mut samples = Vec::with_capacity(audio.len() / (n + 4) * 8 / 2);
    for frame in audio.chunks_exact(frame_bytes) {
        for pos in (0..header.channels).map(|c| c * (n + 4)) {
            let mut v = 0u32;
            for k in 0..n {
                v |= bit(frame, pos + k) << k;
            }
            // sign-extend the n-bit sample
            let shift = 32 - n as u32;
            samples.push(((v << shift) as i32) >> shift);
        }
    }
    Ok(Decoded { header, samples })
}

/// A packet encoder. It counts frames so that the frame-start flag falls on the first frame of
/// every block of 192.
#[derive(Debug, Clone)]
pub struct Encoder {
    channels: usize,
    bits: u32,
    channel_id: u8,
    frames: u64,
}

impl Encoder {
    /// An encoder for `channels` (2, 4, 6 or 8) channels of `bits` (16, 20 or 24) bits.
    pub fn new(channels: usize, bits: u32) -> Result<Encoder> {
        if !matches!(channels, 2 | 4 | 6 | 8) {
            return Err(Error::InvalidInput("SMPTE 302M carries 2, 4, 6 or 8 channels"));
        }
        if !matches!(bits, 16 | 20 | 24) {
            return Err(Error::InvalidInput("SMPTE 302M samples have 16, 20 or 24 bits"));
        }
        Ok(Encoder { channels, bits, channel_id: 0, frames: 0 })
    }

    /// Set the identification of the first channel written in the header (0 by default).
    pub fn channel_id(mut self, id: u8) -> Encoder {
        self.channel_id = id;
        self
    }

    /// The most frames one packet can hold.
    pub fn max_frames(&self) -> usize {
        MAX_AUDIO_BYTES / (self.channels / 2 * ((self.bits as usize + 4) * 2 / 8))
    }

    /// Encode interleaved samples (a whole number of frames, at most [`Encoder::max_frames`], each
    /// within the sample size) as one packet payload.
    pub fn encode(&mut self, samples: &[i32]) -> Result<Vec<u8>> {
        if samples.is_empty() || !samples.len().is_multiple_of(self.channels) {
            return Err(Error::InvalidInput("the samples are not a whole number of frames"));
        }
        let frames = samples.len() / self.channels;
        if frames > self.max_frames() {
            return Err(Error::InvalidInput("too many frames for one packet"));
        }
        let n = self.bits as usize;
        let limit = 1i64 << (n - 1);
        if samples.iter().any(|&s| i64::from(s) < -limit || i64::from(s) >= limit) {
            return Err(Error::InvalidInput("a sample is outside the sample size"));
        }
        let frame_bytes = self.channels / 2 * ((n + 4) * 2 / 8);
        let audio_bytes = frames * frame_bytes;
        let code = match self.bits {
            16 => 0u16,
            20 => 1,
            _ => 2,
        };
        let word = (((self.channels / 2 - 1) as u16) << 14) | (u16::from(self.channel_id) << 6) | (code << 4);
        let mut out = Vec::with_capacity(4 + audio_bytes);
        out.extend((audio_bytes as u16).to_be_bytes());
        out.extend(word.to_be_bytes());
        out.resize(4 + audio_bytes, 0);
        for (f, frame) in samples.chunks_exact(self.channels).enumerate() {
            let start = (self.frames + f as u64).is_multiple_of(192);
            let base = 4 + f * frame_bytes;
            for (c, &s) in frame.iter().enumerate() {
                let pos = c * (n + 4);
                let v = (s as u32) & ((1u32 << n) - 1);
                for k in 0..n {
                    set_bit(&mut out, base, pos + k, (v >> k) & 1);
                }
                // flags V, U, C, then the frame-start flag on the left subframe of each pair
                if c % 2 == 0 && start {
                    set_bit(&mut out, base, pos + n + 3, 1);
                }
            }
        }
        self.frames += frames as u64;
        Ok(out)
    }
}

fn set_bit(out: &mut [u8], base: usize, i: usize, v: u32) {
    if v != 0
        && let Some(b) = out.get_mut(base + i / 8)
    {
        *b |= 0x80 >> (i % 8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_packet() {
        // stereo 16-bit, one frame: L = 0x1234, R = 0xFEFF, the first frame of a block
        let packet = [0x00, 0x05, 0x00, 0x00, 0x2c, 0x48, 0x1f, 0xf7, 0xf0];
        let d = decode(&packet).unwrap();
        assert_eq!(d.samples, vec![0x1234, -257]);
        assert_eq!((d.header.channels, d.header.bits), (2, 16));
        let mut e = Encoder::new(2, 16).unwrap();
        assert_eq!(e.encode(&[0x1234, -257]).unwrap(), packet);
    }

    #[test]
    fn settings_and_packets_are_checked() {
        assert!(Encoder::new(1, 16).is_err());
        assert!(Encoder::new(2, 18).is_err());
        let mut e = Encoder::new(2, 16).unwrap();
        assert!(e.encode(&[]).is_err());
        assert!(e.encode(&[1]).is_err());
        assert!(e.encode(&[40_000, 0]).is_err());
        assert!(decode(&[0, 1]).is_err());
        assert!(decode(&[0, 5, 0, 0x30, 0, 0, 0, 0, 0]).is_err());
        assert!(decode(&[0, 5, 0, 0, 0, 0]).is_err());
        assert!(decode(&[0, 4, 0, 0, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn header_frame_bytes_overflow_does_not_panic() {
        let h = Header { channels: usize::MAX, channel_id: 0, bits: 24, alignment: 0, audio_bytes: 0 };
        assert_eq!(h.frame_bytes(), 0);
    }

    #[test]
    fn header_frame_bytes_rejects_unsupported_depth() {
        let h_max = Header { channels: 2, channel_id: 0, bits: u32::MAX, alignment: 0, audio_bytes: 0 };
        assert_eq!(h_max.frame_bytes(), 0);

        let h_unsupported = Header { channels: 2, channel_id: 0, bits: 18, alignment: 0, audio_bytes: 0 };
        assert_eq!(h_unsupported.frame_bytes(), 0);

        let h_zero = Header { channels: 2, channel_id: 0, bits: 0, alignment: 0, audio_bytes: 0 };
        assert_eq!(h_zero.frame_bytes(), 0);
    }
}

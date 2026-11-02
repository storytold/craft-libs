//! TOC byte and frame packing (RFC 6716 §3).

use crate::Error;

/// Coding mode of an Opus frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    SilkOnly,
    Hybrid,
    CeltOnly,
}

/// Audio bandwidth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Bandwidth {
    /// 4 kHz.
    Narrowband,
    /// 6 kHz.
    Mediumband,
    /// 8 kHz.
    Wideband,
    /// 12 kHz.
    SuperWideband,
    /// 20 kHz.
    Fullband,
}

impl Bandwidth {
    /// Number of CELT bands coded for this bandwidth.
    pub fn celt_end_band(self) -> usize {
        match self {
            Bandwidth::Narrowband => 13,
            Bandwidth::Mediumband | Bandwidth::Wideband => 17,
            Bandwidth::SuperWideband => 19,
            Bandwidth::Fullband => 21,
        }
    }
}

/// Decoded table-of-contents byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Toc {
    pub config: u8,
    pub mode: Mode,
    pub bandwidth: Bandwidth,
    pub stereo: bool,
    /// Frame count code (0..=3).
    pub code: u8,
}

impl Toc {
    pub fn parse(b: u8) -> Toc {
        let config = b >> 3;
        let (mode, bandwidth) = match config {
            0..=3 => (Mode::SilkOnly, Bandwidth::Narrowband),
            4..=7 => (Mode::SilkOnly, Bandwidth::Mediumband),
            8..=11 => (Mode::SilkOnly, Bandwidth::Wideband),
            12..=13 => (Mode::Hybrid, Bandwidth::SuperWideband),
            14..=15 => (Mode::Hybrid, Bandwidth::Fullband),
            16..=19 => (Mode::CeltOnly, Bandwidth::Narrowband),
            20..=23 => (Mode::CeltOnly, Bandwidth::Wideband),
            24..=27 => (Mode::CeltOnly, Bandwidth::SuperWideband),
            _ => (Mode::CeltOnly, Bandwidth::Fullband),
        };
        Toc { config, mode, bandwidth, stereo: b & 4 != 0, code: b & 3 }
    }

    /// Samples per frame at 48 kHz.
    pub fn frame_samples_48k(&self) -> usize {
        self.frame_samples(48000)
    }

    /// Samples per frame at `rate`.
    pub fn frame_samples(&self, rate: u32) -> usize {
        let rate = rate as usize;
        let c = self.config as usize;
        match self.mode {
            Mode::CeltOnly => (rate << (c & 3)) / 400,
            Mode::Hybrid => {
                if c & 1 != 0 {
                    rate / 50
                } else {
                    rate / 100
                }
            }
            Mode::SilkOnly => match c & 3 {
                3 => rate * 60 / 1000,
                s => (rate << s) / 100,
            },
        }
    }

    pub fn channels(&self) -> usize {
        if self.stereo { 2 } else { 1 }
    }
}

/// A parsed packet: TOC plus the byte ranges of each frame.
#[derive(Clone, Debug)]
pub struct Packet<'a> {
    pub toc: Toc,
    pub frames: Vec<&'a [u8]>,
    /// Bytes of padding (code 3 only).
    pub padding: usize,
}

fn parse_size(data: &[u8]) -> Option<(usize, usize)> {
    match data.first() {
        None => None,
        Some(&b) if b < 252 => Some((b as usize, 1)),
        Some(&b) => data.get(1).map(|&b1| (4 * b1 as usize + b as usize, 2)),
    }
}

impl<'a> Packet<'a> {
    /// Parses a packet (RFC 6716 section 3.2), enforcing requirements R1 to R7.
    pub fn parse(data: &'a [u8]) -> Result<Packet<'a>, Error> {
        Self::parse_impl(data, false).map(|(p, _)| p)
    }

    /// Parses a self-delimited packet (RFC 6716 Appendix B, used for all but the last stream of
    /// a multistream packet); returns the packet and the number of bytes it occupies.
    pub fn parse_self_delimited(data: &'a [u8]) -> Result<(Packet<'a>, usize), Error> {
        Self::parse_impl(data, true)
    }

    fn parse_impl(data: &'a [u8], self_delimited: bool) -> Result<(Packet<'a>, usize), Error> {
        const BAD: Error = Error::InvalidPacket("malformed packet framing");
        if data.is_empty() {
            return Err(Error::InvalidPacket("empty packet"));
        }
        let toc = Toc::parse(data[0]);
        let fs = toc.frame_samples_48k();
        let mut pos = 1usize;
        let mut len = data.len() as isize - 1;
        let mut sizes: Vec<isize> = Vec::new();
        let mut padding = 0usize;
        let mut cbr = false;
        let count;
        let mut last_size = len;
        let read_size = |pos: &mut usize, len: &mut isize| -> Result<isize, Error> {
            let avail = &data[*pos..(*pos + (*len).max(0) as usize).min(data.len())];
            let (s, n) = parse_size(avail).ok_or(BAD)?;
            *pos += n;
            *len -= n as isize;
            Ok(s as isize)
        };
        match toc.code {
            0 => count = 1,
            1 => {
                count = 2;
                cbr = true;
                if !self_delimited {
                    if len & 1 != 0 {
                        return Err(BAD);
                    }
                    last_size = len / 2;
                    sizes.push(last_size);
                }
            }
            2 => {
                count = 2;
                let s0 = read_size(&mut pos, &mut len)?;
                if s0 > len {
                    return Err(BAD);
                }
                sizes.push(s0);
                last_size = len - s0;
            }
            _ => {
                if len < 1 {
                    return Err(BAD);
                }
                let ch = data[pos];
                pos += 1;
                len -= 1;
                count = (ch & 0x3F) as usize;
                if count == 0 || fs * count > 5760 {
                    return Err(BAD);
                }
                if ch & 0x40 != 0 {
                    loop {
                        if len <= 0 {
                            return Err(BAD);
                        }
                        let b = data[pos];
                        pos += 1;
                        len -= 1;
                        let tmp = if b == 255 { 254 } else { b as isize };
                        len -= tmp;
                        padding += tmp as usize;
                        if b != 255 {
                            break;
                        }
                    }
                }
                if len < 0 {
                    return Err(BAD);
                }
                cbr = ch & 0x80 == 0;
                if !cbr {
                    last_size = len;
                    for _ in 0..count - 1 {
                        let before = len;
                        let s = read_size(&mut pos, &mut len)?;
                        if s > len {
                            return Err(BAD);
                        }
                        sizes.push(s);
                        last_size -= (before - len) + s;
                    }
                    if last_size < 0 {
                        return Err(BAD);
                    }
                } else if !self_delimited {
                    last_size = len / count as isize;
                    if last_size * count as isize != len {
                        return Err(BAD);
                    }
                    for _ in 0..count - 1 {
                        sizes.push(last_size);
                    }
                }
            }
        }
        if self_delimited {
            let before = len;
            let s = read_size(&mut pos, &mut len)?;
            if s > len {
                return Err(BAD);
            }
            if cbr {
                if s * count as isize > len {
                    return Err(BAD);
                }
                sizes = vec![s; count - 1];
            } else if (before - len) + s > last_size {
                return Err(BAD);
            }
            last_size = s;
        }
        if last_size > 1275 || sizes.iter().any(|&s| s > 1275) {
            return Err(BAD);
        }
        sizes.push(last_size);
        let mut frames = Vec::with_capacity(count);
        for &s in &sizes {
            let s = s as usize;
            if pos + s > data.len() {
                return Err(BAD);
            }
            frames.push(&data[pos..pos + s]);
            pos += s;
        }
        let total = if self_delimited { pos + padding } else { data.len() };
        if total > data.len() {
            return Err(BAD);
        }
        Ok((Packet { toc, frames, padding }, total))
    }
}

/// Number of samples (at 48 kHz) in a packet, or an error for malformed framing.
pub fn packet_samples_48k(data: &[u8]) -> Result<usize, Error> {
    let p = Packet::parse(data)?;
    Ok(p.toc.frame_samples_48k() * p.frames.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toc_configs() {
        for cfg in 0u8..32 {
            let t = Toc::parse(cfg << 3);
            let ms10 = t.frame_samples(48000) * 10 / 48; // tenths of ms
            let expect = match cfg {
                0..=11 => [100, 200, 400, 600][cfg as usize & 3],
                12..=15 => [100, 200][cfg as usize & 1],
                _ => [25, 50, 100, 200][cfg as usize & 3],
            };
            assert_eq!(ms10, expect, "config {cfg}");
        }
        assert!(Toc::parse(0x04).stereo);
    }

    #[test]
    fn frame_codes() {
        // code 0
        let p = Packet::parse(&[0x00, 1, 2, 3]).unwrap();
        assert_eq!(p.frames, vec![&[1u8, 2, 3][..]]);
        // code 1 (odd payload is invalid)
        assert!(Packet::parse(&[0x01, 1, 2, 3]).is_err());
        let p = Packet::parse(&[0x01, 1, 2, 3, 4]).unwrap();
        assert_eq!(p.frames.len(), 2);
        // code 2
        let p = Packet::parse(&[0x02, 1, 9, 8, 7]).unwrap();
        assert_eq!(p.frames, vec![&[9u8][..], &[8u8, 7][..]]);
        assert!(Packet::parse(&[0x02, 5, 1]).is_err());
        // code 3 CBR with padding: 2 frames of 2 bytes, 3 bytes padding
        let p = Packet::parse(&[0x03, 0x42, 3, 1, 2, 3, 4, 0, 0, 0]).unwrap();
        assert_eq!(p.frames, vec![&[1u8, 2][..], &[3u8, 4][..]]);
        assert_eq!(p.padding, 3);
        // code 3 VBR
        let p = Packet::parse(&[0x03, 0x83, 1, 2, 10, 20, 21, 30]).unwrap();
        assert_eq!(p.frames, vec![&[10u8][..], &[20u8, 21][..], &[30u8][..]]);
        // too many frames (> 120 ms)
        assert!(Packet::parse(&[0x03 | (3 << 3), 0x03, 0]).is_err());
        // zero frames
        assert!(Packet::parse(&[0x03, 0x00]).is_err());
        assert!(Packet::parse(&[]).is_err());
    }

    #[test]
    fn two_byte_sizes() {
        let mut pkt = vec![0x02u8, 252, 1];
        pkt.extend(std::iter::repeat_n(7u8, 256));
        pkt.extend([1, 2]);
        let p = Packet::parse(&pkt).unwrap();
        assert_eq!(p.frames[0].len(), 256);
        assert_eq!(p.frames[1], &[1, 2]);
    }

    #[test]
    fn self_delimited() {
        // code 0 with explicit size, followed by trailing data of the next stream.
        let (p, used) = Packet::parse_self_delimited(&[0x00, 2, 5, 6, 0xAA, 0xBB]).unwrap();
        assert_eq!(p.frames, vec![&[5u8, 6][..]]);
        assert_eq!(used, 4);
        // code 1: one size for both frames.
        let (p, used) = Packet::parse_self_delimited(&[0x01, 1, 5, 6, 9]).unwrap();
        assert_eq!(p.frames, vec![&[5u8][..], &[6u8][..]]);
        assert_eq!(used, 4);
        // code 3 VBR, 2 frames: size of frame 0, then size of last frame.
        let (p, used) = Packet::parse_self_delimited(&[0x03, 0x82, 1, 2, 7, 8, 9, 0xFF]).unwrap();
        assert_eq!(p.frames, vec![&[7u8][..], &[8u8, 9][..]]);
        assert_eq!(used, 7);
    }
}

// Ported from Apple's ALAC reference implementation (ALACDecoder.cpp, macosforge/alac@c38887c,
// https://github.com/macosforge/alac). Copyright (c) 2011 Apple Inc.
// Licensed under the Apache License, Version 2.0 (LICENSE-APACHE at the repository root).
// Modified: translated to Rust and changed as the `alac` module docs describe.
// SPDX-License-Identifier: Apache-2.0

//! The packet decoder, from Apple's `ALACDecoder.cpp`.

use super::ag::{self, AgParams};
use super::bits::BitReader;
use super::dp::{unpc_block, unpc_first_order_in_place};
use super::matrix::{Mix, unmix, unshift};
use super::{Error, ID_CCE, ID_CPE, ID_DSE, ID_END, ID_FIL, ID_LFE, ID_PCE, ID_SCE, MAX_CHANNELS, MAX_FRAME_LENGTH, Result, SpecificConfig};

/// Decodes ALAC packets to planar `i32` (see the [module docs](super) for the sample format).
///
/// Packets decode independently of each other: there is no state to reset after a seek.
#[derive(Debug, Clone)]
pub struct Decoder {
    config: SpecificConfig,
}

/// One channel's prediction parameters from an element header.
struct ChannelParams {
    mode: u32,
    den_shift: u32,
    pb_factor: u32,
    coefs: [i16; 32],
    num_coefs: usize,
}

impl Decoder {
    /// A decoder for the stream `cookie` (the magic cookie, [`SpecificConfig::parse`]) describes.
    pub fn new(cookie: &[u8]) -> Result<Self> {
        Self::from_config(SpecificConfig::parse(cookie)?)
    }

    /// A decoder for `config`.
    pub fn from_config(config: SpecificConfig) -> Result<Self> {
        if config.compatible_version > 0 {
            return Err(Error::Unsupported("a compatible version above 0"));
        }
        if !(1..=32).contains(&config.bit_depth) {
            return Err(Error::InvalidConfig("bit depth outside 1 to 32"));
        }
        if config.channels == 0 || config.channels > MAX_CHANNELS {
            return Err(Error::InvalidConfig("channel count outside 1 to 8"));
        }
        if config.frame_length == 0 || config.frame_length > MAX_FRAME_LENGTH {
            return Err(Error::InvalidConfig("frame length outside 1 to 2^20"));
        }
        Ok(Self { config })
    }

    /// The stream configuration.
    pub fn config(&self) -> &SpecificConfig {
        &self.config
    }

    /// Channel count.
    pub fn channels(&self) -> usize {
        usize::from(self.config.channels)
    }

    /// Sample rate from the cookie.
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    /// Bits per sample.
    pub fn bit_depth(&self) -> u32 {
        u32::from(self.config.bit_depth)
    }

    /// The frames `packet` holds, from its first element's header alone (for concealing a packet
    /// that fails to decode): the partial-frame count when the header has one, else the frame
    /// length. `None` when the packet does not start with an audio element or its header is cut
    /// short or out of range.
    pub fn packet_frames(&self, packet: &[u8]) -> Option<usize> {
        header_frames(packet, u64::from(self.config.frame_length)).map(|n| n as usize)
    }

    /// Decode one packet into one `Vec` per channel, in ALAC channel order. Channels the packet
    /// has no element for are silent, as in the reference decoder.
    ///
    /// After the last channel the packet must hold a whole element tag (the end tag, after any
    /// fill and data stream elements): a packet cut short after its samples is an error. Other
    /// elements there end the packet, as in the reference decoder.
    pub fn decode(&mut self, packet: &[u8]) -> Result<Vec<Vec<i32>>> {
        let channels = self.channels();
        let mut out: Vec<Vec<i32>> = vec![Vec::new(); channels];
        let mut r = BitReader::new(packet);
        let mut num_samples = self.config.frame_length as usize;
        // the sample count of the elements decoded so far
        let mut frames: Option<usize> = None;
        let mut index = 0;
        // whether the packet ended (or stopped as the reference stops) before the last channel
        let mut stopped = false;
        while index < channels {
            if r.position() / 8 >= packet.len() {
                return Err(Error::Eof);
            }
            match r.read(3)? {
                tag @ (ID_SCE | ID_LFE | ID_CPE) => {
                    let pair = tag == ID_CPE;
                    if pair && index + 2 > channels {
                        // a pair past the channel count: the reference stops here
                        stopped = true;
                        break;
                    }
                    let (a, b) = self.element(&mut r, pair, &mut num_samples)?;
                    if frames.is_some_and(|f| f != a.len()) {
                        return Err(Error::Bitstream("the elements of a frame differ in length"));
                    }
                    frames = Some(a.len());
                    out[index] = a;
                    index += 1;
                    if let Some(b) = b {
                        out[index] = b;
                        index += 1;
                    }
                }
                ID_CCE | ID_PCE => return Err(Error::Unsupported("coupling channel or program config element")),
                ID_DSE => data_stream_element(&mut r)?,
                ID_FIL => fill_element(&mut r)?,
                ID_END => {
                    r.byte_align();
                    stopped = true;
                    break;
                }
                _ => return Err(Error::Bitstream("element tag")),
            }
        }
        if !stopped {
            trailer(&mut r)?;
        }
        let frames = frames.unwrap_or(0);
        for ch in out.iter_mut().filter(|c| c.len() != frames) {
            *ch = vec![0; frames];
        }
        Ok(out)
    }

    /// One SCE/LFE (`pair` false) or CPE element after its tag: the channel's (or both channels')
    /// samples. A partial-frame header changes `num_samples` for this and later elements.
    fn element(&self, r: &mut BitReader, pair: bool, num_samples: &mut usize) -> Result<(Vec<i32>, Option<Vec<i32>>)> {
        let cfg = &self.config;
        let bit_depth = u32::from(cfg.bit_depth);
        let _instance = r.read(4)?;
        if r.read(12)? != 0 {
            return Err(Error::Bitstream("unused element header bits are set"));
        }
        let header = r.read(4)?;
        let partial = header >> 3 != 0;
        let mut bytes_shifted = (header >> 1) & 3;
        if bytes_shifted == 3 {
            return Err(Error::Bitstream("three shifted-off bytes"));
        }
        let escape = header & 1 != 0;
        if partial {
            let n = (r.read(16)? << 16) | r.read(16)?;
            if n > cfg.frame_length {
                return Err(Error::Bitstream("a partial frame longer than the frame length"));
            }
            *num_samples = n as usize;
        }
        let n = *num_samples;
        let nch = if pair { 2 } else { 1 };
        // the prediction (or escaped sample) width
        let chan_bits =
            |bits: i64| u32::try_from(bits).ok().filter(|b| (1..=32).contains(b)).ok_or(Error::Bitstream("sample width outside 1 to 32 bits"));
        let mut mixed = [vec![0i32; n], if pair { vec![0i32; n] } else { Vec::new() }];
        let mut shift_buf: Vec<u16> = Vec::new();
        let mut mix = Mix { bits: 0, res: 0 };
        if !escape {
            mix = Mix { bits: r.read(8)?, res: i32::from(r.read(8)? as u8 as i8) };
            let mut params = Vec::with_capacity(nch);
            for _ in 0..nch {
                let a = r.read(8)?;
                let b = r.read(8)?;
                let mut p = ChannelParams { mode: a >> 4, den_shift: a & 0xF, pb_factor: b >> 5, coefs: [0; 32], num_coefs: (b & 0x1F) as usize };
                for c in &mut p.coefs[..p.num_coefs] {
                    *c = r.read(16)? as u16 as i16;
                }
                params.push(p);
            }
            let bits = chan_bits(i64::from(bit_depth) - 8 * i64::from(bytes_shifted) + i64::from(pair))?;
            // the shifted-off low bytes come before the residuals; read them after
            let shift_at = r.position();
            if bytes_shifted != 0 {
                r.skip((bytes_shifted as usize * 8).saturating_mul(nch).saturating_mul(n))?;
            }
            let mut pred = vec![0i32; n];
            for (p, out) in params.iter_mut().zip(mixed.iter_mut()) {
                let pb = u32::from(cfg.pb) * p.pb_factor / 4;
                ag::decompress(AgParams::new(u32::from(cfg.mb), pb, u32::from(cfg.kb)), r, &mut pred, bits)?;
                if p.mode != 0 {
                    unpc_first_order_in_place(&mut pred, n, bits);
                }
                unpc_block(&pred, out, n, &mut p.coefs, p.num_coefs, bits, p.den_shift);
            }
            if bytes_shifted != 0 {
                let mut sr = *r;
                sr.set_position(shift_at)?;
                shift_buf = Vec::with_capacity(n * nch);
                for _ in 0..n * nch {
                    shift_buf.push(sr.read(bytes_shifted * 8)? as u16);
                }
            }
        } else {
            // uncompressed: a pair's samples are always full width
            let bits = chan_bits(if pair { i64::from(bit_depth) } else { i64::from(bit_depth) - 8 * i64::from(bytes_shifted) })?;
            for i in 0..n {
                for ch in mixed.iter_mut().take(nch) {
                    ch[i] = r.read_signed(bits)?;
                }
            }
            bytes_shifted = 0;
        }
        let [u, v] = mixed;
        if pair {
            let (mut l, mut rr) = (vec![0i32; n], vec![0i32; n]);
            unmix((&u, &v), (&mut l, &mut rr), mix, &shift_buf, bytes_shifted, bit_depth);
            Ok((l, Some(rr)))
        } else {
            let mut out = vec![0i32; n];
            unshift(&u, &shift_buf, &mut out, n, bytes_shifted, bit_depth);
            Ok((out, None))
        }
    }
}

/// What follows the last channel: fill and data stream elements, then the end tag. The
/// reference decoder stops after the last channel; this needs the next tag to be whole, so a
/// packet missing its last byte is found, and like the reference ignores any other element.
fn trailer(r: &mut BitReader) -> Result<()> {
    loop {
        match r.read(3)? {
            ID_DSE => data_stream_element(r)?,
            ID_FIL => fill_element(r)?,
            _ => return Ok(()),
        }
    }
}

/// Skip a fill element (`FillElement`): a 4-bit count, 15 meaning 14 plus an 8-bit count.
fn fill_element(r: &mut BitReader) -> Result<()> {
    let mut count = r.read(4)? as usize;
    if count == 15 {
        count = (count + r.read(8)? as usize).saturating_sub(1);
    }
    r.skip(count * 8)
}

/// Skip a data stream element (`DataStreamElement`).
fn data_stream_element(r: &mut BitReader) -> Result<()> {
    let _instance = r.read(4)?;
    let align = r.read(1)? != 0;
    let mut count = r.read(8)? as usize;
    if count == 255 {
        count += r.read(8)? as usize;
    }
    if align {
        r.byte_align();
    }
    r.skip(count * 8)
}

/// [`Decoder::packet_frames`] for a stream of `frame_length` frames a packet.
pub(crate) fn header_frames(packet: &[u8], frame_length: u64) -> Option<u64> {
    let mut r = BitReader::new(packet);
    if !matches!(r.read(3).ok()?, ID_SCE | ID_LFE | ID_CPE) {
        return None;
    }
    let _instance_and_unused = r.read(16).ok()?;
    let flags = r.read(4).ok()?;
    let frames = if flags & 0b1000 != 0 { u64::from(r.read(32).ok()?) } else { frame_length };
    (frames <= frame_length).then_some(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo16() -> Vec<u8> {
        SpecificConfig::new(44_100, 2, 16, 4096).to_cookie()
    }

    #[test]
    fn cookies_out_of_range_are_refused() {
        let ok = SpecificConfig::new(44_100, 2, 16, 4096);
        for bad in [
            SpecificConfig { compatible_version: 1, ..ok },
            SpecificConfig { bit_depth: 0, ..ok },
            SpecificConfig { bit_depth: 33, ..ok },
            SpecificConfig { channels: 0, ..ok },
            SpecificConfig { channels: 9, ..ok },
            SpecificConfig { frame_length: 0, ..ok },
            SpecificConfig { frame_length: MAX_FRAME_LENGTH + 1, ..ok },
        ] {
            assert!(Decoder::from_config(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn an_escaped_stereo_frame() {
        // CPE, instance 0, 12 zero bits, flags: partial + escape, 2 frames, then L R L R.
        let mut w = super::super::bits::BitWriter::new();
        use super::super::bits::BitSink;
        w.put(ID_CPE, 3);
        w.put(0, 4);
        w.put(0, 12);
        w.put(0b1001, 4);
        w.put(2, 32);
        for s in [1i32, -1, 32767, -32768] {
            w.put(s as u32, 16);
        }
        w.put(ID_END, 3);
        let mut dec = Decoder::new(&stereo16()).unwrap();
        assert_eq!(dec.decode(&w.into_bytes()).unwrap(), vec![vec![1, 32767], vec![-1, -32768]]);
    }

    /// An escaped mono packet that lost its last byte keeps every sample but not its end tag.
    #[test]
    fn a_packet_without_its_end_tag_is_an_error() {
        use super::super::bits::{BitSink, BitWriter};
        let mut w = BitWriter::new();
        for (value, bits) in [(ID_SCE, 3), (0, 4), (0, 12), (0b1001, 4), (3, 32)] {
            w.put(value, bits);
        }
        for s in [5u32, 6, 7] {
            w.put(s, 16);
        }
        w.put(ID_END, 3);
        let packet = w.into_bytes();
        let mut dec = Decoder::new(&SpecificConfig::new(44_100, 1, 16, 4096).to_cookie()).unwrap();
        assert_eq!(dec.decode(&packet).unwrap(), vec![vec![5, 6, 7]]);
        assert_eq!(dec.decode(&packet[..packet.len() - 1]), Err(Error::Eof));
        // a fill element before the end tag is skipped
        let mut w = BitWriter::new();
        for (value, bits) in [(ID_SCE, 3), (0, 4), (0, 12), (0b1001, 4), (1, 32), (9, 16), (ID_FIL, 3), (1, 4), (0xA5, 8), (ID_END, 3)] {
            w.put(value, bits);
        }
        assert_eq!(dec.decode(&w.into_bytes()).unwrap(), vec![vec![9]]);
    }

    #[test]
    fn the_frames_of_a_packet_come_from_its_header() {
        use super::super::bits::{BitSink, BitWriter};
        let dec = Decoder::new(&SpecificConfig::new(44_100, 2, 16, 4096).to_cookie()).unwrap();
        let header = |flags: u32, frames: Option<u32>| {
            let mut w = BitWriter::new();
            for (value, bits) in [(ID_CPE, 3), (0, 4), (0, 12), (flags, 4)] {
                w.put(value, bits);
            }
            if let Some(n) = frames {
                w.put(n, 32);
            }
            w.into_bytes()
        };
        assert_eq!(dec.packet_frames(&header(0b1001, Some(300))), Some(300));
        assert_eq!(dec.packet_frames(&header(0b0001, None)), Some(4096));
        assert_eq!(dec.packet_frames(&header(0b1001, Some(5000))), None);
        assert_eq!(dec.packet_frames(&header(0b1001, None)), None);
        assert_eq!(dec.packet_frames(&[0xE0]), None); // ID_END first
    }

    #[test]
    fn hostile_packets_fail_or_decode_without_panicking() {
        let mut dec = Decoder::new(&SpecificConfig::new(48_000, 8, 24, 4096).to_cookie()).unwrap();
        let mut seed = 0x1234_5678u32;
        for len in 0..400 {
            let packet: Vec<u8> = (0..len)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    seed as u8
                })
                .collect();
            if let Ok(chans) = dec.decode(&packet) {
                assert_eq!(chans.len(), 8);
                assert!(chans.iter().all(|c| c.len() == chans[0].len() && c.len() <= 4096));
            }
        }
        assert_eq!(dec.decode(&[]), Err(Error::Eof));
    }
}

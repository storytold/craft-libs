//! A native FLAC decoder (RFC 9639): [`decode`] for whole `.flac` files and [`Decoder`] for single
//! frames (the packets of FLAC in MP4, Matroska or Ogg).
//!
//! Every frame is checked against its CRC-8 and CRC-16. Subframe types constant, verbatim, fixed
//! and LPC; Rice partitions with 4- and 5-bit parameters and escaped partitions; wasted bits;
//! left/side, side/right and mid/side stereo; fixed and variable block sizes; 4 to 32 bits per
//! sample. The STREAMINFO MD5 is not verified.
//!
//! Samples come back as `i32`, one `Vec` per channel, in the FLAC channel order.

/// What went wrong decoding FLAC.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// The data ends early.
    #[error("truncated FLAC data")]
    Truncated,
    /// The data is not valid FLAC.
    #[error("invalid FLAC data: {0}")]
    Invalid(&'static str),
    /// A frame or header failed its CRC.
    #[error("FLAC CRC mismatch in {0}")]
    Crc(&'static str),
    /// Valid FLAC that is not decoded.
    #[error("unsupported FLAC feature: {0}")]
    Unsupported(&'static str),
    /// The stream is longer than the sample limit.
    #[error("FLAC stream exceeds the sample limit")]
    TooLarge,
}

/// `Result` of this module.
pub type Result<T> = std::result::Result<T, DecodeError>;

/// The STREAMINFO metadata block (RFC 9639 section 8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamInfo {
    /// Smallest block size in samples.
    pub min_block_size: u16,
    /// Largest block size in samples.
    pub max_block_size: u16,
    /// Smallest frame in bytes, 0 if unknown.
    pub min_frame_size: u32,
    /// Largest frame in bytes, 0 if unknown.
    pub max_frame_size: u32,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count, 1 to 8.
    pub channels: u8,
    /// Bits per sample, 4 to 32.
    pub bits_per_sample: u8,
    /// Samples per channel in the stream, 0 if unknown.
    pub total_samples: u64,
    /// MD5 of the unencoded audio, all zero if not set.
    pub md5: [u8; 16],
}

impl StreamInfo {
    /// Parse the 34 bytes of a STREAMINFO block body.
    pub fn parse(b: &[u8]) -> Result<StreamInfo> {
        let b = b.get(..34).ok_or(DecodeError::Truncated)?;
        let packed = u64::from_be_bytes([b[10], b[11], b[12], b[13], b[14], b[15], b[16], b[17]]);
        let info = StreamInfo {
            min_block_size: u16::from_be_bytes([b[0], b[1]]),
            max_block_size: u16::from_be_bytes([b[2], b[3]]),
            min_frame_size: u32::from_be_bytes([0, b[4], b[5], b[6]]),
            max_frame_size: u32::from_be_bytes([0, b[7], b[8], b[9]]),
            sample_rate: (packed >> 44) as u32,
            channels: ((packed >> 41) & 7) as u8 + 1,
            bits_per_sample: ((packed >> 36) & 31) as u8 + 1,
            total_samples: packed & 0xF_FFFF_FFFF,
            md5: b[18..34].try_into().map_err(|_| DecodeError::Truncated)?,
        };
        if info.bits_per_sample < 4 {
            return Err(DecodeError::Invalid("bits per sample below 4"));
        }
        if info.min_block_size != 0
            && (info.min_block_size < 16 && info.min_block_size != info.max_block_size || info.max_block_size < info.min_block_size)
        {
            return Err(DecodeError::Invalid("inconsistent block sizes"));
        }
        Ok(info)
    }
}

/// One decoded frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Bits per sample.
    pub bits_per_sample: u8,
    /// `true` when `number` is the first sample's index (variable block size), `false` when it is
    /// the frame's index (fixed block size).
    pub variable_block_size: bool,
    /// The coded number of the frame header.
    pub number: u64,
    /// Samples per channel in the frame.
    pub block_size: usize,
    /// One `Vec` per channel, after undoing any stereo decorrelation.
    pub channels: Vec<Vec<i32>>,
    /// Bytes the frame took, including the header and CRC footer.
    pub size: usize,
}

// ---- checksums ----

fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { crc << 1 ^ 0x07 } else { crc << 1 };
        }
    }
    crc
}

fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { crc << 1 ^ 0x8005 } else { crc << 1 };
        }
    }
    crc
}

// ---- bits ----

struct Bits<'a> {
    data: &'a [u8],
    /// Position in bits.
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0 }
    }

    fn left(&self) -> usize {
        self.data.len() * 8 - self.pos
    }

    fn bit(&mut self) -> Result<u32> {
        let byte = *self.data.get(self.pos / 8).ok_or(DecodeError::Truncated)?;
        let v = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Ok(u32::from(v))
    }

    /// `n` bits (0 to 32) as an unsigned number.
    fn bits(&mut self, n: u32) -> Result<u64> {
        if n as usize > self.left() {
            return Err(DecodeError::Truncated);
        }
        let mut v = 0u64;
        let mut n = n as usize;
        while n > 0 {
            let byte = u64::from(self.data[self.pos / 8]);
            let avail = 8 - self.pos % 8;
            let take = avail.min(n);
            v = v << take | (byte >> (avail - take)) & ((1 << take) - 1);
            self.pos += take;
            n -= take;
        }
        Ok(v)
    }

    /// `n` bits (0 to 33) as a two's complement number.
    fn signed(&mut self, n: u32) -> Result<i64> {
        if n == 0 {
            return Ok(0);
        }
        let v = self.bits(n)? as i64;
        Ok(if v >> (n - 1) & 1 == 1 { v - (1 << n) } else { v })
    }

    /// Zero bits up to the next one bit (which is consumed).
    fn unary(&mut self) -> Result<u64> {
        let mut n = 0u64;
        loop {
            // Skip whole zero bytes when aligned.
            if self.pos.is_multiple_of(8) {
                while self.data.get(self.pos / 8) == Some(&0) {
                    self.pos += 8;
                    n += 8;
                }
            }
            if self.bit()? == 1 {
                return Ok(n);
            }
            n += 1;
        }
    }

    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

// ---- frame ----

const BLOCK_SIZES: [usize; 16] = [0, 192, 576, 1152, 2304, 4608, 0, 0, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768];

/// A frame decoder. `info` supplies what a frame header leaves to the STREAMINFO.
#[derive(Debug, Clone, Copy, Default)]
pub struct Decoder {
    info: Option<StreamInfo>,
}

impl Decoder {
    /// A decoder with the stream's STREAMINFO (needed for frames whose header defers the sample
    /// rate or bit depth to it).
    pub fn new(info: StreamInfo) -> Decoder {
        Decoder { info: Some(info) }
    }

    /// A decoder without STREAMINFO: frames that defer to it are an error.
    pub fn without_streaminfo() -> Decoder {
        Decoder { info: None }
    }

    /// Decode the frame at the start of `data`.
    pub fn decode_frame(&self, data: &[u8]) -> Result<Frame> {
        // StreamInfo has public fields: container callers can construct it without
        // parse(), so validate before it can control shifts and sample reads.
        if self.info.is_some_and(|i| !(4..=32).contains(&i.bits_per_sample) || !(1..=8).contains(&i.channels) || i.sample_rate == 0) {
            return Err(DecodeError::Invalid("invalid decoder STREAMINFO"));
        }
        let mut r = Bits::new(data);
        if r.bits(15)? != 0x7FFC {
            return Err(DecodeError::Invalid("missing frame sync code"));
        }
        let variable = r.bit()? == 1;
        let (bs_code, sr_code) = (r.bits(4)? as usize, r.bits(4)? as usize);
        let (ch_code, bps_code) = (r.bits(4)? as usize, r.bits(3)? as usize);
        if r.bit()? != 0 {
            return Err(DecodeError::Invalid("reserved bit set"));
        }
        let number = coded_number(&mut r)?;
        let block_size = match bs_code {
            0 => return Err(DecodeError::Invalid("reserved block size")),
            6 => r.bits(8)? as usize + 1,
            7 => r.bits(16)? as usize + 1,
            c => BLOCK_SIZES[c],
        };
        let sample_rate = match sr_code {
            0 => self.info.map(|i| i.sample_rate).ok_or(DecodeError::Invalid("sample rate only in STREAMINFO"))?,
            1 => 88_200,
            2 => 176_400,
            3 => 192_000,
            4 => 8_000,
            5 => 16_000,
            6 => 22_050,
            7 => 24_000,
            8 => 32_000,
            9 => 44_100,
            10 => 48_000,
            11 => 96_000,
            12 => r.bits(8)? as u32 * 1000,
            13 => r.bits(16)? as u32,
            14 => r.bits(16)? as u32 * 10,
            _ => return Err(DecodeError::Invalid("forbidden sample rate code")),
        };
        let header_len = r.pos.div_ceil(8);
        let stored_crc = r.bits(8)? as u8;
        if crc8(&data[..header_len]) != stored_crc {
            return Err(DecodeError::Crc("a frame header"));
        }
        let bits = match bps_code {
            0 => self.info.map(|i| u32::from(i.bits_per_sample)).ok_or(DecodeError::Invalid("bit depth only in STREAMINFO"))?,
            1 => 8,
            2 => 12,
            4 => 16,
            5 => 20,
            6 => 24,
            7 => 32,
            _ => return Err(DecodeError::Invalid("reserved bit depth")),
        };
        let (channels, mode) = match ch_code {
            0..=7 => (ch_code + 1, Stereo::Independent),
            8 => (2, Stereo::LeftSide),
            9 => (2, Stereo::SideRight),
            10 => (2, Stereo::MidSide),
            _ => return Err(DecodeError::Invalid("reserved channel assignment")),
        };
        let mut out: Vec<Vec<i64>> = Vec::with_capacity(channels);
        for c in 0..channels {
            let extra = match (mode, c) {
                (Stereo::LeftSide, 1) | (Stereo::SideRight, 0) | (Stereo::MidSide, 1) => 1,
                _ => 0,
            };
            out.push(subframe(&mut r, block_size, bits + extra)?);
        }
        r.align();
        let size = r.pos / 8;
        let crc = r.bits(16)? as u16;
        if crc16(&data[..size]) != crc {
            return Err(DecodeError::Crc("a frame"));
        }
        let size = size + 2;
        if mode != Stereo::Independent {
            let (first, rest) = out.split_at_mut(1);
            let (a, b) = (&mut first[0], &mut rest[0]);
            for (x, y) in a.iter_mut().zip(b.iter_mut()) {
                match mode {
                    Stereo::LeftSide => *y = x.wrapping_sub(*y),
                    Stereo::SideRight => *x = x.wrapping_add(*y),
                    _ => {
                        let m = x.wrapping_shl(1) | (*y & 1);
                        let side = *y;
                        *x = m.wrapping_add(side) >> 1;
                        *y = m.wrapping_sub(side) >> 1;
                    }
                }
            }
        }
        let mut chans = Vec::with_capacity(channels);
        for c in out {
            let v: Result<Vec<i32>> = c.into_iter().map(|s| checked_sample(s, bits)).collect();
            chans.push(v?);
        }
        Ok(Frame { sample_rate, bits_per_sample: bits as u8, variable_block_size: variable, number, block_size, channels: chans, size })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stereo {
    Independent,
    LeftSide,
    SideRight,
    MidSide,
}

/// The UTF-8-like coded number of a frame header (up to 36 bits, 7 bytes).
fn coded_number(r: &mut Bits) -> Result<u64> {
    let first = r.bits(8)? as u8;
    let extra = match first {
        0x00..=0x7F => return Ok(u64::from(first)),
        0xC0..=0xDF => 1,
        0xE0..=0xEF => 2,
        0xF0..=0xF7 => 3,
        0xF8..=0xFB => 4,
        0xFC..=0xFD => 5,
        0xFE => 6,
        _ => return Err(DecodeError::Invalid("bad coded number")),
    };
    let mut v = if extra == 6 { 0 } else { u64::from(first & (0x3F >> extra)) };
    for _ in 0..extra {
        let b = r.bits(8)?;
        if b & 0xC0 != 0x80 {
            return Err(DecodeError::Invalid("bad coded number"));
        }
        v = v << 6 | (b & 0x3F);
    }
    Ok(v)
}

/// One subframe: `bits` is the frame bit depth (plus 1 for a side channel).
fn subframe(r: &mut Bits, n: usize, bits: u32) -> Result<Vec<i64>> {
    if r.bit()? != 0 {
        return Err(DecodeError::Invalid("subframe header padding bit set"));
    }
    let kind = r.bits(6)? as u32;
    let wasted = if r.bit()? == 1 { r.unary()? as u32 + 1 } else { 0 };
    if wasted >= bits {
        return Err(DecodeError::Invalid("wasted bits leave no sample bits"));
    }
    let bps = bits - wasted;
    let mut s = vec![0i64; n];
    match kind {
        0 => {
            let v = r.signed(bps)?;
            s.fill(v);
        }
        1 => {
            for v in s.iter_mut() {
                *v = r.signed(bps)?;
            }
        }
        8..=12 => {
            let order = (kind - 8) as usize;
            warmup(r, &mut s, order, bps)?;
            residual(r, &mut s, order)?;
            for i in order..n {
                let at = |k: usize| s[i - k];
                // Wrapping: hostile residuals can overflow; such samples fail the range check later.
                let p = match order {
                    0 => 0,
                    1 => at(1),
                    2 => at(1).wrapping_mul(2).wrapping_sub(at(2)),
                    3 => at(1).wrapping_mul(3).wrapping_sub(at(2).wrapping_mul(3)).wrapping_add(at(3)),
                    _ => at(1).wrapping_mul(4).wrapping_sub(at(2).wrapping_mul(6)).wrapping_add(at(3).wrapping_mul(4)).wrapping_sub(at(4)),
                };
                s[i] = s[i].wrapping_add(p);
            }
        }
        32..=63 => {
            let order = (kind - 31) as usize;
            warmup(r, &mut s, order, bps)?;
            let precision = r.bits(4)? as u32 + 1;
            if precision == 16 {
                return Err(DecodeError::Invalid("forbidden coefficient precision"));
            }
            let shift = r.signed(5)?;
            if shift < 0 {
                return Err(DecodeError::Invalid("negative prediction shift"));
            }
            let mut coefs = [0i64; 32];
            for c in coefs.iter_mut().take(order) {
                *c = r.signed(precision)?;
            }
            residual(r, &mut s, order)?;
            for i in order..n {
                let mut sum = 0i64;
                for (k, &c) in coefs.iter().take(order).enumerate() {
                    sum = sum.wrapping_add(c.wrapping_mul(s[i - 1 - k]));
                }
                s[i] = s[i].wrapping_add(sum >> shift);
            }
        }
        _ => return Err(DecodeError::Invalid("reserved subframe type")),
    }
    if wasted > 0 {
        for v in s.iter_mut() {
            *v = v.wrapping_shl(wasted);
        }
    }
    Ok(s)
}

fn warmup(r: &mut Bits, s: &mut [i64], order: usize, bps: u32) -> Result<()> {
    if order > s.len() {
        return Err(DecodeError::Invalid("predictor order above the block size"));
    }
    for v in s.iter_mut().take(order) {
        *v = r.signed(bps)?;
    }
    Ok(())
}

/// The coded residual (RFC 9639 section 9.2.7), into `s[order..]`.
fn residual(r: &mut Bits, s: &mut [i64], order: usize) -> Result<()> {
    let method = r.bits(2)?;
    let (param_bits, escape) = match method {
        0 => (4, 15),
        1 => (5, 31),
        _ => return Err(DecodeError::Invalid("reserved residual coding method")),
    };
    let part_order = r.bits(4)? as u32;
    let n = s.len();
    let parts = 1usize << part_order;
    if !n.is_multiple_of(parts) {
        return Err(DecodeError::Invalid("partition order does not divide the block size"));
    }
    let per = n >> part_order;
    if per < order {
        return Err(DecodeError::Invalid("partition shorter than the predictor order"));
    }
    let mut i = order;
    for p in 0..parts {
        let count = if p == 0 { per - order } else { per };
        let k = r.bits(param_bits)? as u32;
        if k == escape {
            let width = r.bits(5)? as u32;
            for v in s[i..i + count].iter_mut() {
                *v = r.signed(width)?;
            }
        } else {
            for v in s[i..i + count].iter_mut() {
                let q = r.unary()?;
                if q > (1 << 32) {
                    return Err(DecodeError::Invalid("Rice quotient too large"));
                }
                let low = r.bits(k)?;
                let folded = q << k | low;
                *v = if folded & 1 == 0 { (folded >> 1) as i64 } else { -((folded >> 1) as i64) - 1 };
            }
        }
        i += count;
    }
    Ok(())
}

// ---- files ----

/// A decoded file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    /// The STREAMINFO block.
    pub info: StreamInfo,
    /// One `Vec` of samples per channel.
    pub channels: Vec<Vec<i32>>,
}

/// Most sample values (samples x channels) [`decode`] will produce.
pub const DEFAULT_MAX_VALUES: u64 = 1 << 28;

/// Decode a whole `.flac` file (an ID3v2 tag in front is skipped).
pub fn decode(bytes: &[u8]) -> Result<Decoded> {
    decode_with_limit(bytes, DEFAULT_MAX_VALUES)
}

/// [`decode`] with a limit on the sample values (samples x channels) produced.
pub fn decode_with_limit(bytes: &[u8], max_values: u64) -> Result<Decoded> {
    let mut pos = 0usize;
    if bytes.starts_with(b"ID3") {
        let size = bytes.get(6..10).ok_or(DecodeError::Truncated)?;
        if size.iter().any(|&b| b & 0x80 != 0) {
            return Err(DecodeError::Invalid("bad ID3v2 size"));
        }
        pos = 10 + size.iter().fold(0usize, |a, &b| a << 7 | usize::from(b));
    }
    if bytes.get(pos..pos + 4) != Some(b"fLaC") {
        return Err(DecodeError::Invalid("missing fLaC marker"));
    }
    pos += 4;
    let mut info: Option<StreamInfo> = None;
    loop {
        let head = bytes.get(pos..pos + 4).ok_or(DecodeError::Truncated)?;
        let last = head[0] & 0x80 != 0;
        let kind = head[0] & 0x7F;
        let len = usize::from(head[1]) << 16 | usize::from(head[2]) << 8 | usize::from(head[3]);
        let body = bytes.get(pos + 4..pos + 4 + len).ok_or(DecodeError::Truncated)?;
        match (kind, &info) {
            (0, None) => info = Some(StreamInfo::parse(body)?),
            (0, Some(_)) => return Err(DecodeError::Invalid("a second STREAMINFO block")),
            (_, None) => return Err(DecodeError::Invalid("STREAMINFO is not the first block")),
            (127, _) => return Err(DecodeError::Invalid("forbidden metadata block type")),
            _ => {}
        }
        pos += 4 + len;
        if last {
            break;
        }
    }
    let info = info.ok_or(DecodeError::Invalid("no STREAMINFO"))?;
    let dec = Decoder::new(info);
    let mut channels: Vec<Vec<i32>> = vec![Vec::new(); usize::from(info.channels)];
    let mut total = 0u64;
    while pos + 2 <= bytes.len() && bytes[pos] == 0xFF && bytes[pos + 1] & 0xFE == 0xF8 {
        let frame = dec.decode_frame(&bytes[pos..])?;
        if frame.channels.len() != channels.len() {
            return Err(DecodeError::Invalid("the channel count changes inside the stream"));
        }
        // One file has one sample rate and depth. Otherwise callers would play
        // samples under incompatible STREAMINFO metadata.
        if frame.sample_rate != info.sample_rate || frame.bits_per_sample != info.bits_per_sample {
            return Err(DecodeError::Invalid("frame format differs from STREAMINFO"));
        }
        total += frame.block_size as u64;
        if total.saturating_mul(channels.len() as u64) > max_values {
            return Err(DecodeError::TooLarge);
        }
        for (dst, src) in channels.iter_mut().zip(frame.channels) {
            dst.extend_from_slice(&src);
        }
        pos += frame.size;
    }
    // Unknown total_samples must not turn a corrupt/truncated next frame into
    // a successfully decoded shorter recording.
    if pos != bytes.len() {
        return Err(DecodeError::Invalid("trailing data or missing frame sync"));
    }
    if info.total_samples != 0 && total != info.total_samples {
        return Err(DecodeError::Truncated);
    }
    Ok(Decoded { info, channels })
}

// Validate the advertised precision, not just the storage type. Malformed
// predictors may produce an i32 that is still invalid for 4–24-bit audio.
fn checked_sample(sample: i64, bits: u32) -> Result<i32> {
    if !(4..=32).contains(&bits) {
        return Err(DecodeError::Invalid("invalid sample bit depth"));
    }
    let bound = 1i64 << (bits - 1);
    if !(-bound..bound).contains(&sample) {
        return Err(DecodeError::Invalid("sample outside the bit depth"));
    }
    i32::try_from(sample).map_err(|_| DecodeError::Invalid("sample outside the bit depth"))
}

#[cfg(test)]
mod sample_bounds_tests {
    use super::checked_sample;

    #[test]
    fn samples_must_fit_declared_precision() {
        for bits in 4..=32 {
            let bound = 1i64 << (bits - 1);
            assert_eq!(checked_sample(-bound, bits).map(i64::from), Ok(-bound));
            assert_eq!(checked_sample(bound - 1, bits).map(i64::from), Ok(bound - 1));
            assert!(checked_sample(-bound - 1, bits).is_err());
            assert!(checked_sample(bound, bits).is_err());
        }
        assert!(checked_sample(0, 0).is_err());
        assert!(checked_sample(0, 255).is_err());
    }
}

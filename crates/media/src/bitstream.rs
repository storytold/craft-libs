//! Bit-level reading and writing for codec and container code.
//!
//! - [`BitReader`]: MSB-first reader with Exp-Golomb (`ue(v)`, `se(v)`), as used by H.264/HEVC/AV1 headers.
//! - [`BitWriter`]: the matching writer (for encoders and header rewriting).
//! - [`unescape_rbsp`] / [`escape_rbsp`]: remove/insert H.264/HEVC emulation-prevention bytes.
//! - [`annexb_nals`] / [`length_prefixed_nals`]: split Annex-B byte streams and AVCC/HVCC samples into NAL units.

use std::fmt;

/// Error from bit-level reading and NAL splitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitError {
    /// Tried to read past the end of the buffer.
    Eof,
    /// An Exp-Golomb code longer than 32 leading zeros.
    InvalidExpGolomb,
    /// An AVCC/HVCC NAL length prefix width outside 1 to 4 bytes.
    InvalidNalLengthSize,
}

impl fmt::Display for BitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BitError::Eof => f.write_str("unexpected end of bitstream"),
            BitError::InvalidExpGolomb => f.write_str("invalid Exp-Golomb code"),
            BitError::InvalidNalLengthSize => f.write_str("NAL length prefix must be 1 to 4 bytes"),
        }
    }
}
impl std::error::Error for BitError {}

/// Result type for the functions in this module, with [`BitError`] as the error.
pub type Result<T> = std::result::Result<T, BitError>;

/// MSB-first bit reader over a byte slice.
#[derive(Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    /// Position in bits.
    pos: usize,
    /// Bit position of the last set bit (the `rbsp_stop_one_bit`), found once so that
    /// [`Self::more_rbsp_data`] does not rescan a long zero suffix on every call.
    rbsp_stop: Option<usize>,
}

impl<'a> BitReader<'a> {
    /// Creates a reader at bit 0 of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        let rbsp_stop = data.iter().rposition(|&b| b != 0).map(|i| i.saturating_mul(8).saturating_add(7 - data[i].trailing_zeros() as usize));
        Self { data, pos: 0, rbsp_stop }
    }
    /// The whole underlying slice, regardless of the read position.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }
    /// Bit position from the start.
    pub fn position(&self) -> usize {
        self.pos
    }
    /// Number of unread bits.
    pub fn bits_left(&self) -> usize {
        self.data.len().saturating_mul(8).saturating_sub(self.pos)
    }
    /// True when the read position is on a byte boundary.
    pub fn is_byte_aligned(&self) -> bool {
        self.pos.is_multiple_of(8)
    }
    /// Advances the position to the next byte boundary (no-op when already aligned).
    pub fn byte_align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
    /// Byte offset of the current position (rounded down).
    pub fn byte_pos(&self) -> usize {
        self.pos / 8
    }
    /// Moves the read position to absolute bit `pos` from the start. Not range-checked; later reads fail with [`BitError::Eof`] when `pos` is past the end.
    pub fn seek_bits(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// Peek `n` bits without consuming, zero-padded past the end. For `n` above 32 this returns
    /// what [`read_bits`](Self::read_bits) would: the low 32 bits of the `n`-bit field.
    #[inline]
    pub fn peek(&self, n: u32) -> u32 {
        if n > 32 {
            let mut r = self.clone();
            r.pos = r.pos.saturating_add((n - 32) as usize);
            return r.peek(32);
        }
        if n == 0 {
            return 0;
        }
        let byte = self.pos / 8;
        let shift = self.pos % 8;
        // One 8-byte load while at least 8 bytes are left (`shift` <= 7 leaves 57 valid bits).
        if let Some(w) = self.data.get(byte..).and_then(<[u8]>::first_chunk::<8>) {
            return ((u64::from_be_bytes(*w) << shift) >> (64 - n)) as u32;
        }
        self.peek_tail(n, byte, shift)
    }

    /// [`Self::peek`] for the last 7 bytes and past the end: byte loads, zero padded.
    #[cold]
    fn peek_tail(&self, n: u32, byte: usize, shift: usize) -> u32 {
        let mut v: u64 = 0;
        for i in 0..5 {
            v = (v << 8) | *self.data.get(byte.saturating_add(i)).unwrap_or(&0) as u64;
        }
        // v holds 40 bits starting at `byte`
        ((v << (24 + shift)) >> (64 - n)) as u32
    }

    /// Read `n` bits. For `n` above 32 this consumes `n` bits and returns the low 32 of them.
    #[inline]
    pub fn read_bits(&mut self, n: u32) -> Result<u32> {
        if n as usize > self.bits_left() {
            return Err(BitError::Eof);
        }
        if n > 32 {
            self.pos += (n - 32) as usize;
            return self.read_bits(32);
        }
        let v = self.peek(n);
        self.pos += n as usize;
        Ok(v)
    }

    /// Reads `n` bits (up to 64) as a big-endian value.
    pub fn read_bits_u64(&mut self, n: u32) -> Result<u64> {
        if n <= 32 {
            return self.read_bits(n).map(u64::from);
        }
        let hi = self.read_bits(n - 32)? as u64;
        let lo = self.read_bits(32)? as u64;
        Ok((hi << 32) | lo)
    }

    /// Reads one bit.
    #[inline]
    pub fn read_bit(&mut self) -> Result<bool> {
        if self.bits_left() == 0 {
            return Err(BitError::Eof);
        }
        // `pos` is below `len * 8` here, so the byte exists.
        let byte = self.data.get(self.pos / 8).copied().unwrap_or(0);
        let bit = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Ok(bit == 1)
    }
    /// Reads one bit as a flag (`u(1)` syntax element).
    pub fn read_flag(&mut self) -> Result<bool> {
        self.read_bit()
    }

    /// Skips `n` bits, failing with [`BitError::Eof`] if fewer remain.
    pub fn skip(&mut self, n: usize) -> Result<()> {
        if n > self.bits_left() {
            return Err(BitError::Eof);
        }
        self.pos += n;
        Ok(())
    }

    /// Raw Exp-Golomb codeNum, up to 32 leading zeros (so below 2^33).
    #[inline]
    fn read_code_num(&mut self) -> Result<u64> {
        // Codes up to 31 bits (15 leading zeros) that lie wholly inside the data: one peek.
        let w = self.peek(32);
        let zeros = w.leading_zeros();
        if zeros < 16 {
            let len = 2 * zeros + 1;
            if len as usize <= self.bits_left() {
                self.pos += len as usize;
                // The `len`-bit field is 2^zeros + suffix; codeNum is that minus one.
                return Ok(u64::from((w >> (32 - len)) - 1));
            }
        }
        self.read_code_num_slow()
    }

    /// [`Self::read_code_num`] bit by bit: long codes, the end of the data and invalid codes.
    #[cold]
    fn read_code_num_slow(&mut self) -> Result<u64> {
        let mut zeros = 0u32;
        while !self.read_bit()? {
            zeros += 1;
            if zeros > 32 {
                return Err(BitError::InvalidExpGolomb);
            }
        }
        if zeros == 0 {
            return Ok(0);
        }
        let v = self.read_bits_u64(zeros)?;
        Ok((1u64 << zeros) - 1 + v)
    }

    /// Unsigned Exp-Golomb `ue(v)`.
    pub fn read_ue(&mut self) -> Result<u32> {
        u32::try_from(self.read_code_num()?).map_err(|_| BitError::InvalidExpGolomb)
    }

    /// `ue(v) + add` (the `..._minus1` / `..._minus8` fields), failing instead of overflowing on a
    /// hostile value near `u32::MAX`.
    pub fn read_ue_plus(&mut self, add: u32) -> Result<u32> {
        self.read_ue()?.checked_add(add).ok_or(BitError::InvalidExpGolomb)
    }

    /// Signed Exp-Golomb `se(v)`.
    pub fn read_se(&mut self) -> Result<i32> {
        // codeNum 2^32 is -2^31, so the mapping is done before narrowing to 32 bits.
        let k = self.read_code_num()? as i64;
        let v = if k & 1 == 1 { (k + 1) / 2 } else { -(k / 2) };
        i32::try_from(v).map_err(|_| BitError::InvalidExpGolomb)
    }

    /// Truncated Exp-Golomb `te(v)` with range `max`.
    pub fn read_te(&mut self, max: u32) -> Result<u32> {
        if max > 1 { self.read_ue() } else { Ok(!self.read_bit()? as u32) }
    }

    /// H.264/HEVC `more_rbsp_data()`: true if there is data before the rbsp trailing bits.
    pub fn more_rbsp_data(&self) -> bool {
        self.rbsp_stop.is_some_and(|stop| self.pos < stop)
    }
}

/// MSB-first bit writer.
#[derive(Default, Clone)]
pub struct BitWriter {
    buf: Vec<u8>,
    acc: u64,
    nbits: u32,
}

impl BitWriter {
    /// Creates an empty writer.
    pub fn new() -> Self {
        Self::default()
    }
    /// Write the low `n` bits of `value`. For `n` above 32 the extra high bits are zeros;
    /// `n` above 64 writes 64 bits.
    pub fn write_bits(&mut self, value: u32, n: u32) {
        let mut n = n.min(64);
        while n > 32 {
            let pad = (n - 32).min(32);
            self.write_bits(0, pad);
            n -= pad;
        }
        if n == 0 {
            return;
        }
        let v = (value as u64) & ((1u64 << n) - 1);
        self.acc = (self.acc << n) | v;
        self.nbits += n;
        while self.nbits >= 8 {
            self.nbits -= 8;
            self.buf.push((self.acc >> self.nbits) as u8);
        }
        self.acc &= (1u64 << self.nbits) - 1;
    }
    /// Writes one bit.
    pub fn write_bit(&mut self, b: bool) {
        self.write_bits(b as u32, 1);
    }
    /// Writes an unsigned Exp-Golomb `ue(v)` code.
    pub fn write_ue(&mut self, v: u32) {
        self.write_code_num(u64::from(v));
    }
    /// Exp-Golomb codeNum up to 2^32 (the code of `se(v)` -2^31).
    fn write_code_num(&mut self, v: u64) {
        let x = v + 1;
        let len = 64 - x.leading_zeros();
        let zeros = len - 1;
        if zeros + len <= 32 {
            // The leading zeros are the high bits of a (zeros + len)-bit field.
            self.write_bits(x as u32, zeros + len);
            return;
        }
        self.write_bits(0, zeros);
        if len > 32 {
            self.write_bits((x >> 32) as u32, len - 32);
            self.write_bits(x as u32, 32);
        } else {
            self.write_bits(x as u32, len);
        }
    }
    /// Writes a signed Exp-Golomb `se(v)` code.
    pub fn write_se(&mut self, v: i32) {
        let m = u64::from(v.unsigned_abs()) * 2;
        self.write_code_num(if v > 0 { m - 1 } else { m });
    }
    /// Number of bits written so far, including a partial last byte.
    pub fn bit_len(&self) -> usize {
        self.buf.len() * 8 + self.nbits as usize
    }
    /// True when the written bit count is a multiple of 8.
    pub fn is_byte_aligned(&self) -> bool {
        self.nbits == 0
    }
    /// Pad with zero bits to a byte boundary.
    pub fn align_zero(&mut self) {
        if self.nbits > 0 {
            self.write_bits(0, 8 - self.nbits);
        }
    }
    /// `rbsp_trailing_bits()`: a one bit then zeros to alignment.
    pub fn rbsp_trailing(&mut self) {
        self.write_bit(true);
        self.align_zero();
    }
    /// Appends whole bytes. Fast path when aligned; otherwise each byte is written bit by bit.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        if self.nbits == 0 {
            self.buf.extend_from_slice(bytes);
        } else {
            for &b in bytes {
                self.write_bits(b as u32, 8);
            }
        }
    }
    /// Pads to a byte boundary with zero bits and returns the written bytes.
    pub fn finish(mut self) -> Vec<u8> {
        self.align_zero();
        self.buf
    }
    /// The completed bytes written so far, excluding any partial last byte.
    pub fn bytes(&self) -> &[u8] {
        &self.buf
    }
}

/// Remove emulation-prevention bytes (`00 00 03` → `00 00`) from a NAL payload.
pub fn unescape_rbsp(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    // `data[start..]` is not copied yet; a removed `03` restarts the zero count at `start`.
    let mut start = 0;
    // Candidate position of an `03` preceded by two zeros at or after `start`.
    let mut i = 2;
    while let Some(&b) = data.get(i) {
        if b != 0 {
            // A nonzero byte at `i` rules out an `00 00 03` ending at i + 1 or i + 2.
            if b == 3 && data.get(i - 2..i) == Some(&[0, 0][..]) {
                out.extend_from_slice(data.get(start..i).unwrap_or_default());
                start = i + 1;
                i = start + 2;
            } else {
                i += 3;
            }
        } else {
            i += 1;
        }
    }
    out.extend_from_slice(data.get(start..).unwrap_or_default());
    out
}

/// Insert emulation-prevention bytes so no `00 00 0x` (x ≤ 3) sequence appears.
pub fn escape_rbsp(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 64);
    let mut zeros = 0;
    for &b in data {
        if zeros >= 2 && b <= 3 {
            out.push(3);
            zeros = 0;
        }
        out.push(b);
        zeros = if b == 0 { zeros + 1 } else { 0 };
    }
    out
}

/// Split an Annex-B byte stream (start codes `00 00 01` / `00 00 00 01`) into NAL units (escaped payloads).
pub fn annexb_nals(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(starts.len());
    for (k, &s) in starts.iter().enumerate() {
        let mut e = if k + 1 < starts.len() { starts[k + 1] - 3 } else { data.len() };
        // trim trailing zero bytes (part of a 4-byte start code or trailing_zero_8bits)
        while e > s && data[e - 1] == 0 {
            e -= 1;
        }
        if e > s {
            out.push(&data[s..e]);
        }
    }
    out
}

/// Split a length-prefixed sample (AVCC/HVCC, `length_size` 1 to 4 bytes) into NAL units.
/// Any other `length_size` returns [`BitError::InvalidNalLengthSize`], even for empty data.
pub fn length_prefixed_nals(data: &[u8], length_size: usize) -> Result<Vec<&[u8]>> {
    if !(1..=4).contains(&length_size) {
        return Err(BitError::InvalidNalLengthSize);
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        if length_size > data.len() - i {
            return Err(BitError::Eof);
        }
        let mut len = 0usize;
        for k in 0..length_size {
            len = (len << 8) | data[i + k] as usize;
        }
        i += length_size;
        if len > data.len() - i {
            return Err(BitError::Eof);
        }
        out.push(&data[i..i + len]);
        i += len;
    }
    Ok(out)
}

/// CRC-32 (IEEE, reflected) — used by containers (e.g. Matroska CRC-32 elements, PNG).
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[cfg(test)]
mod tests;

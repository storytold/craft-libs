//! The FFV1 Golomb-Rice coder (RFC 9043 section 3.8.2): a most-significant-bit-first bit reader,
//! the adaptive per-context VLC state of section 3.8.2.4 and the run lengths of section 3.8.2.2.1.

use super::{Error, Result};

/// RFC 9043 section 3.8.2.2.1: bits of the run length below the run prefix, by `run_index`.
pub(crate) const LOG2_RUN: [u32; 41] =
    [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24];

/// The largest Golomb-Rice parameter `k` a stream of up to 17-bit coded samples can reach: the mean
/// magnitude a context tracks stays below `2^17`, so `k` stays below 18. A larger one is damage.
const MAX_K: u32 = 24;

/// Reads bits most significant first; reading past the end is [`Error::Truncated`].
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    /// Bits read so far.
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> BitReader<'a> {
        BitReader { data, pos: 0 }
    }

    pub(crate) fn bit(&mut self) -> Result<bool> {
        let byte = self.data.get(self.pos >> 3).ok_or(Error::Truncated)?;
        let b = byte >> (7 - (self.pos & 7)) & 1;
        self.pos += 1;
        Ok(b == 1)
    }

    /// `n` bits (`n` at most 32) as an unsigned number.
    pub(crate) fn bits(&mut self, n: u32) -> Result<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            v = v << 1 | u32::from(self.bit()?);
        }
        Ok(v)
    }

    /// RFC 9043 Figure 26: `get_ur_golomb`; `bits` is the width of the escape value.
    fn unsigned(&mut self, k: u32, bits: u32) -> Result<i64> {
        for prefix in 0..12i64 {
            if self.bit()? {
                return Ok(i64::from(self.bits(k)?) + (prefix << k));
            }
        }
        Ok(i64::from(self.bits(bits)?) + 11)
    }

    /// RFC 9043 Figure 27: `get_sr_golomb`.
    fn signed(&mut self, k: u32, bits: u32) -> Result<i64> {
        let v = self.unsigned(k, bits)?;
        Ok(if v & 1 == 1 { -(v >> 1) - 1 } else { v >> 1 })
    }
}

/// The adaptive state of one context (RFC 9043 section 3.8.2.4), with the initial values of
/// section 3.8.2.5.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VlcState {
    drift: i64,
    error_sum: i64,
    bias: i64,
    count: i64,
}

impl Default for VlcState {
    fn default() -> Self {
        VlcState { drift: 0, error_sum: 4, bias: 0, count: 1 }
    }
}

/// RFC 9043 section 3.8.2.3: the low `bits` bits of `v` as a two's complement number.
fn sign_extend(v: i64, bits: u32) -> i64 {
    let negative = 1i64 << (bits - 1);
    let out = v & (negative - 1);
    if v & negative != 0 { out - negative } else { out }
}

/// RFC 9043 section 3.8.2.4: `get_vlc_symbol`, one sample difference of `bits` bits.
pub(crate) fn vlc_symbol(br: &mut BitReader, st: &mut VlcState, bits: u32) -> Result<i32> {
    let mut i = st.count;
    let mut k = 0u32;
    while i < st.error_sum {
        k += 1;
        if k > MAX_K {
            return Err(Error::Invalid("Golomb-Rice parameter out of range"));
        }
        i += i;
    }
    let mut v = br.signed(k, bits)?;
    if 2 * st.drift < -st.count {
        v = -1 - v;
    }
    let ret = sign_extend(v + st.bias, bits);
    st.error_sum += v.abs();
    st.drift += v;
    if st.count == 128 {
        st.count >>= 1;
        st.drift >>= 1;
        st.error_sum >>= 1;
    }
    st.count += 1;
    if st.drift <= -st.count {
        st.bias = (st.bias - 1).max(-128);
        st.drift = (st.drift + st.count).max(-st.count + 1);
    } else if st.drift > 0 {
        st.bias = (st.bias + 1).min(127);
        st.drift = (st.drift - st.count).min(0);
    }
    // `sign_extend` keeps `bits` (at most 17) bits.
    Ok(ret as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_codes_match_the_rfc_examples() {
        // RFC 9043 Table 3, with `bits` 8 for the escape.
        for (k, code, len, want) in
            [(0, 0b1, 1, 0), (0, 0b001, 3, 2), (2, 0b100, 3, 0), (2, 0b110, 3, 2), (2, 0b0101, 4, 5), (0, 0b0000_0000_0000_1000_0000, 20, 139)]
        {
            let bytes = ((code as u32) << (32 - len)).to_be_bytes();
            assert_eq!(BitReader::new(&bytes).unsigned(k, 8).unwrap(), want, "k={k} code={code:b}");
        }
    }
}

//! The FFV1 range coder (RFC 9043 section 3.8.1): binary decisions with adaptive 8-bit states,
//! and the signed/unsigned integer coding built on them.

/// RFC 9043 Figure 24: the default state transition table for a coded 1.
const DEFAULT_ONE_STATE: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48,
    49, 50, 51, 52, 53, 54, 55, 56, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 75, 76, 77, 78, 79, 80, 81, 82,
    83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 94, 95, 96, 97, 98, 99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113,
    114, 114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 129, 130, 131, 132, 133, 133, 134, 135, 136, 137, 138, 139, 140,
    141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 151, 152, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168,
    169, 170, 171, 171, 172, 173, 174, 175, 176, 177, 178, 179, 180, 181, 182, 183, 184, 185, 186, 187, 188, 189, 190, 190, 191, 192, 194, 194, 195,
    196, 197, 198, 199, 200, 201, 202, 202, 204, 205, 206, 207, 208, 209, 209, 210, 211, 212, 213, 215, 215, 216, 217, 218, 219, 220, 220, 222, 223,
    224, 225, 226, 227, 227, 229, 229, 230, 231, 232, 234, 234, 235, 236, 237, 238, 239, 240, 241, 242, 243, 244, 245, 246, 247, 248, 248, 0, 0, 0,
    0, 0, 0, 0,
];

/// The state machine: the next state after a coded 0 or 1.
#[derive(Clone)]
pub(crate) struct Transitions {
    one: [u8; 256],
    zero: [u8; 256],
}

impl Transitions {
    /// The default table plus `delta` (`state_transition_delta`, indexed 1 to 255).
    pub(crate) fn new(delta: &[i32; 256]) -> Transitions {
        let mut one = [0u8; 256];
        for (i, o) in one.iter_mut().enumerate() {
            *o = (i32::from(DEFAULT_ONE_STATE[i]) + delta[i]) as u8;
        }
        let mut zero = [0u8; 256];
        for i in 1..256 {
            zero[i] = (256 - i32::from(one[256 - i])) as u8;
        }
        Transitions { one, zero }
    }

    pub(crate) fn default_table() -> Transitions {
        Transitions::new(&[0; 256])
    }
}

/// Reads binary decisions from `data`; bytes past its end read as 0 (the "Closed" mode of 3.8.1.1.1).
pub(crate) struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
    low: u32,
    range: u32,
}

impl<'a> Decoder<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Decoder<'a> {
        let b0 = u32::from(data.first().copied().unwrap_or(0));
        let b1 = u32::from(data.get(1).copied().unwrap_or(0));
        let mut low = b0 << 8 | b1;
        let range = 0xFF00;
        if low >= range {
            low = range;
        }
        Decoder { data, pos: 2, low, range }
    }

    /// Bytes the coder has read so far.
    pub(crate) fn consumed(&self) -> usize {
        self.pos
    }

    fn refill(&mut self) {
        if self.range < 256 {
            self.range <<= 8;
            self.low <<= 8;
            self.low += u32::from(self.data.get(self.pos).copied().unwrap_or(0));
            self.pos += 1;
        }
    }

    pub(crate) fn bit(&mut self, state: &mut u8, t: &Transitions) -> bool {
        let off = (self.range * u32::from(*state)) >> 8;
        self.range -= off;
        if self.low < self.range {
            *state = t.zero[usize::from(*state)];
            self.refill();
            false
        } else {
            self.low -= self.range;
            *state = t.one[usize::from(*state)];
            self.range = off;
            self.refill();
            true
        }
    }

    /// RFC 9043 Figure 21: `get_symbol`.
    pub(crate) fn symbol(&mut self, state: &mut [u8; 32], signed: bool, t: &Transitions) -> i32 {
        if self.bit(&mut state[0], t) {
            return 0;
        }
        let mut e = 0usize;
        while self.bit(&mut state[1 + e.min(9)], t) {
            e += 1;
            if e > 24 {
                // No valid stream codes a magnitude this large; stop instead of spinning on garbage.
                return 0;
            }
        }
        let mut a = 1i32;
        for i in (0..e).rev() {
            a = a * 2 + i32::from(self.bit(&mut state[22 + i.min(9)], t));
        }
        if signed && self.bit(&mut state[11 + e.min(10)], t) { -a } else { a }
    }
}

/// Writes binary decisions; the inverse of [`Decoder`].
pub(crate) struct Encoder {
    out: Vec<u8>,
    low: u32,
    range: u32,
}

impl Encoder {
    pub(crate) fn new() -> Encoder {
        Encoder { out: Vec::new(), low: 0, range: 0xFF00 }
    }

    fn shift(&mut self) {
        while self.range < 256 {
            // Bits 15..8 of `low` are final once a carry (bit 16) is added to the bytes before them.
            let carry = self.low >> 16;
            let byte = ((self.low >> 8) & 0xFF) as u8;
            if carry != 0 {
                for b in self.out.iter_mut().rev() {
                    let (v, over) = b.overflowing_add(1);
                    *b = v;
                    if !over {
                        break;
                    }
                }
            }
            self.out.push(byte);
            self.low = (self.low & 0xFF) << 8;
            self.range <<= 8;
        }
    }

    pub(crate) fn bit(&mut self, bit: bool, state: &mut u8, t: &Transitions) {
        let off = (self.range * u32::from(*state)) >> 8;
        if bit {
            self.low += self.range - off;
            self.range = off;
            *state = t.one[usize::from(*state)];
        } else {
            self.range -= off;
            *state = t.zero[usize::from(*state)];
        }
        self.shift();
    }

    pub(crate) fn symbol(&mut self, state: &mut [u8; 32], v: i32, signed: bool, t: &Transitions) {
        if v == 0 {
            self.bit(true, &mut state[0], t);
            return;
        }
        self.bit(false, &mut state[0], t);
        let a = v.unsigned_abs();
        let e = (31 - a.leading_zeros()) as usize;
        for i in 0..e {
            self.bit(true, &mut state[1 + i.min(9)], t);
        }
        self.bit(false, &mut state[1 + e.min(9)], t);
        for i in (0..e).rev() {
            self.bit((a >> i) & 1 == 1, &mut state[22 + i.min(9)], t);
        }
        if signed {
            self.bit(v < 0, &mut state[11 + e.min(10)], t);
        }
    }

    /// Finish the stream with the two bytes of the *top* of the final range: whatever bytes follow
    /// (zeros, or a CRC in a container's configuration record) still decode inside the range.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.low += self.range - 1;
        let carry = self.low >> 16;
        if carry != 0 {
            for b in self.out.iter_mut().rev() {
                let (v, over) = b.overflowing_add(1);
                *b = v;
                if !over {
                    break;
                }
            }
        }
        self.out.push(((self.low >> 8) & 0xFF) as u8);
        self.out.push((self.low & 0xFF) as u8);
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_round_trip_through_the_range_coder() {
        let t = Transitions::default_table();
        let values: Vec<i32> = (0..4000).map(|i| ((i * 7919) % 2001) - 1000).chain([0, 0, 0, 65535, -65535, 1, -1]).collect();
        let mut enc = Encoder::new();
        let mut st = [128u8; 32];
        for &v in &values {
            enc.symbol(&mut st, v, true, &t);
        }
        let bytes = enc.finish();
        let mut dec = Decoder::new(&bytes);
        let mut st = [128u8; 32];
        for &v in &values {
            assert_eq!(dec.symbol(&mut st, true, &t), v);
        }
    }

    #[test]
    fn transition_tables_follow_the_rfc() {
        let t = Transitions::default_table();
        assert_eq!(t.one[8], 20);
        assert_eq!(t.one[247], 248);
        // zero_state_i = 256 - one_state_(256 - i)
        assert_eq!(t.zero[128], (256 - i32::from(t.one[128])) as u8);
        assert_eq!(t.zero[1], (256 - i32::from(t.one[255])) as u8);
    }
}

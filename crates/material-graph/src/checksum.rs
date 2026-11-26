//! Checksums used by the container formats: CRC-32 (PNG chunks, ZIP entries) and Adler-32 (zlib streams).
//! Both come as a one-shot function and as an incremental hasher for streamed data.

/// Slicing-by-8 tables for the reflected CRC-32 polynomial 0xEDB88320, built at compile time.
const CRC_TABLES: [[u32; 256]; 8] = crc_tables();

// Only ever evaluated at compile time (for `CRC_TABLES`), where an out-of-range index is a build error, not a runtime
// panic; `get`/`get_mut` are not usable in const fns.
#[allow(clippy::indexing_slicing)]
const fn crc_tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[0][n] = c;
        n += 1;
    }
    let mut s = 1;
    while s < 8 {
        let mut n = 0;
        while n < 256 {
            let prev = t[s - 1][n];
            t[s][n] = (prev >> 8) ^ t[0][(prev & 0xff) as usize];
            n += 1;
        }
        s += 1;
    }
    t
}

/// Incremental CRC-32 (ISO 3309 / ITU-T V.42, as used by PNG, ZIP and gzip).
#[derive(Clone, Copy, Debug)]
pub struct Crc32 {
    state: u32,
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    pub fn new() -> Self {
        Crc32 { state: 0xffff_ffff }
    }

    pub fn update(&mut self, data: &[u8]) {
        let [t0, t1, t2, t3, t4, t5, t6, t7] = &CRC_TABLES;
        let mut c = self.state;
        let (words, rest) = data.as_chunks::<8>();
        for &[b0, b1, b2, b3, b4, b5, b6, b7] in words {
            let lo = c ^ u32::from_le_bytes([b0, b1, b2, b3]);
            c = at(t7, lo)
                ^ at(t6, lo >> 8)
                ^ at(t5, lo >> 16)
                ^ at(t4, lo >> 24)
                ^ at(t3, b4 as u32)
                ^ at(t2, b5 as u32)
                ^ at(t1, b6 as u32)
                ^ at(t0, b7 as u32);
        }
        for &b in rest {
            c = at(t0, c ^ b as u32) ^ (c >> 8);
        }
        self.state = c;
    }

    pub fn finish(&self) -> u32 {
        self.state ^ 0xffff_ffff
    }
}

/// Table entry for the low byte of `i` (the mask keeps the index in range, so the check folds away).
#[inline(always)]
fn at(t: &[u32; 256], i: u32) -> u32 {
    t.get((i & 0xff) as usize).copied().unwrap_or(0)
}

/// CRC-32 of `data`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut h = Crc32::new();
    h.update(data);
    h.finish()
}

/// Incremental Adler-32 (RFC 1950).
#[derive(Clone, Copy, Debug)]
pub struct Adler32 {
    a: u32,
    b: u32,
}

impl Default for Adler32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Adler32 {
    const MOD: u32 = 65521;
    /// Largest n such that 255 n (n + 1) / 2 + (n + 1)(MOD - 1) fits in u32: the sums can be deferred that long.
    const NMAX: usize = 5552;

    pub fn new() -> Self {
        Adler32 { a: 1, b: 0 }
    }

    pub fn update(&mut self, data: &[u8]) {
        let (mut a, mut b) = (self.a, self.b);
        for block in data.chunks(Self::NMAX) {
            for &d in block {
                a += d as u32;
                b += a;
            }
            a %= Self::MOD;
            b %= Self::MOD;
        }
        self.a = a;
        self.b = b;
    }

    pub fn finish(&self) -> u32 {
        (self.b << 16) | self.a
    }
}

/// Adler-32 of `data`.
pub fn adler32(data: &[u8]) -> u32 {
    let mut h = Adler32::new();
    h.update(data);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414f_a339);
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn incremental_matches_one_shot() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let (mut c, mut a) = (Crc32::new(), Adler32::new());
        for part in data.chunks(777) {
            c.update(part);
            a.update(part);
        }
        assert_eq!(c.finish(), crc32(&data));
        assert_eq!(a.finish(), adler32(&data));
        // Bytewise reference for the sliced CRC.
        let mut r = 0xffff_ffffu32;
        for &b in &data {
            r = CRC_TABLES[0][((r ^ b as u32) & 0xff) as usize] ^ (r >> 8);
        }
        assert_eq!(r ^ 0xffff_ffff, crc32(&data));
        // Reference Adler with a modulo per byte, over 0xff bytes (worst case for deferred reduction).
        let ff = vec![0xffu8; 20_000];
        let (mut x, mut y) = (1u32, 0u32);
        for &d in &ff {
            x = (x + d as u32) % 65521;
            y = (y + x) % 65521;
        }
        assert_eq!((y << 16) | x, adler32(&ff));
    }
}

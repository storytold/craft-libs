//! Checksums of the PNG writer: CRC-32 (PNG chunks) and Adler-32 (zlib streams).

pub(super) use adler2::adler32_slice as adler32;
#[cfg(test)]
pub(super) use crc32fast::hash as crc32;

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
        let (mut c, mut a) = (crc32fast::Hasher::new(), adler2::Adler32::new());
        for part in data.chunks(777) {
            c.update(part);
            a.write_slice(part);
        }
        assert_eq!(c.finalize(), crc32(&data));
        assert_eq!(a.checksum(), adler32(&data));
        // Bitwise reference for CRC-32, with the reflected polynomial.
        let mut r = 0xffff_ffffu32;
        for &b in &data {
            r ^= u32::from(b);
            for _ in 0..8 {
                r = (r >> 1) ^ if r & 1 != 0 { 0xedb8_8320 } else { 0 };
            }
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

//! Test fixtures from Craft Anything's testkit: known-good compressed streams built with the
//! reference encoders (`xz`, `bzip2`, `zstd`), a stored-only ZIP writer and a noise generator.

#![allow(clippy::cast_possible_truncation, dead_code)]

use crate::lenient::crc32;

/// Plaintext of [`XZ_FIXTURE`] and [`LZMA_FIXTURE`].
pub const XZ_PLAIN: &[u8] = b"Hello, xz compression world!\n";
/// An xz stream of [`XZ_PLAIN`].
pub const XZ_FIXTURE: &[u8] = &[
    0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00, 0x00, 0x04, 0xe6, 0xd6, 0xb4, 0x46, 0x02, 0x00, 0x21, 0x01, 0x16, 0x00, 0x00, 0x00, 0x74, 0x2f, 0xe5, 0xa3,
    0x01, 0x00, 0x1c, 0x48, 0x65, 0x6c, 0x6c, 0x6f, 0x2c, 0x20, 0x78, 0x7a, 0x20, 0x63, 0x6f, 0x6d, 0x70, 0x72, 0x65, 0x73, 0x73, 0x69, 0x6f, 0x6e,
    0x20, 0x77, 0x6f, 0x72, 0x6c, 0x64, 0x21, 0x0a, 0x00, 0x00, 0x00, 0x00, 0x1d, 0x5f, 0x47, 0xab, 0x2a, 0x4c, 0xd2, 0xb2, 0x00, 0x01, 0x35, 0x1d,
    0x44, 0x1b, 0xb1, 0xe1, 0x1f, 0xb6, 0xf3, 0x7d, 0x01, 0x00, 0x00, 0x00, 0x00, 0x04, 0x59, 0x5a,
];
/// Plaintext of [`BZIP2_FIXTURE`].
pub const BZIP2_PLAIN: &[u8] = b"Hello, bzip2!\n";
/// A bzip2 stream of [`BZIP2_PLAIN`].
pub const BZIP2_FIXTURE: &[u8] = &[
    0x42, 0x5a, 0x68, 0x39, 0x31, 0x41, 0x59, 0x26, 0x53, 0x59, 0x04, 0xad, 0x5a, 0x6b, 0x00, 0x00, 0x02, 0xdd, 0x80, 0x00, 0x10, 0x60, 0x04, 0x10,
    0x00, 0x00, 0x40, 0x12, 0x24, 0xc0, 0x10, 0x20, 0x00, 0x22, 0x00, 0x03, 0x42, 0x01, 0xa0, 0x05, 0x67, 0xde, 0x89, 0xa1, 0xa0, 0x80, 0xf1, 0x77,
    0x24, 0x53, 0x85, 0x09, 0x00, 0x4a, 0xd5, 0xa6, 0xb0,
];
/// Plaintext of [`ZSTD_FIXTURE`].
pub const ZSTD_PLAIN: &[u8] = b"Hello, zstd!\n";
/// A Zstandard frame of [`ZSTD_PLAIN`].
pub const ZSTD_FIXTURE: &[u8] = &[
    0x28, 0xb5, 0x2f, 0xfd, 0x24, 0x0d, 0x69, 0x00, 0x00, 0x48, 0x65, 0x6c, 0x6c, 0x6f, 0x2c, 0x20, 0x7a, 0x73, 0x74, 0x64, 0x21, 0x0a, 0x41, 0xb4,
    0x53, 0xb2,
];

/// A stored-only ZIP of `files`.
pub fn zip_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut cd = Vec::new();
    for (name, body) in files {
        let off = out.len() as u32;
        let crc = crc32(body);
        let len = body.len() as u32;
        out.extend_from_slice(b"PK\x03\x04\x14\0\0\0\0\0\0\0\0\0");
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(body);
        cd.extend_from_slice(b"PK\x01\x02\x14\0\x14\0\0\0\0\0\0\0\0\0");
        cd.extend_from_slice(&crc.to_le_bytes());
        cd.extend_from_slice(&len.to_le_bytes());
        cd.extend_from_slice(&len.to_le_bytes());
        cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
        cd.extend_from_slice(&[0; 12]);
        cd.extend_from_slice(&off.to_le_bytes());
        cd.extend_from_slice(name.as_bytes());
    }
    let cd_off = out.len() as u32;
    out.extend_from_slice(&cd);
    out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// Deterministic pseudo-random bytes (xorshift) for fuzz-like loops.
pub fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

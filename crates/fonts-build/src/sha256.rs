//! SHA-256 (FIPS 180-4) of a font file, checked against its manifest line, and of a cut face, for
//! its served file name and Subresource Integrity value.

use std::io::{self, Read as _};
use std::path::Path;

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3,
    0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// A SHA-256 fed in pieces, so a font file is hashed as it is read rather than copied whole.
pub(crate) struct Sha256 {
    h: [u32; 8],
    /// Bytes of the next block not yet compressed: `block[..filled]`.
    block: [u8; 64],
    filled: usize,
    len: u64,
}

impl Sha256 {
    pub(crate) fn new() -> Self {
        Self {
            h: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19],
            block: [0; 64],
            filled: 0,
            len: 0,
        }
    }

    pub(crate) fn update(&mut self, mut data: &[u8]) {
        self.len = self.len.wrapping_add(u64::try_from(data.len()).unwrap_or(u64::MAX));
        if self.filled > 0 {
            let take = (64 - self.filled).min(data.len());
            let (head, rest) = data.split_at(take);
            if let Some(free) = self.block.get_mut(self.filled..self.filled + take) {
                free.copy_from_slice(head);
            }
            self.filled += take;
            data = rest;
            if self.filled < 64 {
                return;
            }
            compress(&mut self.h, &self.block);
            self.filled = 0;
        }
        let (blocks, rest) = data.as_chunks::<64>();
        for block in blocks {
            compress(&mut self.h, block);
        }
        if let Some(start) = self.block.get_mut(..rest.len()) {
            start.copy_from_slice(rest);
        }
        self.filled = rest.len();
    }

    /// The lowercase hex digest.
    pub(crate) fn finish(mut self) -> String {
        let bits = self.len.wrapping_mul(8);
        // A 0x80 byte and zeros up to 56 bytes into a block, then the length in bits.
        let padding = if self.filled < 56 { 56 - self.filled } else { 120 - self.filled };
        let mut tail = [0u8; 64];
        tail[0] = 0x80;
        self.update(tail.get(..padding).unwrap_or(&tail));
        self.update(&bits.to_be_bytes());
        self.h.iter().map(|v| format!("{v:08x}")).collect()
    }
}

fn compress(h: &mut [u32; 8], block: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
        w[i] = u32::from_be_bytes(*word);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
    }
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
    for (k, w) in K.iter().zip(w) {
        let t1 = hh
            .wrapping_add(e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25))
            .wrapping_add((e & f) ^ (!e & g))
            .wrapping_add(*k)
            .wrapping_add(w);
        let t2 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22)).wrapping_add((a & b) ^ (a & c) ^ (b & c));
        (hh, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
    }
    for (h, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
        *h = h.wrapping_add(v);
    }
}

/// The lowercase hex SHA-256 of `data`, as craft-fonts' manifest writes it.
#[cfg(any(feature = "subset", test))]
pub(crate) fn hex(data: &[u8]) -> String {
    let mut sha = Sha256::new();
    sha.update(data);
    sha.finish()
}

/// The lowercase hex SHA-256 of the regular file `path`, read in pieces; an error when it holds
/// more than `limit` bytes.
pub(crate) fn file_hex(path: &Path, limit: u64) -> io::Result<String> {
    // Opening a FIFO with no writer blocks, so refuse anything that isn't a file before opening.
    if !std::fs::metadata(path)?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let mut file = file.take(limit.saturating_add(1));
    let mut sha = Sha256::new();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        sha.update(buffer.get(..read).unwrap_or(&buffer));
    }
    if sha.len > limit {
        return Err(io::Error::other(format!("larger than {limit} bytes")));
    }
    Ok(sha.finish())
}

/// Whether `value` is a SHA-256 written as 64 hex digits.
pub(crate) fn is_hex_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

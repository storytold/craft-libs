//! Archive builders and damage generators for the tests, from Craft Anything's testkit.

#![allow(clippy::indexing_slicing, clippy::cast_possible_truncation)]

fn tar_header(name: &str, typeflag: u8, size: usize, link: &str) -> Vec<u8> {
    let mut h = vec![0u8; 512];
    let n = name.as_bytes();
    h[..n.len().min(100)].copy_from_slice(&n[..n.len().min(100)]);
    h[100..108].copy_from_slice(b"0000644\0");
    h[108..116].copy_from_slice(b"0000000\0");
    h[116..124].copy_from_slice(b"0000000\0");
    h[124..136].copy_from_slice(format!("{size:011o}\0").as_bytes());
    h[136..148].copy_from_slice(b"00000000000\0");
    h[156] = typeflag;
    let l = link.as_bytes();
    h[157..157 + l.len().min(100)].copy_from_slice(&l[..l.len().min(100)]);
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h
}

fn pad512(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(512) {
        v.push(0);
    }
}

/// A tar entry for [`tar_entries`]: (name, typeflag, data, link target).
pub type TarEntry<'a> = (&'a str, u8, &'a [u8], &'a str);

/// A ustar archive from raw entries; names longer than 100 bytes get a GNU long-name record.
pub fn tar_entries(entries: &[TarEntry<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, flag, data, link) in entries {
        if name.len() > 100 {
            let mut long = name.as_bytes().to_vec();
            long.push(0);
            out.extend(tar_header("././@LongLink", b'L', long.len(), ""));
            out.extend_from_slice(&long);
            pad512(&mut out);
        }
        out.extend(tar_header(name, *flag, data.len(), link));
        out.extend_from_slice(data);
        pad512(&mut out);
    }
    out.extend(vec![0u8; 1024]);
    out
}

/// A ustar archive of files (names ending in `/` become folders).
pub fn tar(files: &[(&str, &[u8])]) -> Vec<u8> {
    let e: Vec<TarEntry<'_>> = files.iter().map(|(n, d)| (*n, if n.ends_with('/') { b'5' } else { b'0' }, *d, "")).collect();
    tar_entries(&e)
}

/// A cpio newc archive from (name, data, mode). Symlinks (mode 0o120000) store the target as data.
pub fn cpio_newc(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut push = |name: &str, data: &[u8], mode: u32, ino: u32| {
        let hdr = format!(
            "070701{ino:08x}{mode:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
            0,
            0,
            1,
            0,
            data.len(),
            0,
            0,
            0,
            0,
            name.len() + 1,
            0
        );
        out.extend_from_slice(hdr.as_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
        out.extend_from_slice(data);
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
    };
    for (i, (n, d, m)) in files.iter().enumerate() {
        push(n, d, *m, i as u32 + 1);
    }
    push("TRAILER!!!", b"", 0, 0);
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

/// Every way to damage a sample used by robustness tests: truncations at many lengths and
/// single-byte corruptions spread across the file.
pub fn mutations(sample: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let n = sample.len();
    let step = (n / 64).max(1);
    let mut len = 0;
    while len < n {
        out.push(sample[..len].to_vec());
        len += step;
    }
    for (i, seed) in (0..n).step_by(step).zip(1u64..) {
        let mut m = sample.to_vec();
        let noise = noise(seed, 8);
        for (j, v) in noise.iter().enumerate() {
            if let Some(b) = m.get_mut(i + j) {
                *b = *v;
            }
        }
        out.push(m);
        let mut m = sample.to_vec();
        if let Some(b) = m.get_mut(i) {
            *b = 0xff;
        }
        out.push(m);
    }
    out
}

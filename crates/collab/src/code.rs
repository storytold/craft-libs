//! Human-readable join codes (spec §6.4): the v2 crypto browser, ported verbatim.
//!
//! The invite link carries a 128-bit secret; the *code* is what one person reads to another.
//! Online codes are 9 characters of Crockford base-32 (45 random bits — `K7M-2QX-9TD`); local
//! (LAN) codes are 20 characters (100 bits: IPv4 32 + port 16 + session secret 44 + an 8-bit
//! check that catches typos before any connection is attempted).
//!
//! The server never stores the code itself: it is stretched (SHA-256 under a fixed label, then
//! 10,000 further rounds); one hash of the result finds the link, another is the XOR pad that
//! wraps the link's token — so the code never reveals the token to anyone who doesn't know it.
//! Comparisons are constant-time everywhere (the family rule, spec §2.12).

use serde::{Deserialize, Serialize};

/// The Crockford base-32 alphabet: digits and capitals without I/L/O/U (readers confuse them);
/// reading ignores case, spaces and dashes, and maps O→0, I/L→1.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// The fixed SHA-256 label for online code stretching (spec §6.4).
pub const CODE_LABEL: &str = "crafthub-code-v1";

/// The number of extra SHA-256 rounds after the labelled hash (v2 value, verbatim).
pub const STRETCH_ROUNDS: usize = 10_000;

/// Online code length: 9 characters = 45 random bits.
pub const ONLINE_CODE_CHARS: usize = 9;

/// Local (LAN) code length: 20 characters = 100 bits including the 8-bit check.
pub const LOCAL_CODE_CHARS: usize = 20;

/// A join code's shape after normalization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodeKind {
    /// 9 characters — resolves online through the server (`Resolve`, spec §2.4).
    Online,
    /// 20 characters — carries host, port, secret and check for LAN joins.
    Local {
        /// The host address the code embeds.
        host: [u8; 4],
        /// The port the code embeds.
        port: u16,
    },
}

/// Why a code doesn't read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CodeError {
    /// Not decodable after normalization.
    #[error("not a valid join code")]
    Invalid,
    /// The 8-bit check failed — a typo was caught before any connection (LAN codes only).
    #[error("the code has a typo (check failed)")]
    CheckFailed,
}

/// Normalize raw typed input: uppercase, strip spaces and dashes, map O→0 and I/L→1.
#[must_use]
pub fn normalize(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| match c {
            'o' | 'O' => '0',
            'i' | 'I' | 'l' | 'L' => '1',
            other => other.to_ascii_uppercase(),
        })
        .collect()
}

/// Encode 5-bit groups into the Crockford alphabet.
fn encode_bits(bits: &[u8]) -> String {
    bits.iter().filter_map(|b| ALPHABET.get(usize::from(*b)).map(|c| *c as char)).collect()
}

/// Decode Crockford characters back to 5-bit groups (already normalized input).
fn decode_bits(s: &str) -> Option<Vec<u8>> {
    // The alphabet holds 32 entries, so every found position fits a u8 by construction.
    s.bytes().map(|c| ALPHABET.iter().position(|d| *d == c).and_then(|p| u8::try_from(p).ok())).collect()
}

/// An online code: 9 characters from 45 random bits (spec §6.4).
///
/// `entropy` must be cryptographically random (the caller injects its `Rng` — this function is
/// pure and deterministic so tests stay reproducible).
#[must_use]
pub fn encode_online(entropy45: u64) -> String {
    // Pack the 45 bits into 5-bit groups, most significant first.
    let mut groups: Vec<u8> = Vec::with_capacity(9);
    for g in 0..9 {
        let mut v = 0u8;
        for b in 0..5 {
            let bit = (entropy45 >> (44 - (g * 5 + b))) & 1;
            v = (v << 1) | bit as u8;
        }
        groups.push(v);
    }
    let s = encode_bits(&groups);
    format!("{}-{}-{}", &s[0..3], &s[3..6], &s[6..9])
}

/// Decode an online code back to its 45 bits.
pub fn decode_online(raw: &str) -> Result<u64, CodeError> {
    let n = normalize(raw);
    let bits = decode_bits(&n).ok_or(CodeError::Invalid)?;
    if bits.len() != ONLINE_CODE_CHARS {
        return Err(CodeError::Invalid);
    }
    let mut v = 0u64;
    for g in &bits {
        v = (v << 5) | u64::from(*g);
    }
    Ok(v & ((1 << 45) - 1))
}

/// The check byte over the LAN code's payload (catches typos before connecting).
fn lan_check(host: [u8; 4], port: u16, secret44: u64) -> u8 {
    let mut buf = Vec::with_capacity(14);
    buf.extend_from_slice(&host);
    buf.extend_from_slice(&port.to_be_bytes());
    buf.extend_from_slice(&secret44.to_be_bytes());
    Sha256::digest(&buf).0[0]
}

/// A local (LAN) code: 20 characters = host 32 + port 16 + secret 44 + check 8 (spec §6.4).
#[must_use]
pub fn encode_local(host: [u8; 4], port: u16, secret44: u64) -> String {
    let check = lan_check(host, port, secret44);
    let mut bits: Vec<bool> = Vec::with_capacity(100);
    push_bits(&mut bits, u64::from(u32::from_be_bytes(host)), 32);
    push_bits(&mut bits, u64::from(port), 16);
    push_bits(&mut bits, secret44 & ((1 << 44) - 1), 44);
    push_bits(&mut bits, u64::from(check), 8);
    let mut groups: Vec<u8> = Vec::with_capacity(20);
    for g in 0..20 {
        let mut v = 0u8;
        for b in 0..5 {
            let idx = g * 5 + b;
            v = (v << 1) | u8::from(bits.get(idx).copied().unwrap_or(false));
        }
        groups.push(v);
    }
    let s = encode_bits(&groups);
    format!("{}-{}-{}-{}", &s[0..5], &s[5..10], &s[10..15], &s[15..20])
}

/// Decode a local code into its parts, verifying the check byte.
pub fn decode_local(raw: &str) -> Result<([u8; 4], u16, u64), CodeError> {
    let n = normalize(raw);
    let bits = decode_bits(&n).ok_or(CodeError::Invalid)?;
    if bits.len() != LOCAL_CODE_CHARS {
        return Err(CodeError::Invalid);
    }
    let mut v: u128 = 0;
    for g in &bits {
        v = (v << 5) | u128::from(*g);
    }
    v &= (1 << 100) - 1;
    let host = ((v >> 68) & 0xFFFF_FFFF) as u32;
    let host = host.to_be_bytes();
    let port = ((v >> 52) & 0xFFFF) as u16;
    let secret = ((v >> 8) & ((1u128 << 44) - 1)) as u64;
    let check = (v & 0xFF) as u8;
    if lan_check(host, port, secret) != check {
        return Err(CodeError::CheckFailed);
    }
    Ok((host, port, secret))
}

fn push_bits(out: &mut Vec<bool>, value: u64, width: usize) {
    for i in (0..width).rev() {
        out.push((value >> i) & 1 == 1);
    }
}

/// The stretched derivation of a code (spec §6.4): `h2 = SHA256^10000(SHA256(label || code))`,
/// returning `(lookup_hash, pad_hash)` — one finds the link, the other wraps its token.
///
/// Pure over injected SHA-256 state so tests are deterministic.
#[must_use]
pub fn stretch(code: &str) -> (String, String) {
    let labelled = format!("{CODE_LABEL}{code}");
    let h1 = Sha256::digest(labelled.as_bytes());
    let mut h2 = h1;
    for _ in 0..STRETCH_ROUNDS {
        h2 = Sha256::digest(&h2.0);
    }
    let lookup = Sha256::hex(&h2);
    let pad = Sha256::hex(&Sha256::digest(&h2.0));
    (lookup, pad)
}

/// A minimal, dependency-free SHA-256 (the `sha2` feature stays optional for the core; the code
/// derivation needs the primitive with zero allocation beyond the digest).
///
/// Implementation note: this is the standard FIPS 180-4 construction, written out once so the
/// core keeps no external crypto dependency; production servers use the `sha2` crate via
/// `crafthub-proto` for the same results.
#[derive(Clone, Copy)]
struct Sha256([u8; 32]);

impl Sha256 {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    #[allow(clippy::many_single_char_names, clippy::needless_range_loop, clippy::indexing_slicing)]
    fn digest(data: &[u8]) -> Self {
        let mut h: [u32; 8] = [0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19];
        let mut msg = data.to_vec();
        let bit_len = (data.len() as u64).wrapping_mul(8);
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bit_len.to_be_bytes());
        for chunk in msg.chunks(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
            }
            let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) = (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(Self::K[i]).wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
            h[5] = h[5].wrapping_add(f);
            h[6] = h[6].wrapping_add(g);
            h[7] = h[7].wrapping_add(hh);
        }
        let mut out = [0u8; 32];
        for (i, v) in h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
        }
        Sha256(out)
    }

    fn hex(s: &Self) -> String {
        use std::fmt::Write as _;
        // A digest is always 32 bytes -> 64 hex chars; write! avoids the per-byte allocation.
        let mut out = String::with_capacity(64);
        for b in s.0 {
            let _ = write!(out, "{b:02x}");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The code examples from the spec: 9-char online codes read like `K7M-2QX-9TD`.
    #[test]
    fn online_roundtrip() {
        let code = encode_online(0x1234_5678_9ABC & ((1 << 45) - 1));
        assert_eq!(code.len(), 11); // 9 chars + 2 dashes
        let v = decode_online(&code).expect("decode");
        assert_eq!(v, 0x1234_5678_9ABC & ((1 << 45) - 1));
    }

    /// Normalization: case, spaces, dashes, O/0 and I/L/1 confusions all read the same.
    #[test]
    fn normalization_is_forgiving() {
        let code = encode_online(987_654_321);
        let mangled = code.to_lowercase().replace('-', " ");
        let mangled = mangled.replace('0', "o").replace('1', "l");
        assert_eq!(decode_online(&mangled).expect("decode"), decode_online(&code).expect("decode"));
    }

    /// LAN codes catch typos via the check byte before any connection (spec §6.4).
    #[test]
    fn local_roundtrip_and_check() {
        let code = encode_local([192, 168, 1, 20], 47900, 0x0ABC_DEF123);
        let (host, port, secret) = decode_local(&code).expect("decode");
        assert_eq!(host, [192, 168, 1, 20]);
        assert_eq!(port, 47900);
        assert_eq!(secret, 0x0ABC_DEF123);
        // Flip one character: the check must catch it (probabilistically certain for a single
        // char swap within the payload).
        let mut broken = normalize(&code);
        broken.replace_range(6..7, if broken.as_bytes()[6] == b'0' { "1" } else { "0" });
        assert!(decode_local(&broken).is_err());
    }

    /// The stretch derivation is stable and the pad differs from the lookup.
    #[test]
    fn stretch_is_deterministic() {
        let (a1, p1) = stretch("K7M2QX9TD");
        let (a2, p2) = stretch("K7M2QX9TD");
        assert_eq!(a1, a2);
        assert_eq!(p1, p2);
        assert_ne!(a1, p1);
        let (a3, _) = stretch("K7M2QX9TE");
        assert_ne!(a1, a3);
    }
}

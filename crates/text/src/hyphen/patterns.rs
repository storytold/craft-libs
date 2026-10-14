//! Liang hyphenation patterns (TeX notation, e.g. `.ab1c`, `4t1ion`): parsing and application.
//!
//! The bundled English set is *ours*: trained from the public-domain Moby Hyphenator word list by
//! the crate's patgen-style trainer (see `examples/hyphgen.rs`). Patterns are stored in a hash map keyed by the
//! packed letter string, so applying them to a word costs `O(len × max_pattern_len)` lookups.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Word-boundary marker in patterns.
pub(crate) const DOT: char = '.';
/// Longest pattern the packed key supports.
pub(crate) const MAX_PATTERN_LEN: usize = 9;

/// Small, fast multiplicative hasher for packed `u64` keys (FxHash-style).
#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher(u64);

impl Hasher for FxHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

pub(crate) type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

/// Maps characters to 6-bit codes (1 = word boundary, 2.. = letters); 0 = not in the alphabet.
#[derive(Clone, Debug)]
pub(crate) struct Alphabet {
    ascii: [u8; 128],
    other: Vec<(char, u8)>,
    next: u8,
}

impl Default for Alphabet {
    fn default() -> Self {
        let mut a = Alphabet { ascii: [0; 128], other: Vec::new(), next: 2 };
        a.ascii[DOT as usize] = 1;
        a
    }
}

impl Alphabet {
    pub(crate) fn code(&self, c: char) -> u8 {
        if (c as u32) < 128 { self.ascii[c as usize] } else { self.other.iter().find(|x| x.0 == c).map_or(0, |x| x.1) }
    }
    /// Code for `c`, adding it if new (None when the 6-bit alphabet is full).
    pub(crate) fn intern(&mut self, c: char) -> Option<u8> {
        let k = self.code(c);
        if k != 0 {
            return Some(k);
        }
        if self.next >= 63 {
            return None;
        }
        let k = self.next;
        self.next += 1;
        if (c as u32) < 128 {
            self.ascii[c as usize] = k;
        } else {
            self.other.push((c, k));
        }
        Some(k)
    }
}

/// Packed key of a code string (≤ [`MAX_PATTERN_LEN`] codes).
#[inline]
pub(crate) fn pack(codes: &[u8]) -> u64 {
    let mut k = 0u64;
    for &c in codes {
        k = (k << 6) | c as u64;
    }
    k | ((codes.len() as u64) << 58)
}

/// A compiled pattern set.
#[derive(Clone, Debug, Default)]
pub struct Patterns {
    alpha: Alphabet,
    /// Packed letters → inter-letter values (`len + 1` entries).
    map: FxMap<u64, Box<[u8]>>,
    max_len: usize,
}

impl Patterns {
    /// Parse TeX-style patterns: whitespace-separated, `%`/`#` start a comment line.
    pub fn parse(text: &str) -> Patterns {
        let mut p = Patterns::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('%') || line.starts_with('#') {
                continue;
            }
            for tok in line.split_whitespace() {
                p.insert(tok);
            }
        }
        p
    }

    /// Add one pattern like `a1b`; returns false if it can't be represented.
    pub(crate) fn insert(&mut self, pat: &str) -> bool {
        let mut codes = Vec::with_capacity(pat.len());
        let mut vals = vec![0u8];
        for c in pat.chars() {
            if let Some(d) = c.to_digit(10) {
                *vals.last_mut().unwrap_or(&mut 0) = d as u8;
            } else {
                let Some(k) = self.alpha.intern(c) else { return false };
                codes.push(k);
                vals.push(0);
            }
        }
        if codes.is_empty() || codes.len() > MAX_PATTERN_LEN {
            return false;
        }
        self.max_len = self.max_len.max(codes.len());
        self.map.insert(pack(&codes), vals.into_boxed_slice());
        true
    }

    #[cfg(feature = "generator")]
    pub(crate) fn from_parts(alpha: Alphabet, map: FxMap<u64, Box<[u8]>>, max_len: usize) -> Patterns {
        Patterns { alpha, map, max_len }
    }

    /// Make `letters` part of the alphabet even if no pattern mentions them.
    #[cfg(feature = "generator")]
    pub(crate) fn add_letters(&mut self, letters: impl IntoIterator<Item = char>) {
        for c in letters {
            let _ = self.alpha.intern(c);
        }
    }

    #[cfg(any(test, feature = "generator"))]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Inter-letter values for a lowercase word: `out[i]` is the value *before* char `i`
    /// (`out.len() == word.len() + 1`). None if a char is outside the pattern alphabet.
    pub(crate) fn values(&self, word: &[char]) -> Option<Vec<u8>> {
        let mut codes = Vec::with_capacity(word.len() + 2);
        codes.push(1);
        for &c in word {
            let k = self.alpha.code(c);
            if k == 0 {
                return None;
            }
            codes.push(k);
        }
        codes.push(1);
        let mut vals = vec![0u8; codes.len() + 1];
        apply(&self.map, self.max_len, &codes, &mut vals);
        // Padded position p sits before padded char p; word char i is padded char i + 1.
        Some(vals[1..codes.len()].to_vec())
    }

    /// Break points (char indices, a hyphen goes before that char) of a lowercase word.
    pub fn points(&self, word: &[char]) -> Option<Vec<usize>> {
        let v = self.values(word)?;
        Some((1..word.len()).filter(|&i| v[i] % 2 == 1).collect())
    }

    /// The patterns in TeX notation (sorted by letters), e.g. for writing a pattern file.
    #[cfg(any(test, feature = "generator"))]
    pub fn to_tex(&self) -> Vec<String> {
        let mut rev: Vec<(u8, char)> =
            (0u8..128).filter(|&c| self.alpha.ascii[c as usize] != 0).map(|c| (self.alpha.ascii[c as usize], c as char)).collect();
        rev.extend(self.alpha.other.iter().map(|&(c, k)| (k, c)));
        let ch = |k: u8| rev.iter().find(|x| x.0 == k).map_or('?', |x| x.1);
        let mut out: Vec<(String, String)> = self
            .map
            .iter()
            .map(|(&key, vals)| {
                let n = (key >> 58) as usize;
                let letters: Vec<char> = (0..n).map(|i| ch(((key >> (6 * (n - 1 - i))) & 63) as u8)).collect();
                let mut s = String::new();
                for (i, l) in letters.iter().enumerate() {
                    if vals[i] > 0 {
                        s.push(char::from(b'0' + vals[i]));
                    }
                    s.push(*l);
                }
                if vals[n] > 0 {
                    s.push(char::from(b'0' + vals[n]));
                }
                (letters.into_iter().collect(), s)
            })
            .collect();
        out.sort();
        out.into_iter().map(|x| x.1).collect()
    }
}

/// Apply `map` to padded `codes`, raising `vals` (len = codes.len() + 1).
#[inline]
pub(crate) fn apply(map: &FxMap<u64, Box<[u8]>>, max_len: usize, codes: &[u8], vals: &mut [u8]) {
    let n = codes.len();
    for s in 0..n {
        let mut k = 0u64;
        for len in 1..=max_len.min(n - s) {
            k = (k << 6) | codes[s + len - 1] as u64;
            if let Some(v) = map.get(&(k | ((len as u64) << 58))) {
                for (slot, &x) in vals.iter_mut().skip(s).zip(v.iter()) {
                    *slot = (*slot).max(x);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liang_example() {
        // The classic example from Liang's thesis: "hy-phen-ation".
        let p = Patterns::parse("hy3ph he2n hena4 hen5at 1na n2at 1tio 2io o2n");
        let w: Vec<char> = "hyphenation".chars().collect();
        assert_eq!(p.points(&w).unwrap(), vec![2, 6]);
        let mut tex = p.to_tex();
        tex.sort();
        assert!(tex.contains(&"hen5at".to_string()));
        assert!(p.values(&['h', 'é']).is_none());
    }
}

//! LZ77 match finding with hash chains over the 32 KiB window, and the greedy/lazy parse built on it.

use super::tables::{MAX_MATCH, MIN_MATCH, WINDOW};

const HASH_BITS: u32 = 15;
const HASH_SIZE: usize = 1 << HASH_BITS;
const WINDOW_MASK: usize = WINDOW - 1;
/// Length-3 matches further back than this cost more than three literals.
const TOO_FAR: usize = 4096;

/// Search effort knobs (the same ones zlib tunes per level).
#[derive(Clone, Copy, Debug)]
pub(super) struct Params {
    /// Maximum hash-chain links followed per search.
    pub max_chain: usize,
    /// Stop searching once a match this long is found.
    pub nice_len: usize,
    /// Lazy parse only: when the pending match is at least this long, search a quarter of the chain.
    pub good_len: usize,
    /// Lazy parse: matches at least this long are taken without looking one byte ahead. 0 = greedy parse.
    pub max_lazy: usize,
    /// Greedy parse: positions inside matches longer than this are not added to the hash chains (speed).
    pub max_insert: usize,
}

/// Receives the parse.
pub(super) trait TokenSink {
    fn literal(&mut self, byte: u8);
    fn copy(&mut self, len: usize, dist: usize);
}

/// Hash chains: `head[h]` is the newest position with hash h, `prev[p % WINDOW]` the previous one with the same hash.
/// Positions are stored plus one so that 0 means "none".
struct Chains {
    head: Vec<usize>,
    prev: Vec<usize>,
}

impl Chains {
    fn new() -> Self {
        Chains { head: vec![0; HASH_SIZE], prev: vec![0; WINDOW] }
    }

    #[inline]
    fn hash(data: &[u8], pos: usize) -> usize {
        let Some(&[a, b, c]) = data.get(pos..pos.saturating_add(3)) else {
            return 0;
        };
        let v = (a as u32) | (b as u32) << 8 | (c as u32) << 16;
        (v.wrapping_mul(0x9e37_79b1) >> (32 - HASH_BITS)) as usize
    }

    /// Link `pos` into its chain (needs three bytes of input from `pos`).
    #[inline]
    fn insert(&mut self, data: &[u8], pos: usize) {
        if pos + MIN_MATCH <= data.len() {
            let h = Self::hash(data, pos);
            if let (Some(head), Some(prev)) = (self.head.get_mut(h), self.prev.get_mut(pos & WINDOW_MASK)) {
                *prev = *head;
                *head = pos + 1;
            }
        }
    }

    /// Longest match for `pos` (which must already be inserted) that is longer than `better_than`.
    fn longest(&self, data: &[u8], pos: usize, better_than: usize, mut chain: usize, nice: usize) -> Option<(usize, usize)> {
        let max_len = MAX_MATCH.min(data.len() - pos);
        if max_len < MIN_MATCH || better_than >= max_len {
            return None;
        }
        let nice = nice.min(max_len);
        let mut best = (better_than, 0usize);
        let mut cand = self.link(pos);
        while cand != 0 && chain > 0 {
            let c = cand - 1;
            let dist = pos - c;
            if dist > WINDOW {
                break;
            }
            // Cheap reject: the byte that would extend the best match must agree.
            if data.get(c + best.0) == data.get(pos + best.0) {
                let len = match_len(data, c, pos, max_len);
                if len > best.0 && (len > MIN_MATCH || dist <= TOO_FAR) {
                    best = (len, dist);
                    if len >= nice {
                        break;
                    }
                }
            }
            let next = self.link(c);
            // A slot overwritten by a newer position ends the chain (positions only decrease along it).
            if next >= cand {
                break;
            }
            cand = next;
            chain -= 1;
        }
        (best.1 != 0).then_some(best)
    }

    /// `prev` entry of position `pos` (0 = none).
    #[inline]
    fn link(&self, pos: usize) -> usize {
        self.prev.get(pos & WINDOW_MASK).copied().unwrap_or(0)
    }
}

/// Length of the common prefix of `data[a..]` and `data[b..]`, at most `max` (a < b, b + max <= len).
#[inline]
fn match_len(data: &[u8], a: usize, b: usize, max: usize) -> usize {
    let (Some(x), Some(y)) = (data.get(a..a.saturating_add(max)), data.get(b..b.saturating_add(max))) else {
        return 0;
    };
    let ((xw, xr), (yw, yr)) = (x.as_chunks::<8>(), y.as_chunks::<8>());
    let mut n = 0;
    for (&p, &q) in xw.iter().zip(yw) {
        let (p, q) = (u64::from_le_bytes(p), u64::from_le_bytes(q));
        if p != q {
            return n + ((p ^ q).trailing_zeros() / 8) as usize;
        }
        n += 8;
    }
    n + xr.iter().zip(yr).take_while(|(p, q)| p == q).count()
}

/// Parse `data` into literals and back-references.
pub(super) fn parse(data: &[u8], p: Params, sink: &mut impl TokenSink) {
    let mut chains = Chains::new();
    if p.max_lazy == 0 {
        greedy(data, p, &mut chains, sink);
    } else {
        lazy(data, p, &mut chains, sink);
    }
}

fn greedy(data: &[u8], p: Params, chains: &mut Chains, sink: &mut impl TokenSink) {
    let mut pos = 0;
    while let Some(&byte) = data.get(pos) {
        chains.insert(data, pos);
        match chains.longest(data, pos, MIN_MATCH - 1, p.max_chain, p.nice_len) {
            Some((len, dist)) => {
                sink.copy(len, dist);
                if len <= p.max_insert {
                    for q in pos + 1..pos + len {
                        chains.insert(data, q);
                    }
                }
                pos += len;
            }
            None => {
                sink.literal(byte);
                pos += 1;
            }
        }
    }
}

/// zlib-style lazy evaluation: a match is only taken if the match starting one byte later is not longer.
fn lazy(data: &[u8], p: Params, chains: &mut Chains, sink: &mut impl TokenSink) {
    let mut pos = 0;
    // Match found at pos - 1 whose emission was deferred.
    let mut pending: Option<(usize, usize)> = None;
    while let Some(&byte) = data.get(pos) {
        chains.insert(data, pos);
        let (better_than, chain) = match pending {
            Some((len, _)) if len >= p.good_len => (len, p.max_chain / 4),
            Some((len, _)) => (len, p.max_chain),
            None => (MIN_MATCH - 1, p.max_chain),
        };
        let skip_search = matches!(pending, Some((len, _)) if len >= p.max_lazy);
        let cur = if skip_search { None } else { chains.longest(data, pos, better_than, chain, p.nice_len) };
        match (pending, cur) {
            (Some((len, dist)), None) => {
                // The deferred match wins: it started at pos - 1.
                sink.copy(len, dist);
                let end = pos - 1 + len;
                for q in pos + 1..end {
                    chains.insert(data, q);
                }
                pos = end;
                pending = None;
            }
            (Some(_), Some(m)) => {
                // pos >= 1 whenever a match is pending.
                sink.literal(data.get(pos.wrapping_sub(1)).copied().unwrap_or(0));
                pending = Some(m);
                pos += 1;
            }
            (None, Some(m)) if m.0 < p.max_lazy => {
                pending = Some(m);
                pos += 1;
            }
            (None, Some((len, dist))) => {
                sink.copy(len, dist);
                for q in pos + 1..pos + len {
                    chains.insert(data, q);
                }
                pos += len;
            }
            (None, None) => {
                sink.literal(byte);
                pos += 1;
            }
        }
    }
    if let Some((len, dist)) = pending {
        sink.copy(len, dist);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rebuilds the input from the parse, checking every token is legal.
    struct Replay(Vec<u8>);

    impl TokenSink for Replay {
        fn literal(&mut self, b: u8) {
            self.0.push(b);
        }
        fn copy(&mut self, len: usize, dist: usize) {
            assert!((MIN_MATCH..=MAX_MATCH).contains(&len) && (1..=WINDOW).contains(&dist) && dist <= self.0.len());
            for _ in 0..len {
                let b = self.0[self.0.len() - dist];
                self.0.push(b);
            }
        }
    }

    #[test]
    fn parses_reproduce_input() {
        let mut data = Vec::new();
        let mut s = 1u32;
        for i in 0..200_000u32 {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
            data.push(if i % 1000 < 500 { (s >> 24) as u8 & 7 } else { b"abcabcabd"[(i % 9) as usize] });
        }
        for max_lazy in [0, 16, 258] {
            let p = Params { max_chain: 64, nice_len: 128, good_len: 8, max_lazy, max_insert: 8 };
            let mut r = Replay(Vec::new());
            parse(&data, p, &mut r);
            assert!(r.0 == data, "max_lazy {max_lazy}");
        }
    }
}

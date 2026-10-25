//! Fractional indexing: sibling order as sortable string keys (spec §5.2).
//!
//! A parent's children are ordered by `(position key, id)`. A key can always be made between two
//! others, so inserting or moving one layer writes one key and never renumbers its siblings, and
//! two people inserting into the same frame at the same time never conflict (equal keys fall back
//! to id order, which every replica agrees on).
//!
//! A key is an *integer part* followed by an optional *fraction*, all in base-62 digits
//! (`0-9A-Za-z`, which sort the same as bytes). The integer part's first character gives its
//! length: `a`-`z` are 2..=27 digits counting up, `A`-`Z` are 27..=2 digits counting down below
//! zero. The first key is `a0`; appending counts up (`a1`, `a2` ... `az`, `b00` ...), so keys made
//! by appending grow logarithmically; inserting between two keys extends the fraction (between
//! `a1` and `a2` sits `a1V`). Fractions never end in `0`, so there is always room between two
//! keys. Every function is deterministic: the same inputs give the same key on every machine.
//!
//! Ported from the proven figcraft-collab `frac` module with its semantics intact (spec §2.15);
//! property guarantees P1-P3 of spec §5.2 are tested in `tests/frac_tests.rs`.

use std::fmt;

/// The digits, in sort order.
const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = 62;

/// Keys longer than this are refused (a replica re-spaces a parent's children before its keys get
/// this long; see [`KEY_RESPACE_LEN`]).
pub const MAX_KEY_LEN: usize = 128;

/// A generated key longer than this makes the generator re-space all of a parent's children with
/// short keys instead (spec §5.2: re-spacing is rare — under 0.1% of changes — and never lost).
pub const KEY_RESPACE_LEN: usize = 48;

/// The key a first child gets.
pub const FIRST_KEY: &str = "a0";

/// Why a key is invalid or no key fits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyError {
    /// Not a well-formed key (the string is included, shortened).
    Invalid(String),
    /// The bounds are not in increasing order.
    Order,
    /// No key fits (below the smallest representable key, or above the largest).
    Exhausted,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyError::Invalid(k) => write!(f, "invalid position key {k:?}"),
            KeyError::Order => write!(f, "position keys out of order"),
            KeyError::Exhausted => write!(f, "no position key fits"),
        }
    }
}

impl std::error::Error for KeyError {}

fn digit_value(c: u8) -> Option<usize> {
    match c {
        b'0'..=b'9' => Some(usize::from(c - b'0')),
        b'A'..=b'Z' => Some(usize::from(c - b'A') + 10),
        b'a'..=b'z' => Some(usize::from(c - b'a') + 36),
        _ => None,
    }
}

fn digit(v: usize) -> u8 {
    DIGITS.get(v).copied().unwrap_or(b'0')
}

/// Length of the integer part (head included) for head character `h`.
fn int_len(h: u8) -> Option<usize> {
    match h {
        b'a'..=b'z' => Some(usize::from(h - b'a') + 2),
        b'A'..=b'Z' => Some(usize::from(b'Z' - h) + 2),
        _ => None,
    }
}

/// The smallest integer part ("A" followed by 26 zeros): keys can't go below it.
fn is_smallest_int(i: &[u8]) -> bool {
    i.first() == Some(&b'A') && i.len() == 27 && i.iter().skip(1).all(|c| *c == b'0')
}

fn shorten(k: &str) -> String {
    k.chars().take(40).collect()
}

/// Split a valid key into its integer part and fraction.
fn split(key: &str) -> Result<(&[u8], &[u8]), KeyError> {
    let b = key.as_bytes();
    let n = b.first().and_then(|h| int_len(*h)).ok_or_else(|| KeyError::Invalid(shorten(key)))?;
    if b.len() < n {
        return Err(KeyError::Invalid(shorten(key)));
    }
    Ok(b.split_at(n))
}

/// Check that `key` is a well-formed position key.
pub fn validate(key: &str) -> Result<(), KeyError> {
    let bad = || KeyError::Invalid(shorten(key));
    if key.len() > MAX_KEY_LEN || !key.is_ascii() {
        return Err(bad());
    }
    let (int, frac) = split(key)?;
    if int.iter().skip(1).chain(frac).any(|c| digit_value(*c).is_none()) || frac.last() == Some(&b'0') {
        return Err(bad());
    }
    if is_smallest_int(int) && frac.is_empty() {
        return Err(bad());
    }
    Ok(())
}

fn to_string(v: Vec<u8>) -> String {
    // Keys are built from ASCII digits only.
    String::from_utf8(v).unwrap_or_default()
}

/// A fraction strictly between fractions `a` and `b` (`None`: unbounded), neither ending in `0`,
/// `a < b`.
fn midpoint(a: &[u8], b: Option<&[u8]>) -> Result<Vec<u8>, KeyError> {
    let mut out = Vec::new();
    let (mut a, mut b) = (a, b);
    // Each round consumes at least one digit of `a` or `b`, so this ends; the cap guards hostile input.
    for _ in 0..(MAX_KEY_LEN * 2 + 4) {
        if let Some(bb) = b {
            // Shared prefix (missing digits of `a` read as 0).
            let mut n = 0;
            while n < bb.len() && a.get(n).copied().unwrap_or(b'0') == bb.get(n).copied().unwrap_or(0) {
                n += 1;
            }
            if n > 0 {
                out.extend_from_slice(bb.get(..n).unwrap_or_default());
                a = a.get(n..).unwrap_or_default();
                b = Some(bb.get(n..).unwrap_or_default());
                continue;
            }
        }
        let da = match a.first() {
            Some(c) => digit_value(*c).ok_or(KeyError::Order)?,
            None => 0,
        };
        let db = match b {
            Some(bb) => bb.first().and_then(|c| digit_value(*c)).ok_or(KeyError::Order)?,
            None => BASE,
        };
        if db <= da {
            return Err(KeyError::Order);
        }
        if db - da > 1 {
            out.push(digit((da + db).div_ceil(2)));
            return Ok(out);
        }
        // Consecutive digits: `b`'s first digit alone is between when `b` goes on; otherwise keep
        // `a`'s digit and find a fraction above the rest of `a`.
        if let Some(bb) = b
            && bb.len() > 1
        {
            out.push(bb.first().copied().unwrap_or(b'0'));
            return Ok(out);
        }
        out.push(digit(da));
        a = a.get(1..).unwrap_or_default();
        b = None;
    }
    Err(KeyError::Exhausted)
}

/// The integer part after `int`, or `None` past the largest.
fn increment_int(int: &[u8]) -> Option<Vec<u8>> {
    let (&head, digits) = int.split_first()?;
    let mut d = digits.to_vec();
    let mut carry = true;
    for c in d.iter_mut().rev() {
        let v = digit_value(*c)? + 1;
        if v == BASE {
            *c = b'0';
        } else {
            *c = digit(v);
            carry = false;
            break;
        }
    }
    if !carry {
        let mut out = vec![head];
        out.extend(d);
        return Some(out);
    }
    match head {
        b'Z' => Some(b"a0".to_vec()),
        b'z' => None,
        _ => {
            let h = head + 1;
            if h > b'a' {
                d.push(b'0');
            } else {
                d.pop();
            }
            let mut out = vec![h];
            out.extend(d);
            Some(out)
        }
    }
}

/// The integer part before `int`, or `None` below the smallest.
fn decrement_int(int: &[u8]) -> Option<Vec<u8>> {
    let (&head, digits) = int.split_first()?;
    let mut d = digits.to_vec();
    let mut borrow = true;
    for c in d.iter_mut().rev() {
        let v = digit_value(*c)?;
        if v == 0 {
            *c = digit(BASE - 1);
        } else {
            *c = digit(v - 1);
            borrow = false;
            break;
        }
    }
    if !borrow {
        let mut out = vec![head];
        out.extend(d);
        return Some(out);
    }
    match head {
        b'a' => Some(vec![b'Z', digit(BASE - 1)]),
        b'A' => None,
        _ => {
            let h = head - 1;
            if h < b'Z' {
                d.push(digit(BASE - 1));
            } else {
                d.pop();
            }
            let mut out = vec![h];
            out.extend(d);
            Some(out)
        }
    }
}

/// A key strictly between `a` and `b` (`None`: unbounded on that side). Both must be valid and
/// `a < b`.
pub fn between(a: Option<&str>, b: Option<&str>) -> Result<String, KeyError> {
    for k in [a, b].into_iter().flatten() {
        validate(k)?;
    }
    if let (Some(a), Some(b)) = (a, b)
        && a >= b
    {
        return Err(KeyError::Order);
    }
    match (a, b) {
        (None, None) => Ok(FIRST_KEY.to_string()),
        (None, Some(b)) => {
            let (ib, fb) = split(b)?;
            if is_smallest_int(ib) {
                let mut out = ib.to_vec();
                out.extend(midpoint(b"", Some(fb))?);
                return Ok(to_string(out));
            }
            if !fb.is_empty() {
                return Ok(to_string(ib.to_vec()));
            }
            let d = decrement_int(ib).ok_or(KeyError::Exhausted)?;
            if is_smallest_int(&d) {
                // The smallest integer alone isn't a key: put a fraction after it.
                let mut out = d;
                out.push(digit(BASE - 1));
                return Ok(to_string(out));
            }
            Ok(to_string(d))
        }
        (Some(a), None) => {
            let (ia, fa) = split(a)?;
            if let Some(i) = increment_int(ia) {
                Ok(to_string(i))
            } else {
                let mut out = ia.to_vec();
                out.extend(midpoint(fa, None)?);
                Ok(to_string(out))
            }
        }
        (Some(a), Some(b)) => {
            let (ia, fa) = split(a)?;
            let (ib, fb) = split(b)?;
            if ia == ib {
                let mut out = ia.to_vec();
                out.extend(midpoint(fa, Some(fb))?);
                return Ok(to_string(out));
            }
            if let Some(i) = increment_int(ia)
                && i.as_slice() < b.as_bytes()
            {
                return Ok(to_string(i));
            }
            let mut out = ia.to_vec();
            out.extend(midpoint(fa, None)?);
            Ok(to_string(out))
        }
    }
}

/// `n` keys in increasing order, all strictly between `a` and `b` (`None`: unbounded). Bisects, so
/// keys stay short however many there are.
pub fn n_between(a: Option<&str>, b: Option<&str>, n: usize) -> Result<Vec<String>, KeyError> {
    let mut out = Vec::with_capacity(n.min(1 << 16));
    fill(a, b, n, &mut out, 0)?;
    Ok(out)
}

fn fill(a: Option<&str>, b: Option<&str>, n: usize, out: &mut Vec<String>, depth: usize) -> Result<(), KeyError> {
    if n == 0 {
        return Ok(());
    }
    if depth > 64 {
        return Err(KeyError::Exhausted);
    }
    match (a, b) {
        // Unbounded above: count up (integer keys stay short).
        (_, None) => {
            let mut c = between(a, None)?;
            for _ in 1..n {
                let next = between(Some(&c), None)?;
                out.push(std::mem::replace(&mut c, next));
            }
            out.push(c);
            Ok(())
        }
        // Unbounded below: count down, then reverse.
        (None, Some(_)) => {
            let start = out.len();
            let mut c = between(None, b)?;
            for _ in 1..n {
                let next = between(None, Some(&c))?;
                out.push(std::mem::replace(&mut c, next));
            }
            out.push(c);
            if let Some(tail) = out.get_mut(start..) {
                tail.reverse();
            }
            Ok(())
        }
        (Some(_), Some(_)) => {
            let mid = n / 2;
            let c = between(a, b)?;
            fill(a, Some(&c), mid, out, depth + 1)?;
            let rest = n - mid - 1;
            out.push(c.clone());
            fill(Some(&c), b, rest, out, depth + 1)
        }
    }
}

/// Re-space a parent's children: issue fresh, well-spread keys for every child whose key has
/// reached the [`KEY_RESPACE_LEN`] threshold, keeping short keys in place (spec §5.2 — respace
/// is rare, under 0.1% of changes, and is synced as one ordinary batch of position writes).
///
/// `keys` must be the parent's children in `(key, id)` order. Returns the `(old, new)` pairs.
pub fn respace(keys: &[(String, NodeKeyOwner)]) -> Result<Vec<(String, String)>, KeyError> {
    if !keys.iter().any(|(k, _)| k.len() >= KEY_RESPACE_LEN) {
        return Ok(Vec::new());
    }
    // Fresh evenly-spread keys, one per child, then shift only the long ones onto them.
    let bounds = n_between(None, None, keys.len())?;
    let mut out = Vec::with_capacity(keys.len());
    let mut last: Option<&str> = None;
    let mut bi = 0usize;
    for (k, _) in keys {
        let short = k.len() < KEY_RESPACE_LEN;
        let increasing = last.is_none_or(|p| p < k.as_str());
        if short && increasing {
            last = Some(k.as_str());
        } else {
            let fresh = bounds.get(bi).cloned().ok_or(KeyError::Exhausted)?;
            last = Some(k.as_str());
            out.push((k.clone(), fresh));
        }
        bi = bi.saturating_add(1);
    }
    // The pairs must not collide with keys we kept: re-check monotonicity of the final sequence.
    let mut final_seq: Vec<&str> = Vec::with_capacity(keys.len());
    let mut map = std::collections::HashMap::new();
    for (old, new) in &out {
        map.insert(old.as_str(), new.as_str());
    }
    for (k, _) in keys {
        let effective = map.get(k.as_str()).copied().unwrap_or(k.as_str());
        final_seq.push(effective);
    }
    if !final_seq.is_sorted() || !final_seq.iter().all(|k| validate(k).is_ok()) {
        return Err(KeyError::Exhausted);
    }
    Ok(out)
}

/// The id that owns a position key (tie-break order is `(key, id)`).
pub type NodeKeyOwner = u64;

//! Ordering for designators and pin numbers: `R2` before `R10`.

use std::cmp::Ordering;

/// Compares two strings with runs of ASCII digits taken as numbers, so `R2 < R10` and
/// `A9 < A10`. Leading zeros do not change a number's value; with equal values the string
/// with fewer leading zeros comes first, so the order stays total.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (ea, eb) = (digit_end(a, i), digit_end(b, j));
            let ord = compare_numbers(&a[i..ea], &b[j..eb]);
            if ord != Ordering::Equal {
                return ord;
            }
            i = ea;
            j = eb;
        } else {
            let ord = a[i].cmp(&b[j]);
            if ord != Ordering::Equal {
                return ord;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

fn digit_end(s: &[u8], from: usize) -> usize {
    s[from..].iter().position(|c| !c.is_ascii_digit()).map_or(s.len(), |n| from + n)
}

fn compare_numbers(a: &[u8], b: &[u8]) -> Ordering {
    let strip = |s: &[u8]| -> usize { s.iter().take_while(|&&c| c == b'0').count() };
    let (za, zb) = (strip(a), strip(b));
    let (ta, tb) = (&a[za..], &b[zb..]);
    ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| za.cmp(&zb))
}

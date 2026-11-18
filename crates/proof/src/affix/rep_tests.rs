//! `REP` table limits (storytold/wordcraft#3655).

use super::*;

/// storytold/wordcraft#3655: a hostile `.aff` can't grow the `REP` table past its cap or fill it
/// with patterns and replacements longer than a word.
#[test]
fn rep_table_is_capped_and_drops_oversized_entries() {
    let mut aff = String::from("REP 200000\nREP rz ż\n");
    // Bytes, not letters, bound `to`: 31 four-byte letters are 124 bytes.
    let wide = "\u{1D51E}".repeat(MAX_WORD_BYTES / 4 + 1);
    aff += &format!("REP q {}\nREP {} q\nREP q {wide}\n", "z".repeat(1 << 20), "z".repeat(MAX_WORD_BYTES + 1));
    for i in 0..200_000 {
        aff += &format!("REP q z{i}\n");
    }
    let d = AffixDict::from_hunspell(&aff, "1\nkot\n", &[]).unwrap();
    let check = |d: &AffixDict| {
        assert_eq!(d.rep().len(), MAX_REP_ENTRIES);
        assert_eq!(d.rep().first(), Some(&("rz".to_string(), "ż".to_string())));
        assert!(d.rep().iter().all(|(a, b)| a.chars().count() <= 40 && b.len() <= MAX_WORD_BYTES));
    };
    check(&d);
    // A `from` of 40 four-byte letters (160 bytes) can match a word and is kept; 41 can't.
    let long = "\u{1D51E}".repeat(40);
    let d40 = AffixDict::from_hunspell(&format!("REP 2\nREP {long} a\nREP {long}\u{1D51E} a\n"), "1\nkot\n", &[]).unwrap();
    assert_eq!(d40.rep(), [(long, "a".to_string())]);
    check(&AffixDict::from_bytes(&d.to_bytes().unwrap()).unwrap());

    // Oversized REP lines fed straight to the compact reader are dropped as well.
    // A bare `REP` reaches `push_rep` with an empty `from` here, and is dropped.
    let raw = format!("{MAGIC}\nREP q {}\nREP\nREP a_b c\nWORDS\n\0kot\n", "z".repeat(1 << 20));
    let bytes = miniz_oxide::deflate::compress_to_vec(raw.as_bytes(), 6);
    assert_eq!(AffixDict::from_bytes(&bytes).unwrap().rep(), &[("a b".to_string(), "c".to_string())]);
}

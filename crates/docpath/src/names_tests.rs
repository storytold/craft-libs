//! `craft_docpath::names`.

use crate::names::{CopyNumbering, MAX_NAME_BYTES, safe_file_name, unique_file_name};

// Reserved and control characters become `_`, bidi controls go, trailing dots and spaces go,
// "NUL.txt" becomes "_NUL.txt", a 300-byte name is cut to 200 bytes on a char boundary keeping
// ".pdf", and "..." gives the fallback.
#[test]
fn user_text_becomes_a_safe_file_name() {
    assert_eq!(safe_file_name("Q3: plan?.pdf", "x"), "Q3_ plan_.pdf");
    assert_eq!(safe_file_name("a\tb/c", "x"), "a_b_c");
    assert_eq!(safe_file_name("  report. . ", "x"), "report");
    assert_eq!(safe_file_name("image\u{202e}gnp.exe", "x"), "imagegnp.exe");
    assert_eq!(safe_file_name("NUL.txt", "x"), "_NUL.txt");
    assert_eq!(safe_file_name("...", "Untitled"), "Untitled");
    let long = format!("{}.pdf", "é".repeat(150));
    let cut = safe_file_name(&long, "x");
    assert!(cut.len() <= MAX_NAME_BYTES && cut.ends_with("é.pdf"), "{cut}");
    assert!(safe_file_name(&"a".repeat(300), "x").len() == MAX_NAME_BYTES);
}

// "a.txt", "a 2.txt" taken: Space gives "a 3.txt", Parenthesized "a (2).txt" when only "a.txt"
// is taken; "photo (2).jpg" counts on to "photo (3).jpg"; a name with no extension numbers its end.
#[test]
fn taken_names_are_numbered_before_the_extension() {
    let taken = |names: &'static [&'static str]| move |n: &str| names.contains(&n);
    assert_eq!(unique_file_name("a.txt", CopyNumbering::Space, taken(&["a.txt", "a 2.txt"])), "a 3.txt");
    assert_eq!(unique_file_name("a.txt", CopyNumbering::Parenthesized, taken(&["a.txt"])), "a (2).txt");
    assert_eq!(unique_file_name("a.txt", CopyNumbering::Parenthesized, taken(&[])), "a.txt");
    assert_eq!(unique_file_name("photo (2).jpg", CopyNumbering::Parenthesized, taken(&["photo (2).jpg"])), "photo (3).jpg");
    assert_eq!(unique_file_name("notes", CopyNumbering::Parenthesized, taken(&["notes"])), "notes (2)");
    assert_eq!(unique_file_name("a.tar.gz", CopyNumbering::Space, taken(&["a.tar.gz"])), "a.tar 2.gz");
}

//! `craft_docpath::names`.

use crate::names::{CopyNumbering, MAX_NAME_BYTES, is_plain_file_name, safe_file_name, safe_file_stem, unique_file_name};

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

// ".." leaves nothing; ".PDF" goes in any case; a path stays one name; a 900-byte CJK stem is cut
// to 198 bytes, the last whole character under the limit.
#[test]
fn user_text_becomes_a_safe_file_stem() {
    assert_eq!(safe_file_stem("..", "pdf"), "");
    assert_eq!(safe_file_stem("Report.PDF", "pdf"), "Report");
    assert_eq!(safe_file_stem("Report.pdf.txt", "pdf"), "Report.pdf.txt");
    assert_eq!(safe_file_stem("../../etc/passwd", "pdf"), ".._.._etc_passwd");
    let long = safe_file_stem(&"文".repeat(300), "pdf");
    assert!(long.len() <= MAX_NAME_BYTES && long.chars().all(|c| c == '文'), "{}", long.len());
    assert_eq!(long.len(), 198);
}

// Device names, in any case, with any extension and with the superscript digits, get a `_`;
// longer names that start like one do not.
#[test]
fn windows_device_names_get_a_prefix() {
    for name in ["NUL.tar.gz", "con.txt", "COM1.exe", "LPT9.txt", "COM¹.txt", "LPT³.pdf", "lpt9.tar", "nul"] {
        assert_eq!(safe_file_name(name, "x"), format!("_{name}"));
        assert_eq!(safe_file_stem(name, "zip"), format!("_{name}"));
    }
    assert_eq!(safe_file_stem("COM1.pdf", "pdf"), "_COM1");
    assert_eq!(safe_file_stem("LPT³.pdf", "pdf"), "_LPT³");
    for name in ["COM10", "Console", "COM10.txt", "LPT.txt"] {
        assert_eq!(safe_file_name(name, "x"), name);
        assert_eq!(safe_file_stem(name, "pdf"), name);
    }
}

// Right-to-left scripts, joiners and double extensions are left alone; a name with nothing usable
// gives the fallback; a safe name is already safe.
#[test]
fn ordinary_names_pass_unchanged_and_making_safe_is_stable() {
    for name in ["report.pdf", "archive.tar.gz", "مرحبا.pdf", "שלום.txt", "資料.pdf", "می\u{200c}روم.txt", "👩\u{200d}💻.png"] {
        assert_eq!(safe_file_name(name, "attachment"), name);
    }
    for name in ["", " ", ".", "..", " . . ", "\u{202e}"] {
        assert_eq!(safe_file_name(name, "attachment"), "attachment", "{name:?}");
    }
    let long = format!("{}.pdf", "é".repeat(150));
    for name in ["invoice\u{202e}fdp.exe", "../a.pdf", "NUL.txt", " . . ", "report.pdf . ", "資料.pdf", "a\tb/c", long.as_str()] {
        let once = safe_file_name(name, "attachment");
        assert_eq!(safe_file_name(&once, "attachment"), once, "{name:?}");
    }
}

// A plain name sits directly in its folder: no separator, no drive, no trailing dot or space.
#[test]
fn plain_file_names_stay_directly_in_the_folder() {
    for good in ["report.pdf", "report", ".hidden", "a b.pdf", "資料.pdf"] {
        assert!(is_plain_file_name(good), "{good}");
    }
    for bad in ["", "report.", "report.pdf.", "report.pdf ", "report. . ", " ", "...", ". ", "..", "C:x.pdf", "sub\\x.pdf", "sub/x.pdf"] {
        assert!(!is_plain_file_name(bad), "{bad:?}");
    }
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

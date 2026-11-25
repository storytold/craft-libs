//! `parse_recovering`: what a damaged or truncated file still holds.

use craft_dxf::{DxfError, parse, parse_recovering, section_slices, write_binary};

const WHOLE: &str = "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\nA\n0\nENDSEC\n0\nEOF\n";

#[test]
fn an_intact_file_reads_as_parse_reads_it() {
    let (tags, damaged) = parse_recovering(WHOLE.as_bytes()).expect("reads");
    assert_eq!(damaged, None);
    assert_eq!(tags, parse(WHOLE.as_bytes()).expect("reads"));
    // Binary files are read as before.
    let binary = write_binary(&tags, true);
    assert_eq!(parse_recovering(&binary).expect("reads"), (tags, None));
}

#[test]
fn a_line_that_is_no_group_code_ends_the_reading() {
    let text = "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\nA\n<<garbage>>\nmore\n0\nCIRCLE\n";
    assert!(matches!(parse(text.as_bytes()), Err(DxfError::BadCode { line: 9, .. })));
    let (tags, damaged) = parse_recovering(text.as_bytes()).expect("what came before is kept");
    assert_eq!(damaged, Some(9));
    assert_eq!(tags.len(), 4, "nothing after the damage is guessed at");
    assert_eq!(tags.last().map(|t| (t.code, t.str())), Some((8, "A".to_string())));
}

#[test]
fn a_file_cut_after_a_group_code_keeps_the_groups_before_it() {
    let text = "0\nSECTION\n2\nENTITIES\n0\nLINE\n8";
    assert!(matches!(parse(text.as_bytes()), Err(DxfError::Eof)));
    let (tags, damaged) = parse_recovering(text.as_bytes()).expect("what came before is kept");
    assert_eq!((tags.len(), damaged), (3, Some(7)));
    // The section it was cut in still has its records.
    let s = section_slices(&tags);
    assert_eq!((s.len(), s[0].name, s[0].tags.len()), (1, "ENTITIES", 1));
}

#[test]
fn damage_before_the_first_group_is_still_an_error() {
    assert!(matches!(parse_recovering(b"not a drawing\n"), Err(DxfError::BadCode { line: 1, .. })));
    assert!(matches!(parse_recovering(b"0"), Err(DxfError::Eof)));
    assert!(matches!(parse_recovering(b""), Err(DxfError::NotDxf)));
    assert!(matches!(parse_recovering(b"\n\n"), Err(DxfError::NotDxf)));
}

#[test]
fn a_section_without_endsec_ends_at_eof() {
    let tags = parse(b"0\nSECTION\n2\nENTITIES\n0\nLINE\n0\nEOF\n").expect("reads");
    let s = section_slices(&tags);
    assert_eq!(s[0].tags.len(), 1, "the EOF is not a record of the section");
}

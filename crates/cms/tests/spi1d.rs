use craft_cms::spi1d::{MAX_ENTRIES, Table, parse};

#[test]
fn monochrome_and_three_component_samples_preserve_domain() {
    assert_eq!(
        parse("Version 1\nFrom -1 2\nLength 2\nComponents 1\n{\n-2\n3\n}"),
        Some(Table { values: vec![[-2.0; 3], [3.0; 3]], domain: [-1.0, 2.0] })
    );
    assert_eq!(
        parse("Length 2\nComponents 3\n{\n0 1\n2 3 4 5\n}"),
        Some(Table { values: vec![[0.0, 1.0, 2.0], [3.0, 4.0, 5.0]], domain: [0.0, 1.0] })
    );
}

#[test]
fn legacy_header_and_eof_tolerance_is_preserved() {
    let table = parse("Ignored arbitrary header\nLength 2 extra\n{\n0 1");
    assert!(table.is_some());
    assert_eq!(table, parse("Length 2\n{\n0 1\n} ignored\ninvalid trailing text"));
    assert!(parse("Length 2\n{\nNaN inf\n}").is_some(), "sample sanitization belongs to the caller");
}

#[test]
fn invalid_headers_partial_and_surplus_tables_are_refused() {
    for header in ["Length 1", "Length 2\nComponents 2", "Length 2\nFrom 2 1", "Length 2\nFrom NaN 1"] {
        assert!(parse(&format!("{header}\n{{\n0 1\n}}")).is_none());
    }
    assert!(parse(&format!("Length {}\n{{\n0 1", MAX_ENTRIES + 1)).is_none());
    assert!(parse("Length 2\nComponents 3\n{\n0 1 2 3 4").is_none());
    assert!(parse("Length 2\n{\n0 1 2").is_none());
    assert!(parse("Length 2\n{\n").is_none());
}

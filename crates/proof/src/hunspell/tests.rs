use super::*;

#[test]
fn decodes_latin2_and_utf8() {
    let aff = b"SET ISO8859-2\nTRY a\n";
    let (a, d) = decode(aff, b"2\n\xbf\xf3\xb3w\n").unwrap();
    assert!(a.contains("TRY a"));
    assert_eq!(d, "2\nżółw\n");
    let (_, d) = decode("SET UTF-8\n".as_bytes(), "żółw".as_bytes()).unwrap();
    assert_eq!(d, "żółw");
    let (_, d) = decode(b"SET ISO8859-9\n", b"\xd0\xdd\xde\xf0\xfd\xfe\xd6\xe7").unwrap();
    assert_eq!(d, "ĞİŞğışÖç");
    assert!(decode(b"SET SHIFT_JIS\n", b"").is_err());
    assert!(decode(b"SET UTF-8\n", b"\xff\xfe").is_err());
}

#[test]
fn utf8_dictionaries_keep_single_byte_flags() {
    // The Hungarian dictionary is `SET UTF-8` but writes flags as single bytes above 0x7F,
    // which aren't UTF-8; each byte is one flag. Its `NAME` line is ISO 8859-2.
    let aff = b"SET UTF-8\nNAME Magyar helyes\xedr\xe1si\nAF 2\nAF \xd5a # 1\nAF \xe9 # 2\nSFX \xd5 Y 1\nSFX \xd5 0 ok/2 . 7\nSFX \xe9 Y 1\nSFX \xe9 0 ban .\nSFX a Y 1\nSFX a 0 at .\n";
    let dic = "2\nház/1\t1\nkert/\u{e9}\n".as_bytes();
    let (a, d) = decode(aff, dic).unwrap();
    assert!(a.contains("AF \u{d5}a # 1") && a.contains("AF \u{e9} # 2") && a.contains("SFX \u{d5} 0 ok/2 . 7"));
    assert!(d.contains("kert/\u{c3}\u{a9}"), "a UTF-8 é in the flags is two one-byte flags");
    let dict = crate::affix::AffixDict::from_hunspell(&a, &d, &[]).unwrap();
    assert!(dict.check("ház") && dict.check("házok") && dict.check("házokban") && dict.check("házat"));
    assert!(!dict.check("házban"));
    // `FLAG UTF-8` keeps the whole file UTF-8, and words must still be UTF-8.
    let (_, d) = decode(b"SET UTF-8\nFLAG UTF-8\n", "kert/\u{e9}\n".as_bytes()).unwrap();
    assert_eq!(d, "kert/\u{e9}\n");
    assert!(decode(b"SET UTF-8\nSFX a Y 1\nSFX a 0 \xe9 .\n", b"").is_err(), "an affix must be UTF-8");
    assert!(decode(b"SET UTF-8\n", b"1\nh\xe1z/a\n").is_err(), "a stem must be UTF-8");
}

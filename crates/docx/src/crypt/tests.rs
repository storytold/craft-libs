#[test]
fn protection_password_round_trip() {
    use super::PasswordCheck::*;
    let attrs = super::protection_hash("s3cret", 1000).unwrap();
    assert_eq!(super::check_protection_password(&attrs, "s3cret"), Correct);
    assert_eq!(super::check_protection_password(&attrs, "S3cret"), Incorrect);
    // Damaged attributes are refused, never accepted.
    let mut bad = attrs.clone();
    bad.retain(|(k, _)| k != "w:hashValue");
    bad.push(("w:hashValue".into(), "not base64!".into()));
    assert_eq!(super::check_protection_password(&bad, "s3cret"), Unsupported);
    let mut odd = attrs.clone();
    odd.retain(|(k, _)| k != "w:algorithmName");
    odd.push(("w:algorithmName".into(), "WHIRLPOOL".into()));
    assert_eq!(super::check_protection_password(&odd, "s3cret"), Unsupported);
    assert_eq!(super::check_protection_password(&[("w:cryptProviderType".into(), "rsaAES".into())], "x"), NoPassword);
}

/// The worked example of ECMA-376 Part 4 §14.8.1: "Example" gives the key 0x64CEED7E, hashed
/// as "7EEDCE64".
#[test]
fn legacy_key_matches_the_specification_example() {
    assert_eq!(super::legacy_key("Example"), 0x64CE_ED7E);
    assert_eq!(super::legacy_key(""), 0);
    // A leading BOM is dropped; only the first 15 characters count.
    assert_eq!(super::legacy_key("\u{FEFF}Example"), 0x64CE_ED7E);
    assert_eq!(super::legacy_key("abcdefghijklmnopq"), super::legacy_key("abcdefghijklmno"));
    assert_ne!(super::legacy_key("abcdefghijklmno"), super::legacy_key("abcdefghijklmn"));
}

/// The legacy form (`w:cryptAlgorithmSid`, `w:hash`, `w:salt`, `w:cryptSpinCount`), as Word
/// writes it. The expected hash was computed independently from the specification's stages.
#[test]
fn legacy_protection_hash_is_verified() {
    use super::PasswordCheck::*;
    let legacy = |sid: &str, hash: &str, spin: &str| -> Vec<(String, String)> {
        vec![
            ("w:cryptProviderType".into(), "rsaFull".into()),
            ("w:cryptAlgorithmClass".into(), "hash".into()),
            ("w:cryptAlgorithmType".into(), "typeAny".into()),
            ("w:cryptAlgorithmSid".into(), sid.into()),
            ("w:cryptSpinCount".into(), spin.into()),
            ("w:hash".into(), hash.into()),
            ("w:salt".into(), "AAECAwQFBgcICQoLDA0ODw==".into()),
        ]
    };
    let attrs = legacy("4", KNOWN_LEGACY_SHA1, "100000");
    assert_eq!(super::check_protection_password(&attrs, "Example"), Correct);
    assert_eq!(super::check_protection_password(&attrs, "example"), Incorrect);
    assert_eq!(super::check_protection_password(&attrs, ""), Incorrect);
    // MD5 (and the other algorithms WordCraft lacks), hostile spin counts: refused.
    assert_eq!(super::check_protection_password(&legacy("3", KNOWN_LEGACY_SHA1, "100000"), "Example"), Unsupported);
    assert_eq!(super::check_protection_password(&legacy("4", KNOWN_LEGACY_SHA1, "4294967295"), "Example"), Unsupported);
    assert_eq!(super::check_protection_password(&legacy("4", KNOWN_LEGACY_SHA1, "-1"), "Example"), Unsupported);
    let mut ext = attrs.clone();
    ext.push(("w:algIdExt".into(), "0000000A".into()));
    assert_eq!(super::check_protection_password(&ext, "Example"), Unsupported);
    // No algorithm named: the very old bare-hash form isn't defined, so it's refused.
    let bare = vec![("w:hash".to_string(), KNOWN_LEGACY_SHA1.to_string())];
    assert_eq!(super::check_protection_password(&bare, "Example"), Unsupported);
}

/// SHA-1 of the salt 00 01 … 0F and "7EEDCE64" as UTF-16LE, iterated 100,000 times.
const KNOWN_LEGACY_SHA1: &str = "pEWelKoH6SVpvMmyPAJ8bHA/g/s=";

fn zip_package() -> Vec<u8> {
    craft_opc::zip_entries(&[("word/document.xml".to_string(), b"<w:document/>".to_vec())]).unwrap()
}

/// The error mapping: what the reader and the app tell the user.
#[test]
fn round_trip_and_errors_through_craft_offcrypto() {
    let pkg = zip_package();
    let file = super::encrypt_with_spin_count(&pkg, "pw", 1000).unwrap();
    assert!(super::is_encrypted(&file) && !super::is_encrypted(&pkg));
    assert_eq!(super::decrypt(&file, None), Err(crate::DocxError::PasswordRequired));
    assert_eq!(super::decrypt(&file, Some("nope")), Err(crate::DocxError::WrongPassword));
    assert_eq!(super::decrypt(&file, Some("pw")).unwrap(), pkg);
    let std = super::encrypt_standard_for_tests(&pkg, "pw", 128).unwrap();
    assert_eq!(super::decrypt(&std, Some("pw")).unwrap(), pkg);
    assert!(matches!(super::decrypt(b"not a compound file", Some("pw")), Err(crate::DocxError::Encryption(_))));
    assert!(matches!(super::check_password(""), Err(crate::DocxError::Encryption(_))));
    assert!(matches!(super::encrypt(&pkg, &"x".repeat(super::MAX_PASSWORD_CHARS + 1)), Err(crate::DocxError::Encryption(_))));
}

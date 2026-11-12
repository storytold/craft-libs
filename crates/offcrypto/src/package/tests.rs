use super::*;

/// A stand-in for a zip package: the crate never looks inside it.
fn zip_package() -> Vec<u8> {
    (0..9000u32).map(|i| (i * 31 % 251) as u8).collect()
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

#[test]
fn round_trip_with_a_password() {
    let pkg = zip_package();
    let enc = encrypt_with_spin_count(&pkg, "pässwörd 🔑", 16).unwrap();
    assert!(is_encrypted(&enc));
    assert!(!is_encrypted(&pkg));
    // The compound file carries the data spaces Word expects.
    let comp = craft_cfb::CompoundFile::open(&enc).unwrap();
    for stream in [
        "\u{6}DataSpaces/Version",
        "\u{6}DataSpaces/DataSpaceMap",
        "\u{6}DataSpaces/DataSpaceInfo/StrongEncryptionDataSpace",
        "\u{6}DataSpaces/TransformInfo/StrongEncryptionTransform/\u{6}Primary",
    ] {
        assert!(comp.has_stream(stream), "{stream}");
    }
    assert_eq!(decrypt(&enc, Some("pässwörd 🔑")).unwrap(), pkg);
    // Without a password, or with the wrong one, it says so.
    assert_eq!(decrypt(&enc, None), Err(Error::PasswordRequired));
    assert_eq!(check(&enc), Ok(()));
    assert_eq!(decrypt(&enc, Some("Pässwörd 🔑")), Err(Error::WrongPassword));
    // A package that is a whole number of segments, and an empty one.
    for n in [0usize, SEGMENT, SEGMENT * 2 + 1] {
        let data: Vec<u8> = (0..n).map(|i| (i * 7) as u8).collect();
        let enc = encrypt_with_spin_count(&data, "x", 1).unwrap();
        assert_eq!(decrypt(&enc, Some("x")).unwrap(), data);
    }
    assert!(matches!(encrypt(&pkg, ""), Err(Error::BadPassword(_))));
    assert!(matches!(encrypt(&pkg, &"x".repeat(MAX_PASSWORD_CHARS + 1)), Err(Error::BadPassword(_))));
    assert!(encrypt_with_spin_count(&pkg, &"x".repeat(MAX_PASSWORD_CHARS), 1).is_ok());
}

#[test]
fn a_file_without_data_integrity_is_refused() {
    let (info, package) = streams(&encrypt_with_spin_count(&zip_package(), "pw", 4).unwrap());
    let xml = String::from_utf8(info[8..].to_vec()).unwrap();
    let mut stripped = info[..8].to_vec();
    stripped.extend_from_slice(xml.replacen("<dataIntegrity", "<nothing", 1).as_bytes());
    assert!(matches!(decrypt(&cfb_with(&stripped, &package), Some("pw")), Err(Error::Malformed(m)) if m.contains("dataIntegrity")));
    // An empty or ragged field is damage, not a pass.
    for attr in ["encryptedHmacKey=\"", "encryptedHmacValue=\""] {
        let at = xml.find(attr).unwrap() + attr.len();
        let end = at + xml[at..].find('"').unwrap();
        for value in ["", "AA=="] {
            let mut bad = info[..8].to_vec();
            bad.extend_from_slice(format!("{}{value}{}", &xml[..at], &xml[end..]).as_bytes());
            assert!(matches!(decrypt_streams(&bad, &package, Some("pw")), Err(Error::Malformed(_))), "{attr}{value}");
        }
    }
}

#[test]
fn tampered_package_fails_the_integrity_check() {
    let pkg = zip_package();
    let (info, package) = streams(&encrypt_with_spin_count(&pkg, "pw", 4).unwrap());
    for at in [8usize, 100, package.len() - 1] {
        let mut data = package.clone();
        data[at] ^= 1;
        assert!(matches!(decrypt(&cfb_with(&info, &data), Some("pw")), Err(Error::Malformed(m)) if m.contains("integrity")), "byte {at}");
    }
    // The MAC covers the whole stream, StreamSize included: changing the stated size alone fails.
    let mut resized = package.clone();
    resized[..8].copy_from_slice(&((pkg.len() - 1) as u64).to_le_bytes());
    assert!(matches!(decrypt_streams(&info, &resized, Some("pw")), Err(Error::Malformed(m)) if m.contains("integrity")));
    // And the descriptor can't vouch for it either: a swapped integrity value is refused.
    let xml = String::from_utf8(info[8..].to_vec()).unwrap();
    let at = 8 + xml.find("encryptedHmacValue=\"").unwrap() + "encryptedHmacValue=\"".len();
    let mut swapped = info.clone();
    swapped[at] = if swapped[at] == b'A' { b'B' } else { b'A' };
    assert!(matches!(decrypt_streams(&swapped, &package, Some("pw")), Err(Error::Malformed(_))));
    assert_eq!(decrypt_streams(&info, &package, Some("pw")).unwrap(), pkg);
}

/// A compound file with the given EncryptionInfo and EncryptedPackage streams.
fn cfb_with(info: &[u8], package: &[u8]) -> Vec<u8> {
    craft_cfb::write_streams(craft_cfb::Version::V3, &[("EncryptionInfo", info), ("EncryptedPackage", package)]).unwrap()
}

#[test]
fn hostile_encryption_info_errors_without_panicking() {
    let good = encrypt_with_spin_count(&zip_package(), "pw", 2).unwrap();
    let (info, package) = streams(&good);
    let xml = String::from_utf8(info[8..].to_vec()).unwrap();
    // (EncryptionInfo, EncryptedPackage, must fail). Truncations only must not panic: the
    // XML reader is lenient, so cutting the closing tags still reads.
    let mut cases: Vec<(Vec<u8>, Vec<u8>, bool)> = Vec::new();
    for n in (0..info.len()).step_by(7) {
        cases.push((info[..n].to_vec(), package.clone(), n < info.len() / 2));
    }
    for n in [0, 4, 8, 9, 100, package.len() - 1] {
        cases.push((info.clone(), package[..n].to_vec(), true));
    }
    // Standard and unknown versions, absurd numbers, bad base64, a huge StreamSize.
    let with = |from: &str, to: &str| {
        let mut v = info[..8].to_vec();
        v.extend_from_slice(xml.replacen(from, to, 2).as_bytes());
        v
    };
    cases.push(([3u8, 0, 2, 0, 0x24, 0, 0, 0].to_vec(), package.clone(), true));
    cases.push(([9u8, 0, 9, 0].to_vec(), package.clone(), true));
    cases.push((with("spinCount=\"2\"", "spinCount=\"4294967295\""), package.clone(), true));
    cases.push((with("keyBits=\"256\"", "keyBits=\"8\""), package.clone(), true));
    cases.push((with("blockSize=\"16\"", "blockSize=\"4096\""), package.clone(), true));
    cases.push((with("hashSize=\"64\"", "hashSize=\"0\""), package.clone(), true));
    cases.push((with("saltSize=\"16\"", "saltSize=\"99999999999999999999\""), package.clone(), true));
    cases.push((with("SHA512", "MD5"), package.clone(), true));
    cases.push((with("ChainingModeCBC", "ChainingModeCFB"), package.clone(), true));
    cases.push((with("saltValue=\"", "saltValue=\"!!"), package.clone(), true));
    cases.push((with("encryptedKeyValue=\"", "encryptedKeyValue=\"AAAA"), package.clone(), true));
    // Data integrity is mandatory: a file without it is refused, never opened unchecked.
    cases.push((with("<dataIntegrity", "<nothing"), package.clone(), true));
    let mut huge = package.clone();
    huge[..8].copy_from_slice(&u64::MAX.to_le_bytes());
    cases.push((with("<dataIntegrity", "<nothing"), huge, true));
    for (i, p, must_fail) in cases {
        let file = cfb_with(&i, &p);
        let r = decrypt(&file, Some("pw"));
        assert!(!must_fail || r.is_err());
        let _ = check(&file);
    }
    // A file that only looks like a compound file.
    let mut fake = craft_cfb::SIGNATURE.to_vec();
    fake.extend_from_slice(&[0u8; 600]);
    assert!(!is_encrypted(&fake));
    assert!(decrypt(&fake, Some("pw")).is_err());
    assert!(!is_encrypted(b"PK\x03\x04") && !is_encrypted(&[]));
}

/// Schemes that are not read are named, so a caller can say which, before asking for a password.
#[test]
fn unsupported_schemes_are_named() {
    let (info, package) = streams(&encrypt_with_spin_count(&zip_package(), "pw", 2).unwrap());
    let xml = String::from_utf8(info[8..].to_vec()).unwrap();
    let agile = |from: &str, to: &str| {
        let mut v = info[..8].to_vec();
        v.extend_from_slice(xml.replace(from, to).as_bytes());
        v
    };
    let unsupported = |info: &[u8]| match check(&cfb_with(info, &package)) {
        Err(Error::Unsupported(s)) => s,
        other => panic!("expected Unsupported, got {other:?}"),
    };
    let (mut rc4, _) = streams(&encrypt_standard(b"x", "pw", 128).unwrap());
    rc4[4..8].copy_from_slice(&F_CRYPTO_API.to_le_bytes()); // fCryptoAPI without fAES
    assert_eq!(unsupported(&rc4), "RC4");
    let (mut external, _) = streams(&encrypt_standard(b"x", "pw", 128).unwrap());
    external[4..8].copy_from_slice(&(STANDARD_FLAGS | F_EXTERNAL).to_le_bytes());
    assert_eq!(unsupported(&external), "external");
    assert_eq!(unsupported(&[3, 0, 3, 0, 0, 0, 0, 0]), "Extensible");
    assert_eq!(unsupported(&[1, 0, 1, 0, 0, 0, 0, 0]), "version 1.1");
    assert_eq!(unsupported(&agile("cipherAlgorithm=\"AES\"", "cipherAlgorithm=\"3DES\"")), "3DES");
    assert_eq!(unsupported(&agile("SHA512", "MD5")), "MD5 hash");
    assert_eq!(unsupported(&agile("ChainingModeCBC", "ChainingModeCFB")), "ChainingModeCFB chaining");
    assert_eq!(
        unsupported(&agile(
            "keyEncryptor uri=\"http://schemas.microsoft.com/office/2006/keyEncryptor/password\"",
            "keyEncryptor uri=\"http://schemas.microsoft.com/office/2006/keyEncryptor/certificate\""
        )),
        "certificate"
    );
    // Rights management keeps the package but has no EncryptionInfo.
    let irm =
        craft_cfb::write_streams(craft_cfb::Version::V3, &[("EncryptedPackage", &package[..]), ("\u{6}DataSpaces/Version", &[0u8; 8][..])]).unwrap();
    assert!(is_encrypted(&irm));
    assert_eq!(check(&irm), Err(Error::Unsupported("rights-management".into())));
    assert_eq!(decrypt(&irm, Some("pw")), Err(Error::Unsupported("rights-management".into())));
    // Another compound file is not an encrypted package at all.
    let other = craft_cfb::write_streams(craft_cfb::Version::V3, &[("Workbook", &[0u8; 8][..])]).unwrap();
    assert!(!is_encrypted(&other));
    assert!(matches!(check(&other), Err(Error::Malformed(_))));
}

/// Keys and IVs computed independently with msoffcrypto-tool 6.0.0 (MIT) for a synthetic
/// password and salts.
#[test]
fn agile_keys_match_an_independent_implementation() {
    let salt: Vec<u8> = (0u8..16).collect();
    let h = iterate(Hash::Sha512, &salt, "Pass1234", 1000);
    assert_eq!(
        derive_key(Hash::Sha512, &h, &BK_VERIFIER_INPUT, 32).to_vec(),
        hex("c57def04938e1e83497271f6ec099e890bfd2bd9cf05ff208f8b2405df361ae6")
    );
    assert_eq!(derive_key(Hash::Sha512, &h, &BK_KEY_VALUE, 32).to_vec(), hex("4ab08cf2ed95199d58d04f19331049a8a07cc1ff508431a1dd0785c84e261d03"));
    let h = iterate(Hash::Sha1, &salt, "Pass1234", 1000);
    assert_eq!(derive_key(Hash::Sha1, &h, &BK_VERIFIER_INPUT, 16).to_vec(), hex("88c7567d028eae33a9a55f65729dcd2b"));
    let key_data_salt: Vec<u8> = (16u8..32).collect();
    assert_eq!(make_iv(Hash::Sha512, &key_data_salt, Some(&1u32.to_le_bytes()), 16), hex("da22ccba062aff577d207b2dee4e165b"));
    // SHA-1 gives 20 bytes; an AES-256 key needs 32, so 12 bytes of 0x36 follow.
    let k = derive_key(Hash::Sha1, b"a", b"b", 32);
    assert_eq!((&k[..20], &k[20..]), (&Hash::Sha1.hash(&[b"ab"])[..], &[0x36u8; 12][..]));
    // Passwords hash as UTF-16LE, surrogate pairs included: "Pässwörd€" and a key emoji.
    assert_eq!(
        iterate(Hash::Sha512, &salt, "P\u{e4}ssw\u{f6}rd\u{20ac}\u{1F511}", 1000).to_vec(),
        hex("2bca96ca0a7e8d88d9314a2eb1819a1ece94ff9ce34266ff82f891a88c4240c4c9f39c729b7c5c410c27fc8e2b7ad90d8c005eb68a603377b8a4ab858041c76a")
    );
    assert_eq!(standard_key(&salt, "Pass1234", 16).to_vec(), hex("ae0b402aeb23a779471a130843e431d4"));
    assert_eq!(standard_key(&salt, "Pass1234", 32).to_vec(), hex("ae0b402aeb23a779471a130843e431d47b416314b5146b491cb815283663d26f"));
}

/// The key a real-world Standard-encrypted package derives: Apache POI's test file
/// `protected_passtika.xlsx` (Apache-2.0 test data), password "tika", written by Excel
/// itself. The value was computed from [MS-OFFCRYPTO] §2.3.4.7 independently of this code —
/// the file decrypts to a valid package under it and nothing else opens that file — so real
/// Word 2007 documents stay pinned, not just spec-derived ones.
#[test]
fn standard_key_matches_a_real_world_package() {
    let salt: [u8; 16] = [0x19, 0xd0, 0xc7, 0xf3, 0x95, 0x28, 0x78, 0x43, 0x6a, 0x49, 0x49, 0xd7, 0xf0, 0xa2, 0xcf, 0x1e];
    const KEY: &str = "8d4d00143a5f2e4918af7e6e7410d46c";
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    assert_eq!(hex(&standard_key(&salt, "tika", 16)), KEY);
    // A different password or salt gives a different key; the key is exactly the length
    // asked for, for each AES width.
    assert_ne!(hex(&standard_key(&salt, "tika2", 16)), KEY);
    assert_ne!(hex(&standard_key(&[0; 16], "tika", 16)), KEY);
    assert_eq!(standard_key(&salt, "tika", 24).len(), 24);
    assert_eq!(standard_key(&salt, "tika", 32).len(), 32);
}

/// Standard encryption's key derivation (§2.3.4.7) for the password "Password1234_" and the
/// salt 00 01 … 0F. The expected keys were computed outside this crate (Python's hashlib)
/// following the specification's steps, so the encryptor and decryptor can't share a mistake.
#[test]
fn standard_key_matches_an_independent_derivation() {
    let salt: Vec<u8> = (0..16).collect();
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    assert_eq!(hex(&standard_key(&salt, "Password1234_", 16)), "340fc93a8e2154db8304d055df81cebc");
    assert_eq!(hex(&standard_key(&salt, "Password1234_", 32)), "340fc93a8e2154db8304d055df81cebcef5e89fb8e5eb520efdca76f3330e032");
    assert_eq!(standard_key(&salt, "Password1234_", 24).len(), 24);
    // AES-ECB against FIPS-197 Appendix C.1 and C.3.
    let pt: Vec<u8> = (0..16u8).map(|i| i * 0x11).collect();
    let k128: Vec<u8> = (0..16).collect();
    let k256: Vec<u8> = (0..32).collect();
    assert_eq!(hex(&ecb(&k128, &pt, true).unwrap()), "69c4e0d86a7b0430d8cdb78070b4c55a");
    assert_eq!(hex(&ecb(&k256, &pt, true).unwrap()), "8ea2b7ca516745bfeafc49904b496089");
    assert_eq!(ecb(&k256, &ecb(&k256, &pt, true).unwrap(), false).unwrap(), pt);
    assert!(ecb(&k128, &pt[..15], false).is_err());
}

/// Standard encryption (Word 2007's AES-ECB method) opens with the right password, for each
/// AES key size, and says when the password is missing or wrong — as agile files do.
#[test]
fn standard_encryption_round_trips() {
    let pkg = zip_package();
    for bits in [128, 192, 256] {
        let enc = encrypt_standard(&pkg, "pässwörd 🔑", bits).unwrap();
        assert!(is_encrypted(&enc));
        assert_eq!(decrypt(&enc, Some("pässwörd 🔑")), Ok(pkg.clone()), "AES-{bits}");
        assert_eq!(decrypt(&enc, None), Err(Error::PasswordRequired));
        assert_eq!(decrypt(&enc, Some("Pässwörd 🔑")), Err(Error::WrongPassword));
        assert_eq!(check(&enc), Ok(()));
    }
    // Sizes on and off the block boundary, and empty.
    for n in [0usize, 1, 15, 16, 17, SEGMENT + 3] {
        let data: Vec<u8> = (0..n).map(|i| (i * 7) as u8).collect();
        assert_eq!(decrypt(&encrypt_standard(&data, "x", 256).unwrap(), Some("x")).unwrap(), data);
    }
    assert!(encrypt_standard(&pkg, "pw", 64).is_err());
    assert!(encrypt_standard(&pkg, "", 128).is_err());
}

/// A Standard-encrypted file assembled byte by byte from [MS-OFFCRYPTO] §2.3.4.5, §2.3.2, §2.3.3
/// and §2.3.4.4 with literal field values, without the test encryptor, so a layout mistake
/// shared by the encryptor and the reader can't pass. Only the key derivation and AES-ECB
/// (pinned by `standard_key_matches_an_independent_derivation`) are reused.
fn hand_built_standard(payload: &[u8], password: &str) -> Vec<u8> {
    let salt: [u8; 16] = [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE, 0xAF];
    let verifier: [u8; 16] = *b"verifier-16bytes";
    let key = standard_key(&salt, password, 16);
    let mut hash = sha1::Sha1::digest(verifier).to_vec();
    hash.resize(32, 0); // the hash, zero-padded to two AES blocks
    let csp: Vec<u8> = "Test provider".encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();

    let mut header = Vec::new();
    header.extend_from_slice(&0x24u32.to_le_bytes()); // Flags: fCryptoAPI | fAES
    header.extend_from_slice(&0u32.to_le_bytes()); // SizeExtra
    header.extend_from_slice(&0x660Eu32.to_le_bytes()); // AlgID: AES-128
    header.extend_from_slice(&0x8004u32.to_le_bytes()); // AlgIDHash: SHA-1
    header.extend_from_slice(&128u32.to_le_bytes()); // KeySize, in bits
    header.extend_from_slice(&0x18u32.to_le_bytes()); // ProviderType: AES
    header.extend_from_slice(&0u32.to_le_bytes()); // Reserved1
    header.extend_from_slice(&0u32.to_le_bytes()); // Reserved2
    header.extend_from_slice(&csp); // CSPName: UTF-16LE, NUL-terminated

    let mut info = Vec::new();
    info.extend_from_slice(&3u16.to_le_bytes()); // Version major
    info.extend_from_slice(&2u16.to_le_bytes()); // Version minor
    info.extend_from_slice(&0x24u32.to_le_bytes()); // Flags: fCryptoAPI | fAES
    info.extend_from_slice(&(header.len() as u32).to_le_bytes()); // HeaderSize
    info.extend_from_slice(&header);
    info.extend_from_slice(&16u32.to_le_bytes()); // SaltSize
    info.extend_from_slice(&salt); // Salt
    info.extend_from_slice(&ecb(&key, &verifier, true).unwrap()); // EncryptedVerifier
    info.extend_from_slice(&20u32.to_le_bytes()); // VerifierHashSize (SHA-1)
    info.extend_from_slice(&ecb(&key, &hash, true).unwrap()); // EncryptedVerifierHash

    let mut padded = payload.to_vec();
    padded.resize(payload.len().next_multiple_of(16), 0);
    let mut package = (payload.len() as u64).to_le_bytes().to_vec(); // StreamSize
    package.extend_from_slice(&ecb(&key, &padded, true).unwrap());
    package.extend_from_slice(&[0xEE, 0xEE, 0xEE]); // trailing padding the spec allows
    cfb_with(&info, &package)
}

#[test]
fn standard_package_built_by_hand_from_the_spec_opens() {
    let payload: Vec<u8> = (0..37u8).map(|i| i.wrapping_mul(5)).collect();
    let file = hand_built_standard(&payload, "pw");
    assert!(is_encrypted(&file));
    assert_eq!(decrypt(&file, Some("pw")), Ok(payload.clone()));
    assert_eq!(decrypt(&file, Some("PW")), Err(Error::WrongPassword));
    assert_eq!(decrypt(&file, None), Err(Error::PasswordRequired));
    // The test encryptor writes the same fixed fields: version and flags (bytes 0..8) and the
    // header's eight words (12..44); HeaderSize differs with the CSP name, the rest is random.
    let (hand, _) = streams(&file);
    let (made, _) = streams(&encrypt_standard(&payload, "pw", 128).unwrap());
    assert_eq!(hand[..8], made[..8]);
    assert_eq!(hand[12..44], made[12..44]);
}

/// The two streams of an encrypted package.
fn streams(file: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let comp = craft_cfb::CompoundFile::open(file).unwrap();
    (comp.read_path("EncryptionInfo").unwrap(), comp.read_path("EncryptedPackage").unwrap())
}

#[test]
fn hostile_standard_encryption_info_errors_without_panicking() {
    let data = b"a small package".to_vec();
    let (info, package) = streams(&encrypt_standard(&data, "pw", 128).unwrap());
    // Layout: version 0, flags 4, header size 8, header 12 (flags, sizeExtra, algId 20,
    // algIdHash 24, keySize 28, provider, 2 reserved, an empty CSP name), then the verifier at
    // 46: saltSize, salt 50, encrypted verifier 66, verifierHashSize 82, its hash 86..118.
    assert_eq!(info.len(), 118);
    let put = |at: usize, v: u32| {
        let mut i = info.clone();
        i[at..at + 4].copy_from_slice(&v.to_le_bytes());
        i
    };
    let mut cases: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for n in 0..info.len() {
        cases.push((info[..n].to_vec(), package.clone()));
    }
    for v in [0, 4, 31, 0x1000, u32::MAX] {
        cases.push((put(8, v), package.clone()));
    }
    // No AES, no CryptoAPI, an external (extensible) provider.
    for v in [F_CRYPTO_API, F_AES, STANDARD_FLAGS | F_EXTERNAL, 0] {
        cases.push((put(4, v), package.clone()));
    }
    // RC4, MD5, key sizes that don't fit the algorithm.
    cases.push((put(20, 0x6801), package.clone()));
    cases.push((put(24, 0x8003), package.clone()));
    for v in [0, 64, 192, 256, u32::MAX] {
        cases.push((put(28, v), package.clone()));
    }
    for v in [0, 15, 17, u32::MAX] {
        cases.push((put(46, v), package.clone()));
    }
    for v in [0, 16, 32, u32::MAX] {
        cases.push((put(82, v), package.clone()));
    }
    // The package: truncated, an absurd StreamSize, a StreamSize past the data, data that
    // isn't whole blocks.
    for n in [0, 4, 8, 9, package.len() - 1] {
        cases.push((info.clone(), package[..n].to_vec()));
    }
    for size in [u64::MAX, 1 << 40, package.len() as u64] {
        let mut p = package.clone();
        p[..8].copy_from_slice(&size.to_le_bytes());
        cases.push((info.clone(), p));
    }
    for (i, p) in cases {
        let file = cfb_with(&i, &p);
        assert!(decrypt(&file, Some("pw")).is_err());
    }
    // Version 2.2 and 4.2 are Standard too; trailing bytes after the last block are ignored.
    for v in [2u8, 4] {
        let mut i = info.clone();
        i[0] = v;
        assert_eq!(decrypt(&cfb_with(&i, &package), Some("pw")).unwrap(), data);
    }
    let mut p = package.clone();
    p.extend_from_slice(&[1, 2, 3]);
    assert_eq!(decrypt(&cfb_with(&info, &p), Some("pw")).unwrap(), data);
    // AlgID and AlgIDHash 0 mean "from the flags": AES-128 and SHA-1.
    assert_eq!(decrypt(&cfb_with(&put(20, 0), &package), Some("pw")).unwrap(), data);
    assert_eq!(decrypt(&cfb_with(&put(24, 0), &package), Some("pw")).unwrap(), data);
}

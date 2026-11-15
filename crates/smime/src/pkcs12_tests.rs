// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! PKCS #12 files opened (#30 R13). The files were made with OpenSSL 3 for synthetic
//! `example.org` people (an RSA key for Alice, a P-256 key for Bob, both under a test authority
//! made for these tests alone), with the passphrase `example-pass`:
//! `openssl pkcs12 -export` (PBES2, AES-256-CBC, HMAC-SHA-256), with `-keypbe PBE-SHA1-3DES
//! -certpbe PBE-SHA1-3DES -macalg sha1` (the old scheme), `-legacy` (RC2-40), `-nomac`,
//! `-keypbe AES-128-CBC -certpbe NONE -macalg sha512` (Bob's), `-passout pass:` (no passphrase)
//! and `-iter 2000000`.

use proptest::prelude::*;

use crate::pkcs12::{Error, KeyKind, MAX_P12_LEN, looks_like, open};

const PASS: &str = "example-pass";

fn bytes(b64: &str) -> Vec<u8> {
    mailcraft_charset::base64::STANDARD.decode(b64).unwrap()
}

fn der(pem: &str) -> Vec<u8> {
    crate::import::certificates_in(pem.as_bytes()).remove(0)
}

#[test]
fn a_modern_file_opens_with_its_certificate_its_authority_and_its_key() {
    let opened = open(&bytes(MODERN), PASS).unwrap();
    assert_eq!(opened.certificate, der(ALICE_PEM));
    assert_eq!(opened.others, vec![der(CA_PEM)]);
    assert_eq!(opened.key_kind, KeyKind::Rsa { bits: 2048 }, "RSA keys aren't used here: the window is told");
    // PKCS #8 PrivateKeyInfo, RSA.
    assert_eq!(opened.private_key.first(), Some(&0x30));
    assert!(opened.private_key.windows(9).any(|w| w == [0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01]));
    // Never shown, even in a log.
    let shown = format!("{opened:?}");
    assert!(shown.contains("redacted"), "{shown}");
    assert!(!shown.contains("48, 130"), "{shown}");
}

#[test]
fn the_old_triple_des_scheme_opens() {
    let opened = open(&bytes(LEGACY), PASS).unwrap();
    assert_eq!(opened.certificate, der(ALICE_PEM));
    assert_eq!(opened.others.len(), 1);
}

#[test]
fn an_ec_key_finds_its_certificate_by_its_public_key() {
    let opened = open(&bytes(EC), PASS).unwrap();
    let info = crate::import::certificate_info(&opened.certificate).unwrap();
    assert_eq!(info.addresses, vec!["bob@example.org".to_string()]);
    assert_eq!(opened.key_kind, KeyKind::P256);
    assert!(opened.others.is_empty());
}

#[test]
fn a_file_with_no_passphrase_opens_with_none() {
    assert!(open(&bytes(EMPTY), "").is_ok());
    assert_eq!(open(&bytes(EMPTY), PASS).err(), Some(Error::WrongPassphrase));
}

#[test]
fn a_wrong_passphrase_is_said_as_one() {
    for file in [MODERN, LEGACY, EC] {
        assert_eq!(open(&bytes(file), "wrong").err(), Some(Error::WrongPassphrase));
    }
    // A passphrase with a character past the BMP can't be one a file was made with.
    assert_eq!(open(&bytes(MODERN), "\u{1F600}").err(), Some(Error::WrongPassphrase));
}

#[test]
fn a_changed_file_fails_its_mac() {
    let mut file = bytes(MODERN);
    // The MAC's digest is near the end: change its last byte, then a byte of the content.
    let last = file.len() - 30;
    file[last] ^= 1;
    assert_eq!(open(&file, PASS).err(), Some(Error::WrongPassphrase));
    let mut file = bytes(MODERN);
    file[200] ^= 0x40;
    assert!(open(&file, PASS).is_err());
}

#[test]
fn what_is_no_longer_safe_or_unvouched_for_is_refused() {
    assert_eq!(open(&bytes(RC2), PASS).err(), Some(Error::WeakEncryption));
    assert_eq!(open(&bytes(NO_MAC), PASS).err(), Some(Error::NoMac));
    assert_eq!(open(&bytes(SLOW), PASS).err(), Some(Error::Unreasonable));
    assert_eq!(open(&vec![0x30; MAX_P12_LEN + 1], PASS).err(), Some(Error::TooLarge));
    assert_eq!(open(b"not a file", PASS).err(), Some(Error::Unreadable));
    assert_eq!(open(ALICE_PEM.as_bytes(), PASS).err(), Some(Error::Unreadable));
}

#[test]
fn a_pkcs12_file_is_told_from_a_certificate() {
    for file in [MODERN, LEGACY, RC2, NO_MAC, EC] {
        assert!(looks_like(&bytes(file)));
    }
    assert!(!looks_like(&der(ALICE_PEM)));
    assert!(!looks_like(b""));
}

/// A DER element of `tag` around `content`.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes: Vec<u8> = len.to_be_bytes().into_iter().skip_while(|b| *b == 0).collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(content);
    out
}

fn seq(parts: &[Vec<u8>]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

fn oid(dotted: &str) -> Vec<u8> {
    use der::Encode;
    const_oid::ObjectIdentifier::new(dotted).unwrap().to_der().unwrap()
}

/// A bag of `kind` holding `value`, with the local key id `id`.
fn bag(kind: &str, value: Vec<u8>, id: &[u8]) -> Vec<u8> {
    let local_key_id = seq(&[oid("1.2.840.113549.1.9.21"), tlv(0x31, &tlv(0x04, id))]);
    seq(&[oid(kind), tlv(0xa0, &value), tlv(0x31, &local_key_id)])
}

/// A PKCS #12 file of one plain safe (no encryption at all) holding `key` in a plain key bag
/// and `certificate`, both with the local key id `id`, under a SHA-256 MAC with [`PASS`]: what
/// `openssl pkcs12 -export -keypbe NONE -certpbe NONE` writes, and a file that pairs any key with
/// any certificate.
fn plain_p12(key: &[u8], certificate: &[u8], id: &[u8]) -> Vec<u8> {
    use hmac::{KeyInit, Mac};
    let key_bag = bag("1.2.840.113549.1.12.10.1.1", key.to_vec(), id);
    let cert = seq(&[oid("1.2.840.113549.1.9.22.1"), tlv(0xa0, &tlv(0x04, certificate))]);
    let cert_bag = bag("1.2.840.113549.1.12.10.1.3", cert, id);
    let safe = seq(&[key_bag, cert_bag]);
    let data = |content: &[u8]| seq(&[oid("1.2.840.113549.1.7.1"), tlv(0xa0, &tlv(0x04, content))]);
    let auth_safe = seq(&[data(&safe)]);
    let salt = b"example salt";
    let password: Vec<u8> = PASS.encode_utf16().flat_map(u16::to_be_bytes).chain([0, 0]).collect();
    let mac_key = pkcs12::kdf::derive_key::<sha2::Sha256>(&password, salt, pkcs12::kdf::Pkcs12KeyType::Mac, 2048, 32).unwrap();
    let mut mac = <hmac::Hmac<sha2::Sha256> as KeyInit>::new_from_slice(&mac_key).unwrap();
    mac.update(&auth_safe);
    let digest = mac.finalize().into_bytes();
    let sha256 = seq(&[oid("2.16.840.1.101.3.4.2.1"), vec![0x05, 0x00]]);
    let mac_data = seq(&[seq(&[sha256, tlv(0x04, &digest)]), tlv(0x04, salt), tlv(0x02, &[0x08, 0x00])]);
    seq(&[tlv(0x02, &[3]), data(&auth_safe), mac_data])
}

/// The private keys of the fixtures, as PKCS #8.
fn rsa_key() -> Vec<u8> {
    open(&bytes(MODERN), PASS).unwrap().private_key.to_vec()
}

fn p256_key() -> Vec<u8> {
    open(&bytes(EC), PASS).unwrap().private_key.to_vec()
}

fn p256_certificate() -> Vec<u8> {
    open(&bytes(EC), PASS).unwrap().certificate
}

#[test]
fn the_kind_said_is_the_private_keys_own() {
    // A P-256 key with its own certificate, in a plain safe: it opens, and is said to be in
    // clear in the file.
    let opened = open(&plain_p12(&p256_key(), &p256_certificate(), b"1"), PASS).unwrap();
    assert_eq!(opened.key_kind, KeyKind::P256);
    assert!(!opened.key_encrypted, "a plain key bag in a plain safe");
    assert!(open(&bytes(EC), PASS).unwrap().key_encrypted);
    assert!(open(&bytes(MODERN), PASS).unwrap().key_encrypted);
    // An RSA key with a P-256 certificate sharing its local key id: no certificate is its.
    assert_eq!(open(&plain_p12(&rsa_key(), &p256_certificate(), b"1"), PASS).err(), Some(Error::NoCertificate));
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn any_bytes_open_without_a_panic(junk in proptest::collection::vec(any::<u8>(), 0..1024)) {
        let _ = open(&junk, PASS);
        let _ = looks_like(&junk);
    }

    #[test]
    fn a_damaged_file_opens_without_a_panic(at in 0usize..4096, byte in any::<u8>()) {
        static FILE: std::sync::LazyLock<Vec<u8>> = std::sync::LazyLock::new(|| bytes(EC));
        let mut file = FILE.clone();
        let len = file.len();
        if let Some(b) = file.get_mut(at % len) {
            *b = byte;
        }
        let _ = open(&file, PASS);
    }
}

pub(crate) const MODERN: &str = "\
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\
L4kCAggA";

const LEGACY: &str = "\
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\
BBA5cgeI/EDEl/gkbG7WnPM4AgIIAA==";

const RC2: &str = "\
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";

const NO_MAC: &str = "\
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";

const EC: &str = "\
MIIE1AIBAzCCBGIGCSqGSIb3DQEHAaCCBFMEggRPMIIESzCCAvoGCSqGSIb3DQEHAaCCAusEggLnMIIC4zCCAt8GCyqGSIb3DQEM\
CgEDoIICpzCCAqMGCiqGSIb3DQEJFgGgggKTBIICjzCCAoswggFzoAMCAQICEwWY7mm9ZFkbsOorJB5uhnxOV68wDQYJKoZIhvcN\
AQELBQAwLDEYMBYGA1UEAwwPRXhhbXBsZSBUZXN0IENBMRAwDgYDVQQKDAdFeGFtcGxlMCAXDTI2MTExMDIzNTczNFoYDzIxMjYx\
MDE3MjM1NzM0WjAWMRQwEgYDVQQDDAtCb2IgRXhhbXBsZTBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABFTBl+in37wcV2KoZBpe\
8HdIa3I67Y9fM249azdt06A2enfiMt60EGb1OYAn6qD/7HBmfSt++w736js0fPFRj6CjgYQwgYEwGgYDVR0RBBMwEYEPYm9iQGV4\
YW1wbGUub3JnMA4GA1UdDwEB/wQEAwIHgDATBgNVHSUEDDAKBggrBgEFBQcDBDAdBgNVHQ4EFgQUDX/llMpNqwOzpdvg6ZW6gpow\
7jkwHwYDVR0jBBgwFoAUA1dgSumWAOSyoT0IokPfLUhr+yYwDQYJKoZIhvcNAQELBQADggEBABUyJYbCAXO4DRftivaELtGfecBe\
lAyblNSgoObqXGpjfDbP4GR628maNzFSCSNpT9GaBtY2nsHyqwbTBbrZcrbXrE8IXhyF+QfXW4KjQXd4zl/VDL79lv997I2ocl/G\
RYq8jMPO22zCDkzJW/ihqgTw77YfuJy3TRG1KFDCYpt+N8jy+ua7E8nVCcjAUIGdyoEZFJmvznih0qh/gAZrHH1JF3dvgmml3R1E\
DBY8ZueV7uMWcBGHKwc4x+qd8zhyfvKibwl5Xd/NhtBqclmgKTR2IoZA4tojridkfdzmnrB2acqkp/AEByd9E8axNymFFExtlSrE\
N3rxIi94y3IQXgcxJTAjBgkqhkiG9w0BCRUxFgQUjInlR11rxzqXIkizr2OoL11D9JIwggFJBgkqhkiG9w0BBwGgggE6BIIBNjCC\
ATIwggEuBgsqhkiG9w0BDAoBAqCB9zCB9DBfBgkqhkiG9w0BBQ0wUjAxBgkqhkiG9w0BBQwwJAQQqWdl82FHAGkVnnehJZkd3wIC\
CAAwDAYIKoZIhvcNAgkFADAdBglghkgBZQMEAQIEEJgmrQYD1UonngDfy0Jb2UUEgZA+uRWJCzYL7FxB1NaX04qn17h2kOUbLwAT\
MC2Qj28y4C1jXfvwP2/jJ66n1CYMQ5jrOyp7b2kn1CF3jYyfvUcuy1eRX5re20d3MO3BvVyJLACAwfNrG3XQhPBiqhi238zukWbC\
Ub4PuKJIQ/w7Ro4e77aBGzhH1WN2okRbfbKJNwGO8+oD40su43K6/Pyg2ssxJTAjBgkqhkiG9w0BCRUxFgQUjInlR11rxzqXIkiz\
r2OoL11D9JIwaTBRMA0GCWCGSAFlAwQCAwUABEB4m6CI+/V15b2q/hZZfc/EPIUaI48ZKAFYhEF3kRVR2NvgJt69KYUM0148qW+S\
ddMTgHUukfMFkYfvSNHlPoByBBCW3D8EXEwst+W0w1AVuNT1AgIIAA==";

const EMPTY: &str = "\
MIIKZwIBAzCCChUGCSqGSIb3DQEHAaCCCgYEggoCMIIJ/jCCBGoGCSqGSIb3DQEHBqCCBFswggRXAgEAMIIEUAYJKoZIhvcNAQcB\
MF8GCSqGSIb3DQEFDTBSMDEGCSqGSIb3DQEFDDAkBBDF87jePepeWTUT3j7S2+0SAgIIADAMBggqhkiG9w0CCQUAMB0GCWCGSAFl\
AwQBKgQQMtr209mOOryP12JAnbtuGoCCA+ByOsav2ReVfVr96Z8DrUpT7Yk491/QW8XjxxzhgZjZ8Vomefv54s/bqm7gd3q9ygOl\
VMeA0zyOAvdnjFcy65fdJgAsjfj7p8rR46PWk8LsbV5OGgaueG+hDuNNwydEswqCrAN6E66X3X4zADJTzTLU85bbFdMGmdZ86Oly\
T4eXoQOO9Fc4Sxl6CDMYsSqXft1bdl9byBazMjSpZuvKu+B4OwJx/u7g8f88QoMzRwnai82y/Dn9+aqg4cPq9deK308Yaq7Q4hML\
uNHfpwUYjbULLBSq4NAzh0brm0vhjcFGJdjKi/S1KkiSFG7+xCzTPU9SFJ7AU/hti1Pe7ibP37dFm4IiYYylkiwNAPQs3q/P6sBc\
rBYOT6Dyg2e8sQlXvaLp3B/cOheP6vU2k6WbM/2RePREH4Amx2qVdrXvNEM+i1TXMZXdurelNAS7DaG0gyGhmwzeyxApH4QqwNdv\
DkgkW+sUONsxgJntlBNKEOyiRxi1YYCCFPgtbRucQ6lUSUjnzRr6HiuYApEk71aW/PZAMVhGKnJuoye2K59abhCyH8kxdz/umFzd\
PNOm5Bjf8oOTtrmYP7JpPcVrDbYOUtJnzhPZEcg+SE9p1gt/lx6GjvqE8+/tZKgSNtd99xFNMat32Qbn8HG75vu0xcq8aVrN17qJ\
fn68gd2UfGDsNfNwVcIWlTxsMlWHDfCL4QN3af4oFG1phxdMGlID7ALLcnC5VHc2fBaxH4DXTP2mriGGR0NMoZaZTqwyk16gbW5S\
5bK4AvANC+ej9+PjEEYoHCi5T5Pad7HhkRUHyyrqN/jc+idS8JIzQoSwIH6+PGaDjofhaxr7LtKODmQ3nIAcUMs7n5BqPdcC318H\
G9NqJ6zvAsbImrWVVYUkGhvA4T3qFqZqUgEzCf0NmTpyb0FHkGwSsmHlMdqr/DhicpBE3TU8Z8NJZOyYplaVRqOc3dRIGuzLqg/1\
32+qblfFnE/9XidGQepwCMumitELsptymiU1FwdRfLSQjIf6DVbHAy/37RtttDHTrHTK2W8EqCiSSCSHZ4G+ORgTt5jZAn4VQbnC\
BdtBwky2Cf/afKJYTgmV8He2KJ2DTzRX2APCQ4WRxu5wgnXA+aB/RwJyaXW653X0zh34tuaWTgikN8fJaeqTXq1cSc++vyDGVCpN\
V5igIWfeNi7bUab6+BaBLxyJ0pIlJtGE5CuVf1bS8vx9/NcA+lyLNKuRNZAC5roLuF+ZcoviCjiVqhVpi6vMo3LjGKxZ+Pc8zl+w\
G4W28K7Af2ZifdxR9H+5Rqa0ZiGFJOaS7akFeWHuMIML0ghkLrp0C1oG1jCCBYwGCSqGSIb3DQEHAaCCBX0EggV5MIIFdTCCBXEG\
CyqGSIb3DQEMCgECoIIFOTCCBTUwXwYJKoZIhvcNAQUNMFIwMQYJKoZIhvcNAQUMMCQEEBwwyhmnKSJx/KzrtErA4x0CAggAMAwG\
CCqGSIb3DQIJBQAwHQYJYIZIAWUDBAEqBBDf4dc8FXT+aYODSe2aT2gyBIIE0PsozHos8DialF+NaxO4hOL1U8n9PytgmrFH7MmM\
HMVc3ScUVcgyC94+ZQjBJ3JWOjDo/2k+aH4c3V2k85+mXutBQU+idGNz8gh1DjHKWri8oX6SaogwG0dIVTpgAkeSVApXI1Tdzb7d\
wxazLFIlFveL2bRGMGPpspMLgF+079AsL8phJ+EtI7G8kONL+406KYqIBr0nsqXravDIIFppB1pX83UqsBEs5Mxgkqh2P/Z9PXyQ\
1zBCTofzXnwDISsM9WgVZfLyOSchUaO+LXnfxNAsSzm40nBn6N/s3k//JYWzyGQHIK0i/0C09J8AbiS2sGQd2tdDoEYofqECeMRl\
JdjTv0wmJUMJldjSStDKWVNPTGBiGElWaNmDfhJrTXqIcVvoQRJZHvjCTqaYNqj12Oise2Y4p+u39U7NIVr3P1NoxdV3btaKUczd\
MtqML1dHQJ5/0KlZBR9KINVNu7XpQpcgEu/KaGEIn6JILvQs5ISjKNmahNsfoCpUGtfV3xF3JzwRYrT9dzL7mr9Mv3s7ArZ34n3m\
AqrGBGX6gQieMzYAtEJqUxtoNDck3SUNlgbWGs6Z9LHLrnMcCxN+k9xeanj2xnfaXNzCEcjzECvFHSr+RTCR6J+3sLj8QcAN0Pel\
BjJICGrNHyyo4SqbixYYYe3iRpKFedTEC8ydoVmmeR7MtpSNg9K7a9fbd8cRwb/xCbi2u5t3/kFKy4UAeGVme0Tq95gvGai4/5nV\
WkjzyXT1BPrnUIszrp0UNTs8MP6oJ1ndvKD4odlAdpQU0Ctl/eqQ+om7kqmWkSFIjUrex+YUv03euMsmUIqu6CpgCCcPV+SKHmU3\
LPnrGH+oVYU7bPYQJcPfvL0lgcWeDiTp7AHOTB/qxEQ0OgjFc+JjNrmDwhV8qgjvAHtXRJHhRseBEWJs61IHtZ0JMvap53WfIKEn\
IF0EihMfU1G3ht01uH4mBbUelfItlS9NGrMT+fXIy/qCB8a00doLv6b2xx2s+TBj1F1tXRe47ozNtWKxmTQw3gFhHoXXdRFX+XNa\
5H+Qm6uGagZp58JQ0ybeit3p1Ck0TO1jYRPeLW1CW06kHCHFsvoYebhGtd9WqNk9KygjVU2h9aCg+Z+8c+mZ3nqxHs0VPMMNGrGE\
TyUS4TechVigtVMJmj/cJA1Mme4eVyl7W2bdSvZntX+y9VTFMAJk8IHyxN6gOwvHgh/YfmvZQw/6ziL4BwEDcSVzJIGBW6X6PxcN\
4gQ5kZhfshXwUIEKRebtmo3fTKn8D2WhUVbu3nge1I2G46ol+F2eZMtQisgjbAAxbGLr5Slz4SxijMrVgo8QHXXQhwWU2eeyCrTF\
PKBUVHQG9CzNGRHsMmEmYk6WTyozR5ma2epCTYRPhR8fUxHxdjDuc/5qEWjPBg8m0FPCo0KbIYuhpZQTl52QRH+0mMs61kQsadow\
bGTs+NMLOk10piUoMTDjz7A5vJwI5cS4WYmWALpIOXCVwSMLs97yJDOmFj/gFx+vbyn6W9rciQljO93vnpt7BvSUG7RQNufkePDZ\
3VazbEmFWY0kvdOJDCeUF+AfqiOde94mOY5MNQx377q2yuBH4RNTGgY99r7/dJVd1/okNuLtK9wulLxqnZuV/iUDoF/egG4Egisd\
Z+MXMSUwIwYJKoZIhvcNAQkVMRYEFIVDIKpsK9ynC1lADgpTGrG04AAzMEkwMTANBglghkgBZQMEAgEFAAQgcAZkx5WYR7XZrovc\
Pgmgw0IuEAIj4FjBS78Zv6IAsScEEIk50Yy2OM0LI60aLfWObdcCAggA";

const SLOW: &str = "\
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";

pub(crate) const ALICE_PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIDfDCCAmSgAwIBAgITBZjuab1kWRuw6iskHm6GfE5XrjANBgkqhkiG9w0BAQsF\nADAsMRgwFgYDVQQDDA9FeGFtcGxlIFRlc3QgQ0ExEDAOBgNVBAoMB0V4YW1wbGUw\nIBcNMjYxMTEwMjM1NzM0WhgPMjEyNjEwMTcyMzU3MzRaMDoxFjAUBgNVBAMMDUFs\naWNlIEV4YW1wbGUxIDAeBgkqhkiG9w0BCQEWEWFsaWNlQGV4YW1wbGUub3JnMIIB\nIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAxQrWVfKXXVZryeb/vXPHnjxv\nQs0CL+YA3iCrERXXKQdPaul+Ysc4F7/2PM22JvjEU6robJAQjm6HJGK2fCy/ZzJD\npDgEx1fOUZ3uReGFfQJScb9rngD32q7UrDCfluzG+/W+IDQW3AQ2T9FlB8ABkih/\nkyVnFOZSKZHvj/B+lKmUDa1WvWXbsliROVzAQTvmhMYeeAxVwv6yiZ1WJXAvxbyD\nReatJH6C5IXiUtp3ETV0i/SSJbmE0hTl5NyP/lf9KQYOC+27sryxcaTlzNMuA90j\nO1WFY0lSAEXsNbwi8j1sW4cEip765ra9Y08MKce8dqtuXM/wjwrn70NSp3HW4wID\nAQABo4GGMIGDMBwGA1UdEQQVMBOBEWFsaWNlQGV4YW1wbGUub3JnMA4GA1UdDwEB\n/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDBDAdBgNVHQ4EFgQUzgoHCBtIIUUf\n0pUEHAOVQUQ3oOQwHwYDVR0jBBgwFoAUA1dgSumWAOSyoT0IokPfLUhr+yYwDQYJ\nKoZIhvcNAQELBQADggEBAHhlqgdhOJdXxg2JANwLzQvXXeEzXxu4yKGnJG9Zi99r\nohF5smEkNXT0qYjWEM/XHyynR5ozGsN4shlkezmAPkjqR1SYixYa0TgiCjlSYlQ+\ntxiNy4TtW2ZD23PBMGAUvixzCR9lsBrjf59l78Gqd3IyzYIzEApwxXuPPD8tussQ\njoE/QxepD8H3CPRVzBXSmuz0zGVgNtNtLQWv4D61f7RVETf0gUqjn1ezu3/0ih1M\nGkzEhaFVntSRuHw3KETY1Ez/LpzNM+w70UIL/M8XZTnbfzXa/5+RzQKcZLraQS6O\nZEPpVOU/V4dDTj04SPMJT9Ydh0v1UUWhazBvzsb4B7g=\n-----END CERTIFICATE-----\n";

pub(crate) const CA_PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIDSzCCAjOgAwIBAgIUQiksTfPBn790EMSMwNs3Jn0jaiUwDQYJKoZIhvcNAQEL\nBQAwLDEYMBYGA1UEAwwPRXhhbXBsZSBUZXN0IENBMRAwDgYDVQQKDAdFeGFtcGxl\nMCAXDTI2MTExMDIzNTczNFoYDzIxMjYxMDE3MjM1NzM0WjAsMRgwFgYDVQQDDA9F\neGFtcGxlIFRlc3QgQ0ExEDAOBgNVBAoMB0V4YW1wbGUwggEiMA0GCSqGSIb3DQEB\nAQUAA4IBDwAwggEKAoIBAQCLZYmAht/KhFBvK9uNCHWCQ0viLErOUqeX1aXmT1gD\nK7pnO17kenetbfhEY1tBB8bvhkLYcW+A10RZslY/ZMJsf8PTHA+B+gZB5GdIDtAC\nASHTmiBJIr9bdJiD+eZ1hDNfSvXm2dlF7qEnaaUj+GoowKnyqkGQ4EbfFcVT1rJR\n09Gx1yu1Ti5gKG4/BoAZ9Q/Ho/TGKXhCOLbecWzFzFHMbwLU+oY5rVjibp/FVbwT\nC54cuuqmwS1z+t/ABBvIUv5cVZloWXcoe1Sw2KSN7IaGGM3ziCSDBM6RZiUsD/Fr\n6CkW5m2c+XPOSTwfHPz8h39bCITS0DbB86fXDy7kZQTTAgMBAAGjYzBhMB0GA1Ud\nDgQWBBQDV2BK6ZYA5LKhPQiiQ98tSGv7JjAfBgNVHSMEGDAWgBQDV2BK6ZYA5LKh\nPQiiQ98tSGv7JjAPBgNVHRMBAf8EBTADAQH/MA4GA1UdDwEB/wQEAwIBBjANBgkq\nhkiG9w0BAQsFAAOCAQEAaT1BKjztiC5bgQ3hT0zRsqSLXlzGGRYAMC49eLPAAPvO\n6CL+ZPvNByZKTRy6pt+Tr06nbj7YH92z4xO601vILQBmBwwhsQNMotKDZ/Qedb8t\ng6maxc83l5rKOR/XHze6nIoFQR1uYWrvLT/JpwTZ9SfDk5pbMVCRyExACSAQL3NF\nS1boSb2RndOIHAcwKUF7uEYm+uoMbttCFcFZTcNcO1swL43/562cJ8w2AWwCBv4j\nz0Kcna6idZx/eTZxkHizZwMGzJL9Hzbvui0gPBu0RNAbBUTeJawC/1IOEFQq43Fj\nMxttmH5D5Ax3nffhFlrAH2IgjiHTI+IbzIzNR1J0Xw==\n-----END CERTIFICATE-----\n";

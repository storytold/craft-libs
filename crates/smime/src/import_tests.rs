// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Certificates read for import, and private keys told apart (#30). The certificates are those
//! of `pkcs12_tests` (synthetic `example.org` people under a test authority), and one made for a
//! web server with Bob's key.

use proptest::prelude::*;

use crate::import::{certificate_info, certificates_in, holds_private_key};
use crate::pkcs12_tests::{ALICE_PEM, CA_PEM, MODERN};

/// Bob's key, certified for `www.example.org` and for serving the web only.
pub(crate) const SERVER_PEM: &str = "-----BEGIN CERTIFICATE-----\nMIICeTCCAWGgAwIBAgITBZjuab1kWRuw6iskHm6GfE5XsDANBgkqhkiG9w0BAQsF\nADAsMRgwFgYDVQQDDA9FeGFtcGxlIFRlc3QgQ0ExEDAOBgNVBAoMB0V4YW1wbGUw\nIBcNMjYxMTEwMjM1NzM0WhgPMjEyNjEwMTcyMzU3MzRaMBYxFDASBgNVBAMMC0Jv\nYiBFeGFtcGxlMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMGX6KffvBxXYqhk\nGl7wd0hrcjrtj18zbj1rN23ToDZ6d+Iy3rQQZvU5gCfqoP/scGZ9K377DvfqOzR8\n8VGPoKNzMHEwGgYDVR0RBBMwEYIPd3d3LmV4YW1wbGUub3JnMBMGA1UdJQQMMAoG\nCCsGAQUFBwMBMB0GA1UdDgQWBBQNf+WUyk2rA7Ol2+DplbqCmjDuOTAfBgNVHSME\nGDAWgBQDV2BK6ZYA5LKhPQiiQ98tSGv7JjANBgkqhkiG9w0BAQsFAAOCAQEAUlZu\nif7xv5ZXlsWt0KX458+G2vekBcqdE3UD21hTvJ9tbCrnfIuXa9pigZsYUzqV5Xwt\npaXU+7JedPYNZs0qqk2nPw+VULsxBNhaKre5V+v4tdHxq16MTsAalT6GfeOtX8+G\nZ5CwUqqe5SW9kkUi0QnaR/BDZuTKIOKKBCJ4AEOmi9cbDWUQiKkQRR4mEQH7wAJM\nfBXlNb7YHRikSE/ZHAjsLKNwSEpOsKnKav8KxPy/moR+emB9AFIHAsGjmPTMNhdG\nQ5DrRP3eAtKBGCrFGOj+JSxXW4O2eqKGuMHy7fLrCD2st7qBv4Lm7lEw1SDsXAQ/\nx1BBQKVBaJYfE2pKOA==\n-----END CERTIFICATE-----\n";

#[test]
fn certificates_are_read_from_pem_and_der() {
    let both = format!("{ALICE_PEM}{CA_PEM}");
    let read = certificates_in(both.as_bytes());
    assert_eq!(read.len(), 2);
    assert_eq!(certificates_in(&read[0]), vec![read[0].clone()], "DER");
    assert!(certificates_in(b"no certificate").is_empty());
    assert!(certificates_in(b"").is_empty());
}

#[test]
fn what_a_certificate_is_for_is_told() {
    let alice = certificate_info(&certificates_in(ALICE_PEM.as_bytes())[0]).unwrap();
    assert_eq!(alice.addresses, vec!["alice@example.org".to_string()], "its alternative name and its subject's address, once");
    assert!(alice.for_mail);
    assert!(!alice.authority);
    assert_eq!(alice.fingerprint, "10:33:B4:36:A1:40:16:60:4A:21:D5:BC:F3:B1:69:38:60:22:DF:01:85:71:8D:BC:F5:BB:63:4B:09:CB:AA:FC");
    assert!(alice.not_after > 4_000_000_000);
    let server = certificate_info(&certificates_in(SERVER_PEM.as_bytes())[0]).unwrap();
    assert!(!server.for_mail, "for the web only");
    assert!(server.addresses.is_empty());
    let ca = certificate_info(&certificates_in(CA_PEM.as_bytes())[0]).unwrap();
    assert!(ca.authority);
    assert!(certificate_info(b"junk").is_none());
}

#[test]
fn a_private_key_in_any_form_is_told() {
    for label in ["PRIVATE KEY", "ENCRYPTED PRIVATE KEY", "RSA PRIVATE KEY", "EC PRIVATE KEY", "OPENSSH PRIVATE KEY", "PGP PRIVATE KEY BLOCK"] {
        let pem = format!("-----BEGIN {label}-----\nAAAA\n-----END {label}-----\n");
        assert!(holds_private_key(pem.as_bytes()), "{label}");
    }
    assert!(holds_private_key(&mailcraft_charset::base64::STANDARD.decode(MODERN).unwrap()), "a PKCS #12 file");
    // PKCS #8: version 0, an algorithm, the key in an OCTET STRING.
    let pkcs8 = [0x30, 0x0b, 0x02, 0x01, 0x00, 0x30, 0x03, 0x06, 0x01, 0x2a, 0x04, 0x01, 0x00];
    assert!(holds_private_key(&pkcs8));
    // EncryptedPrivateKeyInfo.
    assert!(holds_private_key(&[0x30, 0x08, 0x30, 0x03, 0x06, 0x01, 0x2a, 0x04, 0x01, 0x00]));
    // SEC 1: version 1, the key.
    assert!(holds_private_key(&[0x30, 0x06, 0x02, 0x01, 0x01, 0x04, 0x01, 0x07]));
    // PKCS #1: nine INTEGERs, the first 0.
    let mut rsa = vec![0x30, 27];
    for _ in 0..9 {
        rsa.extend_from_slice(&[0x02, 0x01, 0x00]);
    }
    assert!(holds_private_key(&rsa));
    for public in [ALICE_PEM.as_bytes(), &certificates_in(ALICE_PEM.as_bytes())[0], b"-----BEGIN PGP PUBLIC KEY BLOCK-----", b""] {
        assert!(!holds_private_key(public));
    }
}

proptest! {
    #[test]
    fn any_bytes_are_read_without_a_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let _ = certificates_in(&bytes);
        let _ = certificate_info(&bytes);
        let _ = holds_private_key(&bytes);
    }
}

use proptest::prelude::*;

use crate::fake::{Canned, FakeAgent};
use crate::{Digest, Error, ErrorKind, GpgAgent, HashAlgorithm, KeyStorage, Keygrip, PinentryHints, Protection, Sexp};

const GRIP: &str = "0123456789ABCDEF0123456789ABCDEF01234567";
const OTHER: &str = "89ABCDEF0123456789ABCDEF0123456789ABCDEF";

fn grip() -> Keygrip {
    Keygrip::parse(GRIP).unwrap()
}

#[test]
fn keygrips() {
    assert_eq!(grip().to_string(), GRIP);
    assert_eq!(Keygrip::parse(&GRIP.to_lowercase()), Some(grip()));
    for bad in ["", "0123", &format!("{GRIP}0"), &GRIP.replace('A', "G"), "é123456789ABCDEF0123456789ABCDEF0123456"] {
        assert_eq!(Keygrip::parse(bad), None, "{bad}");
    }
}

#[test]
fn only_sha2_digests_of_the_right_size() {
    assert!(Digest::new(HashAlgorithm::Sha256, &[0; 32]).is_some());
    assert!(Digest::new(HashAlgorithm::Sha384, &[0; 48]).is_some());
    assert!(Digest::new(HashAlgorithm::Sha512, &[0; 64]).is_some());
    // A SHA-1 or MD5 sized digest is no SHA-256 one.
    assert!(Digest::new(HashAlgorithm::Sha256, &[0; 20]).is_none());
    assert!(Digest::new(HashAlgorithm::Sha256, &[0; 16]).is_none());
    assert!(Digest::new(HashAlgorithm::Sha512, &[0; 32]).is_none());
}

#[test]
fn have_key() {
    let agent = FakeAgent::new()
        .on(&format!("HAVEKEY {GRIP}"), "OK\n")
        .on(&format!("HAVEKEY {GRIP}"), "ERR 67108881 No secret key <GPG Agent>\n")
        .on(&format!("HAVEKEY {GRIP}"), "ERR 67109041 IPC call has been cancelled\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    assert!(agent.have_key(&grip()).unwrap());
    assert!(!agent.have_key(&grip()).unwrap());
    assert!(matches!(agent.have_key(&grip()), Err(Error::Server(_))));
}

#[test]
fn key_info_on_disk_and_on_a_card() {
    let agent = FakeAgent::new()
        .on(&format!("KEYINFO {GRIP}"), format!("S KEYINFO {GRIP} D - - 1 P - - -\nOK\n"))
        .on(&format!("KEYINFO {GRIP}"), format!("S KEYINFO {GRIP} T D2760001240103040006123456780000 OPENPGP.1 - - - - -\nOK\n"))
        .on(&format!("KEYINFO {GRIP}"), "ERR 67108891 Not found <GPG Agent>\n")
        .on(&format!("KEYINFO {GRIP}"), format!("S KEYINFO {OTHER} D - - - P - - -\nOK\n"));
    let mut agent = GpgAgent::open(agent).unwrap();
    let disk = agent.key_info(&grip()).unwrap().unwrap();
    assert_eq!(
        (disk.storage, disk.passphrase_cached, disk.protection, disk.card_serial.as_deref()),
        (KeyStorage::Disk, true, Protection::Protected, None)
    );
    let card = agent.key_info(&grip()).unwrap().unwrap();
    assert_eq!(card.storage, KeyStorage::Smartcard);
    assert_eq!(card.card_serial.as_deref(), Some("D2760001240103040006123456780000"));
    assert_eq!(card.card_key.as_deref(), Some("OPENPGP.1"));
    assert_eq!(card.protection, Protection::Unknown);
    assert_eq!(agent.key_info(&grip()).unwrap(), None);
    // An answer about another key is not taken for this one.
    assert!(matches!(agent.key_info(&grip()), Err(Error::Protocol(_))));
}

#[test]
fn list_keys_skips_lines_it_cant_read() {
    let agent = FakeAgent::new()
        .on("KEYINFO --list", format!("S KEYINFO {GRIP} D - - - C - - -\nS KEYINFO nonsense\nS PROGRESS 1\nS KEYINFO {OTHER} X - - - - - - -\nOK\n"));
    let mut agent = GpgAgent::open(agent).unwrap();
    let keys = agent.list_keys().unwrap();
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[0].protection, Protection::Clear);
    assert_eq!(keys[1].storage, KeyStorage::Unknown);
}

#[test]
fn read_key() {
    let agent = FakeAgent::new().on(&format!("READKEY {GRIP}"), "D (10:public-key(3:ecc(5:curve7:Ed25519)(1:q2:%0A%25)))\nOK\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    let key = agent.read_key(&grip()).unwrap();
    assert_eq!(key.find("ecc").and_then(|e| e.value("q")), Some(&b"\n%"[..]));
}

#[test]
fn signing_a_digest_sends_the_key_the_description_and_the_hash() {
    let digest = Digest::new(HashAlgorithm::Sha256, &[0xAB; 32]).unwrap();
    let agent = FakeAgent::new()
        .on(&format!("SIGKEY {GRIP}"), "OK\n")
        .on("SETKEYDESC Sign+a+message+from+ann@example.org%0A100%25+sure", "OK\n")
        .on(&format!("SETHASH 8 {}", "AB".repeat(32)), "OK\n")
        .on("PKSIGN", "INQUIRE PINENTRY_LAUNCHED 1234 gtk2 1.2.1 - - -\n")
        .on("END", "S PROGRESS x\nD (7:sig-val(3:rsa(1:s4:\x01\x02\x03\x04)))\nOK\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    let sig = agent.sign(&grip(), &digest, Some("Sign a message from ann@example.org\n100% sure")).unwrap();
    assert_eq!(sig.algorithm, "rsa");
    assert_eq!(sig.part("s"), Some(&b"\x01\x02\x03\x04"[..]));
    assert_eq!(sig.part("r"), None);
    let fake = agent.close();
    assert!(fake.wrong.is_empty(), "{:?}", fake.wrong);
}

#[test]
fn an_ecdsa_signature_and_sha512() {
    let digest = Digest::new(HashAlgorithm::Sha512, &[1; 64]).unwrap();
    let agent = FakeAgent::new()
        .on(&format!("SIGKEY {GRIP}"), "OK\n")
        .on("SETHASH 10 *", "OK\n")
        .on("PKSIGN", "D (7:sig-val(5:ecdsa(1:r1:%01)(1:s1:%02)))\nOK\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    let sig = agent.sign(&grip(), &digest, None).unwrap();
    assert_eq!((sig.algorithm.as_str(), sig.part("r"), sig.part("s")), ("ecdsa", Some(&[1u8][..]), Some(&[2u8][..])));
}

#[test]
fn a_cancelled_pinentry_and_a_bad_signature() {
    let digest = Digest::new(HashAlgorithm::Sha256, &[0; 32]).unwrap();
    let agent = FakeAgent::new()
        .on("SIGKEY *", "OK\n")
        .on("SETHASH *", "OK\n")
        .on("PKSIGN", "ERR 83886179 Operation cancelled <Pinentry>\n")
        .on("SIGKEY *", "OK\n")
        .on("SETHASH *", "OK\n")
        .on("PKSIGN", "D (6:public)\nOK\n")
        .on("SIGKEY *", "OK\n")
        .on("SETHASH *", "OK\n")
        .on("PKSIGN", "D (7:sig-val)\nOK\n")
        .on("SIGKEY *", "OK\n")
        .on("SETHASH *", "OK\n")
        .on("PKSIGN", "D junk\nOK\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    let e = agent.sign(&grip(), &digest, None).unwrap_err();
    assert_eq!(e.server().map(|e| e.kind()), Some(ErrorKind::Canceled));
    for _ in 0..3 {
        assert!(matches!(agent.sign(&grip(), &digest, None), Err(Error::Protocol(_))));
    }
}

#[test]
fn an_inquiry_other_than_pinentry_launched_is_refused_during_signing() {
    let digest = Digest::new(HashAlgorithm::Sha256, &[0; 32]).unwrap();
    let agent = FakeAgent::new().on("SIGKEY *", "OK\n").on("SETHASH *", "OK\n").on("PKSIGN", "INQUIRE CIPHERTEXT\n").on("CAN", "ERR 99 cancelled\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    assert!(matches!(agent.sign(&grip(), &digest, None), Err(Error::Server(_))));
    assert!(agent.close().done());
}

#[test]
fn a_too_long_description_is_never_sent() {
    let digest = Digest::new(HashAlgorithm::Sha256, &[0; 32]).unwrap();
    let mut agent = GpgAgent::open(FakeAgent::new()).unwrap();
    assert!(matches!(agent.sign(&grip(), &digest, Some(&"x".repeat(301))), Err(Error::Refused(_))));
    assert!(agent.close().sent == ["BYE"]);
}

fn ciphertext() -> Sexp {
    crate::sexp::parse(b"(7:enc-val(3:rsa(1:a3:\n\r%)))").unwrap()
}

#[test]
fn decrypting_answers_the_ciphertext_inquiry() {
    let agent = FakeAgent::new()
        .on(&format!("SETKEY {GRIP}"), "OK\n")
        .on("SETKEYDESC Open+a+message", "OK\n")
        .on("PKDECRYPT", "S INQUIRE_MAXLEN 4096\nINQUIRE CIPHERTEXT\n")
        .on("D (7:enc-val(3:rsa(1:a3:%0A%0D%25)))", "")
        // gpg-agent ends the value with a NUL.
        .on("END", "S PADDING 0\nD (5:value6:\x09key%0A!)\x00\nOK\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    let key = agent.decrypt(&grip(), &ciphertext(), Some("Open a message")).unwrap();
    assert_eq!(key.bytes(), b"\x09key\n!");
    assert_eq!(key.padding(), Some(0));
    assert_eq!(format!("{key:?}"), "SessionKey(<6 bytes>)");
    assert!(agent.close().wrong.is_empty());
}

#[test]
fn a_decrypted_value_must_be_a_value() {
    let agent = FakeAgent::new()
        .on("SETKEY *", "OK\n")
        .on("PKDECRYPT", "INQUIRE CIPHERTEXT\n")
        .on("D *", "")
        .on("END", "D (5:other1:x)\nOK\n")
        .on("SETKEY *", "OK\n")
        .on("PKDECRYPT", "INQUIRE CIPHERTEXT\n")
        .on("D *", "")
        .on("END", "D (5:value(1:x))\nOK\n")
        .on("SETKEY *", "OK\n")
        .on("PKDECRYPT", "ERR 67108881 No secret key\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    assert!(matches!(agent.decrypt(&grip(), &ciphertext(), None), Err(Error::Protocol(_))));
    assert!(matches!(agent.decrypt(&grip(), &ciphertext(), None), Err(Error::Protocol(_))));
    let e = agent.decrypt(&grip(), &ciphertext(), None).unwrap_err();
    assert_eq!(e.server().map(|e| e.kind()), Some(ErrorKind::NoSecretKey));
    // Only an enc-val is sent at all.
    assert!(matches!(agent.decrypt(&grip(), &Sexp::token("x"), None), Err(Error::Refused(_))));
}

#[test]
fn pinentry_hints_become_options_and_a_refused_one_is_skipped() {
    let env = |name: &str| match name {
        "DISPLAY" => Some(":0".to_string()),
        "TERM" => Some("xterm-256color".to_string()),
        "XAUTHORITY" => Some("bad\nvalue".to_string()),
        "WAYLAND_DISPLAY" => Some("wayland-0".to_string()),
        "LC_CTYPE" => Some(String::new()),
        _ => None,
    };
    let hints = PinentryHints::from_env(env);
    let agent = FakeAgent::new()
        .on("OPTION display=:0", "OK\n")
        .on("OPTION ttytype=xterm-256color", "ERR 67109115 Unknown option\n")
        .on("OPTION putenv=WAYLAND_DISPLAY=wayland-0", "OK\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    agent.set_pinentry_hints(&hints).unwrap();
    assert!(agent.close().wrong.is_empty());
}

#[test]
fn version_and_reset() {
    let agent = FakeAgent::new().on("GETINFO version", "D 2.4.7\nOK\n").on("RESET", "OK\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    assert_eq!(agent.version().unwrap(), "2.4.7");
    agent.reset().unwrap();
}

proptest! {
    #[test]
    fn any_agent_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let mut input = b"OK\n".to_vec();
        input.extend_from_slice(&bytes);
        let Ok(mut agent) = GpgAgent::open(Canned::new(&input)) else { return Ok(()) };
        let digest = Digest::new(HashAlgorithm::Sha256, &[7; 32]).unwrap();
        let _ = agent.key_info(&grip());
        let _ = agent.list_keys();
        let _ = agent.sign(&grip(), &digest, Some("x"));
        let _ = agent.decrypt(&grip(), &ciphertext(), None);
        let _ = agent.read_key(&grip());
    }
}

#[test]
fn an_rsa_value_without_padding_status_keeps_its_frame_and_only_one_nul_is_taken_off() {
    // gpg-agent 2.4 answers an OpenPGP RSA decryption so: no PADDING, the PKCS#1 v1.5 frame
    // still on (its leading zero left out), and a NUL after the S-expression, escaped or not.
    let frame = b"\x02\x11\x22\x33\x00\x09KEY";
    let reply = |nul: &str| {
        let mut r = b"D (5:value9:".to_vec();
        r.extend_from_slice(frame);
        r.extend_from_slice(format!("){nul}\nOK\n").as_bytes());
        r
    };
    let agent = FakeAgent::new()
        .on("SETKEY *", "OK\n")
        .on("PKDECRYPT", "INQUIRE CIPHERTEXT\n")
        .on("D *", "")
        .on("END", reply("%00"))
        .on("SETKEY *", "OK\n")
        .on("PKDECRYPT", "INQUIRE CIPHERTEXT\n")
        .on("D *", "")
        .on("END", reply("\x00\x00"));
    let mut agent = GpgAgent::open(agent).unwrap();
    let key = agent.decrypt(&grip(), &ciphertext(), None).unwrap();
    assert_eq!(key.padding(), None);
    assert_eq!(key.bytes(), frame);
    assert!(matches!(agent.decrypt(&grip(), &ciphertext(), None), Err(Error::Protocol(_))));
}

#[test]
fn more_keys_than_status_lines_an_answer_keeps() {
    let mut lines = String::new();
    for i in 0..(crate::MAX_KEYS + 500) {
        lines.push_str(&format!("S KEYINFO {i:040X} D - - - P - - -\n"));
    }
    lines.push_str("OK\n");
    let agent = FakeAgent::new().on("KEYINFO --list", lines).on("GETINFO version", "D 2.4.9\nOK\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    assert_eq!(agent.list_keys().unwrap().len(), crate::MAX_KEYS);
    // The connection is still in step.
    assert_eq!(agent.version().unwrap(), "2.4.9");
}

#[test]
fn each_signature_starts_with_reset_so_no_old_description_is_left() {
    let digest = Digest::new(HashAlgorithm::Sha256, &[0; 32]).unwrap();
    let agent = FakeAgent::new()
        .on("SIGKEY *", "OK\n")
        .on("SETKEYDESC Old+text", "OK\n")
        .on("SETHASH *", "ERR 1 bad hash\n")
        .on("SIGKEY *", "OK\n")
        .on("SETHASH *", "OK\n")
        .on("PKSIGN", "D (7:sig-val(3:rsa(1:s1:x)))\nOK\n")
        .on("SETKEY *", "OK\n")
        .on("PKDECRYPT", "ERR 99 cancelled\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    assert!(agent.sign(&grip(), &digest, Some("Old text")).is_err());
    agent.sign(&grip(), &digest, None).unwrap();
    assert!(agent.decrypt(&grip(), &ciphertext(), None).is_err());
    let fake = agent.close();
    assert!(fake.wrong.is_empty(), "{:?}", fake.wrong);
    let firsts: Vec<&str> = fake.sent.iter().map(|l| l.split(' ').next().unwrap()).filter(|c| ["RESET", "SIGKEY", "SETKEY"].contains(c)).collect();
    assert_eq!(firsts, ["RESET", "SIGKEY", "RESET", "SIGKEY", "RESET", "SETKEY"]);
}

#[test]
fn the_owner_wait_hook_is_on_only_for_the_commands_that_wait() {
    let digest = Digest::new(HashAlgorithm::Sha256, &[0; 32]).unwrap();
    let agent = FakeAgent::new()
        .on("SIGKEY *", "OK\n")
        .on("SETHASH *", "OK\n")
        .on("PKSIGN", "INQUIRE PINENTRY_LAUNCHED 1\n")
        .on("END", "ERR 99 cancelled\n")
        .on("SETKEY *", "OK\n")
        .on("PKDECRYPT", "INQUIRE CIPHERTEXT\n")
        .on("D *", "")
        .on("END", "D (5:value1:k)\nOK\n")
        .on("HAVEKEY *", "OK\n");
    let mut agent = GpgAgent::open(agent).unwrap();
    agent.set_owner_wait(|fake: &mut FakeAgent, long| fake.long_wait = long);
    assert!(agent.sign(&grip(), &digest, None).is_err());
    agent.decrypt(&grip(), &ciphertext(), None).unwrap();
    agent.have_key(&grip()).unwrap();
    let fake = agent.close();
    let long: Vec<&str> = fake.waits.iter().filter(|(_, long)| *long).map(|(l, _)| l.split(' ').next().unwrap()).collect();
    assert_eq!(long, ["PKSIGN", "END", "PKDECRYPT", "D", "END"]);
    assert!(!fake.long_wait);
}

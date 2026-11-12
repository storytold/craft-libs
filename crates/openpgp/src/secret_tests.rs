//! Secret keys read for import (#30 R13), without anything decrypted: synthetic `example.org`
//! keys only.

use pgp::composed::{ArmorOptions, SignedSecretKey};
use pgp::ser::Serialize;
use pgp::types::Password;
use proptest::prelude::*;
use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::fixtures::{Make, armoured_public, public, secret};
use crate::secret::{SigningKind, holds_secret, read};
use crate::{Error, KeyRing};

fn protect(key: &mut SignedSecretKey, primary: bool, subkeys: bool) {
    let password: Password = "correct horse".into();
    if primary {
        key.primary_key.set_password(StdRng::seed_from_u64(3), &password).unwrap();
    }
    if subkeys {
        for sub in &mut key.secret_subkeys {
            sub.key.set_password(StdRng::seed_from_u64(4), &password).unwrap();
        }
    }
}

#[test]
fn a_secret_key_is_read_armoured_or_binary_without_being_decrypted() {
    let key = secret("Ada <ada@example.org>", Make::default());
    let armoured = key.to_armored_string(ArmorOptions::default()).unwrap();
    for bytes in [armoured.as_bytes().to_vec(), key.to_bytes().unwrap()] {
        let info = read(&bytes).unwrap();
        assert_eq!(info.addresses, vec!["ada@example.org".to_string()]);
        assert_eq!(&info.fingerprint, public(&key).fingerprint());
        assert!(!info.protected, "a key with no passphrase");
        assert!(info.can_sign);
        assert_eq!(info.signing_kind, Some(SigningKind::EdDsa));
        // Its public half goes in a ring and finds its signatures.
        let ring: KeyRing = std::iter::once(info.public).collect();
        assert_eq!(ring.for_address("ada@example.org").count(), 1);
        assert!(holds_secret(&bytes));
    }
}

#[test]
fn whether_every_secret_part_is_protected_is_told() {
    let mut key = secret("Ada <ada@example.org>", Make { subkey: true, ..Make::default() });
    protect(&mut key, true, false);
    let info = read(&key.to_bytes().unwrap()).unwrap();
    assert!(!info.protected, "its subkey isn't");
    assert!(info.can_sign, "its signing subkey is here");
    protect(&mut key, false, true);
    assert!(read(&key.to_bytes().unwrap()).unwrap().protected);
}

#[test]
fn a_key_whose_signing_subkey_isnt_here_cant_sign() {
    let mut key = secret("Ada <ada@example.org>", Make { subkey: true, ..Make::default() });
    key.secret_subkeys.clear();
    let info = read(&key.to_bytes().unwrap()).unwrap();
    assert!(!info.can_sign, "the primary key may only certify");
    assert_eq!(info.signing_kind, None);
}

#[test]
fn an_rsa_key_says_so() {
    // (RSA keys are never used in this process: the window refuses them.)
    let key = secret("Ada <ada@example.org>", Make { rsa: Some(2048), ..Make::default() });
    let info = read(&key.to_bytes().unwrap()).unwrap();
    assert_eq!(info.signing_kind, Some(SigningKind::Rsa));
    assert!(info.any_rsa);
}

/// An Ed25519 key that signs, with an RSA subkey for encryption: an RSA part all the same.
#[test]
fn an_rsa_encryption_subkey_says_so_too() {
    let key = secret("Ada <ada@example.org>", Make { rsa_encryption: true, ..Make::default() });
    let info = read(&key.to_bytes().unwrap()).unwrap();
    assert_eq!(info.signing_kind, Some(SigningKind::EdDsa));
    assert!(info.any_rsa);
    let plain = read(&secret("Ada <ada@example.org>", Make::default()).to_bytes().unwrap()).unwrap();
    assert!(!plain.any_rsa);
}

/// The whole file is kept, so the one key checked must be all there is in it.
#[test]
fn a_file_with_more_than_one_key_is_refused() {
    let ada = secret("Ada <ada@example.org>", Make::default());
    let mut bea = secret("Bea <bea@example.org>", Make { seed: 2, ..Make::default() });
    protect(&mut bea, true, true);
    let (a, b) = (ada.to_armored_string(ArmorOptions::default()).unwrap(), bea.to_armored_string(ArmorOptions::default()).unwrap());
    assert_eq!(read(format!("{b}\n{a}").as_bytes()).err(), Some(Error::SeveralKeys), "two armour blocks");
    assert_eq!(read(format!("{b}\n{}", armoured_public(&ada)).as_bytes()).err(), Some(Error::SeveralKeys), "a key and another block");
    let mut both = bea.to_bytes().unwrap();
    both.extend(ada.to_bytes().unwrap());
    assert_eq!(read(&both).err(), Some(Error::SeveralKeys), "two binary keys");
    let mut trailing = bea.to_bytes().unwrap();
    trailing.extend(b"\0and then something else");
    assert_eq!(read(&trailing).err(), Some(Error::SeveralKeys), "something after the key");
    assert!(read(&bea.to_bytes().unwrap()).is_ok());
    assert!(read(b.as_bytes()).is_ok());
}

/// A passphrase that is empty protects nothing.
#[test]
fn an_empty_passphrase_is_no_protection() {
    let mut key = secret("Ada <ada@example.org>", Make { subkey: true, ..Make::default() });
    let empty = Password::empty();
    key.primary_key.set_password(StdRng::seed_from_u64(3), &empty).unwrap();
    for sub in &mut key.secret_subkeys {
        sub.key.set_password(StdRng::seed_from_u64(4), &empty).unwrap();
    }
    assert!(key.primary_key.secret_params().is_encrypted());
    assert!(!read(&key.to_bytes().unwrap()).unwrap().protected, "it opens with nothing typed");
    // Only the subkey's empty: still not protected.
    let mut key = secret("Ada <ada@example.org>", Make { subkey: true, ..Make::default() });
    protect(&mut key, true, false);
    for sub in &mut key.secret_subkeys {
        sub.key.set_password(StdRng::seed_from_u64(4), &empty).unwrap();
    }
    assert!(!read(&key.to_bytes().unwrap()).unwrap().protected);
    for sub in &mut key.secret_subkeys {
        sub.key.remove_password(&empty).unwrap();
    }
    protect(&mut key, false, true);
    assert!(read(&key.to_bytes().unwrap()).unwrap().protected);
}

#[test]
fn a_public_key_is_no_secret_key() {
    let key = secret("Ada <ada@example.org>", Make::default());
    let public = armoured_public(&key);
    assert_eq!(read(public.as_bytes()).err(), Some(Error::Unreadable));
    assert!(!holds_secret(public.as_bytes()));
    assert_eq!(read(&vec![0x95; crate::keys::MAX_KEY_BYTES + 1]).err(), Some(Error::TooLarge));
    assert!(read(b"").is_err());
}

/// A secret key wearing a Public key's armour is still told for one.
#[test]
fn a_secret_key_in_a_public_keys_armour_is_told() {
    let key = secret("Ada <ada@example.org>", Make::default());
    let armoured = key.to_armored_string(ArmorOptions::default()).unwrap().replace("PRIVATE KEY BLOCK", "PUBLIC KEY BLOCK");
    assert!(holds_secret(armoured.as_bytes()));
}

proptest! {
    #[test]
    fn any_bytes_read_without_a_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let _ = read(&bytes);
        let _ = holds_secret(&bytes);
    }

    #[test]
    fn a_damaged_secret_key_reads_without_a_panic(at in 0usize..400, byte in any::<u8>()) {
        static KEY: std::sync::LazyLock<Vec<u8>> = std::sync::LazyLock::new(|| secret("Ada <ada@example.org>", Make::default()).to_bytes().unwrap());
        let mut bytes = KEY.clone();
        let len = bytes.len();
        if let Some(b) = bytes.get_mut(at % len) {
            *b = byte;
        }
        let _ = read(&bytes);
    }
}

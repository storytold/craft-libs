//! The header's profile ID (ICC.1:2022 §7.2.18).

use craft_cms::{Builtin, CmsError, Profile, builtin, profile_id};

fn hex(id: [u8; 16]) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn the_id_matches_an_independent_md5() {
    // Computed outside Rust: MD5 of the shipped file with bytes 44–47, 64–67 and 84–99 zeroed.
    assert_eq!(hex(profile_id(builtin::COATED_CMYK_ICC).unwrap()), "ab1397490b9355815dea5668de6eb073");
}

#[test]
fn encoded_profiles_carry_their_id() {
    for b in [Builtin::Srgb, Builtin::LabD50, Builtin::CoatedFogra39] {
        let p = b.profile().clone();
        let bytes = p.with_encoded_bytes().to_bytes();
        let id = profile_id(&bytes).unwrap();
        assert_ne!(id, [0; 16], "{b:?}");
        assert_eq!(bytes[84..100], id, "{b:?}");
        assert!(Profile::parse(&bytes).is_ok(), "{b:?}");
    }
}

#[test]
fn flags_and_intent_are_left_out_but_tag_data_is_not() {
    let bytes = Builtin::Srgb.profile().to_bytes().to_vec();
    let id = profile_id(&bytes).unwrap();
    for at in [44, 47, 64, 67, 84, 99] {
        let mut b = bytes.clone();
        b[at] ^= 0x5a;
        assert_eq!(profile_id(&b).unwrap(), id, "byte {at} is excluded");
    }
    for at in [8, 100, 128, bytes.len() - 1] {
        let mut b = bytes.clone();
        b[at] ^= 0x5a;
        assert_ne!(profile_id(&b).unwrap(), id, "byte {at} is included");
    }
}

#[test]
fn bytes_after_the_declared_size_are_not_hashed() {
    let bytes = Builtin::Srgb.profile().to_bytes().to_vec();
    let mut padded = bytes.clone();
    padded.extend([0xAB; 7]);
    assert_eq!(profile_id(&padded).unwrap(), profile_id(&bytes).unwrap());
}

#[test]
fn a_short_profile_has_no_id() {
    assert_eq!(profile_id(&[0; 127]), Err(CmsError::Truncated));
    assert!(profile_id(&[0; 128]).is_ok());
}

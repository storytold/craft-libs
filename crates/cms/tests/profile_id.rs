//! The header's profile ID (ICC.1:2022 §7.2.18).

use craft_cms::{Builtin, Profile, builtin, profile_id};

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

use super::*;

fn draft(preset: &str) -> ConnectionDraft {
    let p = PRESETS.iter().find(|p| p.key == preset).unwrap();
    ConnectionDraft::from_preset(p)
}

fn with_key(mut d: ConnectionDraft, key: &str) -> ConnectionDraft {
    d.key = key.to_owned();
    d
}

#[test]
fn a_taken_key_locator_is_never_reused() {
    let mut other = with_key(draft("openai"), "k").prepare(&[]).unwrap().connection;
    // An older file gave some other connection the locator this one would pick.
    other.id = crate::ids::ConnectionId::new("zzz").unwrap();
    other.auth = Auth::ApiKey { secret: crate::ids::SecretRef::new("openai.key").unwrap() };
    let p = with_key(draft("openai"), "k2").prepare(&[other]).unwrap();
    assert_eq!(p.connection.auth.secret_ref().unwrap().as_str(), "openai.key2");
}

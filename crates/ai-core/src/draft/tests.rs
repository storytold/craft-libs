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
fn every_preset_with_an_address_makes_a_valid_connection() {
    for p in PRESETS.iter().filter(|p| !p.endpoint.is_empty()) {
        let d = with_key(ConnectionDraft::from_preset(p), "sk-test-123");
        let prepared = d.prepare(&[]).unwrap_or_else(|e| panic!("{}: {e}", p.key));
        assert!(prepared.connection.validate().is_ok());
        assert_eq!(prepared.connection.auth.secret_ref().is_some(), p.needs_key, "{}", p.key);
    }
    // The custom preset needs an address typed in.
    let custom = with_key(draft("custom"), "k");
    assert_eq!(custom.prepare(&[]).unwrap_err().field, Field::Address);
}

#[test]
fn artcraft_api_key_preset_is_distinct_from_unverified_account_access() {
    let p = PRESETS.iter().find(|p| p.key == "artcraft-omni").unwrap();
    assert_eq!(p.provider, "artcraft-omni");
    assert!(p.needs_key);
    assert_eq!(p.endpoint, "https://api.storyteller.ai");
    assert!(!PRESETS.iter().any(|p| p.key == "artcraft-account"));
    let prepared = with_key(ConnectionDraft::from_preset(p), "test-key").prepare(&[]).unwrap();
    assert_eq!(prepared.connection.models.len(), 2);
    assert!(prepared.connection.models.iter().any(|m| m.id.as_str() == "nano_banana"));
    assert!(prepared.connection.models.iter().any(|m| m.id.as_str() == "seedance_2p0"));
}

#[test]
fn where_it_goes_decides_local_or_remote() {
    assert_eq!(draft("ollama").locality(), Some(Locality::Local));
    assert_eq!(draft("openai").locality(), Some(Locality::Remote));
    let mut d = draft("custom");
    assert_eq!(d.locality(), None, "no address yet");
    d.endpoint = "https://localhost.evil.example".into();
    assert_eq!(d.locality(), Some(Locality::Remote), "a lookalike host is remote");
    d.endpoint = "not a url".into();
    assert_eq!(d.locality(), None);
    // The connection carries what the address says, whatever the preset hoped.
    let mut o = with_key(draft("ollama"), "");
    o.endpoint = "https://api.example.com/v1".into();
    let c = o.prepare(&[]).unwrap().connection;
    assert_eq!(c.locality, Locality::Remote);
}

#[test]
fn plain_http_is_refused_unless_it_is_this_computer() {
    let mut d = with_key(draft("custom"), "k");
    d.endpoint = "http://192.168.1.20:8000/v1".into();
    let e = d.prepare(&[]).unwrap_err();
    assert_eq!(e.field, Field::Address);
    assert!(e.message.contains("https://"), "{e}");
    d.endpoint = "http://user:pw@127.0.0.1/v1".into();
    assert!(d.prepare(&[]).is_err());
}

#[test]
fn keys_must_be_one_clean_line() {
    for bad in ["sk-abc\ndef", "sk abc", "sk-\u{e9}", "sk-abc\u{0}", &"x".repeat(MAX_KEY_BYTES + 1)] {
        let d = with_key(draft("openai"), bad);
        let e = d.prepare(&[]).unwrap_err();
        assert_eq!(e.field, Field::Key, "{bad:?}");
        assert!(!e.message.contains("abc"), "the message must not echo the key");
    }
    // Surrounding whitespace from a paste is trimmed, not refused.
    let d = with_key(draft("openai"), "  sk-good-key\r\n");
    match d.prepare(&[]).unwrap().key {
        KeyChange::Set(k) => assert_eq!(k.expose(), "sk-good-key"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_new_connection_that_needs_a_key_must_have_one() {
    let e = draft("openai").prepare(&[]).unwrap_err();
    assert_eq!(e.field, Field::Key);
    // Turning the key off makes it a no-key connection.
    let mut d = draft("openai");
    d.uses_key = false;
    let p = d.prepare(&[]).unwrap();
    assert_eq!(p.connection.auth, Auth::None);
    assert!(matches!(p.key, KeyChange::Keep), "nothing saved, nothing to clear");
}

#[test]
fn names_are_checked() {
    let mut d = with_key(draft("openai"), "k");
    d.label = "   ".into();
    assert_eq!(d.prepare(&[]).unwrap_err().field, Field::Label);
    d.label = "two\nlines".into();
    assert_eq!(d.prepare(&[]).unwrap_err().field, Field::Label);
    d.label = "x".repeat(MAX_LABEL_CHARS + 1);
    assert_eq!(d.prepare(&[]).unwrap_err().field, Field::Label);
    d.label = "OpenAI".into();
    d.provider = "bad provider!".into();
    assert_eq!(d.prepare(&[]).unwrap_err().field, Field::Provider);
}

#[test]
fn ids_come_from_the_name_and_stay_unique() {
    let first = with_key(draft("openai"), "k").prepare(&[]).unwrap();
    assert_eq!(first.connection.id.as_str(), "openai");
    assert_eq!(first.connection.auth.secret_ref().unwrap().as_str(), "openai.key");
    let second = with_key(draft("openai"), "k2").prepare(std::slice::from_ref(&first.connection)).unwrap();
    assert_eq!(second.connection.id.as_str(), "openai-2");
    // Two connections never share a key, even with the same name.
    assert_eq!(second.connection.auth.secret_ref().unwrap().as_str(), "openai-2.key");
    let mut odd = with_key(draft("openai"), "k");
    odd.label = "!!! Café Ünïcode ???".into();
    let c = odd.prepare(&[]).unwrap().connection;
    assert_eq!(c.id.as_str(), "caf-n-code");
    odd.label = "???".into();
    assert_eq!(odd.prepare(&[]).unwrap().connection.id.as_str(), "connection");
    assert_eq!(c.label, "!!! Café Ünïcode ???", "the label itself is kept as typed");
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

fn saved_openai() -> Connection {
    let mut c = with_key(draft("openai"), "sk-x").prepare(&[]).unwrap().connection;
    c.models = vec![ModelInfo::new(crate::ids::ModelId::new("gpt-x").unwrap(), "GPT X")];
    c
}

#[test]
fn editing_keeps_the_saved_key_unless_a_new_one_is_typed() {
    let saved = saved_openai();
    let mut d = ConnectionDraft::from_connection(&saved);
    assert!(d.key.is_empty());
    d.label = "Work OpenAI".into();
    let p = d.prepare(std::slice::from_ref(&saved)).unwrap();
    assert!(matches!(p.key, KeyChange::Keep));
    assert_eq!(p.connection.id, saved.id, "the id never changes on rename");
    assert_eq!(p.connection.auth, saved.auth, "same secret locator");
    d.key = "sk-new".into();
    assert!(matches!(d.prepare(std::slice::from_ref(&saved)).unwrap().key, KeyChange::Set(_)));
}

#[test]
fn turning_the_key_off_clears_the_saved_one() {
    let saved = saved_openai();
    let mut d = ConnectionDraft::from_connection(&saved);
    d.uses_key = false;
    let p = d.prepare(std::slice::from_ref(&saved)).unwrap();
    assert!(matches!(p.key, KeyChange::Clear));
    assert_eq!(p.connection.auth, Auth::None);
}

#[test]
fn models_follow_the_address() {
    let saved = saved_openai();
    let same = ConnectionDraft::from_connection(&saved);
    assert_eq!(same.prepare(std::slice::from_ref(&saved)).unwrap().connection.models.len(), 1);
    let mut moved = ConnectionDraft::from_connection(&saved);
    moved.endpoint = "https://other.example.com/v1".into();
    assert!(moved.prepare(std::slice::from_ref(&saved)).unwrap().connection.models.is_empty());
}

#[test]
fn models_found_at_the_new_address_are_kept() {
    let saved = saved_openai();
    let found = vec![ModelInfo::new(crate::ids::ModelId::new("llama-x").unwrap(), "Llama X")];
    let mut moved = ConnectionDraft::from_connection(&saved);
    moved.endpoint = "https://other.example.com/v1".into();
    assert!(moved.set_models("https://other.example.com/v1", found.clone()));
    assert_eq!(moved.prepare(std::slice::from_ref(&saved)).unwrap().connection.models, found);
}

#[test]
fn a_retest_that_finds_the_same_models_keeps_them() {
    let saved = saved_openai();
    let mut moved = ConnectionDraft::from_connection(&saved);
    moved.endpoint = "https://other.example.com/v1".into();
    assert!(moved.set_models("https://other.example.com/v1", saved.models.clone()));
    assert_eq!(moved.prepare(std::slice::from_ref(&saved)).unwrap().connection.models, saved.models);
}

#[test]
fn models_found_before_the_address_changed_are_dropped() {
    let saved = saved_openai();
    let found = vec![ModelInfo::new(crate::ids::ModelId::new("llama-x").unwrap(), "Llama X")];
    let mut d = ConnectionDraft::from_connection(&saved);
    let at = d.endpoint.clone();
    assert!(d.set_models(&at, found));
    d.endpoint = "https://other.example.com/v1".into();
    assert!(d.prepare(std::slice::from_ref(&saved)).unwrap().connection.models.is_empty());
}

#[test]
fn editing_something_removed_elsewhere_is_an_error() {
    let saved = saved_openai();
    let d = ConnectionDraft::from_connection(&saved);
    assert!(d.prepare(&[]).is_err());
}

#[test]
fn nothing_secret_shows_in_debug_or_errors() {
    let d = with_key(draft("openai"), "sk-very-secret-value");
    assert!(!format!("{d:?}").contains("secret-value"));
    let mut bad = d.clone();
    bad.endpoint = "nope".into();
    let e = bad.prepare(&[]).unwrap_err();
    assert!(!format!("{e} {e:?}").contains("secret-value"));
    let p = d.prepare(&[]).unwrap();
    assert!(!format!("{p:?}").contains("secret-value"));
}

#[test]
fn hostile_text_never_panics() {
    let nasty = ["", "\0", "\u{202e}rtl", "💥💥💥", &"a".repeat(100_000), "http://[::1", "https://", "%00%00", "' OR 1=1 --"];
    for a in nasty {
        for b in nasty {
            let mut d = draft("custom");
            d.label = a.to_owned();
            d.endpoint = b.to_owned();
            d.provider = a.to_owned();
            d.key = b.to_owned();
            let _ = d.prepare(&[]);
            let _ = d.locality();
        }
    }
}

#[test]
fn seedance_connects_through_byteplus_modelark_with_an_api_key() {
    let p = PRESETS.iter().find(|p| p.key == "seedance").unwrap();
    assert_eq!(p.endpoint, "https://ark.ap-southeast.bytepluses.com/api/v3");
    assert!(p.name.contains("BytePlus"), "{}", p.name);
    assert!(!p.name.contains("Volcengine"), "{}", p.name);
    assert!(p.needs_key);
}

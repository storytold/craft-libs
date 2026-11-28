//! Integration tests of craft-keymap: profiles.

use craft_keymap::{Overrides, ProfileError, Profiles};

#[test]
fn profiles_copy_built_ins_on_edit_and_round_trip_as_json() {
    let mut classic = Overrides::new();
    classic.insert("edit.copy", vec!["Cmd+Shift+C".into()]);
    let builtins = [("Craft", Overrides::new()), ("Classic", classic.clone())];
    let mut p = Profiles::default();
    assert_eq!(p.active_name(&builtins), "Craft");

    let mut edited = Overrides::new();
    edited.insert("edit.copy", vec![]);
    assert_eq!(p.store(edited.clone(), &builtins, "Custom"), Ok(Some("Custom".into())));
    assert_eq!(p.active_name(&builtins), "Custom");
    assert_eq!(p.store(Overrides::new(), &builtins, "Custom"), Ok(None), "a user set is edited in place");
    assert!(p.active_overrides(&builtins).is_empty());

    p.select("Classic", &builtins).unwrap();
    assert_eq!(p.active_overrides(&builtins), &classic);
    assert_eq!(p.store(classic.clone(), &builtins, "Custom"), Ok(None), "no change, no copy");
    assert_eq!(p.store(edited.clone(), &builtins, "Custom"), Ok(Some("Custom 2".into())));
    assert_eq!(builtins[1].1, classic);

    let exported = p.export("Custom 2", &builtins).unwrap();
    assert_eq!(p.import(&exported, &builtins), Ok("Custom 3".into()));
    assert_eq!(p.active_overrides(&builtins), &edited);

    assert_eq!(p.delete("Classic", &builtins), Err(ProfileError::BuiltIn));
    assert_eq!(p.rename("Custom 3", "classic", &builtins), Err(ProfileError::NameTaken));
    assert_eq!(p.rename("Custom 3", "Mine", &builtins), Ok("Mine".into()));
    assert_eq!(p.active(), "Mine");
    assert_eq!(p.save_as(&"x".repeat(65), Overrides::new(), &builtins), Err(ProfileError::BadName));
    p.delete("Mine", &builtins).unwrap();
    assert_eq!(p.active_name(&builtins), "Craft");

    let wrong = exported.replace("craft-keymap", "other");
    assert_eq!(p.import(&wrong, &builtins), Err(ProfileError::WrongFormat));
    let newer = exported.replace("\"version\": 1", "\"version\": 2");
    assert_eq!(p.import(&newer, &builtins), Err(ProfileError::NewerVersion(2)));

    let text = serde_json::to_string(&p).unwrap();
    assert_eq!(serde_json::from_str::<Profiles>(&text).unwrap(), p);
    let junk: Profiles = serde_json::from_str(r#"{"active": 5, "sets": {"": {}, "ok": {"a": ["A"]}, "bad": 3}}"#).unwrap();
    assert_eq!(junk.sets().keys().collect::<Vec<_>>(), ["ok"]);
    let junk: Profiles =
        serde_json::from_str(r#"{"sets": [{"name": "ok", "overrides": [{"command": "a", "keys": ["A"]}, 4]}, {"name": ""}, 7]}"#).unwrap();
    assert_eq!(junk.sets()["ok"].get("a"), Some(&["A".to_string()][..]));
    assert!(text.contains(r#""overrides":[{"command":"edit.copy","keys":[]}]"#), "{text}");
}

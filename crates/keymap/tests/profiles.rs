use craft_keymap::{
    Bindable, Binding, CommandRef, ContextRule, Flavor, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, Keymap, MAX_SETS, Overrides, Platform,
    Policy, ProfileError, Profiles, Scope, Scopes, SetProblem, Start, plan,
};
use serde_json::json;

fn b(text: &str) -> Binding {
    Binding::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn keymap() -> Keymap {
    let catalog = vec![
        Bindable::new("file.save", "Save", Scope::app()).with_path(vec!["File".into(), "Save".into()]).with_default(b("Cmd+S")),
        Bindable::new("edit.copy", "Copy", Scope::app()).with_path(vec!["Edit".into(), "Copy".into()]).with_default(b("Cmd+C")),
        Bindable::new(CommandRef::with_params("window.panel", json!({"panel": "layers"})), "Layers", Scope::app()).with_default(b("F7")),
        Bindable::new("timeline.loop", "Loop", Scope::new("Timeline").unwrap()).with_default(b("L")),
    ];
    Keymap::new(catalog, Scopes::new().with_panel("Timeline"), Platform::Mac)
}

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

#[test]
fn rename_keeps_a_set_whose_name_differs_only_in_case() {
    let builtins = [("Craft", Overrides::new())];
    let mut p: Profiles = serde_json::from_str(r#"{"sets": {"Mine": {"a": "A"}, "mine": {"b": "B"}}}"#).unwrap();
    assert_eq!(p.rename("Mine", "mine", &builtins), Err(ProfileError::NameTaken));
    assert_eq!(p.sets()["mine"].get("b"), Some(&["B".to_string()][..]));
    assert_eq!(p.rename("Mine", "MINE", &builtins), Err(ProfileError::NameTaken));
    p.delete("mine", &builtins).unwrap();
    assert_eq!(p.rename("Mine", "MINE", &builtins), Ok("MINE".into()));
}

#[test]
fn sets_that_are_not_a_list_or_map_load_as_none() {
    for text in [r#"{"active": "Mine", "sets": 5}"#, r#"{"active": "Mine", "sets": null}"#, r#"{"active": "Mine", "sets": "x"}"#] {
        let p: Profiles = serde_json::from_str(text).unwrap_or_else(|e| panic!("{text}: {e}"));
        assert!(p.sets().is_empty(), "{text}");
        assert_eq!(p.active(), "Mine", "{text}");
    }
}

#[test]
fn a_user_set_named_like_a_new_built_in_keeps_its_edits() {
    let mut mine = Overrides::new();
    mine.insert("edit.copy", vec!["Cmd+K".into()]);
    let mut p = Profiles::default();
    p.save_as("Classic", mine.clone(), &[("Craft", Overrides::new())]).unwrap();

    // An update adds a built-in of that name.
    let mut classic = Overrides::new();
    classic.insert("edit.copy", vec!["Cmd+Shift+C".into()]);
    let builtins = [("Craft", Overrides::new()), ("Classic", classic.clone())];
    assert_eq!(p.active_overrides(&builtins), &mine, "the user set is what the name reads");

    let mut edited = mine.clone();
    edited.insert("edit.paste", vec![]);
    assert_eq!(p.store(edited.clone(), &builtins, "Custom"), Ok(Some("Classic 2".into())));
    assert_eq!(p.active_name(&builtins), "Classic 2");
    assert_eq!(p.active_overrides(&builtins), &edited);
    assert_eq!(p.get("Classic", &builtins), Some(&classic));
    assert_eq!(builtins[1].1, classic);

    let mut q: Profiles =
        serde_json::from_value(json!({"active": "CRAFT", "sets": [{"name": "CRAFT", "overrides": {"edit.copy": "Cmd+K"}}]})).unwrap();
    assert!(q.reconcile(&builtins));
    assert_eq!(q.active_name(&builtins), "CRAFT 2");
    assert_eq!(q.active_overrides(&builtins), &mine);
    assert!(!q.reconcile(&builtins));
}

#[test]
fn an_exported_set_imports_as_a_new_set_with_the_same_keymap() {
    let mut km = keymap();
    km.assign(&"file.save".into(), b("Cmd+Alt+S"), Policy::Steal).unwrap();
    km.assign(&CommandRef::with_params("window.panel", json!({"panel": "layers"})), b("Timeline: Cmd+L"), Policy::Steal).unwrap();
    km.clear(&"edit.copy".into());
    let builtins = [("Craft", Overrides::new())];
    let mut p = Profiles::default();
    p.save_as("Mine", km.overrides().clone(), &builtins).unwrap();

    let text = p.export("Mine", &builtins).unwrap();
    assert_eq!(p.import(&text, &builtins), Ok("Mine 2".into()));
    assert_eq!(p.active(), "Mine 2");

    let mut again = keymap();
    again.set_overrides(p.active_overrides(&builtins).clone());
    for bindable in km.catalog() {
        assert_eq!(again.bindings(&bindable.command), km.bindings(&bindable.command), "{}", bindable.command.key());
    }
    assert_eq!(again.overrides(), km.overrides());
}

#[test]
fn duplicate_copies_built_in_and_user_sets() {
    let mut classic = Overrides::new();
    classic.insert("edit.copy", vec!["Cmd+Shift+C".into()]);
    let builtins = [("Craft", Overrides::new()), ("Classic", classic.clone())];
    let mut p = Profiles::default();

    assert_eq!(p.duplicate("Classic", "Classic", &builtins), Ok("Classic 2".into()));
    assert_eq!(p.active(), "Classic 2");
    assert_eq!(p.active_overrides(&builtins), &classic);
    assert_eq!(p.duplicate("Classic 2", " Mine ", &builtins), Ok("Mine".into()));
    assert_eq!(p.duplicate("Mine", "mine", &builtins), Ok("mine 2".into()), "names compare without case");
    assert_eq!(p.duplicate("Nope", "X", &builtins), Err(ProfileError::NotFound));
    assert_eq!(p.duplicate("Craft", "", &builtins), Err(ProfileError::BadName));

    while p.sets().len() < MAX_SETS {
        p.duplicate("Craft", "Spare", &builtins).unwrap();
    }
    assert_eq!(p.duplicate("Craft", "Spare", &builtins), Err(ProfileError::TooMany));
}

#[test]
fn a_foreign_set_imports_as_a_new_set_and_leaves_the_keymap_alone() {
    let km = keymap();
    let read = |text: &str| ForeignKey::read(Some("Default"), text, Flavor::Mac);
    let set = ForeignSet::new(vec![ForeignEntry::new(Some("File"), ["Save"]).with_key(read("Cmd+Shift+S"))]).with_name("Other App");
    let options = ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], start: Start::Defaults, ..ImportOptions::default() };
    let planned = plan(&set, &km, &options);
    let builtins = [("Craft", Overrides::new())];
    let mut p = Profiles::default();

    let (name, report) = p.import_plan(&planned, &km, None, &builtins).unwrap();
    assert_eq!(name, "Other App");
    assert_eq!(report.changed.len(), 1);
    assert!(km.overrides().is_empty());
    assert_eq!(p.active(), "Other App");
    let mut imported = keymap();
    imported.set_overrides(p.active_overrides(&builtins).clone());
    assert_eq!(imported.bindings(&"file.save".into()), [b("Cmd+Shift+S")]);

    let (name, _) = p.import_plan(&planned, &km, Some("Craft"), &builtins).unwrap();
    assert_eq!(name, "Craft 2", "a built-in's name is not reused");
}

#[test]
fn loading_lists_the_sets_it_drops_and_the_store_has_a_version() {
    let text = r#"{"version": 2, "active": "ok", "sets": [
        {"name": "ok", "overrides": {}}, {"name": " pad ", "overrides": {}}, {"name": "ok", "overrides": {"a": "A"}}, {"name": "bad", "overrides": 3}
    ]}"#;
    let p: Profiles = serde_json::from_str(text).unwrap();
    assert_eq!(p.sets().keys().collect::<Vec<_>>(), ["ok"]);
    assert!(p.sets()["ok"].is_empty(), "the first of two sets with one name stays");
    let name = |n: &str| n.to_string();
    assert_eq!(
        p.problems(),
        [
            SetProblem::NewerVersion(2),
            SetProblem::BadName { name: name(" pad ") },
            SetProblem::Duplicate { name: name("ok") },
            SetProblem::BadOverrides { name: name("bad") }
        ]
    );

    let many: Vec<_> = (0..MAX_SETS + 3).map(|i| json!({"name": format!("s{i}"), "overrides": {}})).collect();
    let p: Profiles = serde_json::from_value(json!({ "sets": many })).unwrap();
    assert_eq!((p.sets().len(), p.problems()), (MAX_SETS, &[SetProblem::TooMany { ignored: 3 }][..]));

    assert_eq!(serde_json::to_value(&p).unwrap()["version"], 1);
    let unversioned: Profiles = serde_json::from_str(r#"{"active": "x", "sets": []}"#).unwrap();
    assert!(unversioned.problems().is_empty());
}

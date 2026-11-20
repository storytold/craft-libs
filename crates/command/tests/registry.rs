//! Registry overlay: duplicate and reserved ids, owner removal, cached enablement, enumeration, schema.

use std::sync::Arc;

use serde_json::{Value, json};

use craft_command::schema::from_doc;
use craft_command::{CommandEntry, CommandSpec, DynCommand, Enablement, Menu, Owner, RegisterError, Registry, Visibility};

fn enabled_ok(_session: &()) -> Result<(), String> {
    Ok(())
}

fn run_ok(_session: &mut (), _params: &Value) -> Result<Value, String> {
    Ok(Value::Null)
}

const SAVE_MENU: &[&str] = &["File"];
const OPEN_MENU: &[&str] = &["File", "Open"];

static SPECS: [CommandSpec<(), String>; 2] = [
    CommandSpec {
        id: "file.save",
        label: "Save",
        menu: SAVE_MENU,
        shortcut: Some("ctrl+s"),
        alt_shortcuts: &["ctrl+shift+s"],
        params: "save the file",
        enabled: enabled_ok,
        run: run_ok,
        journal: true,
    },
    CommandSpec {
        id: "file.open",
        label: "Open",
        menu: OPEN_MENU,
        shortcut: None,
        alt_shortcuts: &[],
        params: "open a file",
        enabled: enabled_ok,
        run: run_ok,
        journal: false,
    },
];

fn registry() -> Registry<(), String> {
    Registry::new(&SPECS)
}

fn owner(plugin_id: &str, generation: u64) -> Owner {
    Owner { plugin_id: Arc::from(plugin_id), generation }
}

fn dyn_cmd(id: &str, plugin_id: &str, generation: u64, enablement: Enablement) -> DynCommand<(), String> {
    DynCommand {
        id: Arc::from(id),
        label: Arc::from(id),
        params: Arc::from("blur the clip"),
        shortcut: Some(Arc::from("ctrl+b")),
        menu: Arc::from([Arc::<str>::from("Plugin"), Arc::from("Blur")]),
        owner: owner(plugin_id, generation),
        enablement,
        visibility: Visibility { menu: true, palette: true, shortcut: false },
        journal: true,
        run: Arc::new(|_session, _params| Ok(Value::Null)),
    }
}

fn enablement(reg: &Registry<(), String>, id: &str) -> Enablement {
    match reg.resolve(id) {
        Some(CommandEntry::Dynamic(cmd)) => cmd.enablement.clone(),
        other => panic!("expected dynamic command {id}, got {other:?}"),
    }
}

#[test]
fn duplicate_dynamic_id_is_rejected() {
    let reg = registry();
    let first = dyn_cmd("plugin.alpha.one", "alpha", 1, Enablement::Always);
    assert_eq!(reg.register(first), Ok(()));
    let second = dyn_cmd("plugin.alpha.one", "alpha", 1, Enablement::Always);
    assert_eq!(reg.register(second), Err(RegisterError::Duplicate));
    assert!(reg.resolve("plugin.alpha.one").is_some());
    assert_eq!(reg.unregister_owner(&owner("alpha", 1)), 1);
}

#[test]
fn reserved_id_is_rejected() {
    let reg = registry();
    let bare = dyn_cmd("alpha.one", "alpha", 1, Enablement::Always);
    assert_eq!(reg.register(bare), Err(RegisterError::Reserved));
    let other_plugin = dyn_cmd("plugin.beta.one", "alpha", 1, Enablement::Always);
    assert_eq!(reg.register(other_plugin), Err(RegisterError::Reserved));
    let missing_dot = dyn_cmd("plugin.alpha", "alpha", 1, Enablement::Always);
    assert_eq!(reg.register(missing_dot), Err(RegisterError::Reserved));
    assert!(reg.resolve("alpha.one").is_none());
    assert!(reg.resolve("plugin.beta.one").is_none());
    assert!(reg.schema("plugin.beta.one").is_none());
    assert_eq!(reg.unregister_owner(&owner("alpha", 1)), 0);
}

#[test]
fn static_id_cannot_be_reregistered() {
    let reg = registry();
    let shadow = dyn_cmd("file.save", "alpha", 1, Enablement::Always);
    assert_eq!(reg.register(shadow), Err(RegisterError::Duplicate));
    assert!(matches!(reg.resolve("file.save"), Some(CommandEntry::Static(_))));
}

#[test]
fn unregister_owner_removes_only_that_owner() {
    let reg = registry();
    assert_eq!(reg.register(dyn_cmd("plugin.alpha.one", "alpha", 1, Enablement::Always)), Ok(()));
    assert_eq!(reg.register(dyn_cmd("plugin.alpha.two", "alpha", 1, Enablement::Always)), Ok(()));
    assert_eq!(reg.register(dyn_cmd("plugin.alpha.three", "alpha", 2, Enablement::Always)), Ok(()));
    assert_eq!(reg.register(dyn_cmd("plugin.beta.one", "beta", 1, Enablement::Always)), Ok(()));

    let held = reg.resolve("plugin.alpha.one");
    assert_eq!(reg.unregister_owner(&owner("alpha", 1)), 2);

    assert!(reg.resolve("plugin.alpha.one").is_none());
    assert!(reg.resolve("plugin.alpha.two").is_none());
    assert!(matches!(reg.resolve("plugin.alpha.three"), Some(CommandEntry::Dynamic(_))));
    assert!(matches!(reg.resolve("plugin.beta.one"), Some(CommandEntry::Dynamic(_))));
    match held {
        Some(CommandEntry::Dynamic(cmd)) => assert_eq!(&*cmd.id, "plugin.alpha.one"),
        other => panic!("running resolve must keep its command, got {other:?}"),
    }

    let mut ids = Vec::new();
    reg.for_each(|view| ids.push(view.id.to_string()));
    assert_eq!(ids, vec!["file.save", "file.open", "plugin.alpha.three", "plugin.beta.one"]);
}

#[test]
fn set_enabled_changes_cached_only() {
    let reg = registry();
    assert_eq!(reg.register(dyn_cmd("plugin.cache.cached", "cache", 1, Enablement::Cached(Ok(())))), Ok(()));
    assert_eq!(reg.register(dyn_cmd("plugin.cache.always", "cache", 1, Enablement::Always)), Ok(()));
    assert_eq!(reg.register(dyn_cmd("plugin.cache.doc", "cache", 1, Enablement::HasDocument)), Ok(()));
    assert_eq!(reg.register(dyn_cmd("plugin.cache.sel", "cache", 1, Enablement::HasSelection)), Ok(()));

    let off = Err("off".to_string());
    reg.set_enabled("plugin.cache.cached", off.clone());
    reg.set_enabled("plugin.cache.always", off.clone());
    reg.set_enabled("plugin.cache.doc", off.clone());
    reg.set_enabled("plugin.cache.sel", off.clone());
    reg.set_enabled("file.save", off.clone());
    reg.set_enabled("missing.command", off);

    assert_eq!(enablement(&reg, "plugin.cache.cached"), Enablement::Cached(Err("off".to_string())));
    assert_eq!(enablement(&reg, "plugin.cache.always"), Enablement::Always);
    assert_eq!(enablement(&reg, "plugin.cache.doc"), Enablement::HasDocument);
    assert_eq!(enablement(&reg, "plugin.cache.sel"), Enablement::HasSelection);
    assert!(matches!(reg.resolve("file.save"), Some(CommandEntry::Static(_))));
}

#[test]
fn the_view_carries_alt_shortcuts_for_static_rows_and_none_for_dynamic_ones() {
    let reg = Registry::new(&SPECS);
    reg.register(dyn_cmd("plugin.alpha.one", "alpha", 1, Enablement::Always)).unwrap();
    let mut alts = Vec::new();
    reg.for_each(|view| alts.push((view.id.to_string(), view.alt_shortcuts.to_vec())));
    assert!(alts.contains(&("file.save".to_owned(), vec!["ctrl+shift+s"])));
    assert!(alts.iter().filter(|(id, _)| id != "file.save").all(|(_, a)| a.is_empty()));
}

#[test]
fn for_each_sees_static_and_dynamic_ids() {
    let reg = registry();
    assert_eq!(reg.register(dyn_cmd("plugin.alpha.one", "alpha", 1, Enablement::Always)), Ok(()));

    let mut seen = Vec::new();
    reg.for_each(|view| {
        let menu = match view.menu {
            Menu::Static(items) => items.iter().map(|item| (*item).to_string()).collect::<Vec<_>>(),
            Menu::Shared(items) => items.iter().map(|item| item.to_string()).collect(),
        };
        seen.push((
            view.id.to_string(),
            view.label.to_string(),
            view.dynamic,
            view.journal,
            view.shortcut.map(str::to_string),
            view.params.to_string(),
            menu,
        ));
    });

    assert_eq!(
        seen,
        vec![
            (
                "file.save".to_string(),
                "Save".to_string(),
                false,
                true,
                Some("ctrl+s".to_string()),
                "save the file".to_string(),
                vec!["File".to_string()],
            ),
            (
                "file.open".to_string(),
                "Open".to_string(),
                false,
                false,
                None,
                "open a file".to_string(),
                vec!["File".to_string(), "Open".to_string()],
            ),
            (
                "plugin.alpha.one".to_string(),
                "plugin.alpha.one".to_string(),
                true,
                true,
                Some("ctrl+b".to_string()),
                "blur the clip".to_string(),
                vec!["Plugin".to_string(), "Blur".to_string()],
            ),
        ]
    );
}

#[test]
fn schema_is_some_for_known_id_and_none_for_unknown() {
    let reg = registry();
    assert_eq!(reg.register(dyn_cmd("plugin.alpha.one", "alpha", 1, Enablement::Always)), Ok(()));

    let save = reg.schema("file.save").expect("built-in id has a schema");
    assert_eq!(save.as_ref(), &from_doc("save the file").value);
    assert_eq!(save["x-craft-source"], json!("opaque"));
    assert_eq!(save["description"], json!("save the file"));

    let dynamic = reg.schema("plugin.alpha.one").expect("dynamic id has a schema");
    assert_eq!(dynamic.as_ref(), &from_doc("blur the clip").value);

    assert!(reg.schema("missing.command").is_none());
}

//! Photoshop .kys sets.

use super::kys::{self, KysError, MAX_BYTES};
use crate::keymap::{Binding, CommandDef, CommandKind, Keymap};
use crate::{CommandRef, ImportOptions, Platform, Scope, Scopes, Trigger, plan};

fn binding(text: &str) -> Binding {
    Binding::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn trigger(text: &str) -> Trigger {
    binding(text).trigger
}

fn keys(entry: &crate::ForeignEntry) -> Vec<Trigger> {
    entry.keys.iter().map(|k| k.trigger.clone().unwrap_or_else(|e| panic!("{}: {e}", k.text))).collect()
}

fn set(rows: &str, filename: &str) -> String {
    format!("<photoshop-keyboard-shortcuts version=\"4\" filename=\"{filename}\">{rows}</photoshop-keyboard-shortcuts>")
}

fn command(id: &str, label: &str, defaults: &[&str]) -> CommandDef {
    defaults.iter().fold(CommandDef::new(id, label, Scope::app()), |c, d| c.with_default(binding(d)))
}

fn tool(id: &str, label: &str, defaults: &[&str]) -> CommandDef {
    command(id, label, defaults).with_kind(CommandKind::Tool { group: None })
}

fn keymap(catalog: Vec<CommandDef>) -> Keymap {
    Keymap::new(catalog, Scopes::new(), Platform::Mac)
}

/// Every entry's id with its keys, in file order.
fn by_id(set: &crate::ForeignSet) -> Vec<(String, Vec<Trigger>)> {
    set.entries.iter().map(|e| (e.id.clone().unwrap_or_default(), keys(e))).collect()
}

#[test]
// A Mac set's Control+Opt+Cmd+G and a Windows set's Ctrl+Alt+G read as the same chord with Cmd; the set name comes from the filename attribute.
fn reads_a_mac_and_a_windows_set() {
    let mac = kys::read(&set(
        "<command kind=\"dynamic\" name=\"Gaussian Blur...\"><shortcut>Opt+Cmd+G</shortcut></command>\
         <command name=\"Rulers\"><shortcut>Control+K</shortcut></command>\
         <command name=\"No Key\"/>\
         <command><shortcut>Cmd+Q</shortcut></command>\
         <tool name=\"Move Tool\">V</tool>",
        "$$$/FileName/Presets/KeyboardCustomization/Retouch 1/2=Retouch 1/2",
    ))
    .unwrap();
    assert_eq!(mac.name.as_deref(), Some("Retouch 1/2"));
    let names: Vec<&str> = mac.entries.iter().map(|e| e.path[0].as_str()).collect();
    assert_eq!(
        names,
        ["Gaussian Blur...", "Rulers", "No Key", "Move Tool"],
        "a row without a name is skipped, one without a key stays, tools come last"
    );
    assert_eq!(keys(&mac.entries[0]), [trigger("Cmd+Alt+G")]);
    assert_eq!(keys(&mac.entries[1]), [trigger("Ctrl+K")], "a Mac set's Control is the Control key");
    assert!(mac.entries[2].keys.is_empty());
    assert!(mac.entries[3].is_tool);

    let windows = kys::read(&set(
        "<command name=\"Gaussian Blur...\"><shortcut>Ctrl+Alt+G</shortcut></command><command name=\"Copy\"><shortcut>F3</shortcut></command>",
        "",
    ))
    .unwrap();
    assert_eq!(windows.name, None);
    assert_eq!(keys(&windows.entries[0]), [trigger("Cmd+Alt+G")]);

    assert_eq!(kys::read("<other/>").unwrap_err(), KysError::NotASet("no <photoshop-keyboard-shortcuts> root".into()));
    assert!(matches!(kys::read("<a>"), Err(KysError::NotASet(_))));
    assert_eq!(kys::read(&" ".repeat(MAX_BYTES + 1)).unwrap_err(), KysError::TooLarge);
    let deep = set(&format!("{}{}", "<a>".repeat(600), "</a>".repeat(600)), "");
    assert!(matches!(kys::read(&deep), Err(KysError::NotASet(m)) if m.contains("nested")), "a set nested past the depth limit is refused");
    let declared = format!("<!DOCTYPE photoshop-keyboard-shortcuts [<!ENTITY a \"b\">]>{}", set("", ""));
    assert!(matches!(kys::read(&declared), Err(KysError::NotASet(_))), "a document type declaration is refused");

    let lines = kys::read(
        "<photoshop-keyboard-shortcuts filename=\"Lines\">\n<command name=\"Copy\"><shortcut>Cmd+C</shortcut></command>\n\
         <tool name=\"Move Tool\">V</tool></photoshop-keyboard-shortcuts>",
    )
    .unwrap();
    let at: Vec<Option<usize>> = lines.entries.iter().map(|e| e.line).collect();
    assert_eq!(at, [Some(2), Some(3)], "each entry reports the line its row starts on");
}

#[test]
// The `+` key spelled alone, or after a modifier as `Cmd++`, is a key and not a separator, both when written and when read.
fn a_bare_plus_key_is_written_and_read() {
    let km = keymap(vec![command("view.zoomIn", "Zoom In", &["Cmd++"]), tool("tool.zoom", "Zoom Tool", &["+"])]);
    let xml = kys::write("Mine", &km);
    assert!(xml.contains("<shortcut>Cmd++</shortcut>"), "{xml}");
    assert!(xml.contains(">+</tool>"), "{xml}");
    let back = kys::read(&xml).unwrap();
    assert_eq!(by_id(&back), [("view.zoomIn".to_string(), vec![trigger("Cmd++")]), ("tool.zoom".to_string(), vec![trigger("+")])]);

    // A Windows set: its Ctrl row makes Ctrl the command key, and the `+` row stays the bare key.
    let windows =
        kys::read(&set("<command name=\"Copy\"><shortcut>Ctrl+C</shortcut></command><command name=\"Zoom In\"><shortcut>+</shortcut></command>", ""))
            .unwrap();
    assert_eq!(keys(&windows.entries[0]), [trigger("Cmd+C")]);
    assert_eq!(keys(&windows.entries[1]), [trigger("+")]);
}

#[test]
// A set the suite writes spells keys as Photoshop's Mac sets do and reads back into the same bindings, matched by id even where the labels differ.
fn a_written_set_reads_back_with_photoshops_key_spelling() {
    let km = keymap(vec![
        command("edit.copy", "Copy", &["Cmd+C", "F3"]),
        command("filter.blur", "Gaussian Blur…", &["Cmd+Ctrl+Alt+Shift+G"]),
        command("edit.unbound", "Unbound", &[]),
        tool("tool.move", "Move Tool", &["V"]),
    ]);
    let xml = kys::write(" My & Set ", &km);
    assert!(xml.contains("KeyboardCustomization/My &amp; Set=My &amp; Set"), "{xml}");
    assert!(
        xml.contains(
            "<command kind=\"static\" name=\"Gaussian Blur...\" craft-id=\"filter.blur\"><shortcut>Control+Opt+Shift+Cmd+G</shortcut></command>"
        ),
        "{xml}"
    );
    assert!(xml.contains("<shortcut>Cmd+C</shortcut><shortcut>F3</shortcut>"), "{xml}");
    assert!(!xml.contains("Unbound"), "a command without a key is left out: {xml}");

    let back = kys::read(&xml).unwrap();
    assert_eq!(back.name.as_deref(), Some("My & Set"));
    assert_eq!(
        by_id(&back),
        [
            ("edit.copy".to_string(), vec![trigger("Cmd+C"), trigger("F3")]),
            ("filter.blur".to_string(), vec![trigger("Cmd+Ctrl+Alt+Shift+G")]),
            ("tool.move".to_string(), vec![trigger("V")]),
        ]
    );

    // Into a keymap whose labels say something else and that has no keys: the ids decide.
    let mut fresh = keymap(vec![
        command("edit.copy", "Duplicate To Clipboard", &[]),
        command("filter.blur", "Soften", &[]),
        command("edit.unbound", "Unbound", &[]),
        tool("tool.move", "Arrow", &[]),
    ]);
    plan(&back, &fresh, &ImportOptions::default()).apply(&mut fresh);
    for id in ["edit.copy", "filter.blur", "edit.unbound", "tool.move"] {
        let c = CommandRef::new(id);
        assert_eq!(fresh.bindings(&c), km.bindings(&c), "{id}");
    }
}

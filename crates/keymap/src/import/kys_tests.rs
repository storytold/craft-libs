//! Photoshop .kys sets.

use super::kys::{self, KysError, MAX_BYTES};
use crate::Trigger;
use crate::keymap::Binding;

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
    // roxmltree recurses once per level: a deep set is refused before it can overflow the stack.
    let deep = set(&format!("{}{}", "<a>".repeat(20_000), "</a>".repeat(20_000)), "");
    let refused = std::thread::Builder::new().stack_size(512 * 1024).spawn(move || kys::read(&deep).err()).unwrap().join().unwrap();
    assert!(matches!(refused, Some(KysError::NotASet(m)) if m.contains("nested")));
}

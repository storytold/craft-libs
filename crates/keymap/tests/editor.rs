use craft_keymap::editor::{MAX_UNDO_BYTES, search};
use craft_keymap::{
    Bindable, Binding, Chord, CommandRef, DOUBLE_PRESS, EditSession, Keymap, MAX_BINDING_TEXT, MAX_BINDINGS, MAX_COMMANDS, MAX_UNDO, Overrides,
    Platform, Policy, Recorded, Recorder, Scope, Scopes, Trigger,
};

fn chord(text: &str) -> Chord {
    Chord::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn b(text: &str) -> Binding {
    Binding::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn keymap(platform: Platform) -> Keymap {
    let catalogue = vec![
        Bindable::new("file.save", "Save", Scope::app()).with_default(b("Cmd+S")).with_path(vec!["File".into(), "Save".into()]),
        Bindable::new("edit.copy", "Copy", Scope::app()).with_default(b("Cmd+C")).with_alias("Kopieren"),
        Bindable::new("view.zoomIn", "Zoom In", Scope::app()).with_default(b("Cmd+=")),
        Bindable::new("tool.brush", "Brush", Scope::app()).with_default(b("B")).with_path(vec!["Tools".into(), "Paint".into()]),
    ];
    Keymap::new(catalogue, Scopes::new(), platform)
}

fn keys(km: &Keymap, id: &str) -> Vec<String> {
    km.bindings(&CommandRef::new(id)).iter().map(|b| b.to_string()).collect()
}

#[test]
fn edits_undo_redo_and_cancel() {
    let mut km = keymap(Platform::Windows);
    let mut session = EditSession::begin(&km);
    let save = CommandRef::new("file.save");

    let assigned = session.edit(&mut km, |km| km.assign(&save, b("Cmd+Shift+S"), Policy::Steal));
    assert!(assigned.is_ok());
    session.edit(&mut km, |km| km.clear(&CommandRef::new("edit.copy")));
    assert_eq!(keys(&km, "file.save"), ["Cmd+S", "Cmd+Shift+S"]);
    assert!(keys(&km, "edit.copy").is_empty());
    assert!(session.is_modified(&km));

    // An edit that changes nothing is no undo step.
    session.edit(&mut km, |km| km.reset(&CommandRef::new("tool.brush")));

    assert!(session.undo(&mut km));
    assert_eq!(keys(&km, "edit.copy"), ["Cmd+C"]);
    assert!(session.undo(&mut km));
    assert_eq!(keys(&km, "file.save"), ["Cmd+S"]);
    assert!(!session.undo(&mut km));
    assert!(!session.is_modified(&km));

    assert!(session.redo(&mut km));
    assert_eq!(keys(&km, "file.save"), ["Cmd+S", "Cmd+Shift+S"]);
    // A new edit drops what could be redone.
    session.edit(&mut km, |km| km.clear(&CommandRef::new("tool.brush")));
    assert!(!session.redo(&mut km));

    session.cancel(&mut km);
    assert!(!session.is_modified(&km));
    assert_eq!(keys(&km, "file.save"), ["Cmd+S"]);
    assert_eq!(keys(&km, "tool.brush"), ["B"]);
    assert!(!session.can_undo());
}

#[test]
fn undo_keeps_the_newest_steps() {
    let mut km = keymap(Platform::Windows);
    let mut session = EditSession::begin(&km);
    let brush = CommandRef::new("tool.brush");
    // Each edit changes the key: N, B, N, … (B is the default).
    let edits = MAX_UNDO + 11;
    for i in 0..edits {
        let key = if i % 2 == 0 { "N" } else { "B" };
        assert!(session.edit(&mut km, |km| km.set_bindings(&brush, vec![b(key)], Policy::Keep)).is_ok());
    }
    let mut steps = 0;
    while session.undo(&mut km) {
        steps += 1;
    }
    assert_eq!(steps, MAX_UNDO);
    // The 11 oldest steps are gone: undo stops after the 11th edit, which set N.
    assert_eq!(keys(&km, "tool.brush"), ["N"]);
    assert!(session.is_modified(&km));
}

#[test]
fn undo_history_is_capped_by_size() {
    let mut km = keymap(Platform::Windows);
    let mut session = EditSession::begin(&km);
    // Full-size snapshots of entries for commands the catalogue does not have.
    let big = |tag: char| {
        let mut o = Overrides::new();
        for n in 0..MAX_COMMANDS {
            assert!(o.insert(format!("junk.{n}"), vec![tag.to_string().repeat(MAX_BINDING_TEXT); MAX_BINDINGS]));
        }
        o
    };
    let (a, b) = (big('a'), big('b'));
    let string = size_of::<String>();
    let per = a.iter().map(|(k, t)| string + 64 + k.len() + t.iter().map(|t| string + 16 + t.len()).sum::<usize>()).sum::<usize>();
    let edits = 2 * MAX_UNDO_BYTES / per + 4;
    for n in 0..edits {
        let next = if n % 2 == 0 { a.clone() } else { b.clone() };
        session.edit(&mut km, |km| km.set_overrides(next));
    }
    let mut steps = 0;
    while session.undo(&mut km) {
        steps += 1;
    }
    assert!(steps >= 2 && steps <= MAX_UNDO_BYTES / per, "{steps} steps of {per} bytes");
}

#[test]
fn recorder_turns_presses_into_triggers() {
    let mut r = Recorder::new();
    let a = chord("A");
    assert_eq!(r.press(a, 1.0), Recorded::Single(Trigger::single(a)));
    assert_eq!(r.press(a, 1.0 + DOUBLE_PRESS / 2.0), Recorded::Double(Trigger::double(a)));
    // The press after a double starts over.
    assert_eq!(r.press(a, 1.4), Recorded::Single(Trigger::single(a)));
    // Too late for a double.
    assert_eq!(r.press(a, 1.4 + DOUBLE_PRESS + 0.1), Recorded::Single(Trigger::single(a)));
    // Another chord in between.
    assert_eq!(r.press(chord("B"), 3.0), Recorded::Single(Trigger::single(chord("B"))));
    assert_eq!(r.press(a, 3.1), Recorded::Single(Trigger::single(a)));

    assert_eq!(r.press(chord("Escape"), 3.2), Recorded::Cancel);
    let shift_escape = chord("Shift+Escape");
    assert_eq!(r.press(shift_escape, 3.3), Recorded::Single(Trigger::single(shift_escape)));
}

#[test]
fn search_reads_labels_aliases_paths_ids_and_keys() {
    let ids = |km: &Keymap, q: &str, p: Platform| search(km, q, p).iter().map(|b| b.command.id.clone()).collect::<Vec<_>>();
    let win = keymap(Platform::Windows);
    let mac = keymap(Platform::Mac);
    assert_eq!(ids(&win, "zoom", Platform::Windows), ["view.zoomIn"]);
    assert_eq!(ids(&win, "KOPIEREN", Platform::Windows), ["edit.copy"]);
    assert_eq!(ids(&win, "paint", Platform::Windows), ["tool.brush"]);
    assert_eq!(ids(&win, "file.sa", Platform::Windows), ["file.save"]);
    assert_eq!(ids(&win, "ctrl+s", Platform::Windows), ["file.save"]);
    assert_eq!(ids(&win, "cmd+c", Platform::Windows), ["edit.copy"]);
    assert_eq!(ids(&mac, "⌘s", Platform::Mac), ["file.save"]);
    assert_eq!(ids(&mac, "⌘s", Platform::Windows), Vec::<String>::new());
    assert_eq!(ids(&mac, "  ", Platform::Mac).len(), 4);
}

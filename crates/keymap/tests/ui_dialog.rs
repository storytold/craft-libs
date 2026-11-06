#![cfg(feature = "ui")]
//! The Keyboard Shortcuts dialog driven through egui_kittest: recording, conflicts, search, areas,
//! platform display, accessibility labels, small windows, sets, import review and the summary.

use std::borrow::Cow;

use craft_keymap::ui::{ConflictMode, DialogState, English, FixedRow, Options, Outcome, ShortcutsDialog, Text};
use craft_keymap::{
    Active, Bindable, Binding, Chord, CommandRef, ConflictKind, ContextRule, Flavor, ForeignEntry, ForeignKey, ForeignSet, ImportOptions, Keymap,
    Kind, Lookup, Overrides, Platform, Profiles, Scope, Scopes, Trigger,
};
use egui::accesskit::Role;
use egui::{Event, Key as EguiKey, Modifiers, Vec2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use ui_fixture::{Pseudo, fixture, with_long_label};

#[path = "support/fonts.rs"]
mod test_fonts;
mod ui_fixture;

struct Fx {
    km: Keymap,
    options: Options,
    state: DialogState,
    text: Box<dyn Text>,
    window: bool,
    /// Profiles and built-in sets, when the dialog shows sets.
    sets: Option<(Profiles, Vec<(&'static str, Overrides)>)>,
    outcomes: Vec<Outcome>,
    /// A key press was still in the frame's input after the dialog ran.
    keys_left: bool,
}

/// The dialog of `fx` for a call outside the frame (import_json, review_import).
fn dialog(fx: &mut Fx) -> (ShortcutsDialog<'_>, &mut DialogState) {
    let Fx { km, options, state, text, sets, .. } = fx;
    let d = ShortcutsDialog::new(km, &**text).options(options);
    let d = match sets {
        Some((p, b)) => d.sets(p, b),
        None => d,
    };
    (d, state)
}

impl Fx {
    fn new(km: Keymap) -> Fx {
        Fx {
            km,
            options: Options::default(),
            state: DialogState::new(),
            text: Box::new(English),
            window: false,
            sets: None,
            outcomes: Vec::new(),
            keys_left: false,
        }
    }

    fn bindings(&self, id: &str) -> Vec<String> {
        self.km.bindings(&CommandRef::from(id)).iter().map(|b| b.to_string()).collect()
    }

    /// Whether any outcome recorded by the dialog harness is `Outcome::Cancel`.
    fn canceled(&self) -> bool {
        self.outcomes.iter().any(|o| matches!(o, Outcome::Cancel))
    }
}

/// Build an egui_kittest harness around `Fx`.
///
/// Installs and checks the pinned fonts every frame, selects window or inline dialog mode, records
/// emitted outcomes, and flags any key press still present after the dialog ran.
fn harness(fx: Fx, size: Vec2) -> Harness<'static, Fx> {
    let mut h = Harness::builder().with_size(size).with_context_setup(test_fonts::configure).build_ui_state(
        |ui, fx: &mut Fx| {
            test_fonts::assert_coverage(ui.ctx());
            let Fx { km, options, state, text, window, sets, outcomes, keys_left } = fx;
            let dialog = ShortcutsDialog::new(km, text.as_ref()).options(options);
            let dialog = match sets {
                Some((p, b)) => dialog.sets(p, b),
                None => dialog,
            };
            let out = if *window { dialog.show(ui.ctx(), state) } else { dialog.ui(ui, state) };
            outcomes.extend(out);
            *keys_left |= ui.input(|i| i.events.iter().any(|e| matches!(e, Event::Key { pressed: true, .. })));
        },
        fx,
    );
    h.run();
    h
}

fn open(fx: Fx) -> Harness<'static, Fx> {
    harness(fx, vec2(1000.0, 700.0))
}

fn mac() -> Harness<'static, Fx> {
    open(Fx::new(fixture(Platform::Mac)))
}

fn click(h: &mut Harness<'_, Fx>, label: &str) {
    h.get_by_label(label).click();
    h.run();
}

fn press(h: &mut Harness<'_, Fx>, modifiers: Modifiers, key: EguiKey) {
    h.key_press_modifiers(modifiers, key);
    h.run();
}

fn raw_key(h: &mut Harness<'_, Fx>, key: EguiKey, time: f64) {
    h.input_mut().time = Some(time);
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    h.input_mut().time = Some(time + 0.01);
    h.event(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.step();
}

fn has(h: &Harness<'_, Fx>, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

/// The table itself has focus (a screen reader sees its selected row, the active descendant).
fn table_focused(h: &Harness<'_, Fx>) -> bool {
    h.get_by_label("Commands").accesskit_node().is_focused_in_tree()
}

fn chord(text: &str, platform: Platform) -> String {
    Trigger::parse(text).unwrap_or_else(|e| panic!("{text}: {e}")).display(platform)
}

// ---- Recording ----

#[test]
fn recording_captures_a_chord_another_command_would_run() {
    let mut h = mac();
    click(&mut h, "Save: ⌘S");
    assert!(has(&h, "Save: press keys"));
    h.state_mut().keys_left = false;
    press(&mut h, Modifiers::COMMAND, EguiKey::E);
    assert!(!h.state().keys_left, "the dialog took the key press");
    assert!(has(&h, "⚠ ⌘E is used by Export…"));
    assert_eq!(h.state().bindings("file.save"), ["Cmd+S"], "nothing applied before an answer");
}

#[test]
fn recording_takes_tab_arrows_and_shift_escape() {
    let mut h = mac();
    click(&mut h, "Save: ⌘S");
    press(&mut h, Modifiers::NONE, EguiKey::Tab);
    assert_eq!(h.state().bindings("file.save"), ["Tab"]);
    assert!(table_focused(&h), "Tab did not move focus");

    click(&mut h, &format!("Save: {}", chord("Tab", Platform::Mac)));
    press(&mut h, Modifiers::NONE, EguiKey::ArrowDown);
    assert_eq!(h.state().bindings("file.save"), ["Down"]);

    click(&mut h, &format!("Save: {}", chord("Down", Platform::Mac)));
    press(&mut h, Modifiers::SHIFT, EguiKey::Escape);
    assert_eq!(h.state().bindings("file.save"), ["Shift+Escape"]);
    assert!(table_focused(&h));
    assert!(h.state().state.is_open());
}

#[test]
fn escape_cancels_recording_only() {
    let mut h = mac();
    click(&mut h, "Save: ⌘S");
    press(&mut h, Modifiers::NONE, EguiKey::Escape);
    assert!(h.state().state.is_open());
    assert!(!h.state().state.is_recording());
    assert!(!h.state().canceled());
    assert_eq!(h.state().bindings("file.save"), ["Cmd+S"]);
    press(&mut h, Modifiers::NONE, EguiKey::Escape);
    assert!(h.state().canceled());
    assert!(!h.state().state.is_open());
}

#[test]
fn copy_event_records_cmd_c() {
    let mut h = mac();
    click(&mut h, "Save: ⌘S");
    h.event(Event::Copy);
    h.run();
    assert_eq!(h.state().bindings("file.save"), ["Cmd+C"]);
}

#[test]
fn double_press_records_k_k() {
    let mut fx = Fx::new(fixture(Platform::Mac));
    fx.options.double_press = true;
    let mut h = open(fx);
    click(&mut h, "Save: ⌘S");
    raw_key(&mut h, EguiKey::K, 10.0);
    assert!(has(&h, "Save: K (press again for a double press)"));
    raw_key(&mut h, EguiKey::K, 10.3);
    assert_eq!(h.state().bindings("file.save"), ["K K"]);

    click(&mut h, "Save: K K");
    raw_key(&mut h, EguiKey::K, 20.0);
    raw_key(&mut h, EguiKey::K, 20.9);
    assert_eq!(h.state().bindings("file.save"), ["K"]);
}

#[test]
fn the_new_shortcut_field_waits_for_assign() {
    let mut fx = Fx::new(fixture(Platform::Mac));
    fx.options.conflicts = ConflictMode::Keep;
    let mut h = open(fx);
    h.get_by_label("Save").click();
    h.run();
    click(&mut h, "New shortcut: None");
    press(&mut h, Modifiers::COMMAND, EguiKey::E);
    assert!(has(&h, "Currently assigned to:"));
    assert_eq!(h.state().bindings("file.save"), ["Cmd+S"]);
    click(&mut h, "Assign");
    assert!(has(&h, "Also used by Export…"), "Keep mode keeps both");
    assert_eq!(h.state().bindings("file.save"), ["Cmd+S", "Cmd+E"]);
    assert_eq!(h.state().bindings("file.export"), ["Cmd+E"]);
}

/// Presses Tab until the control labeled `label` has focus; false after `max` presses.
fn tab_to(h: &mut Harness<'_, Fx>, label: &str, max: usize) -> bool {
    for _ in 0..max {
        press(h, Modifiers::NONE, EguiKey::Tab);
        if h.get_by_label(label).is_focused() {
            return true;
        }
    }
    false
}

#[test]
fn tab_leaves_the_new_shortcut_field_after_recording() {
    let mut h = mac();
    h.get_by_label("Save").click();
    h.run();
    click(&mut h, "New shortcut: None");
    press(&mut h, Modifiers::NONE, EguiKey::F7);
    assert!(!h.state().state.is_recording());
    assert!(h.get_by_label("New shortcut: F7").is_focused());
    assert!(tab_to(&mut h, "Assign", 6), "Tab moves on to Assign");
    assert_eq!(h.state().bindings("file.save"), ["Cmd+S"], "Tab is not recorded");
}

#[test]
fn tab_leaves_search_by_key_after_recording() {
    let mut h = mac();
    click(&mut h, "Search by key");
    press(&mut h, Modifiers::COMMAND, EguiKey::S);
    assert!(!h.state().state.is_recording());
    assert!(h.get_by_label("Search by key").is_focused());
    press(&mut h, Modifiers::NONE, EguiKey::Tab);
    assert!(table_focused(&h), "Tab moves on to the table");
}

// ---- Conflicts ----

#[test]
fn conflict_take_it_steals_and_undo_restores() {
    let mut h = mac();
    click(&mut h, "Export…: ⌘E");
    press(&mut h, Modifiers::COMMAND, EguiKey::S);
    click(&mut h, "Take It");
    assert_eq!(h.state().bindings("file.save"), Vec::<String>::new());
    assert_eq!(h.state().bindings("file.export"), ["Cmd+S"]);
    assert!(has(&h, "⌘S was removed from Save"));

    // The footer's Undo is after the message's.
    let undo = h.get_all_by_label("Undo").last().map(|n| n.rect());
    let Some(undo) = undo else { panic!("no Undo") };
    h.get_all_by_label("Undo").filter(|n| n.rect() == undo).for_each(|n| n.click());
    h.run();
    assert_eq!(h.state().bindings("file.save"), ["Cmd+S"]);
    assert_eq!(h.state().bindings("file.export"), ["Cmd+E"]);
    click(&mut h, "Redo");
    assert_eq!(h.state().bindings("file.export"), ["Cmd+S"]);
    press(&mut h, Modifiers::COMMAND, EguiKey::Z);
    assert_eq!(h.state().bindings("file.export"), ["Cmd+E"], "Cmd+Z undoes when no text field has focus");
}

#[test]
fn conflict_keep_both_lists_the_clash() {
    let mut h = mac();
    click(&mut h, "Export…: ⌘E");
    press(&mut h, Modifiers::COMMAND, EguiKey::S);
    click(&mut h, "Keep Both");
    let clashes = h.state().km.conflicts().into_iter().filter(|c| c.kind == ConflictKind::Clash).count();
    assert_eq!(clashes, 1);
    assert!(has(&h, "Also used by Save"));
    click(&mut h, "⚠ 1 shortcut is assigned more than once");
    assert!(has(&h, "Save: ⌘S"));
    assert!(has(&h, "Export…: ⌘S"));
    assert!(!has(&h, "Save As…: ⇧⌘S"), "the Conflicts filter shows only the clash");
}

#[test]
fn assign_takes_a_held_shortcut_and_keeps_the_others() {
    let key = |text: &str| Binding::parse(text).unwrap_or_else(|e| panic!("{e}"));
    let catalog = vec![
        Bindable::new("file.print", "Print…", Scope::app()).with_default(key("Cmd+P")),
        Bindable::new("type.paragraphStyles", "Paragraph Styles", Scope::app()).with_default(key("Cmd+F11")),
    ];
    let mut h = open(Fx::new(Keymap::new(catalog, Scopes::new(), Platform::Mac)));
    h.get_by_label("Paragraph Styles").click();
    h.run();
    click(&mut h, "New shortcut: None");
    press(&mut h, Modifiers::COMMAND, EguiKey::P);
    assert!(has(&h, "Currently assigned to:"));
    assert!(has(&h, "⚠ Print…"), "the holder is listed under the field");
    click(&mut h, "Assign");
    assert!(has(&h, "⌘P was removed from Print…"));
    click(&mut h, "OK");
    assert!(h.state().outcomes.iter().any(|o| matches!(o, Outcome::Ok)));

    let km = &h.state().km;
    let run = |text: &str| km.lookup(&Chord::parse(text).unwrap_or_else(|e| panic!("{e}")), false, &Active::NONE);
    assert_eq!(run("Cmd+P"), Lookup::Run("type.paragraphStyles".into()));
    assert_eq!(run("Cmd+F11"), Lookup::Run("type.paragraphStyles".into()));
    assert!(h.state().bindings("file.print").is_empty());
}

#[test]
fn steal_mode_says_what_it_took() {
    let mut fx = Fx::new(fixture(Platform::Mac));
    fx.options.conflicts = ConflictMode::Steal;
    let mut h = open(fx);
    click(&mut h, "Export…: ⌘E");
    press(&mut h, Modifiers::COMMAND, EguiKey::S);
    assert!(has(&h, "⌘S was removed from Save"));
    click(&mut h, "Go to Conflict");
    assert!(selected(&h, "Save (Changed)"));
}

/// Whether a row with this label reports itself selected.
fn selected(h: &Harness<'_, Fx>, label: &str) -> bool {
    h.get_all_by_label(label).any(|n| n.accesskit_node().is_selected() == Some(true))
}

#[test]
fn go_to_conflict_selects_and_reveals_the_holder() {
    let mut h = mac();
    choose_area(&mut h, "Tools");
    search(&mut h, "marquee");
    assert!(!has(&h, "Save: ⌘S"));
    click(&mut h, "Marquee Tool: M");
    press(&mut h, Modifiers::COMMAND, EguiKey::S);
    click(&mut h, "Take It and Go to Conflict");
    assert_eq!(h.state().bindings("tool.marquee"), ["Cmd+S"]);
    assert!(selected(&h, "Save (Changed)"), "the holder is selected");
    assert_eq!(h.get_by_label("Search:").value().unwrap_or_default(), "");
    assert!(has(&h, "Export…: ⌘E"), "area All, no filter");
}

#[test]
fn reserved_chord_is_refused() {
    let km = fixture(Platform::Mac).with_reserved(vec![Chord::parse("Cmd+Tab").unwrap_or_else(|e| panic!("{e}"))]);
    let mut h = open(Fx::new(km));
    click(&mut h, "Save: ⌘S");
    press(&mut h, Modifiers::COMMAND, EguiKey::Tab);
    assert!(has(&h, &format!("⚠ {} is reserved by the system", chord("Cmd+Tab", Platform::Mac))));
    assert_eq!(h.state().bindings("file.save"), ["Cmd+S"]);
}

#[test]
fn app_rule_refuses() {
    let mut fx = Fx::new(fixture(Platform::Mac));
    fx.options.accept = Some(Box::new(|b, binding| {
        if matches!(b.kind, Kind::Tool { .. }) && !binding.trigger.chord.mods.is_none() {
            Err("Tools take keys without modifiers".into())
        } else {
            Ok(())
        }
    }));
    let mut h = open(fx);
    click(&mut h, "Marquee Tool: M");
    press(&mut h, Modifiers::COMMAND, EguiKey::K);
    assert!(has(&h, "⚠ Tools take keys without modifiers"));
    assert_eq!(h.state().bindings("tool.marquee"), ["M"]);
}

// ---- Search, areas, display ----

fn search(h: &mut Harness<'_, Fx>, text: &str) {
    let field = h.get_by_label("Search:");
    field.focus();
    h.run();
    h.key_press_modifiers(Modifiers::COMMAND, EguiKey::A);
    h.run();
    h.get_by_label("Search:").type_text(text);
    h.run();
}

fn choose_area(h: &mut Harness<'_, Fx>, area: &str) {
    h.get_by_role_and_label(Role::ComboBox, "Show:").click();
    h.run();
    // The popup's item, after a group header of the same name.
    h.get_all_by_role_and_label(Role::Button, area).last().unwrap_or_else(|| panic!("{area}")).click();
    h.run();
}

struct German;
impl Text for German {
    fn ui<'a>(&'a self, source: &'a str) -> Cow<'a, str> {
        source.into()
    }
    fn plural(&self, n: u64, one: &str, other: &str) -> String {
        if n == 1 { one } else { other }.replace("{n}", &n.to_string())
    }
    fn command<'a>(&'a self, b: &'a Bindable) -> Cow<'a, str> {
        if b.command.id == "file.save" { "Sichern".into() } else { b.label.as_str().into() }
    }
}

#[test]
fn search_by_name_translation_or_chord() {
    let mut h = mac();
    search(&mut h, "save");
    assert!(has(&h, "Save: ⌘S") && has(&h, "Save As…: ⇧⌘S"));
    assert!(!has(&h, "Export…: ⌘E"));
    search(&mut h, "cmd+s");
    assert!(has(&h, "Save: ⌘S") && !has(&h, "Save As…: ⇧⌘S"));
    search(&mut h, "⌘S");
    assert!(has(&h, "Save: ⌘S") && !has(&h, "Save As…: ⇧⌘S"));
    search(&mut h, "zzz");
    assert!(has(&h, "No matching commands"));

    click(&mut h, "Search by key");
    press(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, EguiKey::S);
    assert_eq!(h.get_by_label("Search:").value().unwrap_or_default(), "⇧⌘S");
    assert!(has(&h, "Save As…: ⇧⌘S") && !has(&h, "Save: ⌘S"));

    let mut fx = Fx::new(fixture(Platform::Mac));
    fx.text = Box::new(German);
    let mut h = open(fx);
    search(&mut h, "sichern");
    assert!(has(&h, "Sichern: ⌘S") && !has(&h, "Save As…: ⇧⌘S"));
}

#[test]
fn areas_follow_paths_kind_and_scope() {
    let mut h = mac();
    h.get_by_role_and_label(Role::ComboBox, "Show:").click();
    h.run();
    for area in ["All Commands", "File Menu", "Object Menu", "Tools", "Timeline Panel", "Other"] {
        assert!(h.get_all_by_label(area).count() >= 1, "{area}");
    }
    h.get_all_by_role_and_label(Role::Button, "Tools").last().expect("the Tools item").click();
    h.run();
    assert!(has(&h, "Marquee Tool: M") && has(&h, "Elliptical Marquee Tool: M"));
    assert!(!has(&h, "Save: ⌘S"));
    choose_area(&mut h, "Timeline Panel");
    assert!(has(&h, "Loop: L") && !has(&h, "Marquee Tool: M"));
}

#[test]
fn display_follows_the_keymap_platform() {
    let h = mac();
    assert!(has(&h, "Save As…: ⇧⌘S"));
    assert!(has(&h, "Bold: ⌘B (Text)"), "a scope other than home is named");
    let h = open(Fx::new(fixture(Platform::Windows)));
    assert!(has(&h, "Save As…: Ctrl+Shift+S"));
}

// ---- Labels, focus, small windows, RTL ----

#[test]
fn every_control_has_a_label() {
    let mut h = mac();
    h.get_by_label("Save").click();
    h.run();
    click(&mut h, "Export…: ⌘E");
    press(&mut h, Modifiers::COMMAND, EguiKey::S);
    assert!(has(&h, "Take It"));
    let mut seen = 0;
    for node in h.root().children_recursive() {
        let n = node.accesskit_node();
        if matches!(n.role(), Role::Button | Role::Tab | Role::TextInput | Role::ComboBox) {
            seen += 1;
            assert!(n.label().is_some_and(|l| !l.trim().is_empty()), "{:?} without a label at {:?}", n.role(), node.rect());
        }
    }
    assert!(seen > 20, "{seen} controls");
}

#[test]
fn table_keys_move_record_and_delete() {
    let mut h = mac();
    h.get_by_label("Save").click();
    h.run();
    assert!(table_focused(&h));
    press(&mut h, Modifiers::NONE, EguiKey::ArrowDown);
    assert!(selected(&h, "Save As…"));
    press(&mut h, Modifiers::NONE, EguiKey::Delete);
    assert_eq!(h.state().bindings("file.saveAs"), Vec::<String>::new());
    press(&mut h, Modifiers::NONE, EguiKey::Enter);
    assert!(h.state().state.is_recording());
    press(&mut h, Modifiers::NONE, EguiKey::F7);
    assert_eq!(h.state().bindings("file.saveAs"), ["F7"]);
}

#[test]
fn group_headers_take_focus_and_toggle_with_enter() {
    let mut h = mac();
    h.get_by_label("Save").click();
    h.run();
    assert!(tab_to(&mut h, "File", 3));
    press(&mut h, Modifiers::NONE, EguiKey::Enter);
    assert!(!has(&h, "Export…: ⌘E"), "File is collapsed");
    assert!(h.get_by_label("File").is_focused());
    press(&mut h, Modifiers::NONE, EguiKey::Enter);
    assert!(has(&h, "Export…: ⌘E"));
}

#[test]
fn table_names_the_selected_row_and_messages_are_live() {
    let mut h = mac();
    h.get_by_label("Save").click();
    h.run();
    let row = h.get_by_role_and_label(Role::Button, "Save").accesskit_node().id();
    assert_eq!(h.get_by_label("Commands").accesskit_node().active_descendant().map(|n| n.id()), Some(row));
    click(&mut h, "Export…: ⌘E");
    press(&mut h, Modifiers::COMMAND, EguiKey::S);
    assert_eq!(h.get_by_label("⚠ ⌘S is used by Save").accesskit_node().live(), egui::accesskit::Live::Polite);
}

#[test]
fn small_window_keeps_ok_and_long_labels_truncate() {
    let long = "A very long command label ".repeat(12);
    let mut km = fixture(Platform::Mac);
    let mut catalog = km.catalog().to_vec();
    catalog.insert(0, Bindable::new("file.long", long.trim(), Scope::app()).with_path(vec!["File".into(), long.trim().into()]));
    km = Keymap::new(catalog, Scopes::new().with_panel("Timeline").with_context("Text"), Platform::Mac);
    let mut fx = Fx::new(km);
    fx.window = true;
    let size = vec2(480.0, 360.0);
    let mut h = harness(fx, size);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let ok = h.get_by_label("OK").rect();
    assert!(screen.contains_rect(ok), "OK at {ok:?}");
    let row = h.get_by_label(long.trim());
    assert!(row.rect().max.x <= size.x + 0.5, "the row stays inside: {:?}", row.rect());
    row.hover();
    h.run();
    assert!(h.query_all_by_label_contains(long.trim()).count() >= 2, "the tooltip has the full label");
    click(&mut h, "OK");
    assert!(h.state().outcomes.iter().any(|o| matches!(o, Outcome::Ok)));
}

#[test]
fn window_close_is_cancel() {
    let mut fx = Fx::new(fixture(Platform::Mac));
    fx.window = true;
    let mut h = open(fx);
    click(&mut h, "Save: ⌘S");
    press(&mut h, Modifiers::NONE, EguiKey::F2);
    assert_eq!(h.state().bindings("file.save"), ["F2"]);
    click(&mut h, "Cancel");
    assert!(h.state().canceled());
    assert_eq!(h.state().bindings("file.save"), ["Cmd+S"]);
    let after: Vec<bool> = h.state().outcomes.iter().skip_while(|o| !matches!(o, Outcome::Cancel)).map(|o| matches!(o, Outcome::Changed)).collect();
    assert_eq!(after, [false, true], "Changed follows Cancel, which undid an edit");
    assert!(!h.state().state.is_open());
}

#[test]
fn automation_ids_are_reported() {
    let seen = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let mut fx = Fx::new(fixture(Platform::Mac));
    fx.options.keyboard = true;
    let sink = seen.clone();
    fx.options.on_widget = Some(Box::new(move |id, rect, label| {
        assert!(rect.is_finite() && !label.is_empty(), "{id}");
        sink.borrow_mut().push(id.to_string());
    }));
    let mut h = open(fx);
    h.get_by_label("Save").click();
    h.run();
    let ids = seen.borrow();
    for id in [
        "shortcuts.area",
        "shortcuts.filter",
        "shortcuts.search",
        "shortcuts.searchKey",
        "shortcuts.row.file.save",
        "shortcuts.cell.file.save.0",
        "shortcuts.add.file.save",
        "shortcuts.reset.file.save",
        "shortcuts.remove.0",
        "shortcuts.new",
        "shortcuts.context",
        "shortcuts.assign",
        "shortcuts.clear",
        "shortcuts.undo",
        "shortcuts.redo",
        "shortcuts.resetAll",
        "shortcuts.cancel",
        "shortcuts.ok",
        "shortcuts.keyboard",
        "shortcuts.mod.cmd",
        "shortcuts.key.F7",
    ] {
        assert!(ids.iter().any(|s| s == id), "{id} not reported");
    }
}

#[test]
fn typed_entry_parses_and_shows_errors() {
    let mut h = mac();
    h.get_by_label("Save").click();
    h.run();
    click(&mut h, "Type…");
    h.get_by_role_and_label(Role::TextInput, "New shortcut:").type_text("Cmd+Nope");
    h.run();
    press(&mut h, Modifiers::NONE, EguiKey::Enter);
    assert!(has(&h, "⚠ unknown key Nope"));
    h.get_by_role_and_label(Role::TextInput, "New shortcut:").focus();
    h.run();
    press(&mut h, Modifiers::COMMAND, EguiKey::A);
    h.get_by_role_and_label(Role::TextInput, "New shortcut:").type_text("Cmd+Shift+K");
    h.run();
    press(&mut h, Modifiers::NONE, EguiKey::Enter);
    assert!(!has(&h, "⚠ unknown key Nope"));
    click(&mut h, "Assign");
    assert_eq!(h.state().bindings("file.save"), ["Cmd+S", "Cmd+Shift+K"]);
}

struct Rtl;
impl Text for Rtl {
    fn ui<'a>(&'a self, source: &'a str) -> Cow<'a, str> {
        source.into()
    }
    fn plural(&self, _n: u64, _one: &str, other: &str) -> String {
        other.to_string()
    }
    fn rtl(&self) -> bool {
        true
    }
}

#[test]
fn rtl_puts_the_detail_pane_left_and_keeps_chords() {
    let mut fx = Fx::new(fixture(Platform::Mac));
    fx.text = Box::new(Rtl);
    let h = open(fx);
    let table = h.get_by_label("Commands").rect();
    assert!(table.min.x > 300.0, "table on the right: {table:?}");
    let chip = h.get_by_label("Save As…: ⇧⌘S").rect();
    assert!(chip.center().x < table.center().x, "shortcut column on the table's left: {chip:?} in {table:?}");
}

// ---- Sets ----

fn with_sets(profiles: Profiles) -> Harness<'static, Fx> {
    let mut fx = Fx::new(fixture(Platform::Mac));
    fx.sets = Some((profiles, vec![("Fixture Defaults", Overrides::new())]));
    open(fx)
}

fn profiles<'h>(h: &'h Harness<'_, Fx>) -> &'h Profiles {
    &h.state().sets.as_ref().expect("sets").0
}

/// The last control with this label: a prompt's button rather than the set row's.
fn click_last(h: &mut Harness<'_, Fx>, label: &str) {
    h.get_all_by_label(label).last().unwrap_or_else(|| panic!("{label}")).click();
    h.run();
}

fn new_set(h: &mut Harness<'_, Fx>, name: &str) {
    click(h, "New Set…");
    h.get_by_role_and_label(Role::TextInput, "Name:").type_text(name);
    h.run();
    click(h, "Create");
}

#[test]
fn cancel_restores_keymap_and_sets() {
    let mut p = Profiles::default();
    p.save_as("Mine", Overrides::new(), &[("Fixture Defaults", Overrides::new())]).expect("save_as");
    let before = p.clone();
    let mut h = with_sets(p);
    click(&mut h, "Set:");
    click_last(&mut h, "Mine");
    assert_eq!(profiles(&h).active(), "Mine");
    click(&mut h, "Save: ⌘S");
    press(&mut h, Modifiers::NONE, EguiKey::F2);
    assert!(has(&h, "(modified)"));
    new_set(&mut h, "Other");
    assert_eq!(profiles(&h).active(), "Other");
    assert_eq!(h.state().bindings("file.save"), ["F2"], "based on the current shortcuts");
    click(&mut h, "Delete");
    assert!(has(&h, "Delete the set “Other”?"));
    click_last(&mut h, "Delete");
    assert!(!profiles(&h).sets().contains_key("Other"));
    click(&mut h, "Cancel");
    assert!(h.state().canceled());
    assert_eq!(profiles(&h), &before);
    assert!(h.state().km.overrides().is_empty());
}

#[test]
fn an_open_prompt_holds_the_set_row_table_and_edit_actions() {
    let mut h = with_sets(Profiles::default());
    click(&mut h, "Save: ⌘S");
    press(&mut h, Modifiers::NONE, EguiKey::F2);
    new_set(&mut h, "Other");
    click(&mut h, "Delete");
    assert!(has(&h, "Delete the set “Other”?"));
    for label in ["Set:", "New Set…", "Save: F2", "Undo", "Reset All…", "OK"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label}");
    }
    assert!(h.get_all_by_label("Cancel").all(|n| !n.accesskit_node().is_disabled()), "the prompt's and the dialog's");
    h.get_all_by_label("Cancel").next().expect("the prompt's Cancel").click();
    h.run();
    assert!(h.state().state.is_open());
    assert!(!h.get_by_label("OK").accesskit_node().is_disabled());
}

#[test]
fn ok_stores_edits_of_a_builtin_as_a_copy() {
    let mut h = with_sets(Profiles::default());
    assert!(h.get_by_label("Rename…").accesskit_node().is_disabled(), "built-ins cannot be renamed");
    click(&mut h, "Save: ⌘S");
    press(&mut h, Modifiers::NONE, EguiKey::F2);
    click(&mut h, "OK");
    assert!(h.state().outcomes.iter().any(|o| matches!(o, Outcome::Ok)));
    let p = profiles(&h);
    assert_eq!(p.active(), "Custom");
    assert_eq!(p.sets().get("Custom").and_then(|o| o.get("file.save")), Some(&["F2".to_string()][..]));
}

#[test]
fn set_export_import_round_trip() {
    let mut h = with_sets(Profiles::default());
    new_set(&mut h, "Name");
    click(&mut h, "Save: ⌘S");
    press(&mut h, Modifiers::NONE, EguiKey::F2);
    h.get_all_by_label("Export…").next().expect("the set row's Export…").click();
    h.run();
    let (name, json) = h
        .state()
        .outcomes
        .iter()
        .find_map(|o| if let Outcome::Export { name, json } = o { Some((name.clone(), json.clone())) } else { None })
        .expect("Outcome::Export");
    assert_eq!(name, "Name");
    let edited = h.state().km.overrides().clone();

    let mut fresh = with_sets(profiles(&h).clone());
    let (d, state) = dialog(fresh.state_mut());
    assert_eq!(d.import_json(state, &json), Ok("Name 2".into()));
    fresh.run();
    assert_eq!(profiles(&fresh).active(), "Name 2");
    assert_eq!(fresh.state().km.overrides(), &edited);
    assert_eq!(fresh.get_by_label("Set:").accesskit_node().value().as_deref(), Some("Name 2"), "the picker shows it");
}

fn foreign_set() -> ForeignSet {
    let read = |text: &str| ForeignKey::read(Some("Default"), text, Flavor::Windows);
    ForeignSet::new(vec![
        ForeignEntry::new(Some("File"), ["Save"]).with_key(read("Ctrl+Alt+S")).at_line(3),
        ForeignEntry::new(Some("File"), ["Frobnicate"]).with_key(read("Ctrl+F")).at_line(4),
        ForeignEntry::new(Some("File"), ["Export…"]).with_key(read("Ctrl+Blorp")).at_line(5),
        ForeignEntry::new(Some("Object"), ["Arrange", "Bring to Front"]).with_key(read("Num 7")).at_line(6),
        ForeignEntry::new(Some("File"), ["Sve Az"]).with_key(read("Ctrl+F9")).at_line(7),
    ])
    .with_name("Theirs")
}

fn review(h: &mut Harness<'_, Fx>) {
    let options = ImportOptions { contexts: vec![("Default".into(), ContextRule::Home)], suggest: true, ..ImportOptions::default() };
    let (d, state) = dialog(h.state_mut());
    d.review_import(state, foreign_set(), options);
    h.run();
}

#[test]
fn foreign_import_review_lists_skipped_and_unreadable() {
    let mut h = with_sets(Profiles::default());
    review(&mut h);
    assert!(has(&h, "Import “Theirs”"));
    for (item, line) in [("File > Save", 3), ("File > Frobnicate", 4), ("File > Export…", 5), ("Object > Arrange > Bring to Front", 6)] {
        let found = h.query_all_by_label_contains(item).any(|n| n.accesskit_node().value().is_some_and(|l| l.contains(&format!("line {line}"))));
        assert!(found, "{item} with line {line}");
    }
    assert!(h.query_by_label_contains("unknown key").is_some(), "the unreadable key says why");
    assert!(h.query_by_label_contains("numeric keypad key read as main key").is_some());
    assert!(h.state().km.overrides().is_empty(), "the review changes nothing");
    click(&mut h, "Import as New Set");
    assert_eq!(profiles(&h).active(), "Theirs");
    assert_eq!(h.state().bindings("file.save"), ["Cmd+Alt+S"]);
    assert!(has(&h, "Imported as “Theirs”"));
    click(&mut h, "Done");
    assert!(h.state().state.is_open() && has(&h, "Commands"));
}

#[test]
fn picking_a_suggestion_replans_and_is_remembered() {
    let mut h = with_sets(Profiles::default());
    review(&mut h);
    assert!(h.query_by_label_contains("3 commands will change").is_some());
    click(&mut h, "Command for File > Sve Az");
    click_last(&mut h, "Save As…");
    let remembered = h.state().outcomes.iter().find_map(|o| if let Outcome::Remembered(m) = o { Some(m.clone()) } else { None });
    assert!(remembered.is_some_and(|m| m.values().any(|c| c.id == "file.saveAs")));
    assert!(h.query_by_label_contains("4 commands will change").is_some());
    assert!(h.query_by_label_contains("File > Sve Az · line 7 — Save As…: ").is_some(), "the pick moved to Will change");
    assert!(h.query_by_label("Command for File > Sve Az").is_none());
}

// ---- Summary ----

#[test]
fn summary_escapes_html_and_keeps_text_lines() {
    let mut km = fixture(Platform::Mac);
    let mut catalog = km.catalog().to_vec();
    let label = "<b>\"Tom\" & 'Jerry'</b>\nNext";
    catalog.push(
        Bindable::new("edit.tricky", label, Scope::app())
            .with_path(vec!["Edit".into(), "Tricky".into()])
            .with_default(craft_keymap::Binding::parse("F6").expect("F6")),
    );
    km = Keymap::new(catalog, Scopes::new().with_panel("Timeline").with_context("Text"), Platform::Mac);
    let mut h = open(Fx::new(km));
    click(&mut h, "Summarize…");
    let (html, text) = h
        .state()
        .outcomes
        .iter()
        .find_map(|o| if let Outcome::Summary { html, text, .. } = o { Some((html.clone(), text.clone())) } else { None })
        .expect("Outcome::Summary");
    assert!(html.contains("&lt;b&gt;&quot;Tom&quot; &amp; &#39;Jerry&#39;&lt;/b&gt;"), "{html}");
    assert!(!html.contains("<b>"));
    assert!(html.contains("<kbd dir=\"ltr\">⌘S</kbd>"));
    assert!(text.lines().any(|l| l == "  <b>\"Tom\" & 'Jerry'</b> Next\tF6"), "{text}");
    assert!(text.lines().any(|l| l == "  Save\t⌘S"), "{text}");
}

#[test]
fn narrow_set_row_moves_actions_into_more() {
    let mut fx = Fx::new(fixture(Platform::Mac));
    fx.sets = Some((Profiles::default(), vec![("Fixture Defaults", Overrides::new())]));
    let mut h = harness(fx, vec2(560.0, 600.0));
    assert!(h.query_by_label("Summarize…").is_none(), "Summarize… is in More");
    click(&mut h, "More");
    click(&mut h, "Summarize…");
    assert!(h.state().outcomes.iter().any(|o| matches!(o, Outcome::Summary { name, .. } if name == "Fixture Defaults")));
}

// ---- On-screen keyboard, fixed rows, read-only ----

fn with_keyboard(options: impl FnOnce(&mut Fx)) -> Harness<'static, Fx> {
    let mut fx = Fx::new(fixture(Platform::Mac));
    fx.options.keyboard = true;
    options(&mut fx);
    open(fx)
}

/// What a key's color means, as its accessibility description says it.
fn holders_class(h: &Harness<'_, Fx>, key: &str) -> String {
    h.get_by_role_and_label(Role::Button, key).accesskit_node().description().unwrap_or_default().to_string()
}

#[test]
fn keyboard_colors_keys_and_assigns_to_the_selected_command() {
    let mut h = with_keyboard(|_| {});
    assert_eq!(holders_class(&h, "M: Marquee Tool, Elliptical Marquee Tool"), "Application");
    assert_eq!(holders_class(&h, "L: Loop (Timeline)"), "Panel or context");
    assert_eq!(holders_class(&h, "F7"), "Not assigned");

    click(&mut h, "⌘ Command");
    assert_eq!(holders_class(&h, "⌘S: Save"), "Application");
    assert_eq!(holders_class(&h, "⌘B: Bold (Text)"), "Panel or context");
    // Held modifiers add to the toggles.
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.run();
    assert!(has(&h, "⇧⌘S: Save As…"));
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run();
    click(&mut h, "⌘ Command");

    click(&mut h, "L: Loop (Timeline)");
    assert!(has(&h, "Key: L"));
    assert!(h.get_by_label("Assign to Selected").accesskit_node().is_disabled(), "no command selected");
    click(&mut h, "Save");
    click(&mut h, "Assign to Selected");
    assert_eq!(h.state().bindings("file.save"), ["Cmd+S", "L"]);
    assert_eq!(holders_class(&h, "L: Save, Loop (Timeline)"), "Both");

    // A holder in the key list selects its command.
    click(&mut h, "Loop (Timeline)");
    assert!(selected(&h, "Loop"));
}

#[test]
fn fixed_rows_are_struck_through_when_a_command_takes_their_key() {
    let mut fx = Fx::new(fixture(Platform::Mac));
    let row = |label: &str, keys: &str, chord: Option<&str>| FixedRow {
        group: "Rating".into(),
        label: label.into(),
        keys: keys.into(),
        chord: chord.map(|c| Trigger::parse(c).unwrap_or_else(|e| panic!("{c}: {e}"))),
    };
    fx.options.fixed = vec![row("Rate 1", "1", Some("1")), row("Rate 2", "2", Some("2")), row("Pan", "Space (hold)", None)];
    let mut h = open(fx);
    choose_area(&mut h, "Fixed Keys");
    assert!(has(&h, "Rate 1: 1"));
    assert!(has(&h, "Pan: Space (hold)"));

    choose_area(&mut h, "All Commands");
    click(&mut h, "Save: ⌘S");
    press(&mut h, Modifiers::NONE, EguiKey::Num1);
    assert_eq!(h.state().bindings("file.save"), ["1"]);
    choose_area(&mut h, "Fixed Keys");
    assert!(has(&h, "Rate 1: 1 (Now used by Save)"));
    assert!(has(&h, "Rate 2: 2"));
}

#[test]
fn read_only_has_no_edit_controls() {
    let mut h = with_keyboard(|fx| {
        fx.options.editable = false;
        fx.sets = Some((Profiles::default(), vec![("Fixture Defaults", Overrides::new())]));
    });
    click(&mut h, "Save");
    click(&mut h, "Save: ⌘S");
    assert!(!has(&h, "Save: press keys"), "no recording");
    click(&mut h, "S");
    assert!(has(&h, "Key: S"));
    for label in [
        "Undo",
        "Redo",
        "Reset All…",
        "OK",
        "Cancel",
        "Set:",
        "New Set…",
        "Import…",
        "Add a shortcut to Save",
        "Restore the default for Save",
        "Remove ⌘S from Save",
        "New shortcut:",
        "Context:",
        "Assign",
        "Use Default",
        "Clear",
        "Assign to Selected",
    ] {
        assert!(!has(&h, label), "{label} is shown");
    }
    press(&mut h, Modifiers::COMMAND, EguiKey::Z);
    assert_eq!(h.state().bindings("file.save"), ["Cmd+S"]);
    click(&mut h, "Summarize…");
    assert!(h.state().outcomes.iter().any(|o| matches!(o, Outcome::Summary { .. })));
    click(&mut h, "Close");
    assert!(h.state().canceled());
}

// ---- Pseudo locale ----

/// Text a test put there or read from a foreign file, which no lookup translates.
const RAW: &[&str] = &["Theirs", "Frobnicate", "Sve Az", "Blorp", "Num", "Name 2"];

/// Every control inside the `width` px dialog, and every label with words marked by `Pseudo`.
fn check_pseudo(h: &Harness<'_, Fx>, width: f32, step: &str) {
    let mut seen = 0;
    for node in h.root().children_recursive() {
        let n = node.accesskit_node();
        let Some(label) = n.label() else { continue };
        seen += 1;
        let r = node.rect();
        assert!(r.min.x >= -1.0 && r.max.x <= width + 1.0, "{step}: {:?} {label:?} at {r:?} leaves the {width} px dialog", n.role());
        let mut rest = String::new();
        let mut depth = 0usize;
        for c in label.chars() {
            match c {
                '⟦' => depth += 1,
                '⟧' => depth = depth.saturating_sub(1),
                _ if depth == 0 => rest.push(c),
                _ => {}
            }
        }
        for raw in RAW {
            rest = rest.replace(raw, "");
        }
        let words = rest.split(|c: char| !c.is_ascii_lowercase()).any(|w| w.len() > 1);
        assert!(!words, "{step}: {label:?} has text that did not go through Text");
    }
    assert!(seen > 15, "{step}: {seen} labeled nodes");
}

#[test]
fn pseudo_locale_marks_every_string_and_fits() {
    let p = Pseudo::mark;
    // Each direction at one width: the narrow layout left to right, the wide one right to left.
    for (rtl, width) in [(false, 560.0), (true, 1100.0)] {
        let unlisted = std::rc::Rc::default();
        let mut fx = Fx::new(with_long_label(fixture(Platform::Mac)));
        fx.text = Box::new(Pseudo { unlisted: std::rc::Rc::clone(&unlisted), rtl });
        fx.options.keyboard = true;
        fx.sets = Some((Profiles::default(), vec![("Fixture Defaults", Overrides::new())]));
        let mut h = harness(fx, vec2(width, 720.0));
        let step = |s: &str| format!("{} {width}: {s}", if rtl { "rtl" } else { "ltr" });
        check_pseudo(&h, width, &step("opened"));
        h.state_mut().state.select(&CommandRef::from("object.distribute"));
        h.run();
        check_pseudo(&h, width, &step("long label selected"));
        let export = craft_i18n::fmt(&p("{label}: {chord}"), &[("label", &p("Export…")), ("chord", "⌘E")]);
        click(&mut h, &export);
        press(&mut h, Modifiers::COMMAND, EguiKey::S);
        assert!(has(&h, &p("Take It")), "{}", step("conflict bar"));
        check_pseudo(&h, width, &step("conflict"));
        click(&mut h, &p("Keep Both"));
        check_pseudo(&h, width, &step("kept both"));
        click(&mut h, &p("Keyboard"));
        check_pseudo(&h, width, &step("keyboard hidden"));
        review(&mut h);
        check_pseudo(&h, width, &step("review"));
        assert!(unlisted.borrow().is_empty(), "looked up but not in STRINGS or PLURALS: {:?}", unlisted.borrow());
    }
}

#[test]
fn an_app_error_shows_until_dismissed() {
    let mut h = mac();
    h.state_mut().state.show_error("The file could not be read");
    h.run();
    assert!(has(&h, "⚠ The file could not be read"));
    click(&mut h, "Dismiss");
    assert!(!has(&h, "⚠ The file could not be read"));
}

#[test]
fn one_pointer_click_on_a_chip_of_another_row_records() {
    let mut h = mac();
    click(&mut h, "Save");
    let at = h.get_by_label("Export…: ⌘E").rect().center();
    h.event(Event::PointerMoved(at));
    h.run();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run();
    assert!(has(&h, "Export…: press keys"));
}

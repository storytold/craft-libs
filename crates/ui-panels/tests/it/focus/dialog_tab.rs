//! `RootEnv::dialog_tab`: which stops Tab visits inside a dialog's trap.

use super::{control, focus, focused, id, tap};
use craft_behavior::{DocKey, Input, NavShape, Roving};
use craft_host::target::{Os, ShellInfo};
use craft_ui::runtime::env::RootEnv;
use craft_ui::runtime::focus::{FocusPolicy, FocusTarget, KeyUse, TabReach, request};
use craft_ui::testing::focus::{Group, Stop, pop_scope, push_scope, register_doc, register_group, register_stop, widget};
use craft_ui::testing::{Frames, install_spec, trial};
use egui::{Key, Modifiers, Ui};

/// Frames on `os` whose root env sets `dialog_tab`.
fn frames_on(os: Os, dialog_tab: TabReach) -> Frames {
    let mut frames = Frames::uninstalled();
    let mut spec = install_spec();
    spec.shell_info = ShellInfo::default().os(os);
    craft_ui::install(&frames.ctx, spec).expect("the runtime installs once");
    let mut root = RootEnv::default();
    root.dialog_tab = dialog_tab;
    frames.root(root);
    frames
}

/// A text stop over an egui widget.
fn field(ui: &mut Ui, name: &str) {
    widget(ui, id(name));
    register_stop(ui.ctx(), Stop::new(id(name), KeyUse::of(Input::Text)).text());
}

/// A trap scope with `policy` holding a button, a checkbox and a radio group, between two fields when `fields`.
fn scope(ui: &mut Ui, policy: FocusPolicy, fields: bool) {
    push_scope(ui.ctx(), id("scope"), policy, None);
    if fields {
        field(ui, "first");
    }
    control(ui, "button");
    control(ui, "check");
    widget(ui, id("radio"));
    for member in ["yes", "no"] {
        register_stop(ui.ctx(), Stop::new(id(member), KeyUse::of(Input::Pointer)).root(id("radio")).in_group(id("radio")));
    }
    register_group(ui.ctx(), Group::new(id("radio"), Roving::new(NavShape::Flat)));
    if fields {
        field(ui, "second");
    }
    pop_scope(ui.ctx());
}

/// Shows the scope, focuses `from` and presses Tab (Shift+Tab when `back`) once; returns the stop egui focus is on.
fn tab_from(frames: &mut Frames, policy: FocusPolicy, fields: bool, from: &str, back: bool) -> Option<egui::Id> {
    let mut show = |ui: &mut Ui| scope(ui, policy, fields);
    frames.idle(2, &mut show);
    focus(frames, from, &mut show);
    frames.idle(1, &mut show);
    tap(frames, Key::Tab, if back { Modifiers::SHIFT } else { Modifiers::NONE }, &mut show);
    focused(&frames.ctx)
}

/// With `Fields`, Tab in a dialog visits its text stops only, on every OS: it passes over the button, the checkbox and
/// the radio group and wraps between the two fields. `Root` keeps the window's rule, a popup's trap keeps it under
/// `Fields`, and a dialog with no field leaves focus where it is.
#[trial]
fn tab_in_a_fields_only_dialog_visits_text_stops_only() {
    for os in [Os::Mac, Os::Windows] {
        let mut frames = frames_on(os, TabReach::Fields);
        let mut show = |ui: &mut Ui| scope(ui, FocusPolicy::DIALOG, true);
        frames.idle(2, &mut show);
        focus(&mut frames, "first", &mut show);
        frames.idle(1, &mut show);
        tap(&mut frames, Key::Tab, Modifiers::NONE, &mut show);
        assert_eq!(focused(&frames.ctx), Some(id("second")), "{os:?}: Tab passes over the controls to the next field");
        tap(&mut frames, Key::Tab, Modifiers::NONE, &mut show);
        assert_eq!(focused(&frames.ctx), Some(id("first")), "{os:?}: Tab wraps to the first field");
        tap(&mut frames, Key::Tab, Modifiers::SHIFT, &mut show);
        assert_eq!(focused(&frames.ctx), Some(id("second")), "{os:?}: Shift+Tab wraps back to the last field");
    }

    let root = tab_from(&mut frames_on(Os::Windows, TabReach::Root), FocusPolicy::DIALOG, true, "first", false);
    assert_eq!(root, Some(id("button")), "under `Root` the dialog takes the window's rule: Tab reaches the button");

    let popup = tab_from(&mut frames_on(Os::Windows, TabReach::Fields), FocusPolicy::POPUP, true, "first", false);
    assert_eq!(popup, Some(id("button")), "a popup's trap keeps the window's rule under `Fields`");

    let none = tab_from(&mut frames_on(Os::Windows, TabReach::Fields), FocusPolicy::DIALOG, false, "button", false);
    assert_eq!(none, Some(id("button")), "a dialog with no field leaves Tab where it was");
    let none_back = tab_from(&mut frames_on(Os::Mac, TabReach::Fields), FocusPolicy::DIALOG, false, "check", true);
    assert_eq!(none_back, Some(id("check")), "Shift+Tab too");
}

/// A document key's registration lives one pass: while its component shows, `FocusTarget::Doc` focuses it; once the
/// component has stopped showing for a pass, the request moves no focus and leaves egui focus where it was.
#[trial]
fn a_doc_key_focuses_nothing_once_its_component_stops_showing() {
    const KEY: DocKey = DocKey::new("craft-test.find-field");
    let mut frames = frames_on(Os::Mac, TabReach::Root);
    let mut with_doc = |ui: &mut Ui| {
        control(ui, "other");
        field(ui, "target");
        register_doc(ui.ctx(), KEY, id("target"));
    };
    let mut without_doc = |ui: &mut Ui| control(ui, "other");
    frames.idle(2, &mut with_doc);
    focus(&mut frames, "other", &mut with_doc);
    request(&frames.ctx, FocusTarget::Doc(KEY));
    frames.frame(Vec::new(), &mut with_doc);
    assert_eq!(focused(&frames.ctx), Some(id("target")), "a showing component takes the request");

    focus(&mut frames, "other", &mut with_doc);
    frames.idle(2, &mut without_doc);
    request(&frames.ctx, FocusTarget::Doc(KEY));
    frames.frame(Vec::new(), &mut without_doc);
    assert_eq!(focused(&frames.ctx), Some(id("other")), "a component that stopped showing takes none");
}

//! The egui bar: clicks, hover, the keys, overflow, scrolling, submenu hover intent and AccessKit.

use egui::accesskit::{self, NodeId, Role, Toggled};
use egui::{Event, Id, Key, Modifiers, Pos2, Rect, Vec2, pos2, vec2};

use super::*;
use crate::bar::Labeled;
use crate::bar::nav::{ARROW, Nav, heading, is_open};

#[path = "bar_keys.rs"]
mod keys;

fn item(id: &str, label: &str) -> Item {
    Item::new(id, label)
}

/// A small app's menus, as a desktop layout leaves them.
fn sample() -> MenuBar {
    MenuBar {
        menus: vec![
            Menu::new(
                "File",
                vec![
                    item("file.new", "New").shortcut(Chord::parse("Cmd+N").unwrap()).into(),
                    item("file.open", "Open…").into(),
                    Node::Separator,
                    Node::submenu("Open Recent", vec![item("file.recent.a", "a.txt").into(), item("file.recent.b", "b.txt").into()]),
                    Node::submenu("Nothing Here", vec![item("file.gone", "Gone").enabled(false).into()]),
                    Node::Separator,
                    item("file.locked", "Locked").enabled(false).into(),
                    Item::new(Builtin::Quit.command(), "Quit").into(),
                ],
            )
            .role(MenuRole::File),
            Menu::new("Edit", vec![item("edit.undo", "Undo").into(), item("edit.redo", "Redo").into()]).role(MenuRole::Edit),
            Menu::new(
                "View",
                vec![
                    item("view.grid", "Grid").check(true).shortcut(Chord::parse("G").unwrap()).into(),
                    item("view.rulers", "Rulers").check(false).into(),
                    Node::Separator,
                    item("view.light", "Light").radio(true).into(),
                    item("view.dark", "Dark").radio(false).into(),
                ],
            )
            .role(MenuRole::View),
            Menu::new("Help", vec![item("help.about", "About").into()]).role(MenuRole::Help),
        ],
        documents: Vec::new(),
    }
}

/// The bar alone in a window, driven frame by frame with synthetic input.
struct Bar {
    ctx: egui::Context,
    bar: MenuBar,
    gate: Gate,
    target: Target,
    english: Option<fn(Labeled<'_>) -> Option<String>>,
    preferred: Vec<(&'static str, char)>,
    size: Vec2,
    dt: f64,
    time: f64,
    events: Vec<Event>,
    pointer: Pos2,
    tree: Vec<(NodeId, accesskit::Node)>,
    clicked: Vec<CommandRef>,
    /// `shrink_to_fit`'s limit; the bar then shares a row with a label, which starts at `after`.
    fit: Option<f32>,
    after: Option<f32>,
    /// The viewport commands the bar sent.
    sent: Vec<egui::ViewportCommand>,
}

impl Bar {
    fn new(bar: MenuBar, size: Vec2) -> Bar {
        let ctx = egui::Context::default();
        // Not the host's: the web target's key names follow it.
        ctx.set_os(egui::os::OperatingSystem::Nix);
        ctx.enable_accesskit();
        ctx.all_styles_mut(|s| s.scroll_animation = egui::style::ScrollAnimation::none());
        let mut b = Bar {
            ctx,
            bar,
            gate: Gate::default(),
            target: Target::Linux,
            english: None,
            preferred: Vec::new(),
            size,
            dt: 1.0 / 60.0,
            time: 0.0,
            events: Vec::new(),
            pointer: pos2(-10.0, -10.0),
            tree: Vec::new(),
            clicked: Vec::new(),
            fit: None,
            after: None,
            sent: Vec::new(),
        };
        b.run(3);
        b
    }

    fn frame(&mut self) {
        self.time += self.dt;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            predicted_dt: self.dt as f32,
            events: std::mem::take(&mut self.events),
            ..Default::default()
        };
        let (bar, gate, target, english, preferred, fit) = (&self.bar, self.gate, self.target, self.english, &self.preferred, self.fit);
        let mut out = Vec::new();
        let mut after = None;
        let mut full = self.ctx.run_ui(raw, |ui| {
            let mut ui_bar = MenuBarUi::new(bar).gate(gate).target(target).mnemonics(preferred);
            if let Some(english) = &english {
                ui_bar = ui_bar.english(english);
            }
            match fit {
                Some(max) => {
                    ui.horizontal(|ui| {
                        out = ui_bar.shrink_to_fit(max).show(ui);
                        after = Some(ui.label("after").rect.left());
                    });
                }
                None => out = ui_bar.show(ui),
            }
        });
        full.textures_delta.clear();
        self.after = after;
        if let Some(root) = full.viewport_output.remove(&egui::ViewportId::ROOT) {
            self.sent.extend(root.commands);
        }
        if let Some(update) = full.platform_output.accesskit_update {
            self.tree = update.nodes;
        }
        self.clicked.extend(out);
    }

    fn run(&mut self, frames: usize) {
        for _ in 0..frames {
            self.frame();
        }
    }

    /// The commands chosen since the last call.
    fn take(&mut self) -> Vec<CommandRef> {
        std::mem::take(&mut self.clicked)
    }

    fn nodes(&self, label: &str) -> impl Iterator<Item = &accesskit::Node> {
        self.tree.iter().map(|(_, n)| n).filter(move |n| n.label() == Some(label))
    }

    fn node(&self, label: &str) -> Option<&accesskit::Node> {
        self.nodes(label).next()
    }

    fn shown(&self, label: &str) -> bool {
        self.node(label).is_some()
    }

    /// A title of the bar (not a submenu row of the same name).
    fn title(&self, label: &str) -> Option<&accesskit::Node> {
        let (_, bar) = self.tree.iter().find(|(_, n)| n.role() == Role::MenuBar)?;
        self.tree.iter().filter(|(id, _)| bar.children().contains(id)).map(|(_, n)| n).find(|n| n.label() == Some(label))
    }

    fn rect(&self, label: &str) -> Rect {
        let n = self.node(label).unwrap_or_else(|| panic!("no {label:?}"));
        let b = n.bounds().unwrap_or_else(|| panic!("{label:?} has no bounds"));
        Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32))
    }

    fn move_to(&mut self, p: Pos2) {
        self.pointer = p;
        self.events.push(Event::PointerMoved(p));
        self.frame();
    }

    /// A point on `label`, near its left end: a menu row can be wider than a narrow window.
    fn point(&self, label: &str) -> Pos2 {
        let r = self.rect(label);
        pos2(r.left() + (r.width() / 2.0).min(20.0), r.center().y)
    }

    fn hover(&mut self, label: &str) {
        let p = self.point(label);
        self.move_to(p);
        self.run(3);
    }

    fn button(&mut self, pressed: bool) {
        self.events.push(Event::PointerButton { pos: self.pointer, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        self.frame();
    }

    fn click_at(&mut self, p: Pos2) {
        self.move_to(p);
        self.button(true);
        self.button(false);
        self.run(3);
    }

    fn click(&mut self, label: &str) {
        let p = self.point(label);
        self.click_at(p);
    }

    fn key(&mut self, key: Key) {
        for pressed in [true, false] {
            self.events.push(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
        }
        self.run(3);
    }

    fn nav(&self) -> Nav {
        Nav::current(&self.ctx, Id::unique("craft-menu-bar").with("nav"))
    }

    /// Is row `i` of `level` entirely inside its level's visible (scrolled) part, in the window?
    fn visible(&self, level: usize, i: usize) -> bool {
        let nav = self.nav();
        let (Some(row), Some(view)) = (nav.rows.get(level).and_then(|r| r.get(i)), nav.views.get(level)) else { return false };
        view.expand(0.5).contains_rect(row.rect) && self.ctx.content_rect().expand(0.5).contains_rect(*view)
    }
}

fn wide() -> Bar {
    Bar::new(sample(), vec2(800.0, 600.0))
}

#[test]
fn clicking_an_item_returns_its_command_and_closes_the_menu() {
    let mut b = wide();
    assert!(!b.shown("New"));
    b.click("File");
    assert!(b.shown("New") && is_open(&b.ctx), "a click on a title opens its menu");
    b.click("New");
    assert_eq!(b.take(), vec![CommandRef::new("file.new")]);
    assert!(!b.shown("New") && !is_open(&b.ctx), "choosing an item closes the menu");
}

#[test]
fn a_second_click_escape_or_a_click_outside_closes_the_menu() {
    let mut b = wide();
    b.click("File");
    b.click("File");
    assert!(!b.shown("New"), "a second click on the title");
    b.click("File");
    b.key(Key::Escape);
    assert!(!b.shown("New"), "Esc");
    b.click("File");
    b.click_at(pos2(700.0, 500.0));
    assert!(!b.shown("New"), "a click outside");
    assert!(b.take().is_empty());
}

#[test]
fn press_drag_release_runs_an_item() {
    let mut b = wide();
    let file = b.rect("File").center();
    b.move_to(file);
    b.button(true);
    b.run(3);
    assert!(b.shown("Open…"), "the press opens the menu");
    // Down out of the bar first, then across: crossing the Edit title would open Edit.
    let open = b.rect("Open…").center();
    let below = pos2(file.x, open.y);
    for k in 1..=6 {
        b.move_to(file + (below - file) * (k as f32 / 6.0));
    }
    b.move_to(open);
    b.button(false);
    b.run(2);
    assert_eq!(b.take(), vec![CommandRef::new("file.open")]);
}

#[test]
fn disabled_and_gate_forbidden_items_return_nothing() {
    let mut b = wide();
    b.click("File");
    b.click("Locked");
    assert!(b.take().is_empty(), "a disabled item");
    assert!(b.node("Locked").is_some_and(|n| n.is_disabled()));
    assert!(b.node("Nothing Here").is_some_and(|n| n.is_disabled()), "a submenu of disabled items is disabled");

    b.key(Key::Escape);
    b.gate = Gate { modal: true, ..Gate::default() };
    b.click("File");
    assert!(b.node("New").is_some_and(|n| n.is_disabled()), "a modal dialog disables the app's items");
    b.click("New");
    assert!(b.take().is_empty(), "and they don't run");
    b.click("Quit");
    assert!(b.take().is_empty(), "the bar runs the built-in Quit itself");
    assert!(b.sent.contains(&egui::ViewportCommand::Close), "and it still runs: the window is asked to close");

    b.gate = Gate { text_editing: true, ..Gate::default() };
    b.click("File");
    b.click("New");
    assert_eq!(b.take(), vec![CommandRef::new("file.new")], "a text field having the keys doesn't stop a click");
}

#[test]
fn hovering_another_title_switches_menus_only_while_one_is_open() {
    let mut b = wide();
    b.hover("Edit");
    assert!(!b.shown("Undo"), "hover alone opens nothing");
    b.click("File");
    b.hover("Edit");
    assert!(b.shown("Undo") && !b.shown("New"), "hover switches to Edit");
    b.hover("View");
    assert!(b.shown("Grid") && !b.shown("Undo"));
}

#[test]
fn submenus_open_on_hover() {
    let mut b = wide();
    b.click("File");
    assert!(!b.shown("a.txt"));
    b.hover("Open Recent");
    b.run(3);
    assert!(b.shown("a.txt"), "hovering a submenu row opens it");
    assert_eq!(b.node("Open Recent").and_then(|n| n.is_expanded()), Some(true));
    b.click("b.txt");
    assert_eq!(b.take(), vec![CommandRef::new("file.recent.b")]);
}

#[test]
fn the_arrow_keys_move_open_submenus_and_enter_runs_an_item() {
    let mut b = wide();
    b.click("File");
    b.key(Key::ArrowDown);
    assert_eq!(b.nav().highlighted(0), Some(0), "↓ lights the first row");
    b.key(Key::ArrowDown);
    b.key(Key::ArrowDown);
    let nav = b.nav();
    let row = nav.highlighted(0).and_then(|i| nav.rows[0].get(i)).cloned().unwrap();
    assert!(row.command.is_none(), "↓ skips the separator to Open Recent");
    b.key(Key::ArrowRight);
    let nav = b.nav();
    assert_eq!(nav.level, 1, "→ moves into the submenu");
    assert_eq!(nav.highlighted(1), Some(0), "on its first row");
    assert!(b.shown("a.txt"));
    b.key(Key::ArrowLeft);
    let nav = b.nav();
    assert_eq!((nav.level, nav.rows.len()), (0, 1), "← closes the submenu");
    // ↓ past the disabled submenu and item, then wrapping to the first row.
    b.key(Key::ArrowDown);
    let nav = b.nav();
    assert_eq!(nav.highlighted(0).and_then(|i| nav.rows[0][i].command.clone()), Some(Builtin::Quit.command()), "disabled rows are skipped");
    b.key(Key::ArrowDown);
    assert_eq!(b.nav().highlighted(0), Some(0), "↓ wraps");
    b.key(Key::Enter);
    assert_eq!(b.take(), vec![CommandRef::new("file.new")]);
    assert!(!is_open(&b.ctx), "↩ closes the menu");

    b.click("Edit");
    b.key(Key::ArrowUp);
    b.key(Key::Space);
    assert_eq!(b.take(), vec![CommandRef::new("edit.redo")], "↑ from nothing lights the last row; Space runs it");
}

#[test]
fn left_and_right_switch_menus() {
    let mut b = wide();
    b.click("File");
    b.key(Key::ArrowRight);
    assert!(b.shown("Undo") && !b.shown("New"), "→ on a plain row opens the next menu");
    b.key(Key::ArrowLeft);
    assert!(b.shown("New"), "← back to File");
    b.key(Key::ArrowLeft);
    assert!(b.shown("About"), "← from the first menu wraps to the last");
    b.key(Key::Escape);
    assert!(!is_open(&b.ctx));
}

#[test]
fn a_gate_forbidden_item_doesnt_run_from_the_keys() {
    let mut b = wide();
    b.gate = Gate { recording_shortcut: true, ..Gate::default() };
    b.click("Edit");
    b.key(Key::ArrowDown);
    b.key(Key::Enter);
    assert!(b.take().is_empty());
    assert_eq!(b.nav().highlighted(0), None, "no enabled row to light");
}

#[test]
fn titles_that_dont_fit_move_into_the_overflow_menu() {
    let file_and_edit = {
        let b = wide();
        b.rect("Edit").right() - b.rect("File").left()
    };
    let mut b = Bar::new(sample(), vec2(file_and_edit + 30.0, 400.0));
    assert!(b.title("File").is_some() && b.title("Edit").is_some());
    assert!(b.title("Help").is_none() && b.title("⋯").is_some(), "Help moved into ⋯");
    b.click("⋯");
    assert!(b.shown("View") && b.shown("Help"), "⋯ lists the menus that don't fit");
    b.hover("Help");
    b.run(3);
    b.click("About");
    assert_eq!(b.take(), vec![CommandRef::new("help.about")]);

    let b = Bar::new(sample(), vec2(20.0, 400.0));
    assert!(b.title("File").is_none() && b.title("⋯").is_none());
    assert!(b.title("Menu").is_some(), "one Menu holds all when none fits");
}

#[test]
fn the_overflow_labels_come_from_the_label_hook() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let bar = sample();
    let label = |english: &'static str| if english == "Menu" { "Menü".to_owned() } else { english.to_owned() };
    let mut tree = Vec::new();
    for _ in 0..2 {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(20.0, 300.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            MenuBarUi::new(&bar).label(&label).show(ui);
        });
        out.textures_delta.clear();
        tree = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    }
    assert!(tree.iter().any(|(_, n)| n.label() == Some("Menü")));
}

#[test]
fn accesskit_sees_a_menu_bar_of_menus_with_checked_states_and_shortcuts() {
    let mut b = wide();
    assert!(b.tree.iter().any(|(_, n)| n.role() == Role::MenuBar), "the bar");
    let file = b.title("File").expect("File is a title in the bar");
    assert_eq!(file.role(), Role::MenuItem);
    assert_eq!(file.is_expanded(), Some(false));
    b.click("View");
    assert_eq!(b.title("View").and_then(|n| n.is_expanded()), Some(true));
    assert!(b.tree.iter().any(|(_, n)| n.role() == Role::Menu), "the open menu");
    let state = |label: &str| b.node(label).map(|n| (n.role(), n.toggled()));
    assert_eq!(state("Grid"), Some((Role::MenuItemCheckBox, Some(Toggled::True))));
    assert_eq!(state("Rulers"), Some((Role::MenuItemCheckBox, Some(Toggled::False))));
    assert_eq!(state("Light"), Some((Role::MenuItemRadio, Some(Toggled::True))));
    assert_eq!(state("Dark"), Some((Role::MenuItemRadio, Some(Toggled::False))));
    assert_eq!(b.node("Grid").and_then(|n| n.keyboard_shortcut()), Some("G"), "a display-only shortcut too");
    b.hover("File");
    assert_eq!(b.node("New").map(|n| (n.role(), n.keyboard_shortcut())), Some((Role::MenuItem, Some("Ctrl+N"))));
}

/// In a browser the names are those of the system it runs on, and on a Mac there are no
/// mnemonics: Option types there.
#[test]
fn the_web_shows_the_browsers_system_names_and_the_mac_its_glyphs() {
    use egui::os::OperatingSystem as Os;
    for (target, os, text, mnemonic) in [
        (Target::Web, Os::Nix, "Ctrl+N", true),
        (Target::Web, Os::Mac, "⌘N", false),
        (Target::Windows, Os::Nix, "Ctrl+N", true),
        (Target::Mac, Os::Nix, "⌘N", false),
    ] {
        let mut b = wide();
        b.ctx.set_os(os);
        b.target = target;
        b.click("File");
        let new = b.node("New");
        assert_eq!(new.and_then(|n| n.keyboard_shortcut()), Some(text), "{target:?} on {os:?}");
        assert_eq!(new.and_then(|n| n.access_key()).is_some(), mnemonic, "{target:?} on {os:?}");
    }
}

/// `shrink_to_fit` leaves the rest of the row to other controls; past its limit the menus
/// overflow into "⋯".
#[test]
fn a_bar_shrunk_to_fit_shares_its_row() {
    let mut b = wide();
    b.fit = Some(f32::INFINITY);
    b.run(2);
    let after = b.after.expect("the label after the bar");
    assert!(after < b.size.x / 2.0, "the bar takes only its titles' width: {after}");
    assert!(b.title("Help").is_some() && b.title("⋯").is_none());
    b.fit = Some(100.0);
    b.run(2);
    assert!(b.title("⋯").is_some(), "the menus past 100 points overflow");
    // The bar's 100 points and the row's item spacing (8).
    assert!(b.after.is_some_and(|x| x <= 109.0), "{:?}", b.after);
    // A limit an app computed from a full row can go negative.
    b.fit = Some(-20.0);
    b.run(2);
}

/// A menu far taller than the window.
fn tall() -> Bar {
    let rows = (0..60).map(|i| item(&format!("cmd.{i}"), &format!("Item {i}")).into()).collect();
    Bar::new(MenuBar { menus: vec![Menu::new("Long", rows)], documents: Vec::new() }, vec2(600.0, 300.0))
}

fn open_tall() -> Bar {
    let mut b = tall();
    b.click("Long");
    b.run(3);
    assert_eq!(b.nav().rows[0].len(), 60);
    b
}

#[test]
fn a_tall_menu_stays_in_the_window_below_the_bar_and_shows_scroll_arrows() {
    let b = open_tall();
    let nav = b.nav();
    let screen = b.ctx.content_rect();
    assert!(!b.visible(0, 59), "the menu must overflow for this test to mean anything");
    assert!(nav.views[0].bottom() <= screen.bottom() && nav.views[0].top() >= b.rect("Long").bottom());
    assert!(b.shown("Scroll menu down") && b.shown("Scroll menu up"));
}

#[test]
fn every_row_of_a_tall_menu_is_reachable_with_the_arrow_keys() {
    let mut b = open_tall();
    for i in 0..60 {
        b.key(Key::ArrowDown);
        // The scroll lands a frame after the row is revealed.
        b.run(2);
        assert_eq!(b.nav().highlighted(0), Some(i));
        assert!(b.visible(0, i), "row {i} is scrolled into view: {:?} in {:?}", b.nav().rows[0][i].rect, b.nav().views[0]);
    }
    b.key(Key::ArrowDown);
    b.run(2);
    assert_eq!(b.nav().highlighted(0), Some(0));
    assert!(b.visible(0, 0), "↓ wraps to the first row, scrolling back up");
}

#[test]
fn the_wheel_scrolls_a_tall_menu() {
    let mut b = open_tall();
    let view = b.nav().views[0];
    b.move_to(view.center());
    for _ in 0..60 {
        if b.visible(0, 59) {
            break;
        }
        b.events.push(Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -60.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
        });
        b.run(2);
    }
    assert!(b.visible(0, 59), "the wheel reaches the last row");
    assert_eq!(b.nav().rows[0].len(), 60, "the menu stays open while scrolling");
}

#[test]
fn resting_on_the_scroll_arrows_scrolls() {
    let mut b = open_tall();
    let view = b.nav().views[0];
    b.move_to(pos2(view.center().x, view.bottom() + ARROW / 2.0));
    for _ in 0..240 {
        if b.visible(0, 59) {
            break;
        }
        b.frame();
    }
    assert!(b.visible(0, 59), "resting on ▼ scrolls to the end");
    let view = b.nav().views[0];
    b.move_to(pos2(view.center().x, view.top() - ARROW / 2.0));
    for _ in 0..240 {
        if b.visible(0, 0) {
            break;
        }
        b.frame();
    }
    assert!(b.visible(0, 0), "resting on ▲ scrolls back to the top");
}

/// A menu with two submenus one above the other, as Image › Mode and Image › Adjustments.
fn two_submenus() -> Bar {
    let sub = |name: &str, n: usize| Node::submenu(name, (0..n).map(|i| item(&format!("{name}.{i}"), &format!("{name} {i}")).into()).collect());
    let menu = Menu::new("Image", vec![sub("Mode", 8), sub("Adjust", 8)]);
    let mut b = Bar::new(MenuBar { menus: vec![menu], documents: Vec::new() }, vec2(800.0, 600.0));
    b.dt = 1.0 / 120.0;
    b
}

/// Open Image › Mode with the pointer resting on it; returns where the pointer is, the Adjust
/// row below it, and the row low in Mode's submenu the pointer heads for.
fn rest_on_mode(b: &mut Bar) -> (Pos2, Rect, Rect) {
    b.click("Image");
    let mode = b.rect("Mode");
    let from = pos2(mode.right() - 40.0, mode.center().y);
    for k in 0..=12 {
        b.move_to(from - vec2(30.0 * (1.0 - k as f32 / 12.0), 0.0));
    }
    b.run(30);
    assert_eq!(open_submenu(b), "Mode", "resting on Mode opens it");
    (from, b.rect("Adjust"), b.rect("Mode 6"))
}

fn open_submenu(b: &Bar) -> &'static str {
    match (b.shown("Mode 0"), b.shown("Adjust 0")) {
        (true, false) => "Mode",
        (false, true) => "Adjust",
        (false, false) => "none",
        (true, true) => "both",
    }
}

/// Move the pointer along `path` (0…1) in `frames` frames; `pause` holds it still for that many
/// frames at that fraction of the way.
fn glide(b: &mut Bar, frames: usize, pause: Option<(f32, usize)>, mut path: impl FnMut(f32) -> Pos2) {
    let mut paused = false;
    for k in 0..=frames {
        let t = k as f32 / frames as f32;
        b.move_to(path(t));
        if let Some((_, n)) = pause.filter(|(at, _)| !paused && t >= *at) {
            paused = true;
            b.run(n);
        }
    }
}

/// A person's path from `a` to `b`: it bows, down first and then across, by `bow`.
fn bowed(a: Pos2, b: Pos2, bow: f32) -> impl Fn(f32) -> Pos2 {
    let c = pos2(a.x + (b.x - a.x) * (0.5 - bow), a.y + (b.y - a.y) * (0.5 + bow));
    move |t| {
        let u = 1.0 - t;
        pos2(u * u * a.x + 2.0 * u * t * c.x + t * t * b.x, u * u * a.y + 2.0 * u * t * c.y + t * t * b.y)
    }
}

/// Heading from Mode for a row low in its submenu, the pointer crosses the Adjust row. On a
/// diagonal, a bowed path or with a short pause on the way, Mode's submenu stays open, Adjust's
/// doesn't open, and the crossed row isn't highlighted.
#[test]
fn a_submenu_stays_open_while_the_pointer_heads_for_it() {
    let paths = [("straight", 36, None, 0.0), ("hesitate", 36, Some((0.3, 18)), 0.0), ("bowed", 42, None, 0.2), ("slow and bowed", 96, None, 0.2)];
    let mut wrong = Vec::new();
    for (name, frames, pause, bow) in paths {
        let mut b = two_submenus();
        let (from, adjust, target) = rest_on_mode(&mut b);
        let to = pos2(target.left() + 40.0, target.center().y);
        let path = bowed(from, to, bow);
        let mut crossed = false;
        glide(&mut b, frames, pause, |t| {
            let p = path(t);
            crossed |= adjust.contains(p);
            p
        });
        assert!(crossed, "{name}: the path crosses the Adjust row");
        let open = open_submenu(&b);
        if open != "Mode" {
            wrong.push(format!("{name}: arrived with {open} open, not Mode"));
        }
        if let Some(row) = b.nav().highlighted(0).filter(|&row| row != 0) {
            wrong.push(format!("{name}: row {row} is highlighted, not Mode"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// Resting on the row crossed, or moving away from the submenu onto it, still opens its submenu,
/// and the highlight moves to it.
#[test]
fn resting_on_another_row_or_moving_away_switches_submenus() {
    let mut b = two_submenus();
    let (from, adjust, _) = rest_on_mode(&mut b);
    let stop = pos2(from.x + 15.0, adjust.center().y);
    glide(&mut b, 12, None, |t| from + (stop - from) * t);
    assert_eq!(open_submenu(&b), "Mode", "just arrived on Adjust, still heading for Mode's submenu");
    b.run(48);
    assert_eq!(open_submenu(&b), "Adjust", "resting on Adjust opens it");
    assert_eq!(b.nav().highlighted(0), Some(1));

    for dx in [0.0, -60.0] {
        let mut b = two_submenus();
        let (from, adjust, _) = rest_on_mode(&mut b);
        let to = pos2(from.x + dx, adjust.center().y);
        glide(&mut b, 10, None, |t| from + (to - from) * t);
        b.run(30);
        assert_eq!(open_submenu(&b), "Adjust", "moving away ({dx}) from Mode's submenu opens Adjust's");
        assert_eq!(b.nav().highlighted(0), Some(1));
    }
}

#[test]
fn heading_is_the_triangle_to_the_submenus_near_edge() {
    let sub = Rect::from_min_max(pos2(200.0, 100.0), pos2(400.0, 400.0));
    let from = pos2(150.0, 110.0);
    assert!(heading(from, from, sub));
    assert!(heading(from, pos2(170.0, 160.0), sub), "down and right, toward the submenu");
    assert!(heading(from, pos2(190.0, 105.0), sub), "straight across");
    assert!(!heading(from, pos2(150.0, 160.0), sub), "straight down");
    assert!(!heading(from, pos2(120.0, 140.0), sub), "away to the left");
    assert!(!heading(from, pos2(170.0, 60.0), sub), "up past the submenu's top");
    // A submenu that opened to the left.
    let left = Rect::from_min_max(pos2(0.0, 100.0), pos2(100.0, 400.0));
    assert!(heading(pos2(150.0, 110.0), pos2(130.0, 160.0), left));
    assert!(!heading(pos2(150.0, 110.0), pos2(170.0, 160.0), left));
    assert!(!heading(from, from, Rect::NOTHING));
}

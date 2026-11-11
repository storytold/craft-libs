//! The driver between a platform menu and the app: the gate, display-only shortcuts, key
//! equivalents, clipboard keys and closing an open menu. A fake backend stands
//! in for the platform, so the trials run everywhere.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_menu::{Backend, Builtin, ClipboardKeys, CommandRef, Dispatch, Event, Gate, LayoutOptions, MenuBar, NativeMenu, mac_layout, structure_key};
use craft_ui::contract::{Props as _, Show as _};
use craft_ui::testing::focus::key_release;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::click;
use craft_ui::testing::{Frames, trial};
use craft_ui::text::SelectableText;
use egui::{Key, Modifiers};

use crate::sample::{chord, cmd, held, sample, typing};

fn modal() -> Gate {
    Gate::default().modal(true)
}

type Shared<T> = Rc<RefCell<Vec<T>>>;

#[derive(Default)]
struct Fake {
    synced: Rc<RefCell<Vec<MenuBar>>>,
    events: Rc<RefCell<Vec<Event>>>,
    tracking: Rc<Cell<bool>>,
    cancels: Rc<Cell<usize>>,
    /// What the backend's menu event handler would call.
    dispatch: Rc<RefCell<Option<Dispatch>>>,
}

impl Backend for Fake {
    fn sync(&mut self, bar: &MenuBar) {
        self.synced.borrow_mut().push(bar.clone());
    }
    fn drain(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events.borrow_mut())
    }
    fn tracking(&self) -> bool {
        self.tracking.get()
    }
    fn cancel_tracking(&mut self) {
        self.cancels.set(self.cancels.get() + 1);
        self.tracking.set(false);
    }
    fn attach(&mut self, dispatch: Dispatch) {
        *self.dispatch.borrow_mut() = Some(dispatch);
    }
}

/// A driver over a fake backend, with what it synced, its pending events and its dispatch.
fn driver() -> (NativeMenu, Shared<MenuBar>, Shared<Event>, Dispatch) {
    let fake = Fake::default();
    let (synced, events, dispatch) = (fake.synced.clone(), fake.events.clone(), fake.dispatch.clone());
    let menu = NativeMenu::new(Box::new(fake));
    let dispatch = dispatch.borrow().clone().expect("NativeMenu::new attaches the dispatch");
    (menu, synced, events, dispatch)
}

fn take_clicks(menu: &mut NativeMenu) -> Vec<CommandRef> {
    let ctx = egui::Context::default();
    let mut clicks = Vec::new();
    let mut out = ctx.run_ui(Default::default(), |ui| clicks = menu.take_clicks(ui.ctx()));
    out.textures_delta.clear();
    clicks
}

/// What egui sees of `events` in a frame of `frames` after the menu has been polled: the window host's input
/// filter has dropped the closing click and added the key equivalents, and the runtime has taken the keys it owns.
/// The menu is polled again after the frame, as in an app, so that a close the filter asked for reaches the backend.
fn through(menu: &mut NativeMenu, frames: &mut Frames, events: Vec<egui::Event>) -> Vec<egui::Event> {
    through_in(menu, frames, events, |_| {})
}

/// [`through`] with `show` run in the frame.
fn through_in(menu: &mut NativeMenu, frames: &mut Frames, events: Vec<egui::Event>, mut show: impl FnMut(&mut egui::Ui)) -> Vec<egui::Event> {
    frames.ctx.set_os(egui::os::OperatingSystem::Mac);
    menu.poll(&frames.ctx);
    let mut seen = Vec::new();
    frames.frame(events, |ui| {
        seen = ui.ctx().input(|input| input.events.clone());
        show(ui);
    });
    menu.poll(&frames.ctx);
    seen
}

/// Selectable text that holds key focus with all of it selected: a clipboard key equivalent copies it.
fn selected_text(frames: &mut Frames) -> impl FnMut(&mut egui::Ui) + use<> {
    let rect = Rc::new(Cell::new(egui::Rect::NOTHING));
    let shown = rect.clone();
    let mut show = move |ui: &mut egui::Ui| shown.set(SelectableText::new("Note", "read me").id_salt("note").show(ui).response.rect);
    frames.idle(3, &mut show);
    click(frames, rect.get().center(), &mut show);
    frames.frame(vec![key_press(Key::A, Modifiers::COMMAND)], &mut show);
    frames.frame(vec![], &mut show);
    show
}

/// ⌘ on a Mac keyboard, with ⇧ when `shift`: the modifiers egui reports for a key equivalent.
fn mac_cmd(shift: bool) -> Modifiers {
    Modifiers { shift, mac_cmd: true, command: true, ..Modifiers::NONE }
}

fn key(key: Key, pressed: bool, modifiers: Modifiers) -> egui::Event {
    if pressed { key_press(key, modifiers) } else { key_release(key, modifiers) }
}

fn press() -> egui::Event {
    egui::Event::PointerButton { pos: egui::pos2(300.0, 200.0), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }
}

/// The gate is checked when the app takes the clicks, not only when the menu was open: a click
/// that arrived before a modal dialog opened doesn't run behind it. Built-ins still do.
#[trial]
fn a_click_waiting_when_a_dialog_opens_is_dropped() {
    let (mut menu, _, events, _) = driver();
    events.borrow_mut().extend([Event::Click(cmd("file.save")), Event::Click(Builtin::HideOthers.command())]);
    menu.poll(&egui::Context::default());
    menu.set_gate(modal());
    assert_eq!(take_clicks(&mut menu), [Builtin::HideOthers.command()]);

    events.borrow_mut().push(Event::Click(cmd("file.save")));
    menu.poll(&egui::Context::default());
    menu.set_gate(Gate::default());
    assert_eq!(take_clicks(&mut menu), [cmd("file.save")]);
}

/// While a dialog is open the app's items are greyed and the built-ins aren't. Opening and
/// closing it re-syncs the last menus in place, without laying them out again. While a text field
/// has the keys nothing is greyed.
#[trial]
fn a_modal_gate_greys_the_apps_items() {
    let (mut menu, synced, _, _) = driver();
    let opts = LayoutOptions::new("Sample");
    let builds = Cell::new(0);
    let build = || {
        builds.set(builds.get() + 1);
        mac_layout(&sample(false, true), &opts)
    };
    menu.sync_with(1, build);
    menu.set_gate(modal());
    menu.sync_with(1, build);
    menu.sync_with(1, build);
    menu.set_gate(Gate::default());
    menu.sync_with(1, build);
    menu.set_gate(typing(true));
    menu.sync_with(1, build);
    assert_eq!((builds.get(), synced.borrow().len()), (1, 4), "one layout; synced open, greyed, open, typing");

    let synced = synced.borrow();
    let enabled = |bar: &MenuBar, c: &CommandRef| bar.find(c).unwrap().enabled;
    let greyed = &synced[1];
    assert!(!enabled(greyed, &cmd("file.new")) && !enabled(greyed, &cmd("help.about")));
    // AppKit sends a disabled item's key equivalent nowhere: ⌘Z must reach the dialog's fields.
    assert!(enabled(greyed, &cmd("edit.undo")), "the text-editing keys stay");
    assert!(enabled(greyed, &Builtin::Hide.command()) && enabled(greyed, &Builtin::HideOthers.command()));
    assert!(enabled(&synced[2], &cmd("file.new")));
    assert_eq!(structure_key(greyed), structure_key(&synced[0]), "greying is an update in place");
    assert_eq!(synced[2], synced[0], "an open gate greys nothing");
    assert_eq!(synced[3], synced[0], "text editing greys nothing");
}

/// AppKit takes a key equivalent's key-down but winit still delivers the key-up: the menu adds
/// only the press, before a key-up already queued, so the shortcut runs once. (⌘V as a key, as
/// an app taking the raw clipboard keys gets it.) The press carries the modifiers that were held.
#[trial]
fn key_equivalents_become_one_key_press() {
    let mut frames = Frames::new();
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut menu = NativeMenu::new(Box::new(Fake { events: events.clone(), ..Default::default() })).clipboard_keys(ClipboardKeys::Raw);
    let paste = chord("Cmd+V");
    let release = key(Key::V, false, Modifiers::COMMAND);
    let pressed = key(Key::V, true, mac_cmd(false));

    events.borrow_mut().push(Event::Key(paste));
    let seen = through(&mut menu, &mut frames, vec![egui::Event::ModifiersChanged(Modifiers::COMMAND), release.clone()]);
    assert_eq!(seen, [egui::Event::ModifiersChanged(Modifiers::COMMAND), pressed.clone(), release]);

    events.borrow_mut().push(Event::Key(paste));
    let seen = through(&mut menu, &mut frames, vec![]);
    assert_eq!(seen, [pressed], "no made-up release: the real one comes in a later frame");

    // ⌘⇧Z and ⌃⌘M: ⌘ and ⌃ stay distinct.
    events.borrow_mut().extend([Event::Key(chord("Shift+Cmd+Z")), Event::Key(chord("Ctrl+Cmd+M"))]);
    let seen = through(&mut menu, &mut frames, vec![]);
    let ctrl_cmd = Modifiers { ctrl: true, ..mac_cmd(false) };
    assert_eq!(seen, [key(Key::Z, true, mac_cmd(true)), key(Key::M, true, ctrl_cmd)]);
}

/// A press while a menu is open closes it and is kept from egui, so it starts no canvas gesture;
/// pointer moves stay. With no menu open the press reaches egui (the menu is still asked to close,
/// in case one is on screen).
#[trial]
fn outside_press_dismisses_without_starting_a_canvas_gesture() {
    let mut frames = Frames::new();
    let tracking = Rc::new(Cell::new(true));
    let cancels = Rc::new(Cell::new(0));
    let mut menu = NativeMenu::new(Box::new(Fake { tracking: tracking.clone(), cancels: cancels.clone(), ..Default::default() }));
    let moved = egui::Event::PointerMoved(egui::pos2(300.0, 200.0));
    let seen = through(&mut menu, &mut frames, vec![moved.clone(), press()]);
    assert_eq!(seen, [moved]);
    assert_eq!((cancels.get(), tracking.get()), (1, false));

    let seen = through(&mut menu, &mut frames, vec![press()]);
    assert_eq!(seen, [press()], "ordinary canvas presses still reach egui");
    assert_eq!(cancels.get(), 2);

    through(&mut menu, &mut frames, vec![egui::Event::PointerMoved(egui::pos2(1.0, 1.0))]);
    assert_eq!(cancels.get(), 2, "a pointer move closes nothing");
}

/// AppKit takes ⌘C, ⌘X and ⌘V for the Edit menu, so egui-winit never makes its Copy, Cut and
/// Paste events: the driver makes them, unless the app takes the raw keys.
#[trial]
fn clipboard_key_equivalents_reach_text_fields() {
    let mut frames = Frames::new();
    let events = Rc::new(RefCell::new(Vec::new()));
    let mut menu = NativeMenu::new(Box::new(Fake { events: events.clone(), ..Default::default() }));
    let equivalent = |s: &str| Event::Key(chord(s));
    let mut show = selected_text(&mut frames);
    events.borrow_mut().extend([equivalent("Cmd+C"), equivalent("Cmd+Shift+V")]);
    let seen = through_in(&mut menu, &mut frames, vec![], &mut show);
    assert_eq!(frames.copied(), ["read me"], "⌘C copies the focused field's selection");
    assert_eq!(seen, [key(Key::V, true, mac_cmd(true))], "⌘C is the runtime's; ⌘⇧V is an app shortcut and reaches egui once");

    events.borrow_mut().push(equivalent("Cmd+V"));
    let seen = through_in(&mut menu, &mut frames, vec![], &mut show);
    assert!(seen.is_empty(), "the paste comes from eframe, with the clipboard's text");
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| {
        menu.take_clicks(ui.ctx());
    });
    out.textures_delta.clear();
    let commands = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|o| o.commands).unwrap_or_default();
    assert!(commands.contains(&egui::ViewportCommand::RequestPaste));

    let mut menu = NativeMenu::new(Box::new(Fake { events: events.clone(), ..Default::default() })).clipboard_keys(ClipboardKeys::Raw);
    events.borrow_mut().extend([equivalent("Cmd+C"), equivalent("Cmd+V")]);
    let mut frames = Frames::new();
    let seen = through(&mut menu, &mut frames, vec![]);
    assert_eq!(seen, [key(Key::C, true, mac_cmd(false)), key(Key::V, true, mac_cmd(false))]);
}

/// The layout marks the shortcuts it gives (the app's key handling doesn't bind them), and a key
/// equivalent on such an item comes back as a click.
#[trial]
fn shortcuts_the_layout_assigns_come_back_as_clicks() {
    let (_, _, _, dispatch) = driver();
    let l = mac_layout(&sample(false, true), &LayoutOptions::new("Sample"));
    let assigned = |id: &str| l.bar.find(&cmd(id)).unwrap().layout_shortcut;
    assert!(assigned("file.exit") && assigned("prefs.general") && assigned(Builtin::Hide.id()));
    assert!(!assigned("edit.undo") && !assigned("prefs.keys"));
    let mut bar = sample(false, true);
    bar.menus[0].children.push(craft_menu::Item::new("file.quit", "Quit").shortcut(chord("Cmd+Q")).role(craft_menu::ItemRole::Quit).into());
    bar.tag_item(&cmd("file.exit"), craft_menu::ItemRole::About);
    assert!(!mac_layout(&bar, &LayoutOptions::new("Sample")).bar.find(&cmd("file.quit")).unwrap().layout_shortcut, "the app's own ⌘Q");

    let cmd_q = chord("Cmd+Q");
    let cmd_held = held([true, false, false, false]);
    assert_eq!(dispatch.chosen(&cmd("file.exit"), Some(&cmd_q), true, cmd_held), Event::Click("file.exit".into()));
    assert_eq!(dispatch.chosen(&cmd("file.quit"), Some(&cmd_q), false, cmd_held), Event::Key(cmd_q));
    // ⇧ held on a ⌘R item: the key press that was made.
    assert_eq!(
        dispatch.chosen(&cmd("view.reload"), Some(&chord("Cmd+R")), false, held([true, false, false, true])),
        Event::Key(chord("Cmd+Shift+R"))
    );
    assert_eq!(dispatch.chosen(&cmd("file.quit"), Some(&cmd_q), false, None), Event::Click("file.quit".into()), "a mouse click");
    assert_eq!(
        dispatch.chosen(&Builtin::Minimize.command(), Some(&chord("Ctrl+Cmd+M")), false, held([true, true, false, false])),
        Event::Click(Builtin::Minimize.command())
    );
    // ↩ inside the open menu: on a ⌘ item without ⌘ held, and on a bare tool key.
    let enter = held([false; 4]);
    assert_eq!(dispatch.chosen(&cmd("file.quit"), Some(&cmd_q), false, enter), Event::Click("file.quit".into()));
    assert_eq!(dispatch.chosen(&cmd("tool.brush"), Some(&chord("B")), false, enter), Event::Click("tool.brush".into()));
}

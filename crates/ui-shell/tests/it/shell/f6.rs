//! F6 through the app shell: the pane move visits the shell's slots in their order, then the body, then wraps, with the
//! slots replaying their last run.

use craft_ui::buttons::Button;
use craft_ui::contract::Show as _;
use craft_ui::runtime::env::WindowKey;
use craft_ui::shell::{AppShell, Slot};
use craft_ui::testing::focus::egui_focused;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Frames, trial};
use egui::{Id, Key, Modifiers, Ui};
use std::cell::RefCell;
use std::rc::Rc;

/// The key the trial gives the body pane beside the slot keys.
const BODY: &str = "body";

/// What the shell showed: the id of the one button in each pane, as the pane's last run showed it, and the slot keys in
/// the order the shell laid them out in its last pass.
#[derive(Default)]
struct Seen {
    buttons: Vec<(&'static str, Id)>,
    order: Vec<&'static str>,
}

impl Seen {
    /// The pane whose button has egui focus.
    fn pane_of(&self, focused: Option<Id>) -> Option<&'static str> {
        let focused = focused?;
        self.buttons.iter().find(|(_, id)| *id == focused).map(|(key, _)| *key)
    }
}

/// A shell with a versioned slot at every edge, added out of the shell's order (end before start, a bottom slot before
/// a top one), each holding one button, and a body holding one button.
fn shell(seen: &Rc<RefCell<Seen>>) -> impl FnMut(&mut Ui) {
    let seen = Rc::clone(seen);
    move |ui: &mut Ui| {
        let buttons = RefCell::new(Vec::new());
        let button = |ui: &mut Ui, key: &'static str| {
            let id = Button::new(key).show(ui).response.id;
            buttons.borrow_mut().push((key, id));
        };
        let shown = AppShell::new(WindowKey::main())
            .end(Slot::new("inspector"), |_| Some(0), |ui, _, _: &mut ()| button(ui, "inspector"))
            .status(true, |_| Some(0), |ui, _, _| button(ui, "status"))
            .start(Slot::new("tools"), |_| Some(0), |ui, _, _| button(ui, "tools"))
            .options_bar(true, |_| Some(0), |ui, _, _| button(ui, "options"))
            .body(|ui, _, _| button(ui, BODY))
            .show(ui, &mut ());
        let mut seen = seen.borrow_mut();
        // A replayed slot shows the button of its last run, with that run's id.
        for (key, id) in buttons.into_inner() {
            seen.buttons.retain(|(kept, _)| *kept != key);
            seen.buttons.push((key, id));
        }
        seen.order = shown.regions.slots.iter().map(|(key, _)| *key).collect();
    }
}

/// A press of F6 with `mods` and the frame after it, in which the move's focus lands: the pane that has the focus then.
fn f6(frames: &mut Frames, show: &mut impl FnMut(&mut Ui), seen: &RefCell<Seen>, mods: Modifiers) -> Option<&'static str> {
    frames.frame(vec![key_press(Key::F6, mods)], &mut *show);
    frames.frame(Vec::new(), &mut *show);
    seen.borrow().pane_of(egui_focused(&frames.ctx))
}

/// From nothing focused, F6 visits every slot in the shell's layout order (the F6 order `AppShell` documents), then
/// the body, then wraps to the first slot; Shift+F6 walks the same cycle backwards.
#[trial]
fn f6_cycles_the_slots_the_panes_and_the_body_and_wraps() {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let mut show = shell(&seen);
    let mut frames = Frames::new();
    frames.idle(2, &mut show);
    let mut cycle: Vec<&'static str> = seen.borrow().order.clone();
    assert_eq!(cycle.len(), 4, "every slot shows: {cycle:?}");
    cycle.push(BODY);
    assert_eq!(seen.borrow().pane_of(egui_focused(&frames.ctx)), None, "nothing has the focus before F6");

    let forward: Vec<Option<&'static str>> = (0..cycle.len()).map(|_| f6(&mut frames, &mut show, &seen, Modifiers::NONE)).collect();
    assert_eq!(forward, cycle.iter().copied().map(Some).collect::<Vec<_>>(), "F6 visits the slots in the shell's order, then the body");
    assert_eq!(f6(&mut frames, &mut show, &seen, Modifiers::NONE), Some(cycle[0]), "F6 past the body wraps to the first slot");

    let backward: Vec<Option<&'static str>> = (0..cycle.len()).map(|_| f6(&mut frames, &mut show, &seen, Modifiers::SHIFT)).collect();
    // From the first slot Shift+F6 wraps to the body and walks back to the first slot.
    let reversed: Vec<Option<&'static str>> = cycle.iter().rev().copied().map(Some).collect();
    assert_eq!(backward, reversed, "Shift+F6 walks the cycle backwards, wrapping from the first slot to the body");
}

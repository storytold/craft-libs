//! Synthetic input through the window host: held presses and keys, modifiers, the wheel, Caps Lock, hover rest and
//! document coordinates, as the passes see them.

use std::cell::RefCell;
use std::rc::Rc;

use craft_automation::synthetic::{NO_DOCUMENT, input_method};
use craft_ui::runtime::windows::WindowHost;
use craft_ui::testing::{Fixture, trial};
use egui::{Event, Key, Pos2, ViewportId, pos2};
use serde_json::{Value, json};

/// What one pass saw of the input.
#[derive(Clone, Debug, Default)]
struct Seen {
    primary_down: bool,
    pos: Option<Pos2>,
    alt: bool,
    shift: bool,
    space_down: bool,
    events: Vec<Event>,
    injected: bool,
}

struct Rig {
    fx: Fixture,
    passes: Rc<RefCell<Vec<Seen>>>,
}

/// Document points are screen points doubled, as a canvas at 50 % shows them.
fn map(at: Pos2) -> Option<Pos2> {
    Some(pos2(at.x / 2.0, at.y / 2.0))
}

impl Rig {
    fn new() -> Self {
        let passes = Rc::new(RefCell::new(Vec::new()));
        let seen = passes.clone();
        let fx = Fixture::new(move |ui| {
            let injected = WindowHost::of(ui.ctx()).injected_pass();
            seen.borrow_mut().push(ui.input(|i| Seen {
                primary_down: i.pointer.primary_down(),
                pos: i.pointer.latest_pos(),
                alt: i.modifiers.alt,
                shift: i.modifiers.shift,
                space_down: i.keys_down.contains(&Key::Space),
                events: i.events.clone(),
                injected,
            }));
        });
        Self { fx, passes }
    }

    fn host(&self) -> WindowHost {
        WindowHost::of(self.fx.ctx())
    }

    /// Sends `method` and runs frames until the host has processed the input, then one more.
    fn send(&mut self, method: &str, params: Value) -> &mut Self {
        let host = self.host();
        input_method(method, &params, &host, ViewportId::ROOT, &map).expect("an input method").expect("valid params");
        for _ in 0..32 {
            self.fx.frame(Vec::new());
            if host.injected_idle() {
                break;
            }
        }
        assert!(host.injected_idle(), "{method} was processed");
        self.fx.frame(Vec::new());
        self
    }

    /// What the latest pass saw.
    fn now(&self) -> Seen {
        self.passes.borrow().last().cloned().expect("a pass ran")
    }

    /// Every pass's events since the last call.
    fn take(&self) -> Vec<Seen> {
        std::mem::take(&mut *self.passes.borrow_mut())
    }
}

/// A press held with `hold: true` stays down across passes; moves while held drag; `ui.release` lets it go. The
/// points are document points the application maps.
#[trial]
fn a_held_press_stays_down_until_its_release() {
    let mut rig = Rig::new();
    rig.send("ui.click", json!({"x": 200, "y": 100, "coordinates": "document", "hold": true}));
    rig.fx.idle(3);
    assert!(rig.now().primary_down, "the press is held across passes");
    assert_eq!(rig.now().pos, Some(pos2(100.0, 50.0)), "the document point is mapped");
    rig.send("ui.move", json!({"x": 300, "y": 100, "coordinates": "document"}));
    assert!(rig.now().primary_down && rig.now().pos == Some(pos2(150.0, 50.0)), "a move while held drags");
    rig.send("ui.release", json!({}));
    assert!(!rig.now().primary_down, "the release lets it go");
    rig.send("ui.click", json!({"x": 10, "y": 10}));
    assert!(!rig.now().primary_down, "a click without hold releases");
}

/// A move's modifiers stay held after it (a hover with modifiers); a click that holds nothing lets its own go.
#[trial]
fn a_move_keeps_its_modifiers_and_a_click_lets_them_go() {
    let mut rig = Rig::new();
    rig.send("ui.move", json!({"x": 40, "y": 40, "alt": true}));
    rig.fx.idle(3);
    assert!(rig.now().alt, "the move's modifier stays held");
    rig.send("ui.click", json!({"x": 40, "y": 40, "shift": true}));
    let seen = rig.now();
    assert!(!seen.alt && !seen.shift, "the click names its own modifiers and lets them go");
    let presses: Vec<_> = rig
        .take()
        .into_iter()
        .flat_map(|pass| pass.events)
        .filter_map(|event| match event {
            Event::PointerButton { pressed: true, modifiers, .. } => Some(modifiers),
            _ => None,
        })
        .collect();
    assert!(presses.len() == 1 && presses[0].shift && !presses[0].alt, "the press carries the click's modifiers");
}

/// A key held with `hold: true` stays down across passes until `ui.release {key}`.
#[trial]
fn a_held_key_stays_down_until_its_release() {
    let mut rig = Rig::new();
    rig.send("ui.key", json!({"key": "Space", "hold": true}));
    rig.fx.idle(3);
    assert!(rig.now().space_down);
    rig.send("ui.release", json!({"key": "Space"}));
    assert!(!rig.now().space_down);
    rig.send("ui.key", json!({"key": "Space"}));
    assert!(!rig.now().space_down, "a key without hold is released");
}

/// A wheel turn reaches the pass at its point with its delta and modifiers.
#[trial]
fn a_wheel_turn_carries_its_modifiers() {
    let mut rig = Rig::new();
    rig.send("ui.wheel", json!({"x": 30, "y": 20, "dy": 40, "command": true}));
    let passes = rig.take();
    let turn = passes.iter().find_map(|pass| {
        pass.events.iter().find_map(|event| match event {
            Event::MouseWheel { delta, modifiers, .. } => Some((pass.pos, *delta, *modifiers)),
            _ => None,
        })
    });
    let (pos, delta, modifiers) = turn.expect("a pass saw the turn");
    assert_eq!(pos, Some(pos2(30.0, 20.0)));
    assert!(delta.y > 0.0 && delta.x == 0.0 && modifiers.command);
}

/// The injected Caps Lock state wins over the platform's until it is cleared.
#[trial]
fn injected_caps_lock_holds_until_cleared() {
    let mut rig = Rig::new();
    let platform = craft_ui::runtime::lock_keys::caps_lock(rig.fx.ctx());
    rig.send("ui.capsLock", json!({"on": !platform}));
    assert_eq!(craft_ui::runtime::lock_keys::caps_lock(rig.fx.ctx()), !platform);
    assert_eq!(rig.host().injected_caps_lock(), Some(!platform));
    rig.send("ui.capsLock", json!({"on": null}));
    assert_eq!(rig.host().injected_caps_lock(), None);
}

/// A move with `rest` counts as processed only once the pointer has rested that long.
#[trial]
fn a_hover_rest_holds_the_reply_until_it_has_passed() {
    let mut rig = Rig::new();
    let host = rig.host();
    input_method("ui.move", &json!({"x": 50, "y": 50, "rest": 0.5}), &host, ViewportId::ROOT, &map).expect("a method").expect("valid");
    rig.fx.idle(3);
    assert!(!host.injected_idle(), "the rest has not passed");
    rig.fx.advance(1.0).frame(Vec::new());
    assert!(host.injected_idle(), "the rest has passed");
}

/// Only the passes that carry an injected step act for automation: one per step, none after.
#[trial]
fn injected_passes_are_the_passes_that_carry_a_step() {
    let mut rig = Rig::new();
    rig.take();
    rig.send("ui.click", json!({"x": 20, "y": 20}));
    rig.fx.idle(2);
    let flags: Vec<bool> = rig.take().into_iter().map(|pass| pass.injected).collect();
    assert_eq!(flags.iter().filter(|carried| **carried).count(), 3, "move, press and release: {flags:?}");
    assert!(!flags.last().copied().unwrap_or(true), "idle passes act for no one");
}

/// A document point with no canvas to map it is refused before anything is queued; screen points need no map.
#[trial]
fn document_points_need_a_canvas() {
    let rig = Rig::new();
    let host = rig.host();
    let none = |_: Pos2| None;
    let refused = input_method("ui.move", &json!({"x": 1, "y": 1, "coordinates": "document"}), &host, ViewportId::ROOT, &none);
    assert_eq!(refused, Some(Err(NO_DOCUMENT.to_owned())));
    assert!(host.injected_idle());
    let screen = input_method("ui.move", &json!({"x": 1, "y": 1}), &host, ViewportId::ROOT, &none);
    assert_eq!(screen, Some(Ok(())));
}

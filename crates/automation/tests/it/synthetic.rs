//! Synthetic input through the window host: held presses and keys, modifiers, the wheel, Caps Lock, hover rest,
//! document coordinates and input-method compositions, as the passes see them, and the params refused before
//! anything is queued.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_automation::synthetic::{At, MAX_CLICKS, MAX_SCREEN_POINT, NO_DOCUMENT, NOT_FINITE, input_method};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::runtime::egui::injected::hook_without_pass;
use craft_ui::runtime::windows::WindowHost;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Event, ImeEvent, Key, Pos2, Rect, ViewportId, pos2};
use serde_json::{Value, json};

/// What one pass saw of the input.
#[derive(Clone, Debug, Default)]
struct Seen {
    primary_down: bool,
    pos: Option<Pos2>,
    alt: bool,
    shift: bool,
    space_down: bool,
    any_key_down: bool,
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
                any_key_down: !i.keys_down.is_empty(),
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

    /// A rig that shows one plain text field instead of recording the passes, with the field's text and where it
    /// lies.
    fn with_field() -> (Self, Rc<RefCell<String>>, Rc<Cell<Rect>>) {
        let (text, rect) = (Rc::new(RefCell::new(String::new())), Rc::new(Cell::new(Rect::NOTHING)));
        let (shown_text, shown_rect) = (text.clone(), rect.clone());
        let fx = Fixture::new(move |ui| {
            shown_rect.set(TextField::new("Name", Bind::bound(&mut *shown_text.borrow_mut())).show(ui).response.rect);
        });
        (Self { fx, passes: Rc::default() }, text, rect)
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
    rig.send("ui.wheel", json!({"x": 30, "y": 20, "dy": 40, "alt": true}));
    let passes = rig.take();
    let turn = passes.iter().find_map(|pass| {
        pass.events.iter().find_map(|event| match event {
            Event::MouseWheel { delta, modifiers, .. } => Some((pass.pos, *delta, *modifiers)),
            _ => None,
        })
    });
    let (pos, delta, modifiers) = turn.expect("a pass saw the turn");
    assert_eq!(pos, Some(pos2(30.0, 20.0)));
    assert!(delta.y > 0.0 && delta.x == 0.0 && modifiers.alt);
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

/// A step handed to the input whose pass has not run is withdrawn when the focus moved, and when the application
/// withdraws the input because its document changed: the integration's preserved copy leaves the input, no key
/// stays down, and the waiters fail with `TARGET_CHANGED`.
#[trial]
fn queued_input_is_withdrawn_when_its_target_changes() {
    use craft_automation::control::{ControlChannel, ControlRequest, Reply};
    use craft_automation::synthetic::TARGET_CHANGED;
    use craft_ui::runtime::egui::injected::move_focus_without_pass;

    let waiting = |channel: &mut ControlChannel| {
        let (req, reply) = ControlRequest::new("ui.key", json!({"key": "A"}));
        channel.settle(req, Reply::AfterInput, None, 0.0);
        reply
    };
    let (_transport, requests) = std::sync::mpsc::channel();
    let mut channel = ControlChannel::new(requests);
    let mut rig = Rig::new();
    let (host, ctx) = (rig.host(), rig.fx.ctx().clone());

    input_method("ui.key", &json!({"key": "A"}), &host, ViewportId::ROOT, &map).expect("a method").expect("valid");
    let reply = waiting(&mut channel);
    let handed = hook_without_pass(&ctx, ViewportId::ROOT);
    assert!(!handed.is_empty(), "the press was handed to the input");
    move_focus_without_pass(&ctx, ViewportId::ROOT, egui::Id::unique("elsewhere"));
    rig.take();
    rig.fx.frame(handed);
    assert!(rig.take().iter().flat_map(|pass| &pass.events).all(|event| !matches!(event, Event::Key { .. })), "the preserved press is removed");
    channel.poll_input(&host);
    let answer = reply.try_recv().expect("the waiter is answered");
    assert!(answer["error"].as_str().is_some_and(|error| error.contains(TARGET_CHANGED)), "{answer}");

    input_method("ui.key", &json!({"key": "A"}), &host, ViewportId::ROOT, &map).expect("a method").expect("valid");
    let reply = waiting(&mut channel);
    let handed = hook_without_pass(&ctx, ViewportId::ROOT);
    assert!(!handed.is_empty(), "the press was handed to the input");
    host.withdraw_injected(ViewportId::ROOT);
    rig.take();
    rig.fx.frame(handed);
    let passes = rig.take();
    assert!(passes.iter().flat_map(|pass| &pass.events).all(|event| !matches!(event, Event::Key { .. })), "the preserved press is removed");
    assert!(passes.iter().all(|pass| !pass.any_key_down), "no key stays down");
    channel.poll_input(&host);
    let answer = reply.try_recv().expect("the waiter is answered");
    assert!(answer["error"].as_str().is_some_and(|error| error.contains(TARGET_CHANGED)), "{answer}");
    assert!(host.injected_idle());
}

/// An unknown field, a wrong type, an unknown name, too many clicks or a point too far out refuses the whole request
/// before any of it is queued: no half-queued press or held key is left behind.
#[trial]
fn bad_input_params_are_refused_before_anything_is_queued() {
    let mut rig = Rig::new();
    rig.fx.settle();
    let (host, ctx) = (rig.host(), rig.fx.ctx().clone());
    let far = MAX_SCREEN_POINT * 2.0;
    let refused = [
        ("ui.click", json!({"x": 10, "hold": true, "nudge": 1})),
        ("ui.click", json!({"x": "10"})),
        ("ui.click", json!({"x": 10, "button": "thumb"})),
        ("ui.click", json!({"x": 10, "hold": true, "count": MAX_CLICKS + 1})),
        ("ui.click", json!({"x": far, "hold": true})),
        ("ui.move", json!({"x": 10, "nudge": 1})),
        ("ui.move", json!({"x": 10, "rest": "long"})),
        ("ui.move", json!({"x": 10, "coordinates": "page"})),
        ("ui.move", json!({"y": -far})),
        ("ui.key", json!({"key": "A", "hold": true, "nudge": 1})),
        ("ui.key", json!({"key": 5})),
        ("ui.key", json!({"key": "NoSuchKey", "hold": true})),
        ("ui.ime", json!({"preedit": "ab", "cursor": [1, 9]})),
    ];
    for (method, params) in refused {
        let answer = input_method(method, &params, &host, ViewportId::ROOT, &map);
        assert!(matches!(answer, Some(Err(_))), "{method} {params} is refused: {answer:?}");
        assert!(host.injected_idle(), "{method} {params} left nothing waiting");
        assert!(hook_without_pass(&ctx, ViewportId::ROOT).is_empty(), "{method} {params} queued nothing");
    }
}

/// `ui.ime` queues behind the input before it: typed text, then the composition with its converted segment, then
/// its commit reach the focused field in that order, one step each, and the field ends with the typed and the
/// committed text. A request with both `preedit` and `commit` is refused and queues nothing.
#[trial]
fn ui_ime_reaches_a_focused_text_field_as_a_preedit_then_a_commit_in_order() {
    let (mut rig, text, rect) = Rig::with_field();
    rig.fx.settle();
    let at = rect.get().center();
    rig.send("ui.click", json!({"x": at.x, "y": at.y}));
    assert!(rig.fx.focused().is_some(), "the click focused the field");

    let (host, ctx) = (rig.host(), rig.fx.ctx().clone());
    let queued = [("ui.type", json!({"text": "ab"})), ("ui.ime", json!({"preedit": "kzk", "cursor": [1, 2]})), ("ui.ime", json!({"commit": "漢"}))];
    for (method, params) in queued {
        input_method(method, &params, &host, ViewportId::ROOT, &map).expect("an input method").expect("valid params");
    }
    // Each step is handed to the input, then its own pass runs with it; the next hook hands the next step.
    let (mut steps, mut composing_shown) = (Vec::new(), false);
    for _ in 0..8 {
        let handed = hook_without_pass(&ctx, ViewportId::ROOT);
        if handed.is_empty() {
            break;
        }
        steps.push(handed.iter().filter(|event| matches!(event, Event::Text(_) | Event::Ime(_))).cloned().collect::<Vec<_>>());
        rig.fx.frame(handed);
        composing_shown |= rig.fx.texts().iter().any(|run| run.text.contains("abkzk"));
    }
    let expected = [
        vec![Event::Text("ab".to_owned())],
        vec![Event::Ime(ImeEvent::Preedit { text: "kzk".to_owned(), active_range_chars: Some(1..2) })],
        vec![Event::Ime(ImeEvent::Commit("漢".to_owned()))],
    ];
    assert_eq!(steps, expected, "one step each, in the order queued, the cursor as the converted segment");
    rig.fx.settle();
    assert!(composing_shown, "the field showed the composition after the typed text");
    assert_eq!(*text.borrow(), "ab漢", "the commit replaced the composition");
    assert!(host.injected_idle());

    let both = input_method("ui.ime", &json!({"preedit": "a", "commit": "b"}), &host, ViewportId::ROOT, &map);
    assert!(matches!(both, Some(Err(_))), "{both:?}");
    assert!(hook_without_pass(&ctx, ViewportId::ROOT).is_empty(), "the refused request queued nothing");
}

/// A NaN or infinite point of a request is an error at the request, in either coordinate space, not a panic in the pass.
#[trial]
fn a_point_that_is_not_finite_is_refused() {
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for at in [At::Screen(pos2(bad, 1.0)), At::Screen(pos2(1.0, bad)), At::Document(pos2(bad, 1.0))] {
            assert_eq!(at.resolve(&|p| Some(p)), Err(NOT_FINITE.to_owned()), "{at:?}");
        }
    }
    assert_eq!(At::Screen(pos2(3.0, 4.0)).resolve(&|_| None), Ok(pos2(3.0, 4.0)));
}

/// A rig whose UI refuses every pressed key named in `refused`, reporting it to `channel` as the running pass's
/// refusal, polls the channel's input waiters at the end of each pass as an application does, and records which
/// request each pass carried.
fn refusing_rig(channel: &Rc<RefCell<craft_automation::control::ControlChannel>>, refused: &'static [Key]) -> (Rig, Rc<RefCell<Vec<Option<u64>>>>) {
    let running = Rc::new(RefCell::new(Vec::new()));
    let (seen, channel) = (running.clone(), channel.clone());
    let fx = Fixture::new(move |ui| {
        let host = WindowHost::of(ui.ctx());
        seen.borrow_mut().push(host.injected_running_request());
        let pressed: Vec<Key> =
            ui.input(|i| i.events.iter().filter_map(|e| if let Event::Key { key, pressed: true, .. } = e { Some(*key) } else { None }).collect());
        for key in pressed.into_iter().filter(|key| refused.contains(key)) {
            let request = host.injected_running_request().expect("a pressed key comes from injected input");
            channel.borrow_mut().input_refused(request, &format!("{key:?} is not available"));
        }
        channel.borrow_mut().poll_input(&host);
    });
    (Rig { fx, passes: Rc::default() }, running)
}

/// Queues `ui.key` for `key` in the main window and parks its reply on the request's own number.
fn queue_key(rig: &Rig, channel: &Rc<RefCell<craft_automation::control::ControlChannel>>, key: &str) -> std::sync::mpsc::Receiver<Value> {
    queue_key_in(rig, channel, key, ViewportId::ROOT)
}

/// [`queue_key`] for the window `viewport`.
fn queue_key_in(
    rig: &Rig,
    channel: &Rc<RefCell<craft_automation::control::ControlChannel>>,
    key: &str,
    viewport: ViewportId,
) -> std::sync::mpsc::Receiver<Value> {
    use craft_automation::control::{ControlRequest, Reply};
    let host = rig.host();
    input_method("ui.key", &json!({"key": key}), &host, viewport, &map).expect("a method").expect("valid");
    let (req, reply) = ControlRequest::new("ui.key", json!({"key": key}));
    channel.borrow_mut().settle(req, Reply::after_input_request(host.injected_last_request()), None, 0.0);
    reply
}

/// Two input requests in flight: each reply carries only the refusals its own input caused, and input that caused
/// none still replies `ok`.
#[trial]
fn each_input_reply_carries_only_its_own_requests_refusals() {
    use craft_automation::control::ControlChannel;
    let (_transport, requests) = std::sync::mpsc::channel();
    let channel = Rc::new(RefCell::new(ControlChannel::new(requests)));
    let (mut rig, _) = refusing_rig(&channel, &[Key::B, Key::C]);
    rig.fx.settle();
    let a = queue_key(&rig, &channel, "A");
    let b = queue_key(&rig, &channel, "B");
    let c = queue_key(&rig, &channel, "C");
    let host = rig.host();
    for _ in 0..32 {
        rig.fx.frame(Vec::new());
        if host.injected_idle() {
            break;
        }
    }
    // The pass after the input settled answers its waiters.
    rig.fx.frame(Vec::new());
    assert_eq!(a.try_recv().expect("A is answered"), json!({"ok": true, "result": null}));
    assert_eq!(b.try_recv().expect("B is answered"), json!({"ok": false, "error": "B is not available", "errors": ["B is not available"]}));
    assert_eq!(c.try_recv().expect("C is answered"), json!({"ok": false, "error": "C is not available", "errors": ["C is not available"]}));
}

/// Input cancelled after a request's input ran keeps that request's own reply; the request whose input was still
/// queued or running is cancelled.
#[trial]
fn cancelled_input_keeps_the_replies_of_requests_that_already_ran() {
    use craft_automation::control::ControlChannel;
    let (_transport, requests) = std::sync::mpsc::channel();
    let channel = Rc::new(RefCell::new(ControlChannel::new(requests)));
    let (mut rig, running) = refusing_rig(&channel, &[Key::A]);
    rig.fx.settle();
    let a = queue_key(&rig, &channel, "A");
    let b = queue_key(&rig, &channel, "B");
    let host = rig.host();
    let b_number = host.injected_last_request();
    for _ in 0..32 {
        rig.fx.frame(Vec::new());
        if running.borrow().last().copied().flatten() == Some(b_number) {
            break;
        }
    }
    assert_eq!(running.borrow().last().copied().flatten(), Some(b_number), "B's input is running");
    assert!(a.try_recv().is_err() && b.try_recv().is_err(), "nothing is answered while input remains");
    assert!(channel.borrow_mut().cancel_input(&host, "the test cancelled it"));
    assert_eq!(a.try_recv().expect("A is answered"), json!({"ok": false, "error": "A is not available", "errors": ["A is not available"]}));
    assert_eq!(b.try_recv().expect("B is answered"), json!({"ok": false, "error": "input cancelled: the test cancelled it"}));
}

/// Past `MAX_INPUT_REFUSALS` a reply keeps the first refusals and says it is truncated; a refusal for a request no
/// reply waits on goes to no one.
#[trial]
fn a_reply_with_too_many_refusals_is_truncated_and_idle_passes_report_nothing() {
    use craft_automation::control::{ControlChannel, MAX_INPUT_REFUSALS};
    let (_transport, requests) = std::sync::mpsc::channel();
    let channel = Rc::new(RefCell::new(ControlChannel::new(requests)));
    let flood = channel.clone();
    let mut rig = Rig {
        fx: Fixture::new(move |ui| {
            let host = WindowHost::of(ui.ctx());
            if let Some(request) = host.injected_running_request() {
                for i in 0..MAX_INPUT_REFUSALS + 3 {
                    flood.borrow_mut().input_refused(request, &format!("refusal {i}"));
                }
            }
            flood.borrow_mut().poll_input(&host);
        }),
        passes: Rc::default(),
    };
    rig.fx.settle();
    let reply = queue_key(&rig, &channel, "A");
    let host = rig.host();
    assert!(!channel.borrow_mut().input_refused(host.injected_last_request() + 1, "unknown"), "no reply waits on that request");
    for _ in 0..32 {
        rig.fx.frame(Vec::new());
        if host.injected_idle() {
            break;
        }
    }
    rig.fx.frame(Vec::new());
    let answer = reply.try_recv().expect("answered");
    assert_eq!(answer["truncated"], true, "{answer}");
    assert_eq!(answer["errors"].as_array().map(Vec::len), Some(MAX_INPUT_REFUSALS));
    assert_eq!(answer["error"], "refusal 0");
}

/// A window the rig never runs a pass of: input queued there stays queued.
fn idle_window() -> ViewportId {
    ViewportId::from_hash_of("a window with no passes")
}

/// Request numbers count across windows, but each window runs its own queue: input still queued in
/// one window has not run because a later request ran in another, so a cancel cancels it.
#[trial]
fn input_queued_in_another_window_is_not_taken_for_run() {
    use craft_automation::control::ControlChannel;
    let (_transport, requests) = std::sync::mpsc::channel();
    let channel = Rc::new(RefCell::new(ControlChannel::new(requests)));
    let (mut rig, _) = refusing_rig(&channel, &[Key::B]);
    rig.fx.settle();
    let elsewhere = queue_key_in(&rig, &channel, "A", idle_window());
    let main = queue_key(&rig, &channel, "B");
    let host = rig.host();
    for _ in 0..32 {
        rig.fx.frame(Vec::new());
        if !host.injected_request_queued(host.injected_last_request()) {
            break;
        }
    }
    rig.fx.frame(Vec::new());
    assert!(main.try_recv().is_err() && elsewhere.try_recv().is_err(), "the other window's input keeps both waiting");
    assert!(channel.borrow_mut().cancel_input(&host, "the test cancelled it"));
    assert_eq!(elsewhere.try_recv().expect("A is answered"), json!({"ok": false, "error": "input cancelled: the test cancelled it"}));
    assert_eq!(main.try_recv().expect("B is answered"), json!({"ok": false, "error": "B is not available", "errors": ["B is not available"]}));
}

/// Withdrawing one window's input cancels the requests it dropped; a request queued in another
/// window keeps waiting, and is answered once its own input has run.
#[trial]
fn a_withdrawal_keeps_the_waiters_of_input_it_did_not_drop() {
    use craft_automation::control::ControlChannel;
    use craft_automation::synthetic::TARGET_CHANGED;
    let (_transport, requests) = std::sync::mpsc::channel();
    let channel = Rc::new(RefCell::new(ControlChannel::new(requests)));
    let (mut rig, _) = refusing_rig(&channel, &[]);
    rig.fx.settle();
    let dropped = queue_key(&rig, &channel, "A");
    let elsewhere = queue_key_in(&rig, &channel, "B", idle_window());
    let host = rig.host();
    host.withdraw_injected(ViewportId::ROOT);
    rig.fx.frame(Vec::new());
    let answer = dropped.try_recv().expect("the withdrawn request is answered");
    assert!(answer["error"].as_str().is_some_and(|error| error.contains(TARGET_CHANGED)), "{answer}");
    assert!(elsewhere.try_recv().is_err(), "the other window's request still waits on its input");
    assert!(host.injected_request_queued(host.injected_last_request()), "its input is still queued");
}

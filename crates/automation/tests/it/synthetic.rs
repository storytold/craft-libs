//! Synthetic input through the window host: held presses and keys, modifiers, the wheel, Caps Lock, hover rest,
//! document coordinates and input-method compositions, as the passes see them, and the params refused before
//! anything is queued.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_automation::access::NO_TREE;
use craft_automation::synthetic::{At, Click, Focus, MAX_CLICKS, MAX_SCREEN_POINT, NO_DOCUMENT, NOT_FINITE, input_method};
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

/// Screen points between one page's top and the next's, in a canvas that stacks its pages down the screen.
const PAGE_STEP: f32 = 200.0;

/// Document points are screen points doubled, as a canvas at 50 % shows them, with each page [`PAGE_STEP`] below
/// the one before.
fn map(page: usize, at: Pos2) -> Option<Pos2> {
    let page = u16::try_from(page).ok()?;
    Some(pos2(at.x / 2.0, at.y / 2.0 + f32::from(page) * PAGE_STEP))
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
    rig.send("ui.move", json!({"x": 300, "y": 100, "coordinates": "document", "page": 1}));
    assert_eq!(rig.now().pos, Some(pos2(150.0, 50.0 + PAGE_STEP)), "a document point is mapped on the page it names");
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

/// Per pass: whether the waiter had its reply by the pass's end, and the pointer position the pass saw.
type PassLog = Rc<RefCell<Vec<(bool, Option<Pos2>)>>>;

/// A fixture whose UI polls `channel` at the end of each pass, as an application does after its UI, with `discard`
/// asking egui to run the pass again on the passes it is true for; `answered` records, per pass, whether the waiter
/// had its reply by the pass's end and the pointer position the pass saw.
fn polling_fixture(
    channel: Rc<RefCell<craft_automation::control::ControlChannel>>,
    reply: Rc<std::sync::mpsc::Receiver<craft_automation::control::ControlResponse>>,
    discard: impl Fn(usize) -> bool + 'static,
) -> (Fixture, PassLog) {
    let answered = Rc::new(RefCell::new(Vec::new()));
    let seen = answered.clone();
    let fx = Fixture::new(move |ui| {
        let pass = seen.borrow().len();
        if discard(pass) {
            ui.ctx().request_discard("the test runs this pass again");
        }
        channel.borrow_mut().poll_input(&WindowHost::of(ui.ctx()));
        let pos = ui.input(|i| i.pointer.latest_pos());
        seen.borrow_mut().push((reply.try_recv().is_ok(), pos));
    });
    (fx, answered)
}

/// A waiter on a move is answered at the end of the pass that ran the move, not a pass later: the application polls
/// after its UI, where the running pass has not ended yet.
#[trial]
fn a_waiter_is_answered_by_the_pass_that_ran_its_step() {
    use craft_automation::control::{ControlChannel, ControlRequest, Reply};

    let (_transport, requests) = std::sync::mpsc::channel();
    let channel = Rc::new(RefCell::new(ControlChannel::new(requests)));
    let (req, reply) = ControlRequest::new("ui.move", json!({"x": 30, "y": 20}));
    let (mut fx, answered) = polling_fixture(channel.clone(), Rc::new(reply), |_| false);
    fx.idle(2);
    answered.borrow_mut().clear();
    let host = WindowHost::of(fx.ctx());
    input_method("ui.move", &json!({"x": 30, "y": 20}), &host, ViewportId::ROOT, &map).expect("a method").expect("valid");
    channel.borrow_mut().settle(req, Reply::AfterInput, None, 0.0);
    fx.frame(Vec::new());
    let passes = answered.borrow().clone();
    assert_eq!(passes, vec![(true, Some(pos2(30.0, 20.0)))], "the pass that saw the move answered it");
}

/// A pass egui runs again (`request_discard`) does not answer: the pass that runs again, and draws what the step
/// did, does.
#[trial]
fn a_pass_run_again_leaves_the_answer_to_its_rerun() {
    use craft_automation::control::{ControlChannel, ControlRequest, Reply};

    let (_transport, requests) = std::sync::mpsc::channel();
    let channel = Rc::new(RefCell::new(ControlChannel::new(requests)));
    let (req, reply) = ControlRequest::new("ui.move", json!({"x": 30, "y": 20}));
    let start = Rc::new(Cell::new(usize::MAX));
    let first = start.clone();
    let (mut fx, answered) = polling_fixture(channel.clone(), Rc::new(reply), move |pass| pass == first.get());
    fx.idle(2);
    start.set(answered.borrow().len());
    let host = WindowHost::of(fx.ctx());
    input_method("ui.move", &json!({"x": 30, "y": 20}), &host, ViewportId::ROOT, &map).expect("a method").expect("valid");
    channel.borrow_mut().settle(req, Reply::AfterInput, None, 0.0);
    fx.frame(Vec::new());
    assert_eq!(fx.passes(), 2, "egui ran the discarded pass again");
    let passes: Vec<bool> = answered.borrow()[start.get()..].iter().map(|(done, _)| *done).collect();
    assert_eq!(passes, vec![false, true], "the rerun answers, the discarded pass does not");
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
    let none = |_: usize, _: Pos2| None;
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
/// before any of it is queued: no half-queued press or held key is left behind. A node target takes no point, repeat,
/// hold, modifier or button but the secondary one, and names exactly one of `id` and `label`.
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
        ("ui.move", json!({"x": 10, "y": 10, "nudge": 1})),
        ("ui.move", json!({"x": 10, "y": 10, "rest": "long"})),
        ("ui.move", json!({"x": 10, "y": 10, "coordinates": "page"})),
        ("ui.move", json!({"x": 10, "y": -far})),
        ("ui.key", json!({"key": "A", "hold": true, "nudge": 1})),
        ("ui.key", json!({"key": 5})),
        ("ui.key", json!({"key": "NoSuchKey", "hold": true})),
        ("ui.ime", json!({"preedit": "ab", "cursor": [1, 9]})),
        ("ui.click", json!({"x": 10, "y": 10, "label": "Save"})),
        ("ui.click", json!({"label": "Save", "count": 2})),
        ("ui.click", json!({"label": "Save", "hold": true})),
        ("ui.click", json!({"label": "Save", "shift": true})),
        ("ui.click", json!({"label": "Save", "button": "middle"})),
        ("ui.click", json!({"id": "12", "button": "middle"})),
        ("ui.click", json!({"id": "twelve"})),
        ("ui.focus", json!({"id": "12", "label": "Save"})),
        ("ui.focus", json!({})),
        ("ui.drop", json!({"paths": []})),
        ("ui.drop", json!({"paths": ["a.pdf"], "alt": true})),
    ];
    for (method, params) in &refused {
        let answer = input_method(method, params, &host, ViewportId::ROOT, &map);
        assert!(matches!(answer, Some(Err(_))), "{method} {params} is refused: {answer:?}");
        assert!(host.injected_idle(), "{method} {params} left nothing waiting");
        assert!(hook_without_pass(&ctx, ViewportId::ROOT).is_empty(), "{method} {params} queued nothing");
    }
    // This window keeps no access tree, so a node request is refused there whatever it says; its parameters alone
    // refuse the node rows, and the secondary button, a node's context menu, is allowed.
    for (method, params) in refused.iter().filter(|(_, params)| params.get("id").is_some() || params.get("label").is_some() || params == &json!({})) {
        let by_params = if *method == "ui.focus" { Focus::from_params(params).is_err() } else { Click::from_params(params).is_err() };
        assert!(by_params, "{method} {params} is refused by its parameters");
    }
    assert!(Click::from_params(&json!({"label": "Save", "button": "secondary"})).is_ok());
}

/// A key whose press moves the focus (Tab from one field to the next) still has its release carried out: the release
/// is not cancelled as input whose focus changed, no key stays down, and the waiter is answered as processed. The
/// framework takes the Tab press it acts on out of egui's input, so egui's fields never read it.
#[trial]
fn a_key_release_is_carried_out_after_its_press_moves_focus() {
    use craft_automation::control::{ControlChannel, ControlRequest, Reply};

    let passes: Rc<RefCell<Vec<Seen>>> = Rc::default();
    let rects = Rc::new(Cell::new([Rect::NOTHING; 2]));
    let (seen, shown) = (passes.clone(), rects.clone());
    let (first, second) = (Rc::new(RefCell::new(String::new())), Rc::new(RefCell::new(String::new())));
    let fx = Fixture::new(move |ui| {
        let a = TextField::new("First", Bind::bound(&mut *first.borrow_mut())).show(ui).response.rect;
        let b = TextField::new("Second", Bind::bound(&mut *second.borrow_mut())).show(ui).response.rect;
        shown.set([a, b]);
        seen.borrow_mut().push(ui.input(|i| Seen { any_key_down: !i.keys_down.is_empty(), events: i.events.clone(), ..Seen::default() }));
    });
    let mut rig = Rig { fx, passes };
    rig.fx.settle();
    let at = rects.get()[0].center();
    rig.send("ui.click", json!({"x": at.x, "y": at.y}));
    let before = rig.fx.focused();
    assert!(before.is_some(), "the click focused the first field");

    let (_transport, requests) = std::sync::mpsc::channel();
    let mut channel = ControlChannel::new(requests);
    let host = rig.host();
    rig.take();
    input_method("ui.key", &json!({"key": "Tab"}), &host, ViewportId::ROOT, &map).expect("a method").expect("valid");
    let (req, reply) = ControlRequest::new("ui.key", json!({"key": "Tab"}));
    channel.settle(req, Reply::AfterInput, None, 0.0);
    let mut answer = None;
    for _ in 0..32 {
        rig.fx.frame(Vec::new());
        channel.poll_input(&host);
        answer = reply.try_recv().ok();
        if answer.is_some() {
            break;
        }
    }
    rig.fx.frame(Vec::new());

    let answer = answer.expect("the waiter is answered");
    assert_eq!(answer["ok"], true, "processed, not cancelled: {answer}");
    assert!(rig.fx.focused().is_some() && rig.fx.focused() != before, "the press moved the focus");
    let passes = rig.take();
    let tab = |event: &Event| matches!(event, Event::Key { key: Key::Tab, pressed: true, .. });
    assert!(!passes.iter().any(|pass| pass.events.iter().any(tab)), "the Tab press the framework took reaches no field in egui");
    assert!(passes.last().is_some_and(|pass| !pass.any_key_down), "no key stays down");
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
    let no_controls = &|_: &_| Err(NO_TREE.to_owned());
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for at in [At::Screen(pos2(bad, 1.0)), At::Screen(pos2(1.0, bad)), At::Document(pos2(bad, 1.0), 0)] {
            assert_eq!(at.resolve(&|_, p| Some(p), no_controls), Err(NOT_FINITE.to_owned()), "{at:?}");
        }
    }
    assert_eq!(At::Screen(pos2(3.0, 4.0)).resolve(&|_, _| None, no_controls), Ok(pos2(3.0, 4.0)));
}

/// `ui.wheel` in lines queues whole notches in the line unit, clamped; without `unit` the turn is in points; an
/// unknown unit is refused.
#[trial]
fn a_wheel_in_lines_turns_by_notches() {
    fn turn(rig: &mut Rig, params: Value) -> (egui::MouseWheelUnit, egui::Vec2) {
        rig.send("ui.wheel", params);
        rig.take()
            .into_iter()
            .flat_map(|pass| pass.events)
            .find_map(|event| match event {
                Event::MouseWheel { unit, delta, .. } => Some((unit, delta)),
                _ => None,
            })
            .expect("a pass saw the turn")
    }
    let mut rig = Rig::new();
    assert_eq!(turn(&mut rig, json!({"x": 30, "y": 20, "dy": 2.4, "unit": "lines"})), (egui::MouseWheelUnit::Line, egui::vec2(0.0, 2.0)));
    assert_eq!(turn(&mut rig, json!({"dy": 500, "unit": "lines"})).1, egui::vec2(0.0, 100.0), "notches are clamped");
    assert_eq!(turn(&mut rig, json!({"dy": 2.4})), (egui::MouseWheelUnit::Point, egui::vec2(0.0, 2.4)));
    let refused = input_method("ui.wheel", &json!({"dy": 1, "unit": "pages"}), &rig.host(), ViewportId::ROOT, &map);
    assert!(matches!(refused, Some(Err(_))), "{refused:?}");
    assert!(rig.host().injected_idle(), "the refused turn queued nothing");
}

/// `ui.move` requires both `x` and `y`: omitting either coordinate refuses the request before anything is queued
/// instead of silently moving to 0 on the missing axis.
#[trial]
fn ui_move_refuses_a_missing_coordinate() {
    let mut rig = Rig::new();
    rig.fx.settle();
    let (host, ctx) = (rig.host(), rig.fx.ctx().clone());
    for params in [json!({"x": 1}), json!({"y": 1}), json!({})] {
        let answer = input_method("ui.move", &params, &host, ViewportId::ROOT, &map);
        assert!(matches!(answer, Some(Err(_))), "ui.move {params} is refused: {answer:?}");
        assert!(host.injected_idle(), "ui.move {params} left nothing waiting");
        assert!(hook_without_pass(&ctx, ViewportId::ROOT).is_empty(), "ui.move {params} queued nothing");
    }
}

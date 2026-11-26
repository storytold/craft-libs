//! Latched modifiers (`FrameInput::latched`): the app holds them down for every press, and only for pointer work.

use craft_behavior::Sense;
use craft_keymap::{Chord, Key as ChordKey, Mods};
use craft_ui::contract::{Common, Component, Cx, Focus as _, Pointer as _, Props as _, Role, Show as _};
use craft_ui::runtime::focus::{FocusTarget, KeyUse, request};
use craft_ui::style::StyleCx;
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Id, Key, Modifiers, Pos2, Response, Ui};
use std::cell::{Cell, RefCell};

/// The probe claims K plain and with Shift: which one a press of K takes tells the modifiers key routing read.
const CLAIMS: &[Chord] = &[Chord::plain(ChordKey::K), Chord::new(Mods::SHIFT, ChordKey::K)];

/// What the probe saw.
#[derive(Default)]
struct Log {
    /// The modifiers of each pass's activation.
    activations: Vec<Mods>,
    /// The claimed chords the probe took.
    claimed: Vec<Chord>,
    /// `pointer::frame(..).mods` of every pass.
    pointer: Vec<Mods>,
}

/// A focusable button that logs its activations' modifiers and the claimed chords it takes.
#[derive(Component)]
struct LatchProbe<'a> {
    #[common]
    common: Common,
    log: &'a RefCell<Log>,
}

impl Component for LatchProbe<'_> {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: LatchProbeStyleKey) -> PlainStyle {
        PlainStyle::min(egui::vec2(80.0, 30.0))
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::FOCUS
    }

    fn keys(&self) -> KeyUse {
        Self::KEYS.with_claims(CLAIMS)
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let activation = cx.activation().map(|activation| activation.mods);
        let claimed: Vec<Chord> = CLAIMS.iter().copied().filter(|chord| cx.take_claimed(*chord)).collect();
        let mut log = self.log.borrow_mut();
        log.activations.extend(activation);
        log.claimed.extend(claimed);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The probe's frame closure, the probe's id and rect once shown, and a switch that discards the next pass.
struct Rig {
    log: RefCell<Log>,
    shown: Cell<Option<(Id, egui::Rect)>>,
    discard: Cell<bool>,
}

impl Rig {
    fn new() -> Self {
        Rig { log: RefCell::new(Log::default()), shown: Cell::new(None), discard: Cell::new(false) }
    }

    fn show(&self, ui: &mut Ui) {
        let response = LatchProbe { common: Common::new(), log: &self.log }.id_salt("latch").show(ui);
        self.shown.set(Some((response.id, response.rect)));
        self.log.borrow_mut().pointer.push(craft_ui::runtime::pointer::frame(ui.ctx()).mods);
        if self.discard.take() {
            ui.ctx().request_discard("the trial reruns the pass");
        }
    }

    fn at(&self) -> Pos2 {
        self.shown.get().map_or(Pos2::ZERO, |(_, rect)| rect.center())
    }

    fn clear(&self) {
        *self.log.borrow_mut() = Log::default();
    }
}

/// A click on the probe whose release frame runs a second pass; returns that frame's pass count.
fn click_with_rerun(frames: &mut Frames, rig: &Rig) -> usize {
    let at = rig.at();
    frames.frame(vec![moved(at)], |ui| rig.show(ui));
    frames.frame(vec![press(at)], |ui| rig.show(ui));
    rig.discard.set(true);
    let passes = frames.frame(vec![release(at)], |ui| rig.show(ui));
    frames.idle(1, |ui| rig.show(ui));
    passes
}

#[trial]
fn a_latched_modifier_reaches_every_press_until_released() {
    let rig = Rig::new();
    let mut frames = Frames::new();
    frames.idle(3, |ui| rig.show(ui));
    let id = rig.shown.get().map(|(id, _)| id).expect("the probe showed");
    request(&frames.ctx, FocusTarget::Id(id));
    frames.idle(2, |ui| rig.show(ui));

    frames.latched(Mods::SHIFT);
    rig.clear();
    assert_eq!(click_with_rerun(&mut frames, &rig), 2, "the release frame runs a second pass");
    {
        let log = rig.log.borrow();
        assert_eq!(log.activations, [Mods::SHIFT], "one click, one activation, Shift held by the latch, on the rerun pass too");
        assert!(log.pointer.iter().all(|mods| mods.shift), "every pass's pointer frame holds the latch: {:?}", log.pointer);
    }

    // A typed key keeps the keyboard's own modifiers: K reaches the plain-K claim, not Shift+K.
    rig.clear();
    frames.frame(vec![key_press(Key::K, Modifiers::NONE)], |ui| rig.show(ui));
    frames.idle(1, |ui| rig.show(ui));
    assert_eq!(rig.log.borrow().claimed, [Chord::plain(ChordKey::K)], "no Shift chord fired from a latched Shift");

    frames.latched(Mods::NONE);
    rig.clear();
    click_with_rerun(&mut frames, &rig);
    let log = rig.log.borrow();
    assert_eq!(log.activations, [Mods::NONE], "a released latch leaves the click plain");
    assert!(log.pointer.iter().all(|mods| !mods.shift), "{:?}", log.pointer);
}

#[trial]
fn a_latch_change_adds_no_pass() {
    let rig = Rig::new();
    let mut frames = Frames::new();
    frames.idle(3, |ui| rig.show(ui));
    // The pointer rests on the probe, where a change of the held modifiers could matter to it.
    let at = rig.at();
    frames.frame(vec![moved(at)], |ui| rig.show(ui));
    frames.idle(2, |ui| rig.show(ui));
    let steady = frames.frame(Vec::new(), |ui| rig.show(ui));
    frames.latched(Mods::SHIFT);
    let latched = frames.frame(Vec::new(), |ui| rig.show(ui));
    let held = frames.frame(Vec::new(), |ui| rig.show(ui));
    frames.latched(Mods::NONE);
    let released = frames.frame(Vec::new(), |ui| rig.show(ui));
    assert_eq!(latched, steady, "latching runs no extra pass");
    assert_eq!(held, steady);
    assert_eq!(released, steady, "releasing runs no extra pass");
}

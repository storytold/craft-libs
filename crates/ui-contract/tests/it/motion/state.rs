//! Animated values: state easing at step 12, the motion table, and the one repaint request.

use craft_behavior::{MotionMode, Sense};
use craft_ui::contract::{Animate as _, Common, Component, Cx, PaintUi, Role, Show as _, Styled as _, spec};
use craft_ui::runtime::env::RootEnv;
use craft_ui::style::StyleCx;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Pos2, Response};
use std::cell::Cell;
use std::time::Duration;

/// A component that reports the hover value its paint saw.
#[derive(Component)]
struct Hovered<'a> {
    #[common]
    common: Common,
    seen: &'a Cell<f32>,
}

impl Component for Hovered<'_> {
    const ROLE: Role = Role::BUTTON;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: HoveredStyleKey) -> PlainStyle {
        PlainStyle::min(egui::vec2(40.0, 20.0))
    }

    fn sense(&self) -> Sense {
        Sense::CLICK
    }

    fn paint(&mut self, _: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        self.seen.set(cx.state().t.hover);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// State easing reaches its end value in steps, then asks for no further pass.
#[trial]
fn state_easing_reaches_its_end_value_and_stops_requesting_repaints() {
    let seen = Cell::new(0.0);
    let mut frames = Frames::new();
    let mut show = |ui: &mut egui::Ui| {
        Hovered { common: Common::new(), seen: &seen }.show(ui);
    };
    frames.idle(3, &mut show);
    assert_eq!(frames.repaint_delay(), None, "a resting component asks for nothing");
    let rest = seen.get();
    frames.frame(vec![moved(Pos2::new(10.0, 10.0))], &mut show);
    let mut values = vec![seen.get()];
    while frames.repaint_delay() == Some(Duration::ZERO) && values.len() < 60 {
        frames.frame(Vec::new(), &mut show);
        values.push(seen.get());
    }
    let end = seen.get();
    assert!(end > rest, "the hover value rose from {rest} to its end: {values:?}");
    assert!(values.iter().any(|v| *v > rest && *v < end), "it passed through values between: {values:?}");
    assert!(values.windows(2).all(|pair| pair[0] <= pair[1]), "it rose without stepping back: {values:?}");
    assert_eq!(frames.repaint_delay(), None, "once at its end it requests no repaint");
}

/// The animated value of a boolean that is on.
const ON: f32 = 1.0;

/// What a component's own animations returned in the last pass.
#[derive(Default)]
struct Seen {
    own_on: Cell<f32>,
    presence: Cell<Option<(f32, bool, bool)>>,
    phase: Cell<Option<Option<f32>>>,
}

/// Animates `on` itself (a name the pipeline's state mix also uses), a presence and a loop phase.
#[derive(Component)]
struct Animator<'a> {
    #[common]
    common: Common,
    open: bool,
    seen: &'a Seen,
}

impl Component for Animator<'_> {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: AnimatorStyleKey) -> PlainStyle {
        PlainStyle::min(egui::vec2(40.0, 20.0))
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.seen.own_on.set(cx.anim_bool("on", true, spec::HOVER));
        let presence = cx.presence("panel", self.open, spec::OPEN_FADE);
        self.seen.presence.set(Some((presence.t, presence.visible, presence.exiting)));
        self.seen.phase.set(Some(cx.phase(1.0, None)));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// A component's own `on` animation and the pipeline's `on` state are different tweens: neither retargets the other,
/// so both settle.
#[trial]
fn a_component_animating_on_does_not_fight_the_state_mix() {
    let seen = Seen::default();
    let mut frames = Frames::new();
    let mut show = |ui: &mut egui::Ui| {
        Animator { common: Common::new(), open: true, seen: &seen }.show(ui);
    };
    frames.idle(60, &mut show);
    assert_eq!(seen.own_on.get(), ON, "the component's own value stays at its target");
    let loop_only = frames.repaint_delay();
    assert!(loop_only.is_some(), "the loop phase keeps asking for passes");
    let mut frames = Frames::new();
    frames.root(RootEnv::default().motion(MotionMode::Reduced));
    frames.idle(60, &mut show);
    assert_eq!(frames.repaint_delay(), None, "with no loop under reduced motion, nothing is left to animate");
}

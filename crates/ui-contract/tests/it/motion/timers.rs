//! `wake_at`: a timer fires once at its time, across discarded passes, and asks for nothing before or after.

use craft_behavior::Sense;
use craft_ui::contract::{Animate as _, Common, Component, Cx, Remember as _, Role, Show as _};
use craft_ui::style::StyleCx;
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::Response;
use std::cell::Cell;
use std::time::Duration;

/// Where a half-second timer stands.
#[derive(Clone, Copy, Default)]
enum Timer {
    #[default]
    Idle,
    Armed(f64),
    Fired,
}

/// Arms a timer on its first pass and counts how often it fires.
#[derive(Component)]
struct Alarm<'a> {
    #[common]
    common: Common,
    fired: &'a Cell<u32>,
}

impl Component for Alarm<'_> {
    const ROLE: Role = Role::None;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: AlarmStyleKey) -> PlainStyle {
        PlainStyle::min(egui::vec2(40.0, 20.0))
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let now = cx.now();
        let state = *cx.memory::<Timer>();
        match state {
            Timer::Idle => {
                *cx.memory::<Timer>() = Timer::Armed(now + 0.5);
                cx.wake_at(now + 0.5);
            }
            Timer::Armed(at) if now >= at => {
                *cx.memory::<Timer>() = Timer::Fired;
                self.fired.set(self.fired.get() + 1);
            }
            Timer::Armed(at) => cx.wake_at(at),
            Timer::Fired => {}
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn a_timer_fires_once_at_its_time_across_discarded_passes() {
    let fired = Cell::new(0);
    let discard = Cell::new(true);
    let mut frames = Frames::new();
    let mut show = |ui: &mut egui::Ui| {
        Alarm { common: Common::new(), fired: &fired }.show(ui);
        if fired.get() == 1 && discard.take() {
            ui.ctx().request_discard("the firing frame runs twice");
        }
    };
    frames.idle(3, |_| {});
    frames.frame(Vec::new(), &mut show);
    let wait = frames.repaint_delay().expect("an armed timer asks for a pass at its time");
    assert!(wait > Duration::from_millis(400) && wait <= Duration::from_millis(500), "the wait runs to the deadline: {wait:?}");
    frames.advance(0.3);
    frames.frame(Vec::new(), &mut show);
    let wait = frames.repaint_delay().expect("still armed");
    assert!(wait < Duration::from_millis(250), "the wait shrinks as time passes: {wait:?}");
    assert_eq!(fired.get(), 0, "not before its time");
    frames.advance(0.3);
    let passes = frames.frame(Vec::new(), &mut show);
    assert_eq!(passes, 2, "the firing frame was discarded and repeated");
    assert_eq!(fired.get(), 1, "it fired once across both passes");
    assert_eq!(frames.repaint_delay(), None, "a fired timer asks for nothing");
    frames.idle(3, &mut show);
    assert_eq!(fired.get(), 1, "and never fires again");
}

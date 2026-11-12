//! The ruler: a plain wheel over it zooms the timeline about the pointer, whether or not the ruler has focus.

use std::cell::RefCell;
use std::rc::Rc;

use craft_timeline::ui::{TimelineRuler, TimelineView};
use craft_timeline::{FrameRate, Tick, TimeFormat, TimeView};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Pos2, vec2};

const POINTS_PER_SECOND: f64 = 40.0;

/// A ruler inside a strip view over 60 s, its playhead and view held between passes.
fn ruler() -> (Fixture, Rc<RefCell<TimeView>>) {
    let view = Rc::new(RefCell::new(TimeView::new(Tick::ZERO, POINTS_PER_SECOND)));
    let shown = view.clone();
    let playhead = Rc::new(RefCell::new(Tick::ZERO));
    let mut fixture = Fixture::new(move |ui| {
        let (mut view, mut playhead) = (shown.borrow_mut(), playhead.borrow_mut());
        let format = TimeFormat::timecode(FrameRate::FPS_30);
        TimelineView::new("Time", Bind::bound(&mut *view), format, Tick::from_seconds_f64(60.0)).strip().show(ui, |ui, _| {
            TimelineRuler::new("Time ruler", Bind::bound(&mut *playhead)).show(ui);
        });
    });
    fixture.room(vec2(600.0, 120.0));
    fixture.settle();
    (fixture, view)
}

/// Turns the wheel up over `at` and lets the zoom ease to its end.
fn wheel_at(fixture: &mut Fixture, at: Pos2) {
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::wheel(20.0)]);
    fixture.idle(60);
}

#[trial]
fn a_wheel_over_the_ruler_zooms_the_timeline_about_the_pointer() {
    let (mut fixture, view) = ruler();
    let rect = fixture.by_label("Time ruler").expect("the ruler").rect;
    let at = rect.left_center() + vec2(rect.width() / 3.0, 0.0);
    let under = |view: &TimeView| view.time_at(at.x, rect.left()).seconds();
    let before = under(&view.borrow());
    wheel_at(&mut fixture, at);
    let zoomed = *view.borrow();
    assert!(zoomed.points_per_second > POINTS_PER_SECOND, "the wheel zoomed in: {} points a second", zoomed.points_per_second);
    assert!((under(&zoomed) - before).abs() < 0.05, "the time under the pointer stayed: {before} s, then {} s", under(&zoomed));

    // A click focuses the ruler (and scrubs); the wheel still zooms.
    fixture.click_at(at);
    let focused = view.borrow().points_per_second;
    wheel_at(&mut fixture, at);
    assert!(view.borrow().points_per_second > focused, "the focused ruler passed the wheel to the view");
}

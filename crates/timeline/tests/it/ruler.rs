//! The ruler: a plain wheel over it zooms the timeline about the pointer, whether or not the ruler has focus, and its
//! split-edit marks sit under the tick labels.

use std::cell::RefCell;
use std::rc::Rc;

use craft_timeline::ui::{SplitMarks, TimelineRuler, TimelineView};
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

#[trial]
fn split_edit_marks_sit_under_the_tick_labels_video_above_audio() {
    let mut fixture = Fixture::new(|ui| {
        let mut view = TimeView::new(Tick::ZERO, POINTS_PER_SECOND);
        let mut playhead = Tick::ZERO;
        let mut marks = SplitMarks::default();
        marks.video_in = Some(Tick::from_seconds_f64(2.0));
        marks.audio_in = Some(Tick::from_seconds_f64(2.0));
        let format = TimeFormat::timecode(FrameRate::FPS_30);
        TimelineView::new("Time", Bind::bound(&mut view), format, Tick::from_seconds_f64(60.0)).strip().show(ui, |ui, _| {
            TimelineRuler::new("Time ruler", Bind::bound(&mut playhead)).split_marks(marks).show(ui);
        });
    });
    fixture.room(vec2(600.0, 120.0));
    fixture.settle();
    let ruler = fixture.by_label("Time ruler").expect("the ruler").rect;
    let video = fixture.by_label("Split V In").expect("the video mark").rect;
    let audio = fixture.by_label("Split A In").expect("the audio mark").rect;
    for (name, mark) in [("video", video), ("audio", audio)] {
        assert!(ruler.contains_rect(mark), "the {name} mark {mark:?} stays inside the ruler {ruler:?}");
        // The tick labels take the line above the marks.
        assert!(mark.top() >= ruler.center().y, "the {name} mark {mark:?} sits under the tick labels of {ruler:?}");
    }
    assert!(video.center().y < audio.center().y, "the video mark {video:?} sits above the audio mark {audio:?}");
}

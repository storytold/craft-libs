//! The overview rows and key snapping in the keyframe lane.

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::{EditEvent, Selection, SelectionChange};
use craft_timeline::ui::{
    KeyframeId, KeyframeInterpolation, KeyframeLane, KeyframeMark, LabelPalette, TimelineOverview, TimelineView, overview_runs,
};
use craft_timeline::{FrameRate, Tick, TimeFormat, TimeRange, TimeView, format_time};
use craft_ui::contract::{Bind, Role, Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Modifiers, Rect, vec2};

const ROOM: egui::Vec2 = vec2(600.0, 60.0);
const POINTS_PER_SECOND: f64 = 50.0;

fn seconds(at: f64) -> Tick {
    Tick::from_seconds_f64(at)
}

fn format() -> TimeFormat {
    TimeFormat::timecode(FrameRate::FPS_24)
}

/// Every track gets a row inside the overview, however tall the room it is shown in.
#[trial]
fn every_track_has_a_row_inside_the_overview() {
    const TRACKS: usize = 8;
    let palette = LabelPalette::default().0;
    let tracks: Vec<_> = (0..TRACKS).map(|track| vec![(TimeRange::new(seconds(0.0), seconds(20.0)), palette[track])]).collect();
    let mut fixture = Fixture::new(move |ui| {
        let mut view = TimeView::new(Tick::ZERO, POINTS_PER_SECOND);
        let runs = overview_runs(&tracks, seconds(40.0), ui.available_width());
        TimelineOverview::new("Overview", &runs, Bind::bound(&mut view), seconds(40.0)).show(ui);
    });
    fixture.room(vec2(ROOM.x, ROOM.y * 10.0));
    fixture.settle();
    let overview = fixture.by_role(Role::IMAGE).into_iter().next().expect("the overview is an image").rect;
    let rows: Vec<Rect> =
        fixture.shape_bounds().iter().copied().filter(|b| b.height() < overview.height() / 2.0 && b.width() > overview.width() / 4.0).collect();
    assert_eq!(rows.len(), TRACKS, "one run per track: {rows:?}");
    assert!(rows.iter().all(|row| overview.contains_rect(*row)), "every row lies inside {overview:?}: {rows:?}");
    assert!(
        rows.windows(2).all(|pair| pair[0].center().y < pair[1].top() && pair[0].bottom() < pair[1].center().y),
        "rows stack top to bottom: {rows:?}"
    );
}

/// What a lane fixture reports.
#[derive(Default)]
struct Seen {
    selection: Selection<KeyframeId>,
    changes: Vec<SelectionChange<KeyframeId>>,
    deltas: Vec<Tick>,
    gesture: Vec<EditEvent<Tick>>,
    deleted: bool,
    double_clicked: Option<KeyframeId>,
}

fn keys() -> Vec<KeyframeMark> {
    let key = |id: u64, at: f64| KeyframeMark::new(KeyframeId::new(id), seconds(at), KeyframeInterpolation::Linear);
    vec![key(1, 1.0), key(2, 3.0), key(3, 5.0)]
}

/// A lane whose keys stay inside `bounds` and whose Shift-drags snap to `playhead`.
fn lane_in(bounds: Option<TimeRange>, playhead: Tick) -> (Fixture, Rc<RefCell<Seen>>) {
    let seen = Rc::new(RefCell::new(Seen::default()));
    let shown = seen.clone();
    let view = Rc::new(RefCell::new(TimeView::new(Tick::ZERO, POINTS_PER_SECOND)));
    let mut fixture = Fixture::new(move |ui| {
        let (mut seen, mut view) = (shown.borrow_mut(), view.borrow_mut());
        let keys = keys();
        let selection = seen.selection.clone();
        let area = Rect::from_min_size(ui.max_rect().min, vec2(ROOM.x, 28.0));
        let out = TimelineView::new("Timeline", Bind::bound(&mut *view), format(), seconds(10.0))
            .show(ui, |ui, _| {
                let lane = KeyframeLane::new("Keyframes", &keys, &selection).playhead(playhead);
                match bounds {
                    Some(bounds) => lane.bounds(bounds),
                    None => lane,
                }
                .show_at(ui, area)
            })
            .inner;
        if let Some(out) = out {
            if let Some(change) = out.select {
                seen.changes.push(change.clone());
                match change {
                    SelectionChange::Set(items) => seen.selection = Selection::new(items.clone(), items.last().copied(), items.last().copied()),
                    SelectionChange::Toggle(key) => {
                        let mut items = seen.selection.items.clone();
                        match items.iter().position(|k| *k == key) {
                            Some(at) => drop(items.remove(at)),
                            None => items.push(key),
                        }
                        seen.selection = Selection::new(items, Some(key), Some(key));
                    }
                    _ => seen.selection = Selection::default(),
                }
            }
            seen.gesture.extend(out.delta.clone());
            if let Some(&delta) = out.delta.as_ref().and_then(EditEvent::value) {
                seen.deltas.push(delta);
            }
            seen.deleted |= out.delete;
            seen.double_clicked = seen.double_clicked.or(out.double_clicked);
        }
    });
    fixture.room(ROOM);
    fixture.settle();
    (fixture, seen)
}

/// The option of the key at `at` seconds, named by its time as timecode.
fn key_at(fixture: &Fixture, at: f64) -> Target {
    let name = format_time(seconds(at), &format());
    fixture.by_label(&name).unwrap_or_else(|| panic!("no key named {name}"))
}

/// A dragged key moves by whole frames; with Shift it snaps to the playhead within a few points and not farther.
#[trial]
fn dragging_keys_snaps_to_frames_and_with_shift_to_the_playhead() {
    let frames = |count| FrameRate::FPS_24.tick_of(count);
    let (mut fixture, seen) = lane_in(None, seconds(4.0));
    let key = key_at(&fixture, 3.0);
    fixture.drag(&key, vec2(37.0, 0.0));
    assert_eq!(seen.borrow().deltas.last().copied(), Some(frames(18)), "0.74 s is 17.8 frames: the nearest frame");

    let (mut fixture, seen) = lane_in(None, seconds(4.0));
    let key = key_at(&fixture, 3.0);
    fixture.hold(Modifiers::SHIFT);
    fixture.drag(&key, vec2(48.0, 0.0));
    assert_eq!(seen.borrow().deltas.last().copied(), Some(seconds(1.0)), "2 points from the playhead: snapped to it");

    let (mut fixture, seen) = lane_in(None, seconds(4.0));
    let key = key_at(&fixture, 3.0);
    fixture.hold(Modifiers::SHIFT);
    fixture.drag(&key, vec2(38.0, 0.0));
    assert_eq!(seen.borrow().deltas.last().copied(), Some(frames(18)), "12 points away: only the frame grid");
}

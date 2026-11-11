//! The pointer rows of 22.4: slop, hit areas and the arena winner.

use super::drive::{Plan, input_fixture, root_target};
use super::{ANY, Case, Subject};
use crate::contract::ShapeKind;
use crate::runtime::egui::access::Declared;
use crate::testing::fixture::{Fixture, Target};
use crate::testing::sensing::{ProbeMember, ProbePart, ProbeRoot, RootReport, show};
use crate::testing::story::{EditKind, Grip, Required};
use crate::testing::{Frames, input, pointer};
use craft_behavior::{HitLevel, PartKey, Sense};
use egui::{Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

pub(super) const CASES: &[Case] = &[
    Case::new("pointer-slop", ANY, slop_separates_click_from_drag),
    Case::new("pointer-hit-areas", ANY, parts_take_presses_in_their_hit_areas),
    Case::new("pointer-arena-order", ANY, arena_winner_ignores_show_order),
];

/// A press of `by` points along x, released where it ends.
fn gesture(fixture: &mut Fixture, target: &Target, by: f32) {
    fixture.drag(target, Vec2::new(by, 0.0));
}

/// A release within slop of a press on a clicking component is one click. A press that moves past slop starts one
/// drag on a component that drags and is no click on one that does not. The slop is the runtime's: it installs the
/// theme's `size.hit.slop` as egui's click distance.
fn slop_separates_click_from_drag(subject: &Subject) -> Result<(), String> {
    let probe = subject.probe;
    for story in &subject.stories {
        let (fixture, plan) = subject.base(story).parts();
        let slop = input::max_click_dist(fixture.ctx());
        match plan {
            Plan::Activate if !probe.carries_value() && fixture.tree().declares_key(root_target(fixture).node, Declared::Click) => {
                let part = fixture.click_part().zip(fixture.root_id());
                let target = |fixture: &Fixture| part.and_then(|(part, id)| fixture.part(id, part)).unwrap_or_else(|| root_target(fixture));
                for (by, expected) in [(0.5 * slop, 1), (2.0 * slop, 0)] {
                    let mut fixture = input_fixture(story);
                    let target = target(&fixture);
                    fixture.take_outputs();
                    gesture(&mut fixture, &target, by);
                    let clicks = fixture.take_outputs().iter().filter(|output| output.clicked).count();
                    if clicks != expected {
                        return Err(format!("{}: a press moved {by} points (slop {slop}) gave {clicks} clicks, expected {expected}", story.name));
                    }
                }
            }
            Plan::Drag(Grip { part, .. }) if probe.carries_value() => {
                for (by, begins) in [(0.5 * slop, 0), (2.0 * slop, 1)] {
                    let mut fixture = input_fixture(story);
                    let Some(grip) = fixture.root_id().and_then(|id| fixture.part(id, part)) else { continue };
                    fixture.take_outputs();
                    gesture(&mut fixture, &grip, by);
                    let kinds: Vec<EditKind> = fixture.take_outputs().into_iter().flat_map(|output| output.edits).collect();
                    let started = kinds.iter().filter(|kind| **kind == EditKind::Begin).count();
                    if started != begins {
                        return Err(format!(
                            "{}: a press moved {by} points (slop {slop}) began {started} drags, expected {begins}: {kinds:?}",
                            story.name
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// A point of `part` outside `root` when its hit area leaves the root's rect, else a point inside it off the middle.
fn probe_point(part: Rect, root: Rect) -> Pos2 {
    let across = |fx: f32, fy: f32| Pos2::new(part.min.x + part.width() * fx, part.min.y + part.height() * fy);
    let candidates = [(0.5, 0.5), (0.1, 0.5), (0.9, 0.5), (0.5, 0.1), (0.5, 0.9), (0.1, 0.1), (0.9, 0.9), (0.1, 0.9), (0.9, 0.1)];
    candidates.into_iter().map(|(fx, fy)| across(fx, fy)).find(|point| !root.contains(*point)).unwrap_or_else(|| across(0.75, 0.5))
}

/// A point target: input addresses the middle of its rect.
fn at(target: &Target, point: Pos2) -> Target {
    Target { node: target.node, rect: Rect::from_center_size(point, Vec2::ZERO) }
}

/// A press inside a part's hit area, also where it lies outside the root's rect, is taken by the component; on the
/// part a drag moves, a press off its middle moves the value.
fn parts_take_presses_in_their_hit_areas(subject: &Subject) -> Result<(), String> {
    let probe = subject.probe;
    for story in &subject.stories {
        let (fixture, plan) = subject.base(story).parts();
        let Some(id) = fixture.root_id() else { continue };
        let mut parts = crate::runtime::pointer::sensing_parts(fixture.ctx(), id, Sense::CLICK);
        for part in crate::runtime::pointer::sensing_parts(fixture.ctx(), id, Sense::DRAG) {
            if !parts.contains(&part) {
                parts.push(part);
            }
        }
        for part in parts {
            let mut fixture = input_fixture(story);
            let Some(target) = fixture.root_id().and_then(|id| fixture.part(id, part)) else { continue };
            if fixture.tree().data_of(target.node).is_none_or(|node| node.is_disabled()) || target.rect.area() <= 0.0 {
                continue;
            }
            let point = probe_point(target.rect, root_target(&fixture).rect);
            fixture.press(&at(&target, point));
            if !pointer::press_captured(fixture.ctx()) {
                return Err(format!(
                    "{}: a press at {point:?}, inside the hit area {:?} of the part {part:?}, was not taken",
                    story.name, target.rect
                ));
            }
        }
        if let (Plan::Drag(Grip { part, .. }), true) = (plan, probe.carries_value()) {
            let Some(grip) = fixture.root_id().and_then(|id| fixture.part(id, part)) else { continue };
            let off_middle = Pos2::new(grip.rect.center().x + 0.25 * grip.rect.width(), grip.rect.center().y);
            let mut fixture = input_fixture(story);
            let before = fixture.data().snapshot();
            let travel = 2.0 * input::max_click_dist(fixture.ctx());
            // A refused value is moved while the pointer drags and put back at the release, so it is read before the release.
            let refused = story.covers.contains(&Required::Invalid);
            let moved_live = Cell::new(false);
            fixture.drag_at_pausing(off_middle, Vec2::new(travel, 0.0), |fixture| {
                moved_live.set(!fixture.data().changed_since(&before).is_empty());
            });
            let moved = if refused { moved_live.get() } else { !fixture.data().changed_since(&before).is_empty() };
            if !moved {
                return Err(format!("{}: a drag from {off_middle:?}, inside the part {part:?}, moved no value", story.name));
            }
        }
    }
    Ok(())
}

const PORT: PartKey = PartKey::new("port");
const LATE: PartKey = PartKey::new("late");

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
}

/// The arena the order case shows: member `a` has a handle part over the corner of member `b`'s body. `reversed`
/// shows `b` first, so `a` is above it; `late` adds a member whose handle part covers the whole arena, revealed after the press began.
fn arena(reversed: bool, late: bool) -> ProbeRoot {
    let a = ProbeMember::new("a", rect(40.0, 40.0, 120.0, 60.0), Sense::CLICK)
        .part(ProbePart::new(PORT, rect(120.0, 60.0, 30.0, 30.0), Sense::CLICK).level(HitLevel::Handle, 0));
    let b = ProbeMember::new("b", rect(100.0, 50.0, 120.0, 60.0), Sense::CLICK);
    let mut root = ProbeRoot::new("arena", rect(0.0, 0.0, 400.0, 300.0), Sense::empty()).arena();
    let (first, second) = if reversed { (b, a) } else { (a, b) };
    root = root.member(first).member(second);
    if late {
        root = root.member(
            ProbeMember::new("late", rect(0.0, 0.0, 400.0, 300.0), Sense::empty())
                .part(ProbePart::new(LATE, rect(0.0, 0.0, 400.0, 300.0), Sense::CLICK).level(HitLevel::Handle, 0)),
        );
    }
    root
}

type Log = Rc<RefCell<Vec<RootReport>>>;

/// The frame closure of the arena scene, which logs each pass's report. `late` and `reversed` are read per pass.
fn scene(reversed: Rc<Cell<bool>>, late: Rc<Cell<bool>>) -> (impl FnMut(&mut Ui), Log) {
    let log: Log = Rc::default();
    let shared = log.clone();
    let frame = move |ui: &mut Ui| {
        let reports = show(ui, &[arena(reversed.get(), late.get())]);
        shared.borrow_mut().extend(reports);
    };
    (frame, log)
}

fn clicks_of(log: &Log, member: &'static str, part: Option<PartKey>) -> usize {
    log.borrow()
        .iter()
        .filter_map(|report| report.member(member))
        .filter(|report| part.map_or(report.press.phase, |key| report.phase(key)).is_click())
        .count()
}

/// Whether an earlier case of this process already ran the arena scenes for `shape`: the scenes do not depend on
/// the type, so one type of each shape runs them (22.4 gives the row to the shape).
fn claim(shape: ShapeKind) -> bool {
    static CLAIMED: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let name = match shape {
        ShapeKind::Leaf => "leaf",
        ShapeKind::Body => "body",
        _ => "items",
    };
    CLAIMED.get_or_init(Mutex::default).lock().is_ok_and(|mut claimed| claimed.insert(name))
}

/// A press where a member's handle part overlaps another member's body goes to the member shown last, whatever the
/// levels of the parts below it, and a press keeps its winner when a member whose handle covers the pointer is revealed after it began.
fn arena_winner_ignores_show_order(subject: &Subject) -> Result<(), String> {
    if !claim(subject.probe.shape) {
        return Ok(());
    }
    let overlap = Pos2::new(135.0, 75.0);
    for reversed in [false, true] {
        let (flag, late) = (Rc::new(Cell::new(reversed)), Rc::new(Cell::new(false)));
        let (mut show_frame, log) = scene(flag, late);
        let mut frames = Frames::new();
        frames.idle(2, &mut show_frame);
        log.borrow_mut().clear();
        pointer::click(&mut frames, overlap, &mut show_frame);
        let (handle, body) = (clicks_of(&log, "a", Some(PORT)), clicks_of(&log, "b", None));
        let expected = if reversed { (1, 0) } else { (0, 1) };
        if (handle, body) != expected {
            return Err(format!(
                "a press where a handle part overlaps another member's body gave the handle {handle} clicks and the body {body}, expected {expected:?}: the member shown last wins (show order reversed: {reversed})"
            ));
        }
    }
    let pressed = Pos2::new(60.0, 45.0);
    let (reversed, late) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
    let (mut show_frame, log) = scene(reversed, late.clone());
    let mut frames = Frames::new();
    frames.idle(2, &mut show_frame);
    log.borrow_mut().clear();
    frames.frame(vec![pointer::moved(pressed)], &mut show_frame);
    frames.frame(vec![pointer::press(pressed)], &mut show_frame);
    late.set(true);
    frames.idle(2, &mut show_frame);
    frames.frame(vec![pointer::release(pressed)], &mut show_frame);
    frames.frame(vec![], &mut show_frame);
    let (kept, stolen) = (clicks_of(&log, "a", None), clicks_of(&log, "late", Some(LATE)));
    if (kept, stolen) != (1, 0) {
        return Err(format!("a press kept by member a got {kept} clicks and a member revealed later {stolen}"));
    }
    Ok(())
}

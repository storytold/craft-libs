//! `Snap` (cw.C16): the canvas's one snapping, over its targets, at the tolerance of the view it shows, gated by View >
//! Snap and the suspend key of its options (Control of the OS by default, or Alt).

use craft_behavior::geometry::{SnapKind, SnapOptions, SnapSuspend, SnapTarget, SnapTargets};
use craft_behavior::kurbo::{Point, Rect as DocRect, Vec2};
use craft_behavior::{Axis, CancelReason, CanvasView};
use craft_keymap::{Mods, Platform};
use craft_ui::canvas::{CanvasTool, GestureSnap, Snap, ToolAction, ToolContext, ToolPath, ToolPointer, canvas_snap};
use craft_ui::primitives::Draw;
use craft_ui::testing::{ToolFixture, trial};
use egui::{Modifiers, Rect};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// The point the recording tool snaps each pass: 3 document units from the vertical guide at x = 100 and 2 from the
/// horizontal one at y = 200.
const NEAR: Point = Point::new(103.0, 198.0);

/// A tool that snaps [`NEAR`] through its context every pass, as a tool snaps the pointer.
#[derive(Default)]
struct Recorder {
    snapped: Vec<Point>,
}

impl CanvasTool for Recorder {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        self.snapped.push(cx.snap(NEAR));
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Claimed
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the Select Area... command")
    }
}

fn targets() -> Arc<SnapTargets> {
    let mut targets = SnapTargets::new();
    targets.add(SnapTarget::new(Axis::Y, 100.0, SnapKind::Guide));
    targets.add(SnapTarget::new(Axis::X, 200.0, SnapKind::Guide));
    Arc::new(targets)
}

/// A canvas at zoom 2 over `snap`, and the snap its body read in the last pass.
fn rig(snap: Snap) -> (ToolFixture<Recorder>, Rc<RefCell<Option<Snap>>>) {
    let seen = Rc::new(RefCell::new(None));
    let body = Rc::clone(&seen);
    let view = |room: Rect| CanvasView::new(room, Point::new(150.0, 150.0), 2.0);
    let fixture = ToolFixture::with_snap("Document", Recorder::default(), view, snap, move |_, cx| {
        *body.borrow_mut() = canvas_snap(&cx.env);
    });
    (fixture, seen)
}

/// The tool's last snapped point after one pass with `mods` held.
fn snapped_with(fixture: &mut ToolFixture<Recorder>, mods: Modifiers) -> Point {
    fixture.fixture().hold(mods).frame(Vec::new());
    *fixture.tool().snapped.last().expect("the tool ran")
}

// The body's snap and the tool's both snap within 8 screen points, 4 document units at zoom 2: a coordinate 3 units off
// a guide moves onto it and one 5 units off stays. A point snaps on both axes and a box by its edge, each with the
// lines to draw. View > Snap off returns the input. Control suspends snapping on macOS and Ctrl elsewhere, unless the
// gesture reads that key itself. With Alt as the options' suspend key, Alt suspends instead and Control does not.
#[trial]
fn snap_targets_snap_points_and_boxes_unless_control_suspends() {
    let options = SnapOptions::default();
    let (mut fixture, seen) = rig(Snap::new(targets(), options));
    fixture.fixture().os(Platform::Mac).frame(Vec::new());
    let snap = seen.borrow().clone().expect("the canvas publishes its snap to its body");
    assert_eq!(snap.options(), options);

    assert_eq!(snap.apply(Axis::Y, 103.0, Mods::NONE), 100.0, "3 units off at zoom 2 snaps");
    assert_eq!(snap.apply(Axis::Y, 105.0, Mods::NONE), 105.0, "5 units off at zoom 2 does not");
    let (point, lines) = snap.point(NEAR, Mods::NONE, GestureSnap::default());
    assert_eq!(point, Point::new(100.0, 200.0));
    assert_eq!(lines.len(), 2, "{lines:?}");
    let (by, lines) = snap.rect(DocRect::new(60.0, 10.0, 97.0, 40.0), Mods::NONE);
    assert_eq!(by, Vec2::new(3.0, 0.0), "the right edge meets the guide");
    assert!(lines.iter().any(|line| line.along == Axis::Y && line.position == 100.0 && line.kind == SnapKind::Guide), "{lines:?}");
    assert_eq!(snapped_with(&mut fixture, Modifiers::NONE), Point::new(100.0, 200.0), "the tool snaps through the host");

    // macOS: Control suspends, Cmd does not.
    assert_eq!(snap.apply(Axis::Y, 103.0, Mods::CTRL), 103.0);
    assert_eq!(snap.apply(Axis::Y, 103.0, Mods::CMD), 100.0);
    assert_eq!(
        snap.point(NEAR, Mods::CTRL, GestureSnap::default().reads_suspend_key(true)).0,
        Point::new(100.0, 200.0),
        "a gesture that reads Control keeps snapping"
    );
    assert_eq!(snap.rect(DocRect::new(60.0, 10.0, 97.0, 40.0), Mods::CTRL), (Vec2::ZERO, Vec::new()));
    assert_eq!(snapped_with(&mut fixture, Modifiers::CTRL), NEAR, "Control held suspends the tool's snap");

    // Elsewhere Ctrl is the command key, and it suspends.
    fixture.fixture().hold(Modifiers::NONE).os(Platform::Windows).frame(Vec::new());
    let snap = seen.borrow().clone().expect("the canvas publishes its snap to its body");
    assert_eq!(snap.apply(Axis::Y, 103.0, Mods::CMD), 103.0);
    assert_eq!(snap.apply(Axis::Y, 103.0, Mods::NONE), 100.0);
    assert_eq!(snapped_with(&mut fixture, Modifiers::CTRL), NEAR, "Ctrl held suspends the tool's snap");

    alt_suspends(options);
    snap_off(options);
}

/// With Alt as the suspend key, Alt suspends and Control no longer does, on either OS.
fn alt_suspends(options: SnapOptions) {
    let mut alt = options;
    alt.suspend = SnapSuspend::Alt;
    for os in [Platform::Mac, Platform::Windows] {
        let (mut fixture, seen) = rig(Snap::new(targets(), alt));
        fixture.fixture().os(os).frame(Vec::new());
        let snap = seen.borrow().clone().expect("the canvas publishes its snap to its body");
        assert_eq!(snap.options().suspend, SnapSuspend::Alt);
        assert_eq!(snap.apply(Axis::Y, 103.0, Mods::ALT), 103.0, "Alt suspends on {os:?}");
        assert_eq!(snap.apply(Axis::Y, 103.0, Mods::CTRL), 100.0, "Control keeps snapping on {os:?}");
        assert_eq!(snap.point(NEAR, Mods::ALT, GestureSnap::default()), (NEAR, Vec::new()), "on {os:?}");
        assert_eq!(snap.point(NEAR, Mods::CTRL, GestureSnap::default()).0, Point::new(100.0, 200.0), "on {os:?}");
        assert_eq!(
            snap.point(NEAR, Mods::ALT, GestureSnap::default().reads_suspend_key(true)).0,
            Point::new(100.0, 200.0),
            "a gesture that reads Alt keeps snapping on {os:?}"
        );
        assert_eq!(snap.rect(DocRect::new(60.0, 10.0, 97.0, 40.0), Mods::ALT), (Vec2::ZERO, Vec::new()), "on {os:?}");
        assert_eq!(snap.rect(DocRect::new(60.0, 10.0, 97.0, 40.0), Mods::CTRL).0, Vec2::new(3.0, 0.0), "on {os:?}");
        assert!(!snap.rect(DocRect::new(60.0, 10.0, 97.0, 40.0), Mods::CTRL).1.is_empty(), "Control snaps and reports its line on {os:?}");
        assert_eq!(snapped_with(&mut fixture, Modifiers::ALT), NEAR, "Alt held suspends the tool's snap on {os:?}");
        assert_eq!(snapped_with(&mut fixture, Modifiers::CTRL), Point::new(100.0, 200.0), "Control held snaps the tool on {os:?}");
    }
}

/// With View > Snap off nothing snaps.
fn snap_off(options: SnapOptions) {
    let mut off = options;
    off.on = false;
    let (mut fixture, seen) = rig(Snap::new(targets(), off));
    fixture.fixture().frame(Vec::new());
    let snap = seen.borrow().clone().expect("the canvas publishes its snap to its body");
    assert_eq!(snap.apply(Axis::Y, 103.0, Mods::NONE), 103.0, "View > Snap off");
    assert_eq!(snap.point(NEAR, Mods::NONE, GestureSnap::default()), (NEAR, Vec::new()));
    assert_eq!(snap.rect(DocRect::new(60.0, 10.0, 97.0, 40.0), Mods::NONE), (Vec2::ZERO, Vec::new()));
    assert_eq!(snapped_with(&mut fixture, Modifiers::NONE), NEAR);
}

/// The pixel grid: a snap with whole pixels rounds the point a tool snaps after the target snap, also with View > Snap
/// off and with Control suspending it, and leaves the offset of a moved box to the targets.
#[trial]
fn whole_pixels_round_snapped_points_but_not_box_offsets() {
    const OFF_GRID: Point = Point::new(110.4, 180.6);
    let mut off = SnapOptions::default();
    off.on = false;
    for options in [SnapOptions::default(), off] {
        let (mut fixture, seen) = rig(Snap::new(targets(), options).whole_pixels(true));
        fixture.fixture().os(Platform::Mac).frame(Vec::new());
        let snap = seen.borrow().clone().expect("the canvas publishes its snap to its body");
        assert_eq!(snap.point(OFF_GRID, Mods::NONE, GestureSnap::default()).0, Point::new(110.0, 181.0), "on {}", options.on);
        assert_eq!(snap.point(OFF_GRID, Mods::CTRL, GestureSnap::default()).0, Point::new(110.0, 181.0), "Control held, on {}", options.on);
        assert_eq!(snap.apply(Axis::Y, 110.4, Mods::NONE), 110.0);
        assert_eq!(snap.rect(DocRect::new(10.2, 10.0, 40.2, 40.0), Mods::NONE).0, Vec2::ZERO, "a box away from the targets keeps its offset");
    }
    let (mut fixture, _) = rig(Snap::new(targets(), SnapOptions::default()).whole_pixels(true));
    assert_eq!(snapped_with(&mut fixture, Modifiers::NONE), Point::new(100.0, 200.0), "a point on a target stays on it");
}

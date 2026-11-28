//! `TransformBox`, `TransformOverlay` and `TransformTool`: the parts the pointer takes, what each drag does, the
//! keys, the readout and the overlay's edits.

use super::presses::Presses;
use craft_behavior::geometry::quad::{BoxAllow, BoxHandles, BoxPart, BoxRules, BoxState, ScaleRule, TransformMode};
use craft_behavior::geometry::{SnapKind, SnapOptions, SnapSuspend, SnapTarget, SnapTargets};
use craft_behavior::kurbo::{Affine, Point, Rect};
use craft_behavior::{CanvasView, EditEvent, GesturePhase, RulerUnits, Ticks};
use craft_draw::ui::transform::{
    LineEnd, LineEndDrag, LineEnds, LineLook, RotateZone, TransformBox, TransformBoxOutput, TransformEdit, TransformOverlay, TransformReach,
    TransformSession, TransformSettings,
};
use craft_keymap::{Mods, Platform};
use craft_ui::canvas::{Canvas, RulerText, Snap};
use craft_ui::canvas::{Guide, GuideKey};
use craft_ui::canvas_controls::Guides;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::dialog::Dialog;
use craft_ui::runtime::env::UnitKind;
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::sections::{DialogRole, FooterButton};
use craft_ui::testing::{Fixture, ToolFixture, pointer, trial};
use egui::{Key, Modifiers, Pos2, Shape, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

/// The box before any drag, in document units.
const RECT: Rect = Rect::new(100.0, 80.0, 300.0, 220.0);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Seen {
    Begin,
    Update,
    Commit,
    Cancel,
    Set,
}

#[derive(Clone)]
struct Config {
    rules: BoxRules,
    zone: RotateZone,
    inside_moves: bool,
    reference_point: bool,
    pixel_grid: bool,
    mods: Option<Mods>,
    rotation: f64,
    flip: bool,
    guides: bool,
    snap: bool,
    /// The box is `TransformBox::text_frame`; `rules`, `zone`, `inside_moves` and `reference_point` are then unused.
    text_frame: bool,
    /// Show the frame only while hovered or while its part owns the pointer.
    hover_frame: bool,
    overflow: bool,
    /// The canvas's rulers on both axes.
    rulers: Option<RulerText>,
    /// The host keeps the box in f32, as a text layer keeps its box, and gives it back with its centre as the pivot.
    f32_host: bool,
    centre_mark: bool,
    crossed: bool,
    round_handles: bool,
    /// The host's handle side, `None` for the theme's.
    handle_side: Option<f32>,
    apply_reach: Option<f32>,
    line: Option<LineLook>,
    /// Snap targets beside the guides at [`SNAP_XS`].
    targets: Vec<SnapTarget>,
    /// The key that suspends the canvas's snapping.
    suspend: SnapSuspend,
    /// egui runs each frame twice and gives the frame's input to its first pass only.
    twice: bool,
    /// What of the box takes the pointer and the keys.
    reach: TransformReach,
    /// Whether Enter and Escape from the canvas session are the box's.
    session_commit: bool,
}

/// What the host keeps of the box and what the passes reported.
struct Host {
    state: BoxState,
    shown: Option<BoxState>,
    before: Option<BoxState>,
    events: Vec<Seen>,
    applied: usize,
    discarded: usize,
    picked: Option<BoxPart>,
    hovered: Option<BoxPart>,
    clicks: Vec<(BoxPart, Point)>,
    id: egui::Id,
    view: CanvasView,
    guide_events: usize,
    /// The snap lines the last pass reported.
    snapped: usize,
    /// Every free line end drag the passes reported.
    line_ends: Vec<LineEndDrag>,
}

struct Harness {
    fixture: Fixture,
    host: Rc<RefCell<Host>>,
    /// What the next pass shows; a trial changes it between frames (a modifier pressed mid-drag).
    config: Rc<RefCell<Config>>,
}

fn defaults() -> Config {
    Config {
        rules: BoxRules::default(),
        zone: RotateZone::Outside,
        inside_moves: true,
        reference_point: true,
        pixel_grid: false,
        mods: None,
        rotation: 0.0,
        flip: false,
        guides: false,
        snap: false,
        text_frame: false,
        hover_frame: false,
        overflow: false,
        rulers: None,
        f32_host: false,
        centre_mark: false,
        crossed: false,
        round_handles: false,
        handle_side: None,
        apply_reach: None,
        line: None,
        targets: Vec::new(),
        suspend: SnapSuspend::Control,
        twice: false,
        reach: TransformReach::Box,
        session_commit: true,
    }
}

/// `state` as a host that keeps it in f32 gives it back: each corner rounded to f32, the pivot at the centre.
fn kept_in_f32(state: &BoxState) -> BoxState {
    let quad = state.quad.map(|p| Point::new(f64::from(p.x as f32), f64::from(p.y as f32)));
    BoxState::new(quad, quad[0].midpoint(quad[2]))
}

fn harness(state: BoxState, setup: impl FnOnce(&mut Config)) -> Harness {
    let mut config = defaults();
    setup(&mut config);
    let shown_config = Rc::new(RefCell::new(config));
    let kept_config = Rc::clone(&shown_config);
    let host = Rc::new(RefCell::new(Host {
        state,
        shown: None,
        before: None,
        events: Vec::new(),
        applied: 0,
        discarded: 0,
        picked: None,
        hovered: None,
        clicks: Vec::new(),
        id: egui::Id::NULL,
        view: CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0),
        guide_events: 0,
        snapped: 0,
        line_ends: Vec::new(),
    }));
    let shown_host = Rc::clone(&host);
    let mut guides = vec![Guide::new(GuideKey::new(1), craft_behavior::Axis::Y, 300.0)];
    let mut targets = SnapTargets::new();
    for x in SNAP_XS {
        targets.add(SnapTarget::new(craft_behavior::Axis::Y, x, SnapKind::Guide));
    }
    for target in &shown_config.borrow().targets {
        targets.add(*target);
    }
    let targets = Arc::new(targets);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let config = shown_config.borrow().clone();
        // A box on the pixel grid alone has the grid through a snap that snaps to nothing.
        let mut options = SnapOptions::default();
        options.on = config.snap;
        options.suspend = config.suspend;
        let snap = Snap::new(Arc::clone(&targets), options).whole_pixels(config.pixel_grid);
        let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0)
            .rotation(config.rotation)
            .flip_x(config.flip);
        let state = {
            let mut host = shown_host.borrow_mut();
            host.view = view;
            host.state
        };
        let canvas_id = ui.scope_id().with("document canvas");
        let mut canvas = Canvas::new("Document").id(canvas_id).view(view);
        if config.snap || config.pixel_grid {
            canvas = canvas.snap(snap);
        }
        if let Some(ruler) = config.rulers {
            canvas = canvas.rulers(ruler, ruler);
        }
        let canvas = canvas.show(ui, |ui, body| {
            let mut guide_edits = 0;
            if config.guides {
                let output = Guides::new("Guides", canvas_id, Bind::bound(&mut guides)).interactive(true).show_at(ui, body.rect);
                guide_edits = usize::from(!output.edit.events.is_empty());
            }
            let frame_id = egui::Id::unique("hover frame");
            let frame_rect = egui::Rect::from_two_pos(view.to_screen(RECT.origin()), view.to_screen(Point::new(RECT.x1, RECT.y1))).expand(8.0);
            let hovered = ui.ctx().pointer_hover_pos().is_some_and(|pos| frame_rect.contains(pos));
            if config.hover_frame && !hovered && craft_ui::runtime::pointer::active_part(ui.ctx(), frame_id).is_none() {
                return None;
            }
            let mut control = if config.text_frame {
                TransformBox::text_frame("Free Transform", Bind::given(state), RECT).overflow(config.overflow)
            } else {
                TransformBox::new("Free Transform", Bind::given(state), RECT, 0.0)
                    .rules(config.rules)
                    .rotate_zone(config.zone)
                    .inside_moves(config.inside_moves)
                    .reference_point(config.reference_point)
                    .centre_mark(config.centre_mark)
                    .crossed(config.crossed)
                    .round_handles(config.round_handles)
            };
            if let Some(reach) = config.apply_reach.filter(|_| !config.text_frame) {
                control = control.apply_reach(reach);
            }
            if let Some(mods) = config.mods {
                control = control.mods(mods);
            }
            if let Some(line) = config.line {
                control = control.line(line);
            }
            if let Some(side) = config.handle_side {
                control = control.handle_side(side);
            }
            control = control.reach(config.reach).session_commit(config.session_commit);
            if config.hover_frame {
                control = control.id(frame_id);
            }
            Some((control.show_at(ui, body.rect), guide_edits))
        });
        if config.twice && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the frame runs twice");
        }
        if let Some(Some((output, guide_edits))) = canvas.inner {
            let mut host = shown_host.borrow_mut();
            host.shown = output.edit.shown.as_ref().and_then(|value| value.same()).map(|value| **value);
            host.guide_events += guide_edits;
            absorb(&mut host, &output, config.f32_host);
            host.clicks.extend(output.clicked);
        }
    });
    fixture.settle();
    Harness { fixture, host, config: kept_config }
}

/// Takes what a pass reported into the host: the snap lines, a free line end's drag, the apply and discard counts, the
/// part the keys act on and the part under the pointer, and each edit event into the box the host keeps.
fn absorb(host: &mut Host, output: &TransformBoxOutput, f32_host: bool) {
    host.snapped = output.snapped.len();
    if let Some(drag) = output.line_end {
        host.line_ends.push(drag);
    }
    host.id = output.edit.response.id;
    host.applied += usize::from(output.apply.is_some());
    host.discarded += usize::from(output.discard);
    (host.picked, host.hovered) = (output.picked, output.hovered);
    for event in &output.edit.events {
        match event {
            EditEvent::Begin { .. } => {
                host.events.push(Seen::Begin);
                host.before = Some(host.state);
            }
            EditEvent::Update { value, .. } => {
                host.events.push(Seen::Update);
                host.state = if f32_host { kept_in_f32(value) } else { *value };
            }
            EditEvent::Commit { after, .. } => {
                host.events.push(Seen::Commit);
                host.state = if f32_host { kept_in_f32(after) } else { *after };
            }
            EditEvent::Set { after, .. } => {
                host.events.push(Seen::Set);
                host.state = *after;
            }
            EditEvent::Cancel { .. } => {
                host.events.push(Seen::Cancel);
                host.state = host.before.take().unwrap_or(host.state);
            }
            _ => {}
        }
    }
}

fn upright() -> Harness {
    harness(BoxState::from_rect(RECT), |_| {})
}

fn frame(h: &mut Harness, events: Vec<egui::Event>) {
    h.fixture.frame(events);
}

impl Harness {
    fn screen(&self, p: Point) -> Pos2 {
        self.host.borrow().view.to_screen(p)
    }

    fn state(&self) -> BoxState {
        self.host.borrow().state
    }
}

/// The x of the vertical snap targets of a box with `Config::snap`: 10 units right of an inside point away from the
/// reference point and of the right edge.
const SNAP_XS: [f64; 2] = [160.0, 310.0];

/// A screen displacement.
fn by(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

#[trial]
fn a_pixel_grid_box_fed_back_as_state_does_not_drift_over_a_drag() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.pixel_grid = true);
    let from = h.screen(Point::new(300.0, 220.0));
    frame(&mut h, vec![pointer::moved(from)]);
    frame(&mut h, vec![pointer::press(from)]);
    for step in 1..=12 {
        frame(&mut h, vec![pointer::moved(from + by(0.4 * step as f32, 0.3 * step as f32))]);
    }
    let held = h.state();
    frame(&mut h, vec![pointer::release(from + by(4.8, 3.6))]);
    frame(&mut h, Vec::new());
    for corner in h.state().quad {
        assert!((corner.x - corner.x.round()).abs() < 1e-9 && (corner.y - corner.y.round()).abs() < 1e-9, "{corner:?} is on whole units");
    }
    assert!((h.state().quad[2].x - held.quad[2].x).abs() <= 1.0, "no drift from the held box to the committed one");
}

#[trial]
fn a_release_with_the_pointer_gone_is_not_a_click_outside() {
    let mut h = upright();
    let at = h.screen(Point::new(340.0, 260.0));
    frame(&mut h, vec![pointer::moved(at)]);
    frame(&mut h, vec![pointer::press(at)]);
    frame(&mut h, vec![pointer::gone()]);
    frame(&mut h, vec![pointer::release(at)]);
    frame(&mut h, Vec::new());
    assert_eq!(h.host.borrow().applied, 0);
}

#[trial]
fn the_control_key_turns_snapping_off_unless_it_distorts_the_dragged_corner() {
    // (os, the Control key there, the press point, whether Control changes that drag)
    let cases = [
        (Platform::Mac, Mods::CTRL, Point::new(150.0, 120.0), false),
        (Platform::Windows, Mods::CMD, Point::new(150.0, 120.0), false),
        (Platform::Windows, Mods::CMD, Point::new(300.0, 220.0), true),
    ];
    for (os, control, at, distorts) in cases {
        for held in [None, Some(control)] {
            let mut h = harness(BoxState::from_rect(RECT), |config| {
                config.snap = true;
                config.mods = held;
                // Unlinked, so a plain corner drag follows the pointer exactly and the snapped corner lands on 310.
                config.rules = BoxRules::default().with_aspect_linked(false);
            });
            h.fixture.os(os);
            let from = h.screen(at);
            frame(&mut h, vec![pointer::moved(from)]);
            frame(&mut h, vec![pointer::press(from)]);
            frame(&mut h, vec![pointer::moved(from + by(4.0, 0.0))]);
            frame(&mut h, vec![pointer::moved(from + by(7.0, 0.0))]);
            frame(&mut h, vec![pointer::release(from + by(7.0, 0.0))]);
            let moved = h.state().quad[2].x - RECT.x1;
            let snaps = held.is_none() || distorts;
            let expected = if snaps { 10.0 } else { 7.0 };
            assert!((moved - expected).abs() < 1e-6, "{os:?} {held:?} at {at:?}: moved {moved}, expected {expected}");
        }
    }
}

/// The box's rules with a plain corner or edge drag following the pointer exactly.
fn free_rules() -> BoxRules {
    BoxRules::default().with_scale(ScaleRule::Free).with_aspect_linked(false)
}

/// Moves the pointer to `at` and lets the pass settle, so the hovered part is the one under `at`.
fn hover(h: &mut Harness, at: Point) {
    let to = h.screen(at);
    frame(h, vec![pointer::moved(to)]);
    frame(h, Vec::new());
}

/// Presses at `from`, moves in two steps (the first starts the drag) to `to`, and releases there.
fn drag(h: &mut Harness, from: Point, to: Point) {
    let (start, end) = (h.screen(from), h.screen(to));
    frame(h, vec![pointer::moved(start)]);
    frame(h, vec![pointer::press(start)]);
    frame(h, vec![pointer::moved(start + (end - start) * 0.5)]);
    frame(h, vec![pointer::moved(end)]);
    frame(h, vec![pointer::release(end)]);
    frame(h, Vec::new());
}

/// A handle's accessible location follows the box in the same frame as its painted preview.
#[trial]
fn a_dragged_corner_reports_its_current_location() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.rules = free_rules());
    let start = h.screen(Point::new(RECT.x1, RECT.y0));
    let end = start + Vec2::new(-40.0, 20.0);
    frame(&mut h, vec![pointer::moved(start)]);
    frame(&mut h, vec![pointer::press(start)]);
    frame(&mut h, vec![pointer::moved(end)]);
    let first_move_passes = h.fixture.passes();
    frame(&mut h, vec![pointer::moved(end + Vec2::new(-10.0, 5.0))]);
    assert!(h.fixture.passes() <= first_move_passes, "a continued drag needs no extra pass");
    let shown = h.host.borrow().shown.unwrap();
    let painted_corner = h.screen(shown.quad[1]);
    let accessible_corner = h.fixture.by_label("Top right corner").unwrap().rect.center();
    assert!((accessible_corner - painted_corner).length() < 0.5, "accessible {accessible_corner:?}, painted {painted_corner:?}");
}

#[trial]
fn an_edge_takes_a_press_along_its_side_and_a_corner_wins_where_both_reach() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.rules = free_rules());
    // The right edge runs from (300, 80) to (300, 220); its middle is (300, 150).
    hover(&mut h, Point::new(301.0, 110.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(1)), "a press along the side, away from its middle and from the corners");
    hover(&mut h, Point::new(301.0, 84.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Corner(1)), "where a corner and the side both reach, the corner");
    hover(&mut h, Point::new(300.0, 150.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(1)), "the middle still takes the edge");
    drag(&mut h, Point::new(301.0, 110.0), Point::new(321.0, 110.0));
    let quad = h.state().quad;
    // A pressed handle moves by the drag and keeps the press's offset from the side, as measured on the reference
    // application (a corner pressed 4 px inside or 3 px outside its handle): pressed 1 px outside, the side lands 1 px
    // inside the pointer.
    assert!((quad[1].x - 320.0).abs() < 1e-6 && (quad[2].x - 320.0).abs() < 1e-6, "the dragged side moved by the drag: {quad:?}");
    assert!((quad[0].x - RECT.x0).abs() < 1e-6 && (quad[3].x - RECT.x0).abs() < 1e-6, "the far side stayed: {quad:?}");
}

/// A side's reach on a box whose inside moves, as measured on the reference application at 100 %: its midpoint handle
/// reaches 11 points every way, inside the box too; away from it the side's band runs from its line to 12 outside where
/// the view reads the pointer, half a pixel up and left of it: a press on the right or bottom side's line or inside it
/// moves the box, and one on the top or left side's line takes the side (2026-11-25, away from the handles: on the
/// right side's line and 3 and 6 inside it the box moved, 1, 3, 6 and 10 outside took the side, 13 outside took nothing;
/// on the top and the left side's line the side, a point inside them the box; on the bottom side's line the box, a
/// point below it the side; 13 above the top side nothing).
#[trial]
fn a_sides_midpoint_reaches_into_the_box_and_its_band_starts_on_the_line_as_the_view_reads_the_pointer() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.rules = free_rules());
    // The right edge runs from (300, 80) to (300, 220); its middle is (300, 150).
    let cases = [
        (Point::new(290.0, 150.0), Some(BoxPart::Edge(1)), "10 inside at the middle"),
        (Point::new(296.0, 160.0), Some(BoxPart::Edge(1)), "4 inside, 10 along from the middle"),
        (Point::new(300.0, 160.0), Some(BoxPart::Edge(1)), "on the line, 10 along from the middle"),
        (Point::new(288.0, 150.0), Some(BoxPart::Inside), "12 inside at the middle"),
        (Point::new(296.0, 164.0), Some(BoxPart::Inside), "4 inside, 14 along from the middle"),
        (Point::new(300.0, 110.0), Some(BoxPart::Inside), "on the right line, away from the middle"),
        (Point::new(298.0, 110.0), Some(BoxPart::Inside), "2 inside, away from the middle"),
        (Point::new(301.0, 110.0), Some(BoxPart::Edge(1)), "a point outside, away from the middle"),
        (Point::new(312.0, 110.0), Some(BoxPart::Edge(1)), "12 outside, away from the middle"),
        // The top edge runs from (100, 80) to (300, 80), the left from (100, 220) to (100, 80), the bottom along y 220.
        (Point::new(250.0, 80.0), Some(BoxPart::Edge(0)), "on the top line, away from the middle"),
        (Point::new(250.0, 81.0), Some(BoxPart::Inside), "a point inside the top line"),
        (Point::new(250.0, 69.0), Some(BoxPart::Edge(0)), "11 above the top line"),
        (Point::new(100.0, 110.0), Some(BoxPart::Edge(3)), "on the left line, away from the middle"),
        (Point::new(101.0, 110.0), Some(BoxPart::Inside), "a point inside the left line"),
        (Point::new(250.0, 220.0), Some(BoxPart::Inside), "on the bottom line, away from the middle"),
        (Point::new(250.0, 221.0), Some(BoxPart::Edge(2)), "a point below the bottom line"),
    ];
    for (at, part, what) in cases {
        hover(&mut h, at);
        assert_eq!(h.host.borrow().hovered, part, "{what}");
    }
    for (at, side, what) in [
        (Point::new(313.0, 110.0), BoxPart::Edge(1), "13 outside the right side is past the band"),
        (Point::new(250.0, 67.0), BoxPart::Edge(0), "13 above the top side is past the band"),
    ] {
        hover(&mut h, at);
        assert_ne!(h.host.borrow().hovered, Some(side), "{what}");
    }
}

/// Outside the box beside a corner, the corner takes a press within its own reach, read where the view reads the
/// pointer, and the side beside it takes one beyond (the reference application, 2026-11-25, at the top-right corner of
/// a 302 × 202 box in Scale mode; a press o px outside the right side and a px along it from the corner: o 7 a 9, o 3
/// a 11, o 1 a 11 and o 10 a 5 scaled the box from the corner, o 7 a 11, o 3 a 12 and o 1 a 13 from the right side.
/// The corner's stretch of the side's band took all of them).
#[trial]
fn beyond_a_corners_own_reach_the_side_beside_it_takes_the_press() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.rules = free_rules());
    // The top-right corner is (300, 80); the right side runs down from it.
    let cases = [
        (7.0, 9.0, BoxPart::Corner(1)),
        (3.0, 11.0, BoxPart::Corner(1)),
        (1.0, 11.0, BoxPart::Corner(1)),
        (10.0, 5.0, BoxPart::Corner(1)),
        (7.0, 11.0, BoxPart::Edge(1)),
        (3.0, 12.0, BoxPart::Edge(1)),
        (1.0, 13.0, BoxPart::Edge(1)),
    ];
    for (outside, along, part) in cases {
        hover(&mut h, Point::new(300.0 + outside, 80.0 + along));
        assert_eq!(h.host.borrow().hovered, Some(part), "{outside} outside the right side, {along} along it");
    }
    // Above and right of the corner, within its reach (7 right, 7 up).
    hover(&mut h, Point::new(307.0, 73.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Corner(1)));
}

/// Beside a corner, past the corner's own reach, the earlier of the corner's two sides in the order top, right, bottom,
/// left takes a press within 12 points past its end: everywhere outside both sides, and in the other side's band where
/// the press lies at least as far outside that side as along it from the corner (the reference application at 100 %,
/// 2026-11-25 and -26, Scale mode, 49 presses at the four corners of a 302 x 202 box; `u` is how far outside the
/// upright side the press lies, `v` how far outside the level one, negative on the box's side of that line).
#[trial]
fn past_its_end_the_earlier_side_takes_a_press_beside_a_corner() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.rules = free_rules());
    let (top, right, bottom, left) = (BoxPart::Edge(0), BoxPart::Edge(1), BoxPart::Edge(2), BoxPart::Edge(3));
    // The corner, which way is out of the box there on x and y, and what the measured presses took.
    type Corner<'a> = (f64, f64, f64, f64, &'a [(f64, f64, BoxPart)]);
    let corners: [Corner<'_>; 4] = [
        (
            RECT.x1,
            RECT.y0,
            1.0,
            -1.0,
            &[
                (10.0, -8.0, top),
                (12.0, -9.0, top),
                (11.0, -10.0, top),
                (12.0, -11.0, top),
                (9.0, -8.0, top),
                (7.0, -11.0, right),
                (8.0, -10.0, right),
                (10.0, -11.0, right),
                (9.0, -12.0, right),
                (9.0, 9.0, top),
                (12.0, 2.0, top),
                (12.0, 6.0, top),
                (5.0, 11.0, top),
                (3.0, 11.0, top),
                (-8.0, 10.0, top),
                (-11.0, 7.0, top),
            ],
        ),
        (
            RECT.x1,
            RECT.y1,
            1.0,
            1.0,
            &[
                (10.0, -8.0, right),
                (7.0, -11.0, right),
                (10.0, 10.0, right),
                (5.0, 11.0, right),
                (11.0, 5.0, right),
                (-8.0, 10.0, right),
                (-11.0, 7.0, bottom),
            ],
        ),
        (
            RECT.x0,
            RECT.y1,
            -1.0,
            1.0,
            &[(10.0, -8.0, bottom), (10.0, 10.0, bottom), (5.0, 11.0, bottom), (11.0, 5.0, bottom), (-8.0, 10.0, bottom), (-11.0, 7.0, bottom)],
        ),
        (
            RECT.x0,
            RECT.y0,
            -1.0,
            -1.0,
            &[(10.0, -8.0, top), (7.0, -11.0, left), (10.0, 10.0, top), (5.0, 11.0, top), (11.0, 5.0, top), (-8.0, 10.0, top), (-11.0, 7.0, top)],
        ),
    ];
    for (x, y, out_x, out_y, cases) in corners {
        for &(u, v, part) in cases {
            hover(&mut h, Point::new(x + out_x * u, y + out_y * v));
            assert_eq!(h.host.borrow().hovered, Some(part), "corner ({x}, {y}): {u} outside the upright side, {v} outside the level one");
        }
    }
    // 14 past a side's end is past every zone (top-right corner: 8 below and 8 above the top side's line).
    for (u, v) in [(14.0, -8.0), (14.0, 8.0), (20.0, 5.0)] {
        hover(&mut h, Point::new(RECT.x1 + u, RECT.y0 - v));
        assert!(!matches!(h.host.borrow().hovered, Some(BoxPart::Edge(_) | BoxPart::Corner(_))), "{u} outside the right side, {v} above the top one");
    }
}

/// On a mirrored box the order top, right, bottom, left is the screen's, not the picture's (the reference application,
/// 2026-11-26, after Flip Horizontal: 10 and 10 outside the bottom-left corner took the bottom side, outside the
/// bottom-right corner the right side).
#[trial]
fn beside_a_mirrored_boxs_corner_the_earlier_side_on_the_screen_takes_the_press() {
    // The picture's top-left corner stands at the top right: side 1 is on the left of the screen, side 3 on the right.
    let quad = [Point::new(RECT.x1, RECT.y0), Point::new(RECT.x0, RECT.y0), Point::new(RECT.x0, RECT.y1), Point::new(RECT.x1, RECT.y1)];
    let mut h = harness(BoxState::new(quad, RECT.center()), |config| config.rules = free_rules());
    hover(&mut h, Point::new(RECT.x0 - 10.0, RECT.y1 + 10.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(2)), "bottom left: the bottom side, before the left one");
    hover(&mut h, Point::new(RECT.x1 + 10.0, RECT.y1 + 10.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(3)), "bottom right: the side on the right, before the bottom one");
    hover(&mut h, Point::new(RECT.x1 + 10.0, RECT.y0 - 10.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(0)), "top right: the top side");
}

/// Beside a corner of a turned box (the reference application, 2026-11-26, a box turned 30 degrees, Scale mode; `u`
/// outside the side that was upright, `v` outside the one that was level, along the box's own axes): the zone is
/// within 12 points of the corner on the screen's axes, not the box's; at the top side's corners the top side takes
/// what lies within 12 of its line and the other side the rest past both ends; at the bottom corners the side whose
/// line is nearer takes the press.
#[trial]
fn beside_a_turned_boxs_corner_the_top_side_then_the_nearer_line_takes_the_press() {
    let (sin, cos) = 30f64.to_radians().sin_cos();
    let (centre, half_w, half_h) = (RECT.center(), RECT.width() / 2.0, RECT.height() / 2.0);
    // The box's own axes on the screen: along the top side, and down the right side.
    let at = |x: f64, y: f64| Point::new(centre.x + x * cos - y * sin, centre.y + x * sin + y * cos);
    let quad = [at(-half_w, -half_h), at(half_w, -half_h), at(half_w, half_h), at(-half_w, half_h)];
    let mut h = harness(BoxState::new(quad, centre), |config| config.rules = free_rules());
    let (top, right, bottom, left) = (Some(BoxPart::Edge(0)), Some(BoxPart::Edge(1)), Some(BoxPart::Edge(2)), Some(BoxPart::Edge(3)));
    // Which way is out of the box at the corner along its axes, then `u`, `v` and what the press took.
    let cases = [
        (1.0, -1.0, 10.0, -8.0, top),
        (1.0, -1.0, 12.0, -5.0, top),
        (1.0, -1.0, 7.0, 10.0, top),
        (1.0, -1.0, 3.0, 12.0, right),
        (1.0, -1.0, 4.0, 14.0, right),
        (1.0, -1.0, -8.0, 10.0, top),
        (1.0, 1.0, -6.0, 10.0, right),
        (1.0, 1.0, -3.0, 12.0, right),
        (1.0, 1.0, -9.0, 11.0, bottom),
        (1.0, 1.0, 12.0, 5.0, bottom),
        (1.0, 1.0, 10.0, -8.0, right),
        (-1.0, 1.0, 10.0, -6.0, bottom),
        (-1.0, 1.0, 4.0, 11.0, left),
        (-1.0, -1.0, 11.0, 4.0, top),
    ];
    for (out_x, out_y, u, v, part) in cases {
        hover(&mut h, at(out_x * (half_w + u), out_y * (half_h + v)));
        assert_eq!(h.host.borrow().hovered, part, "corner ({out_x}, {out_y}): {u} outside the upright side, {v} outside the level one");
    }
    // 10 and 10 along the box's axes is 13.7 from the corner on a screen axis: past every zone.
    for (out_x, out_y) in [(1.0, -1.0), (1.0, 1.0)] {
        hover(&mut h, at(out_x * (half_w + 10.0), out_y * (half_h + 10.0)));
        assert!(!matches!(h.host.borrow().hovered, Some(BoxPart::Edge(_) | BoxPart::Corner(_))), "corner ({out_x}, {out_y}): 10 and 10 outside");
    }
}

/// On a box crossed over itself (a bow tie: its top and bottom sides cross), each of the two sides that do not cross
/// takes a press outside its own triangle, not inside it (the reference application, 2026-11-25: on an hourglass made
/// by a Perspective corner dragged past its partner, a press 8 px outside the right side took the side; the winding of
/// the whole box, which the two triangles turn opposite ways, put that side's band inside its triangle).
#[trial]
fn a_bow_ties_uncrossed_sides_take_a_press_outside_their_own_triangle() {
    // The left corners have passed each other: the top side runs from (100, 220) to (300, 80), the bottom side from
    // (300, 220) to (100, 80); they cross at (200, 150).
    let quad = [Point::new(100.0, 220.0), Point::new(300.0, 80.0), Point::new(300.0, 220.0), Point::new(100.0, 80.0)];
    let mut h = harness(BoxState::new(quad, Point::new(200.0, 150.0)), |config| config.rules = free_rules());
    for (at, side, taken, what) in [
        (Point::new(308.0, 110.0), BoxPart::Edge(1), true, "8 outside the right side"),
        (Point::new(292.0, 110.0), BoxPart::Edge(1), false, "8 inside the right side"),
        (Point::new(92.0, 110.0), BoxPart::Edge(3), true, "8 outside the left side"),
        (Point::new(108.0, 110.0), BoxPart::Edge(3), false, "8 inside the left side"),
    ] {
        hover(&mut h, at);
        assert_eq!(h.host.borrow().hovered == Some(side), taken, "{what}: {:?}", h.host.borrow().hovered);
    }
}

/// On a box whose inside is the tool's (a text frame), the side's line itself still takes the side.
#[trial]
fn a_text_frames_side_takes_a_press_on_its_line() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.text_frame = true);
    hover(&mut h, Point::new(300.0, 110.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(1)), "on the line, away from the middle");
}

#[trial]
fn an_edge_band_grabs_a_side_from_inside_the_box_along_its_whole_length() {
    // A frame like a paragraph's: nothing inside moves it, so a press inside is the side's or nothing's.
    let paragraph = |band: f32| {
        harness(BoxState::from_rect(RECT), |config| {
            config.rules = free_rules().with_edge_band(band);
            config.inside_moves = false;
            config.reference_point = false;
        })
    };
    // The right edge runs from (300, 80) to (300, 220); 6 points inside it, away from its middle handle.
    let inside = Point::new(294.0, 110.0);
    let mut plain = paragraph(0.0);
    hover(&mut plain, inside);
    assert_ne!(plain.host.borrow().hovered, Some(BoxPart::Edge(1)), "without a band the side reaches outside only");

    let mut h = paragraph(8.0);
    hover(&mut h, inside);
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(1)), "within the band inside the box, the side");
    hover(&mut h, Point::new(200.0, 110.0));
    assert_eq!(h.host.borrow().hovered, None, "the middle of the box is past the band");
    drag(&mut h, inside, Point::new(314.0, 110.0));
    let quad = h.state().quad;
    // As its handle does, the side moves by the drag (20 points) and keeps the press's offset from it.
    assert!((quad[1].x - 320.0).abs() < 1e-6 && (quad[2].x - 320.0).abs() < 1e-6, "the grab dragged the side as its handle: {quad:?}");
    assert!((quad[0].x - RECT.x0).abs() < 1e-6, "the far side stayed: {quad:?}");
}

#[trial]
fn the_arrows_move_the_whole_box_after_a_handle_drag_leaves_focus_on_the_handle() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.rules = free_rules());
    drag(&mut h, Point::new(300.0, 220.0), Point::new(320.0, 240.0));
    let before = h.state().quad;
    h.fixture.key(egui::Key::ArrowRight, egui::Modifiers::NONE).settle();
    let after = h.state().quad;
    for (a, b) in before.iter().zip(&after) {
        assert!((b.x - a.x - 1.0).abs() < 1e-6 && (b.y - a.y).abs() < 1e-6, "every corner moved one unit right: {before:?} -> {after:?}");
    }
}

#[trial]
fn a_hidden_reference_point_still_follows_alt_and_the_centre_moves_a_small_box() {
    // An Alt press inside moves the hidden point to the press, and a drag moves it on.
    let mut h = harness(BoxState::from_rect(RECT), |config| {
        config.reference_point = false;
        config.mods = Some(Mods::ALT);
    });
    drag(&mut h, Point::new(150.0, 120.0), Point::new(170.0, 130.0));
    let pivot = h.state().pivot;
    // Half a unit up and left of the pointer, where the reference application puts it.
    assert!(pivot.distance(Point::new(169.5, 129.5)) < 1e-6, "the pivot follows the Alt drag: {pivot:?}");
    assert_eq!(h.state().quad, BoxState::from_rect(RECT).quad, "the box itself did not move");

    // A box smaller than two handle reaches: its middle is the whole box's.
    let small = Rect::new(100.0, 80.0, 110.0, 90.0);
    let mut h = harness(BoxState::from_rect(small), |config| config.reference_point = false);
    drag(&mut h, Point::new(105.0, 85.0), Point::new(135.0, 105.0));
    let corner = h.state().quad[0];
    assert!((corner.x - 130.0).abs() < 1.0 && (corner.y - 100.0).abs() < 1.0, "the small box moved by the drag: {corner:?}");
}

#[trial]
fn a_text_frame_edge_drag_stops_at_its_minimum_side_and_alt_mirrors_about_the_centre_also_mid_drag() {
    let edge = Point::new(300.0, 110.0);
    let past = Point::new(50.0, 110.0);
    let text = |mods: Option<Mods>| {
        harness(BoxState::from_rect(RECT), |config| {
            config.text_frame = true;
            config.mods = mods;
        })
    };

    let mut h = text(None);
    drag(&mut h, edge, past);
    let quad = h.state().quad;
    assert!((quad[1].x - quad[0].x - 4.0).abs() < 1e-6, "the dragged side stops 4 units from the opposite one: {quad:?}");

    let mut h = text(Some(Mods::ALT));
    drag(&mut h, edge, past);
    let quad = h.state().quad;
    assert!((quad[0].x - 198.0).abs() < 1e-6 && (quad[1].x - 202.0).abs() < 1e-6, "both sides stop 2 units from the centre: {quad:?}");

    // Alt pressed during the drag: the next move mirrors.
    let mut h = text(None);
    let (start, mid, end) = (h.screen(edge), h.screen(Point::new(290.0, 110.0)), h.screen(past));
    frame(&mut h, vec![pointer::moved(start)]);
    frame(&mut h, vec![pointer::press(start)]);
    frame(&mut h, vec![pointer::moved(mid)]);
    frame(&mut h, vec![pointer::moved(mid + by(-4.0, 0.0))]);
    h.config.borrow_mut().mods = Some(Mods::ALT);
    frame(&mut h, vec![pointer::moved(end)]);
    frame(&mut h, vec![pointer::release(end)]);
    let quad = h.state().quad;
    assert!((quad[0].x - 198.0).abs() < 1e-6 && (quad[1].x - 202.0).abs() < 1e-6, "Alt pressed mid-drag mirrors on the next move: {quad:?}");
}

// A host that applies every move and keeps the box in f32 (a text layer) gives each proposal back rounded: that is the
// proposal, not a change of the document under the drag, so every move of the drag reaches the host and the commit
// holds the last one.
#[trial]
fn a_text_frame_fed_back_through_an_f32_host_follows_every_move_to_the_commit() {
    let mut h = harness(BoxState::from_rect(RECT), |config| {
        config.text_frame = true;
        config.f32_host = true;
        config.rotation = 0.37;
    });
    let (start, mid, end) = (h.screen(Point::new(200.0, 220.0)), h.screen(Point::new(200.0, 250.0)), h.screen(Point::new(200.0, 280.0)));
    frame(&mut h, vec![pointer::moved(start)]);
    frame(&mut h, vec![pointer::press(start)]);
    frame(&mut h, vec![pointer::moved(mid)]);
    let first = h.state().quad[2].y;
    frame(&mut h, vec![pointer::moved(end)]);
    let second = h.state().quad[2].y;
    frame(&mut h, vec![pointer::release(end)]);
    frame(&mut h, Vec::new());
    let events = h.host.borrow().events.clone();
    assert!(!events.contains(&Seen::Cancel), "the rounded write-back cancels nothing: {events:?}");
    assert!((first - 250.0).abs() < 0.01 && (second - 280.0).abs() < 0.01, "each move reaches the host: {first} {second}");
    assert!((h.state().quad[2].y - 280.0).abs() < 0.01 && events.last() == Some(&Seen::Commit), "the commit holds the last move");
}

/// A hover-revealed frame remains declared through release after its drag leaves the original box.
#[trial]
fn a_hover_frame_commits_a_drag_released_outside() {
    let mut h = harness(BoxState::from_rect(RECT), |config| {
        config.text_frame = true;
        config.hover_frame = true;
    });
    let start = h.screen(Point::new(300.0, 150.0));
    let end = start + by(120.0, 0.0);
    frame(&mut h, vec![pointer::moved(start)]);
    frame(&mut h, vec![pointer::press(start)]);
    frame(&mut h, vec![pointer::moved(end)]);
    frame(&mut h, vec![pointer::release(end)]);
    frame(&mut h, Vec::new());
    let events = &h.host.borrow().events;
    assert!(events.contains(&Seen::Commit), "the host receives the release: {events:?}");
}

#[trial]
fn transform_box_shows_each_drag_proposal_in_the_same_frame() {
    let mut h = upright();
    let start = h.screen(Point::new(300.0, 220.0));
    frame(&mut h, vec![pointer::moved(start)]);
    frame(&mut h, vec![pointer::press(start)]);
    let mut previous = h.state();
    for delta in [Vec2::new(30.0, 21.0), Vec2::new(60.0, 42.0)] {
        frame(&mut h, vec![pointer::moved(start + delta)]);
        let host = h.host.borrow();
        assert_ne!(host.state, previous, "the drag changed the box");
        assert_eq!(host.shown, Some(host.state), "the reported preview follows this frame's edit");
        previous = host.state;
    }
}

/// A frame egui runs twice reports each move of a drag once: its second pass proposes the box the first did, and an
/// equal proposal reports no `Update` again, so a host that applies each `Update` (a text layer's box) applies it once
/// per move.
#[trial]
fn a_frame_run_twice_reports_each_move_of_a_drag_once() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.text_frame = true);
    let (start, mid, end) = (h.screen(Point::new(200.0, 220.0)), h.screen(Point::new(200.0, 250.0)), h.screen(Point::new(200.0, 280.0)));
    frame(&mut h, vec![pointer::moved(start)]);
    frame(&mut h, vec![pointer::press(start)]);
    h.config.borrow_mut().twice = true;
    h.host.borrow_mut().events.clear();
    frame(&mut h, vec![pointer::moved(mid)]);
    assert_eq!(h.fixture.passes(), 2, "the frame ran twice");
    frame(&mut h, vec![pointer::moved(end)]);
    frame(&mut h, Vec::new());
    let events = h.host.borrow().events.clone();
    let updates = events.iter().filter(|seen| **seen == Seen::Update).count();
    assert_eq!(updates, 2, "two moves report two updates, a held frame none: {events:?}");
    assert!((h.state().quad[2].y - 280.0).abs() < 0.01, "the last move reached the host");
}

#[trial]
fn a_scale_drag_reads_its_size_in_the_ruler_unit_and_percent_only_in_percent_rulers() {
    // A corner drag from 200 x 140 to 300 x 210 document units; the box follows the pointer.
    let held = |ruler: RulerText| {
        let mut h = harness(BoxState::from_rect(RECT), |config| {
            config.rules = free_rules();
            config.rulers = Some(ruler);
        });
        let start = h.screen(Point::new(300.0, 220.0));
        frame(&mut h, vec![pointer::moved(start)]);
        frame(&mut h, vec![pointer::press(start)]);
        frame(&mut h, vec![pointer::moved(start + by(50.0, 35.0))]);
        frame(&mut h, vec![pointer::moved(start + by(100.0, 70.0))]);
        h.fixture.texts().iter().map(|run| run.text.clone()).collect::<Vec<_>>()
    };
    // The readout paints each row as the label's run and then the value's run.
    let value_after = |texts: &[String], label: &str| -> String {
        let at = texts.iter().position(|text| text == label).unwrap_or_else(|| panic!("no {label:?} row: {texts:?}"));
        texts.get(at + 1).cloned().unwrap_or_default()
    };
    let inches = RulerText::new(RulerUnits::new(0.01, Ticks::decimal(false)), UnitKind::Inches);
    let texts = held(inches);
    let (width, height) = (value_after(&texts, "W:"), value_after(&texts, "H:"));
    assert!(width.contains("3.0"), "the width in inches: {width:?} in {texts:?}");
    assert!(height.contains("2.1"), "the height in inches: {height:?} in {texts:?}");
    assert!(!texts.iter().any(|text| text.contains('%')), "no percent in an inch ruler: {texts:?}");

    let percent = RulerText::new(RulerUnits::pixels(), UnitKind::Percent);
    let texts = held(percent);
    let width = value_after(&texts, "W:");
    assert!(width.contains("150.0") && width.contains('%'), "the scale in percent: {width:?} in {texts:?}");
}

#[trial]
fn the_whole_enlarged_overflow_handle_grabs_in_a_turned_view_and_a_short_box_keeps_its_middle_handles() {
    let turned = |state: BoxState| {
        harness(state, |config| {
            config.text_frame = true;
            config.overflow = true;
            config.rotation = 30.0;
        })
    };
    let mut h = turned(BoxState::from_rect(RECT));
    let corner = h.screen(Point::new(RECT.x1, RECT.y1));
    // The enlarged square is drawn upright on the screen: every part of it takes corner 2.
    for (dx, dy) in [(-5.0, -5.0), (5.0, -5.0), (-5.0, 5.0), (5.0, 5.0), (0.0, 0.0)] {
        frame(&mut h, vec![pointer::moved(corner + by(dx, dy))]);
        frame(&mut h, Vec::new());
        assert_eq!(h.host.borrow().hovered, Some(BoxPart::Corner(2)), "offset ({dx}, {dy}) on the enlarged handle");
    }

    let short = Rect::new(100.0, 80.0, 300.0, 90.0);
    let mut h = turned(BoxState::from_rect(short));
    hover(&mut h, Point::new(200.0, 80.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(0)), "the top middle handle of a short box");
    hover(&mut h, Point::new(200.0, 90.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(2)), "the bottom middle handle of a short box");
}

/// An overlay that offers only its move reach (the type tool's, 38 points): a press in the strip outside the frame is
/// held from the press and its drag moves the box; a press inside the frame, beside a handle or past the reach is
/// not the box's, so nothing is held and the box stays where it is.
#[trial]
fn a_move_only_overlay_takes_a_press_only_in_its_reach_and_moves_the_box() {
    let session = Rc::new(RefCell::new(TransformSession::<()>::new(RECT)));
    let view = Rc::new(RefCell::new(CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0)));
    let held = Rc::new(RefCell::new(Vec::new()));
    let (shown_session, shown_view, shown_held) = (Rc::clone(&session), Rc::clone(&view), Rc::clone(&held));
    let settings = TransformSettings::default();
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let at = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
        *shown_view.borrow_mut() = at;
        let shown = Canvas::new("Document").view(at).show(ui, |ui, body| {
            let session = shown_session.borrow();
            TransformOverlay::new("Type transform", &session, &settings).reach(TransformReach::MoveOnly { points: 38.0 }).show_at(ui, body.rect)
        });
        let Some(out) = shown.inner else { return };
        shown_held.borrow_mut().push(out.held);
        let mut session = shown_session.borrow_mut();
        for edit in out.edits {
            let edit: TransformEdit<()> = edit;
            session.apply(edit, 0.0);
        }
    });
    fixture.settle();
    let screen = |p: Point| view.borrow().to_screen(p);
    // Each pass's held part, from the press through the drag to the frame after the release.
    let mut drag = |from: Point, to: Point| {
        let (start, end) = (screen(from), screen(to));
        fixture.frame(vec![pointer::moved(start)]);
        held.borrow_mut().clear();
        fixture.frame(vec![pointer::press(start)]);
        let at_press = held.borrow().last().copied().flatten();
        fixture.frame(vec![pointer::moved(start + (end - start) * 0.5)]);
        fixture.frame(vec![pointer::moved(end)]);
        fixture.frame(vec![pointer::release(end)]);
        fixture.frame(Vec::new());
        let ever = held.borrow().iter().any(Option::is_some);
        (at_press, ever)
    };

    let untaken = [
        (Point::new(200.0, 150.0), "the inside"),
        (Point::new(RECT.x1 - 3.0, RECT.y1 - 3.0), "just inside a corner, where a handle would be"),
        (Point::new(RECT.x1 - 3.0, 150.0), "just inside a side's middle"),
        (Point::new(RECT.x1 + 60.0, 150.0), "past the reach"),
        (Point::new(RECT.x1 + 60.0, RECT.y1 + 60.0), "past a corner, where the box would rotate"),
        (Point::new(RECT.x1 + 30.0, RECT.y1 + 30.0), "diagonally off a corner, within the reach of each side's line but not of the corner"),
    ];
    for (at, what) in untaken {
        let before = session.borrow().quad();
        let (at_press, ever) = drag(at, Point::new(at.x + 30.0, at.y + 20.0));
        assert!(at_press.is_none() && !ever, "{what}: the box holds no press");
        assert_eq!(session.borrow().quad(), before, "{what}: the box stays");
    }

    // 24 points outside the right side: held from the press, and the drag translates the box.
    let size = |quad: [Point; 4]| (quad[1] - quad[0]).hypot() * (quad[3] - quad[0]).hypot();
    let before = session.borrow().quad();
    let (at_press, _) = drag(Point::new(RECT.x1 + 24.0, 150.0), Point::new(RECT.x1 + 54.0, 170.0));
    assert_eq!(at_press, Some(BoxPart::Inside), "the press in reach is held as a move before any movement");
    let after = session.borrow().quad();
    assert!(
        after[0].x > before[0].x + 1.0 && after[0].y > before[0].y + 1.0 && (size(after) - size(before)).abs() < 1e-6,
        "the box moves and keeps its size: {before:?} -> {after:?}"
    );

    // Within the reach of a corner, off both sides' ends: the round corner holds it.
    let corner = session.borrow().quad()[2];
    let (at_press, _) = drag(Point::new(corner.x + 20.0, corner.y + 20.0), Point::new(corner.x + 40.0, corner.y + 30.0));
    assert_eq!(at_press, Some(BoxPart::Inside), "a press within the reach of a corner moves the box");
}

/// The snap lines of a session box drag reach the tool host through `TransformTool`, as the crop frame's do: the host
/// reports them while a snapped edge drag runs and none once it ends. The edge stretches freely (the link off): one
/// that keeps the box's proportions does not snap.
#[trial]
fn the_session_boxs_snap_lines_reach_the_tool_host() {
    use craft_draw::ui::transform::TransformTool;
    use craft_ui::canvas::{ToolHost, tool_pass};
    let session = Rc::new(RefCell::new(TransformSession::<()>::new(RECT)));
    let view = Rc::new(RefCell::new(CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0)));
    let lines = Rc::new(RefCell::new(Vec::new()));
    let (shown_session, shown_view, shown_lines) = (Rc::clone(&session), Rc::clone(&view), Rc::clone(&lines));
    let mut targets = SnapTargets::new();
    for x in SNAP_XS {
        targets.add(SnapTarget::new(craft_behavior::Axis::Y, x, SnapKind::Guide));
    }
    let snap = Snap::new(Arc::new(targets), SnapOptions::default());
    let settings = TransformSettings { aspect_linked: false, ..TransformSettings::default() };
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let at = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
        *shown_view.borrow_mut() = at;
        let mut response = None;
        let canvas = Canvas::new("Document").view(at).snap(snap.clone()).show(ui, |ui, body| {
            let session = shown_session.borrow();
            let shown = TransformOverlay::new("Free Transform", &session, &settings).show_at(ui, body.rect);
            if let Some(pass) = tool_pass(&body.env) {
                let mut tool = TransformTool::new(&session, shown.snapped.clone());
                response = Some(ToolHost::new().run(ui, &pass, &mut tool));
            }
            shown
        });
        let Some(shown) = canvas.inner else { return };
        shown_lines.borrow_mut().push(response.unwrap_or_default().snapped.len());
        let mut session = shown_session.borrow_mut();
        for edit in shown.edits {
            session.apply(edit, 0.0);
        }
    });
    fixture.settle();
    let from = view.borrow().to_screen(Point::new(300.0, 150.0));
    let mut step = |events: Vec<egui::Event>| {
        lines.borrow_mut().clear();
        fixture.frame(events);
        lines.borrow().iter().copied().max().unwrap_or(0)
    };
    step(vec![pointer::moved(from)]);
    step(vec![pointer::press(from)]);
    step(vec![pointer::moved(from + by(4.0, 0.0))]);
    assert!(step(vec![pointer::moved(from + by(7.0, 0.0))]) > 0, "the right edge snapped to x 310: the host reports the line");
    step(vec![pointer::release(from + by(7.0, 0.0))]);
    assert_eq!(step(Vec::new()), 0, "the lines end with the drag");
}

/// The Ellipse tool's centre mark is drawn only: whatever a press at the middle of the box took without it, it takes
/// with it (the tool's with the inside off, the move with it on).
#[trial]
fn the_centre_mark_takes_no_press() {
    let middle = Point::new(200.0, 150.0);
    for inside_moves in [false, true] {
        let mut hovered = Vec::new();
        for centre_mark in [false, true] {
            let mut h = harness(BoxState::from_rect(RECT), |config| {
                config.reference_point = false;
                config.inside_moves = inside_moves;
                config.centre_mark = centre_mark;
            });
            hover(&mut h, middle);
            hovered.push(h.host.borrow().hovered);
        }
        assert_eq!(hovered[0], hovered[1], "inside moves {inside_moves}: the mark changes nothing a press there takes");
    }
}

/// Every two-point stroke among `shapes`, nested ones included, as its ends.
fn segments(shapes: &[egui::Shape], out: &mut Vec<(egui::Pos2, egui::Pos2)>) {
    for shape in shapes {
        match shape {
            egui::Shape::Vec(inner) => segments(inner, out),
            egui::Shape::LineSegment { points: [a, b], .. } => out.push((*a, *b)),
            egui::Shape::Path(path) if path.points.len() == 2 => out.push((path.points[0], path.points[1])),
            _ => {}
        }
    }
}

/// A crossed box draws both diagonals, corner to corner, as a rotated one does along its turned corners; a plain box
/// draws none. The diagonals take no press: the middle of the box is the inside's either way.
#[trial]
fn a_crossed_box_draws_its_diagonals_and_a_plain_one_none() {
    let turned = {
        let mut state = BoxState::from_rect(RECT);
        let centre = Point::new((RECT.x0 + RECT.x1) / 2.0, (RECT.y0 + RECT.y1) / 2.0);
        let turn = craft_behavior::kurbo::Affine::rotate_about(0.3, centre);
        state.quad = state.quad.map(|corner| turn * corner);
        state
    };
    for state in [BoxState::from_rect(RECT), turned] {
        let mut hovered = Vec::new();
        for crossed in [false, true] {
            let mut h = harness(state, |config| {
                config.reference_point = false;
                config.crossed = crossed;
            });
            let mut found = Vec::new();
            segments(&h.fixture.frame_shapes(), &mut found);
            let corner = |i: usize| h.screen(state.quad[i]);
            let joins = |a: egui::Pos2, b: egui::Pos2| {
                found.iter().any(|&(p, q)| ((p - a).length() < 0.5 && (q - b).length() < 0.5) || ((p - b).length() < 0.5 && (q - a).length() < 0.5))
            };
            for (a, b) in [(0, 2), (1, 3)] {
                assert_eq!(joins(corner(a), corner(b)), crossed, "crossed {crossed}: the diagonal from corner {a} to corner {b}");
            }
            hover(&mut h, state.quad[0].midpoint(state.quad[2]));
            hovered.push(h.host.borrow().hovered);
        }
        assert_eq!(hovered[0], hovered[1], "the cross changes nothing a press at the middle takes");
    }
}

/// A live line's controls: only its two ends take a press, each as the box handle it points to, so an end drag scales
/// the box from the other end. The hidden handles, the inside, the midpoint widget, the outside and the squares on an
/// arrowhead box's sides take none.
#[trial]
fn a_live_line_takes_a_press_only_at_its_ends_and_an_end_drag_scales_from_the_other_end() {
    // The line runs from the box's bottom left corner to its top right one.
    let (from, to) = (Point::new(RECT.x0, RECT.y1), Point::new(RECT.x1, RECT.y0));
    let line = LineLook::new(from, to);
    let mut h = harness(BoxState::from_rect(RECT), |config| {
        config.rules = free_rules();
        config.reference_point = false;
        config.zone = RotateZone::Band;
        config.line = Some(line);
    });
    hover(&mut h, to);
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Corner(1)), "the end at the top right stands for that corner");
    hover(&mut h, from);
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Corner(3)), "the end at the bottom left stands for that corner");
    let untaken = [
        (Point::new(RECT.x0, RECT.y0), "the hidden top left corner"),
        (Point::new(RECT.x1, RECT.y1), "the hidden bottom right corner"),
        (Point::new(200.0, RECT.y0), "the hidden top edge"),
        (from.midpoint(to), "the midpoint widget"),
        (Point::new(150.0, 190.0), "the inside"),
        (Point::new(RECT.x1 + 40.0, RECT.y1 + 40.0), "the outside"),
    ];
    for (at, what) in untaken {
        hover(&mut h, at);
        assert_eq!(h.host.borrow().hovered, None, "{what} takes no press");
    }
    drag(&mut h, to, to + kurbo_vec(20.0, -10.0));
    let quad = h.state().quad;
    assert!((quad[1].x - (RECT.x1 + 20.0)).abs() < 1e-6 && (quad[1].y - (RECT.y0 - 10.0)).abs() < 1e-6, "the dragged corner follows: {quad:?}");
    assert!((quad[3].x - RECT.x0).abs() < 1e-6 && (quad[3].y - RECT.y1).abs() < 1e-6, "the other end's corner stays: {quad:?}");
    assert_eq!(h.host.borrow().events.first(), Some(&Seen::Begin));
    assert_eq!(h.host.borrow().events.last(), Some(&Seen::Commit));

    // With an arrowhead, the box along the segment holds the line 10 units each side and 15 past each end.
    let length = from.distance(to);
    let (u, n) = ((to - from) / length, kurbo_vec(-(to.y - from.y) / length, (to.x - from.x) / length));
    let outline = [from - u * 15.0 - n * 10.0, to + u * 15.0 - n * 10.0, to + u * 15.0 + n * 10.0, from - u * 15.0 + n * 10.0];
    let mut h = harness(BoxState::from_rect(RECT), |config| {
        config.rules = free_rules();
        config.reference_point = false;
        config.zone = RotateZone::Band;
        config.line = Some(line.with_outline(Some(outline)));
    });
    hover(&mut h, to);
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Corner(1)), "the ends still take a press");
    for side in [outline[0].midpoint(outline[1]), outline[2].midpoint(outline[3])] {
        hover(&mut h, side);
        assert_eq!(h.host.borrow().hovered, None, "the square on the side at {side:?} takes no press");
    }
}

/// A press on the rotate band at any corner keeps rotating when the box it began on hands the drag to a box that
/// takes the whole outside (the Move tool's controls opening Free Transform): both declare the outside near corner
/// `i` under one part.
#[trial]
fn a_band_press_at_any_corner_rotates_after_the_hand_off_to_the_whole_outside() {
    for corner in 0..4usize {
        let mut h = harness(BoxState::from_rect(RECT), |config| {
            config.zone = RotateZone::Band;
            config.inside_moves = false;
            config.reference_point = false;
        });
        let quad = h.state().quad;
        let centre = quad[0].midpoint(quad[2]);
        let out = (quad[corner] - centre).normalize() * 14.0;
        let start = h.screen(quad[corner] + out);
        frame(&mut h, vec![pointer::moved(start)]);
        frame(&mut h, vec![pointer::press(start)]);
        {
            let mut config = h.config.borrow_mut();
            config.zone = RotateZone::Outside;
            config.inside_moves = true;
            config.reference_point = true;
        }
        let turn = by(-(start.y - h.screen(centre).y), start.x - h.screen(centre).x).normalized() * 30.0;
        frame(&mut h, vec![pointer::moved(start + turn * 0.5)]);
        frame(&mut h, vec![pointer::moved(start + turn)]);
        frame(&mut h, vec![pointer::release(start + turn)]);
        frame(&mut h, Vec::new());
        let turned = h.state().quad;
        assert!((turned[1].y - turned[0].y).abs() > 1.0, "corner {corner}: the box turns after the hand-off: {turned:?}");
        assert_eq!(h.host.borrow().events.last(), Some(&Seen::Commit), "corner {corner}");
    }
}

/// A border zone rotates only near the box's sides; further out the press is the box's and does nothing.
#[trial]
fn a_border_zone_rotates_only_within_its_reach_of_the_sides() {
    let press_and_turn = |h: &mut Harness, start: Pos2| {
        let centre = h.screen(RECT.center());
        let turn = by(-(start.y - centre.y), start.x - centre.x).normalized() * 30.0;
        frame(h, vec![pointer::moved(start)]);
        frame(h, vec![pointer::press(start)]);
        frame(h, vec![pointer::moved(start + turn * 0.5)]);
        frame(h, vec![pointer::moved(start + turn)]);
        frame(h, vec![pointer::release(start + turn)]);
        frame(h, Vec::new());
    };
    let side = |h: &Harness| h.screen(Point::new(RECT.x1, RECT.center().y));

    let mut near = harness(BoxState::from_rect(RECT), |config| config.zone = RotateZone::border(28.0));
    let start = side(&near) + by(14.0, 0.0);
    press_and_turn(&mut near, start);
    let turned = near.state().quad;
    assert!((turned[1].y - turned[0].y).abs() > 1.0, "a press within the reach turns the box: {turned:?}");
    assert_eq!(near.host.borrow().events.last(), Some(&Seen::Commit));

    let mut far = harness(BoxState::from_rect(RECT), |config| config.zone = RotateZone::border(28.0));
    let start = side(&far) + by(60.0, 0.0);
    press_and_turn(&mut far, start);
    assert!(far.host.borrow().events.is_empty(), "a press past the reach turns nothing: {:?}", far.host.borrow().events);
    assert_eq!(far.state(), BoxState::from_rect(RECT));
    frame(&mut far, vec![pointer::press(start)]);
    frame(&mut far, vec![pointer::release(start)]);
    frame(&mut far, Vec::new());
    assert_eq!(far.host.borrow().applied, 0, "a click past the reach applies nothing");
}

/// With an apply reach, a press that far or further outside the box's bounds applies as the button goes down, with
/// either button, and its drag turns nothing; a nearer press on the outside still turns the box. The reach is measured
/// along each axis from the bounds, so a press beside a corner counts by the further of its two distances, and a moved
/// or scaled box takes its reach along (the reference application, measured 2026-11-26: 201 points at a third and at
/// full zoom, in every mode).
#[trial]
fn a_press_beyond_the_apply_reach_applies_as_it_goes_down() {
    let reach = 60.0;
    let corner = |h: &Harness| h.screen(Point::new(RECT.x1, RECT.y1));
    let side = |h: &Harness| h.screen(Point::new(RECT.x1, RECT.center().y));
    let with_reach = || harness(BoxState::from_rect(RECT), |config| config.apply_reach = Some(reach));

    // Further than the reach right of the side: applies on the press, before any release, and the drag turns nothing.
    for secondary in [false, true] {
        let mut far = with_reach();
        let start = side(&far) + by(reach + 2.0, 0.0);
        frame(&mut far, vec![pointer::moved(start)]);
        frame(&mut far, vec![if secondary { pointer::secondary(start, true) } else { pointer::press(start) }]);
        assert_eq!(far.host.borrow().applied, 1, "the press applies as it goes down (secondary: {secondary})");
        frame(&mut far, vec![pointer::moved(start + by(0.0, 30.0))]);
        frame(&mut far, vec![if secondary { pointer::secondary(start + by(0.0, 30.0), false) } else { pointer::release(start + by(0.0, 30.0)) }]);
        frame(&mut far, Vec::new());
        assert!(far.host.borrow().events.is_empty(), "its drag turns nothing: {:?}", far.host.borrow().events);
        assert_eq!(far.state(), BoxState::from_rect(RECT));
        assert_eq!(far.host.borrow().applied, 1, "and the release applies nothing more");
    }

    // Nearer than the reach: the outside turns the box as before, and nothing applies.
    let mut near = with_reach();
    let start = side(&near) + by(reach - 6.0, 0.0);
    frame(&mut near, vec![pointer::moved(start)]);
    frame(&mut near, vec![pointer::press(start)]);
    frame(&mut near, vec![pointer::moved(start + by(0.0, 30.0))]);
    frame(&mut near, vec![pointer::release(start + by(0.0, 30.0))]);
    frame(&mut near, Vec::new());
    assert_eq!(near.host.borrow().applied, 0, "a press within the reach applies nothing");
    let turned = near.state().quad;
    assert!((turned[1].y - turned[0].y).abs() > 1.0, "it turns the box: {turned:?}");

    // Beside the corner the reach counts along each axis: within it both ways holds on, though further than the
    // reach from the corner itself; past it along one axis applies.
    for (offset, applies) in [(by(reach - 6.0, reach - 6.0), 0), (by(reach - 6.0, reach + 2.0), 1), (by(reach + 2.0, 10.0), 1)] {
        let mut h = with_reach();
        let start = corner(&h) + offset;
        frame(&mut h, vec![pointer::moved(start)]);
        frame(&mut h, vec![pointer::press(start)]);
        assert_eq!(h.host.borrow().applied, applies, "a press {offset:?} off the corner, before its release");
    }

    // Without a reach no press applies by its distance; a click outside applies on its release, as before.
    let mut plain = upright();
    let start = side(&plain) + by(reach + 2.0, 0.0);
    frame(&mut plain, vec![pointer::moved(start)]);
    frame(&mut plain, vec![pointer::press(start)]);
    assert_eq!(plain.host.borrow().applied, 0, "no reach: the press applies nothing");
}

/// A box whose rules refuse the outside (Scale mode) leaves it to the tool, but for the press beyond its apply reach:
/// the reference application ends a transform that way in every mode.
#[trial]
fn a_box_that_refuses_the_outside_still_applies_beyond_its_reach() {
    let reach = 60.0;
    let mut h = harness(BoxState::from_rect(RECT), |config| {
        config.apply_reach = Some(reach);
        config.rules = BoxRules::in_mode(TransformMode::Scale);
    });
    hover(&mut h, Point::new(RECT.x1 + f64::from(reach) - 10.0, RECT.center().y));
    assert_eq!(h.host.borrow().hovered, None, "nearer than the reach the outside is the tool's");
    let far = h.screen(Point::new(RECT.x1, RECT.center().y)) + by(reach + 2.0, 0.0);
    frame(&mut h, vec![pointer::moved(far)]);
    frame(&mut h, vec![pointer::press(far)]);
    assert_eq!(h.host.borrow().applied, 1, "the press beyond the reach applies as it goes down");
    frame(&mut h, vec![pointer::release(far)]);
    frame(&mut h, Vec::new());
    assert!(h.host.borrow().events.is_empty(), "and edits nothing: {:?}", h.host.borrow().events);
}

/// A level line's ends stand for the side handles they point to.
#[trial]
fn a_level_live_lines_ends_stand_for_the_sides() {
    let (from, to) = (Point::new(RECT.x0, 150.0), Point::new(RECT.x1, 150.0));
    let mut h = harness(BoxState::from_rect(RECT), |config| {
        config.rules = free_rules();
        config.reference_point = false;
        config.zone = RotateZone::Band;
        config.line = Some(LineLook::new(from, to));
    });
    hover(&mut h, to);
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(1)));
    hover(&mut h, from);
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(3)));
}

fn kurbo_vec(x: f64, y: f64) -> craft_behavior::kurbo::Vec2 {
    craft_behavior::kurbo::Vec2::new(x, y)
}

/// How far the box's first corner moved from where `RECT` puts it, in screen points (the view is upright at scale 1).
fn moved_by(h: &Harness) -> Vec2 {
    let corner = h.state().quad[0];
    by((corner.x - RECT.x0) as f32, (corner.y - RECT.y0) as f32)
}

#[trial]
fn a_drag_moves_the_box_once_the_pointer_has_gone_four_points_along_an_axis_and_then_by_its_whole_travel() {
    let mut h = upright();
    // Inside, away from the reference point at the box's centre.
    let from = h.screen(Point::new(150.0, 120.0));
    frame(&mut h, vec![pointer::moved(from)]);
    frame(&mut h, vec![pointer::press(from)]);
    frame(&mut h, Vec::new());
    assert!(h.host.borrow().events.is_empty(), "a press alone moves nothing: {:?}", h.host.borrow().events);
    // Under four points on both axes, straight or diagonal, the box stays.
    for offset in [by(3.0, 0.0), by(0.0, 3.0), by(3.0, 3.0), by(-3.5, 2.0)] {
        frame(&mut h, vec![pointer::moved(from + offset)]);
        assert_eq!(moved_by(&h), Vec2::ZERO, "{offset:?} moves nothing");
    }
    assert!(h.host.borrow().events.is_empty(), "no gesture yet: {:?}", h.host.borrow().events);
    // Four points along one axis applies the whole travel at once, and the box keeps up from there.
    for offset in [by(4.0, 1.0), by(2.0, 1.0), by(30.0, 12.0)] {
        frame(&mut h, vec![pointer::moved(from + offset)]);
        assert!((moved_by(&h) - offset).length() < 1e-3, "the box keeps the pointer's offset {offset:?}, moved {:?}", moved_by(&h));
    }
    frame(&mut h, vec![pointer::release(from + by(30.0, 12.0))]);
    frame(&mut h, Vec::new());
    let events = h.host.borrow().events.clone();
    assert_eq!(events.iter().filter(|seen| **seen == Seen::Begin).count(), 1, "one gesture: {events:?}");
    assert_eq!(events.iter().filter(|seen| **seen == Seen::Commit).count(), 1, "one commit: {events:?}");

    // A drag released under four points does nothing, on a handle as inside.
    for at in [Point::new(150.0, 120.0), Point::new(RECT.x1, (RECT.y0 + RECT.y1) / 2.0)] {
        let mut h = upright();
        let from = h.screen(at);
        frame(&mut h, vec![pointer::moved(from)]);
        frame(&mut h, vec![pointer::press(from)]);
        frame(&mut h, vec![pointer::moved(from + by(3.0, 3.0))]);
        frame(&mut h, vec![pointer::release(from + by(3.0, 3.0))]);
        frame(&mut h, Vec::new());
        assert_eq!(h.state(), BoxState::from_rect(RECT), "a 3 + 3 point drag at {at:?} leaves the box");
        assert!(h.host.borrow().events.is_empty(), "no gesture at {at:?}: {:?}", h.host.borrow().events);
    }
}

#[trial]
fn a_click_on_the_box_moves_nothing_and_a_jittery_click_outside_applies_the_box_as_it_was() {
    let mut h = upright();
    let slop = craft_ui::testing::input::slop(h.fixture.ctx());
    let inside = h.screen(Point::new(150.0, 120.0));
    frame(&mut h, vec![pointer::moved(inside)]);
    frame(&mut h, vec![pointer::press(inside)]);
    frame(&mut h, vec![pointer::release(inside)]);
    frame(&mut h, Vec::new());
    assert!(h.host.borrow().events.is_empty(), "a click on the box proposes nothing: {:?}", h.host.borrow().events);
    assert_eq!(h.state(), BoxState::from_rect(RECT));

    let outside = h.screen(Point::new(340.0, 260.0));
    frame(&mut h, vec![pointer::moved(outside)]);
    frame(&mut h, vec![pointer::press(outside)]);
    frame(&mut h, vec![pointer::moved(outside + by(slop / 2.0, slop / 3.0))]);
    frame(&mut h, vec![pointer::release(outside + by(slop / 2.0, slop / 3.0))]);
    frame(&mut h, Vec::new());
    assert!(h.host.borrow().events.is_empty(), "a jittery click outside turns nothing: {:?}", h.host.borrow().events);
    assert_eq!(h.state(), BoxState::from_rect(RECT), "the box stays as it was");
    assert_eq!(h.host.borrow().applied, 1, "the click outside applies");
}

#[trial]
fn inside_click_reports_press_point_but_drag_does_not() {
    let mut h = upright();
    let point = Point::new(150.0, 120.0);
    let inside = h.screen(point);
    frame(&mut h, vec![pointer::moved(inside)]);
    frame(&mut h, vec![pointer::press(inside)]);
    frame(&mut h, vec![pointer::release(inside)]);
    assert_eq!(h.host.borrow().clicks, vec![(BoxPart::Inside, point)]);
    frame(&mut h, vec![pointer::press(inside)]);
    frame(&mut h, vec![pointer::moved(inside + by(40.0, 0.0))]);
    frame(&mut h, vec![pointer::release(inside + by(40.0, 0.0))]);
    assert_eq!(h.host.borrow().clicks, vec![(BoxPart::Inside, point)], "a drag reports no click");
    let mut jitter = upright();
    let at = jitter.screen(point);
    let slop = craft_ui::testing::input::slop(jitter.fixture.ctx());
    frame(&mut jitter, vec![pointer::moved(at)]);
    frame(&mut jitter, vec![pointer::press(at)]);
    frame(&mut jitter, vec![pointer::moved(at + by(slop / 3.0, 0.0))]);
    frame(&mut jitter, vec![pointer::release(at + by(slop / 3.0, 0.0))]);
    assert_eq!(jitter.host.borrow().clicks, vec![(BoxPart::Inside, point)], "a release within slop reports the press point");
}

#[trial]
fn a_grip_click_reports_its_press_point() {
    let mut h = upright();
    let point = Point::new(RECT.x0, RECT.y0);
    let at = h.screen(point);
    frame(&mut h, vec![pointer::moved(at)]);
    frame(&mut h, vec![pointer::press(at)]);
    frame(&mut h, vec![pointer::release(at)]);
    assert_eq!(h.host.borrow().clicks, vec![(BoxPart::Corner(0), point)]);
    let edge = Point::new((RECT.x0 + RECT.x1) / 2.0, RECT.y0);
    let at = h.screen(edge);
    frame(&mut h, vec![pointer::moved(at)]);
    frame(&mut h, vec![pointer::press(at)]);
    frame(&mut h, vec![pointer::release(at)]);
    assert_eq!(h.host.borrow().clicks, vec![(BoxPart::Corner(0), point), (BoxPart::Edge(0), edge)]);
}

// A double click inside whose presses wobble within the slop and come back applies the box as it was: no move is
// committed before the apply.
#[trial]
fn a_jittery_double_click_inside_applies_with_no_move_committed() {
    let mut h = upright();
    let slop = craft_ui::testing::input::slop(h.fixture.ctx());
    let inside = h.screen(Point::new(150.0, 120.0));
    frame(&mut h, vec![pointer::moved(inside)]);
    for _ in 0..2 {
        frame(&mut h, vec![pointer::press(inside)]);
        frame(&mut h, vec![pointer::moved(inside + by(slop / 3.0, slop / 4.0))]);
        frame(&mut h, vec![pointer::moved(inside)]);
        frame(&mut h, vec![pointer::release(inside)]);
    }
    frame(&mut h, Vec::new());
    let events = h.host.borrow().events.clone();
    assert!(!events.contains(&Seen::Commit), "no move is committed: {events:?}");
    assert_eq!(h.state(), BoxState::from_rect(RECT), "the box stays as it was");
    assert_eq!(h.host.borrow().applied, 1, "the double click applies");
}

/// A secondary click on the session's box opens the mode menu (its rows are on the page) and is the box's alone: the
/// tool host under it asks for no menu of its own. With the menu closed again, a secondary click away from the box
/// opens none.
#[trial]
fn a_secondary_click_on_the_box_opens_the_mode_menu() {
    use craft_draw::ui::transform::TransformTool;
    use craft_ui::canvas::{ToolHost, tool_pass};
    let session = TransformSession::<()>::new(RECT);
    let settings = TransformSettings::default();
    let view = Rc::new(RefCell::new(CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0)));
    let host_asked = Rc::new(Cell::new(false));
    let (shown_view, asked) = (Rc::clone(&view), Rc::clone(&host_asked));
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let at = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
        *shown_view.borrow_mut() = at;
        Canvas::new("Document").view(at).show(ui, |ui, body| {
            let shown = TransformOverlay::new("Free Transform", &session, &settings).show_at(ui, body.rect);
            if let Some(pass) = tool_pass(&body.env) {
                let mut tool = TransformTool::new(&session, shown.snapped.clone());
                asked.set(asked.get() || ToolHost::new().run(ui, &pass, &mut tool).context.is_some());
            }
        });
    });
    fixture.settle();
    let right_click = |fixture: &mut Fixture, at: Pos2| {
        fixture.frame(vec![pointer::moved(at)]);
        fixture.frame(vec![pointer::secondary(at, true)]);
        fixture.frame(vec![pointer::secondary(at, false)]);
        fixture.settle();
    };
    let screen = |p: Point| view.borrow().to_screen(p);

    right_click(&mut fixture, screen(Point::new(1000.0, 1000.0)));
    assert!(fixture.by_label("Distort").is_none(), "a secondary click away from the box opens no mode menu");

    host_asked.set(false);
    right_click(&mut fixture, screen(Point::new(200.0, 150.0)));
    assert!(fixture.by_label("Distort").is_some(), "a secondary click on the box opens the mode menu");
    assert!(!host_asked.get(), "the tool host under the box asks for no menu");
}

/// A drag from inside snaps the moved box's edges, not the pointer: the right edge lands on the guide at 310 while the
/// pointer stays far from both guides.
#[trial]
fn a_box_moved_from_inside_snaps_its_edges() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.snap = true);
    let from = h.screen(Point::new(130.0, 120.0));
    frame(&mut h, vec![pointer::moved(from)]);
    frame(&mut h, vec![pointer::press(from)]);
    frame(&mut h, vec![pointer::moved(from + by(4.0, 0.0))]);
    frame(&mut h, vec![pointer::moved(from + by(7.0, 0.0))]);
    assert_eq!(h.host.borrow().snapped, 1, "the right edge's line");
    frame(&mut h, vec![pointer::release(from + by(7.0, 0.0))]);
    let moved = h.state().quad[0].x - RECT.x0;
    assert!((moved - 10.0).abs() < 1e-6, "the box moved {moved}, its right edge onto 310");
}

/// A move snaps in every box mode, Distort too, and from closer than the snapping distance only: the reference
/// application holds a box 7 px from a line and lets go of one 8 px away (2026-11-27).
#[trial]
fn a_box_moved_from_inside_snaps_in_every_mode_from_closer_than_the_snapping_distance() {
    for mode in [TransformMode::Scale, TransformMode::Rotate, TransformMode::Skew, TransformMode::Distort, TransformMode::Perspective] {
        // The pointer goes 7 down, the bottom edge to 227: a line 8 below it leaves the box with the pointer, one 7 below
        // it takes the edge.
        for (line, moved) in [(235.0, 7.0), (234.0, 14.0)] {
            let mut h = harness(BoxState::from_rect(RECT), |config| {
                config.snap = true;
                config.rules = BoxRules::in_mode(mode);
                config.targets = vec![SnapTarget::new(craft_behavior::Axis::X, line, SnapKind::Bounds)];
            });
            let from = h.screen(Point::new(130.0, 120.0));
            frame(&mut h, vec![pointer::moved(from)]);
            frame(&mut h, vec![pointer::press(from)]);
            frame(&mut h, vec![pointer::moved(from + by(0.0, 4.0))]);
            frame(&mut h, vec![pointer::moved(from + by(0.0, 7.0))]);
            frame(&mut h, vec![pointer::release(from + by(0.0, 7.0))]);
            let y = h.state().quad[0].y - RECT.y0;
            assert!((y - moved).abs() < 1e-6, "{mode:?}, a line at {line}: the box moved {y}");
        }
    }
}

/// A slanted box moved from inside snaps by its corners and its centre, not by the bounds around it: a corner inside
/// the bounds lands on a line and so does the centre, the middle of a slanted side does not (the reference
/// application, 2026-11-27: a box with its right side skewed down 60, each of the three brought 4 px from the
/// document's centre line).
#[trial]
fn a_slanted_box_moved_from_inside_snaps_by_its_corners_and_centre() {
    // The right side 60 lower: top-right corner y 140 (inside the bounds 80..280), centre y 180, right side's middle 210.
    let quad = [Point::new(100.0, 80.0), Point::new(300.0, 140.0), Point::new(300.0, 280.0), Point::new(100.0, 220.0)];
    // (the line's y, the pointer's way down, how far the box moves)
    for (line, pointer, moved, what) in [
        (147.0, 4.0, 7.0, "the top-right corner, 3 short of the line, lands on it"),
        (187.0, 4.0, 7.0, "the centre, 3 short of the line, lands on it"),
        (203.0, -4.0, -4.0, "the right side's middle 3 from the line: the box follows the pointer"),
    ] {
        let mut h = harness(BoxState::new(quad, Point::new(200.0, 180.0)), |config| {
            config.snap = true;
            config.targets = vec![SnapTarget::new(craft_behavior::Axis::X, line, SnapKind::Bounds)];
        });
        let from = h.screen(Point::new(150.0, 130.0));
        frame(&mut h, vec![pointer::moved(from)]);
        frame(&mut h, vec![pointer::press(from)]);
        frame(&mut h, vec![pointer::moved(from + by(0.0, pointer / 2.0))]);
        frame(&mut h, vec![pointer::moved(from + by(0.0, pointer))]);
        frame(&mut h, vec![pointer::release(from + by(0.0, pointer))]);
        let y = h.state().quad[0].y - quad[0].y;
        assert!((y - moved).abs() < 1e-6, "{what}: the box moved {y}");
    }
}

/// Free Transform's corner snaps to a layer edge and a guide alike; an explicit Distort corner snaps to guides only,
/// past the opposite diagonal too, where the box turns concave as the reference application's does.
#[trial]
fn a_distort_corner_snaps_to_guides_only_past_the_diagonal_too() {
    let layer_edge = SnapTarget::new(craft_behavior::Axis::X, 230.0, SnapKind::LayerEdge);
    for (mode, expected, lines) in [(TransformMode::Free, Point::new(310.0, 230.0), 2), (TransformMode::Distort, Point::new(310.0, 227.0), 1)] {
        let mut h = harness(BoxState::from_rect(RECT), |config| {
            config.snap = true;
            config.rules = BoxRules::in_mode(mode).with_aspect_linked(false);
            config.targets = vec![layer_edge];
        });
        let from = h.screen(Point::new(300.0, 220.0));
        frame(&mut h, vec![pointer::moved(from)]);
        frame(&mut h, vec![pointer::press(from)]);
        frame(&mut h, vec![pointer::moved(from + by(4.0, 4.0))]);
        frame(&mut h, vec![pointer::moved(from + by(7.0, 7.0))]);
        let corner = h.state().quad[2];
        assert!(corner.distance(expected) < 1e-6, "{mode:?}: {corner:?}, expected {expected:?}");
        assert_eq!(h.host.borrow().snapped, lines, "{mode:?}");
    }

    // The guides cross at (250, 180), past the diagonal from corner 1 to corner 3: corner 0 goes there and the box turns
    // concave (measured on the reference application, 2026-11-24: a Distort corner dragged past the diagonal).
    let mut h = harness(BoxState::from_rect(RECT), |config| {
        config.snap = true;
        config.rules = BoxRules::in_mode(TransformMode::Distort);
        config.targets =
            vec![SnapTarget::new(craft_behavior::Axis::Y, 250.0, SnapKind::Guide), SnapTarget::new(craft_behavior::Axis::X, 180.0, SnapKind::Guide)];
    });
    let from = h.screen(Point::new(100.0, 80.0));
    frame(&mut h, vec![pointer::moved(from)]);
    frame(&mut h, vec![pointer::press(from)]);
    frame(&mut h, vec![pointer::moved(from + by(4.0, 4.0))]);
    frame(&mut h, vec![pointer::moved(from + by(147.0, 97.0))]);
    let q = h.state().quad;
    assert!(q[0].distance(Point::new(250.0, 180.0)) < 1e-6, "the corner snaps to the crossing: {q:?}");
    let turns: Vec<f64> = (0..4).map(|i| (q[(i + 1) % 4] - q[i]).cross(q[(i + 2) % 4] - q[(i + 1) % 4])).collect();
    assert!(turns.iter().any(|t| *t > 0.0) && turns.iter().any(|t| *t < 0.0), "the box is concave: {q:?}");
    assert_eq!(h.host.borrow().snapped, 2, "both guide lines");
}

/// A Perspective mode corner snaps to guides only: next to the document's centre line or edge it stays where the drag
/// puts it, as a Distort corner does (the reference application, 2026-11-25: a corner dragged to
/// 3 px from the document's centre line, and one to 3 px from its top edge with its partner 1 px from the bottom edge,
/// stayed where the drag put them).
#[trial]
fn a_perspective_corner_snaps_to_guides_only() {
    for (kind, y) in [(SnapKind::Center, 227.0), (SnapKind::Bounds, 227.0), (SnapKind::Guide, 230.0)] {
        let mut h = harness(BoxState::from_rect(RECT), |config| {
            config.snap = true;
            config.rules = BoxRules::in_mode(TransformMode::Perspective);
            config.targets = vec![SnapTarget::new(craft_behavior::Axis::X, 230.0, kind)];
        });
        let from = h.screen(Point::new(300.0, 220.0));
        frame(&mut h, vec![pointer::moved(from)]);
        frame(&mut h, vec![pointer::press(from)]);
        frame(&mut h, vec![pointer::moved(from + by(0.0, 4.0))]);
        frame(&mut h, vec![pointer::moved(from + by(0.0, 7.0))]);
        let corner = h.state().quad[2];
        assert!((corner.y - y).abs() < 1e-6 && (corner.x - 300.0).abs() < 1e-6, "{kind:?}: {corner:?}");
    }
}

/// Off macOS Control is ⌘: on an explicit Distort corner it keeps the snapping, in Skew and Perspective it suspends it.
#[trial]
fn control_keeps_snapping_on_an_explicit_distort_corner_only() {
    for (mode, snaps) in [(TransformMode::Distort, true), (TransformMode::Skew, false), (TransformMode::Perspective, false)] {
        let mut h = harness(BoxState::from_rect(RECT), |config| {
            config.snap = true;
            config.mods = Some(Mods::CMD);
            config.rules = BoxRules::in_mode(mode);
        });
        h.fixture.os(Platform::Windows);
        let from = h.screen(Point::new(300.0, 220.0));
        frame(&mut h, vec![pointer::moved(from)]);
        frame(&mut h, vec![pointer::press(from)]);
        frame(&mut h, vec![pointer::moved(from + by(4.0, 0.0))]);
        frame(&mut h, vec![pointer::moved(from + by(7.0, 0.0))]);
        let x = h.state().quad[2].x;
        assert_eq!((x - 310.0).abs() < 1e-6, snaps, "{mode:?}: corner at x {x}");
    }
}

/// The rotate handle stands on its stem above the middle of the top side: a drag from it turns the box about its
/// reference point (Shift in rotate steps); the rest of the outside is the tool's, and a click there or on the handle
/// applies nothing. Where the rules refuse rotating there is no handle.
#[trial]
fn the_rotate_handle_turns_the_box_about_its_reference_point_and_the_rest_of_the_outside_is_the_tools() {
    let handle = |config: &mut Config| config.zone = RotateZone::Handle;
    // The points straight above the top side's middle that the handle takes.
    let handle_reach = |h: &mut Harness| -> Vec<Point> {
        (1..80)
            .map(|up| Point::new(200.0, RECT.y0 - f64::from(up)))
            .filter(|&at| {
                hover(h, at);
                h.host.borrow().hovered == Some(BoxPart::Outside)
            })
            .collect()
    };
    let mut h = harness(BoxState::from_rect(RECT), handle);
    let reach = handle_reach(&mut h);
    assert!(!reach.is_empty(), "the handle stands above the top side");
    assert!(reach[0].y < RECT.y0 - 8.0, "on a stem, clear of the top edge's handle: {reach:?}");
    let knob = reach[reach.len() / 2];

    let centre = RECT.center();
    // The rotate handle is a control of its own: it turns with the pointer as it is, not from where the view reads a
    // turn from outside the box.
    let end = Affine::rotate_about(20f64.to_radians(), centre) * knob;
    for (mods, angle, within) in [(None, 20.0, 0.01), (Some(Mods::SHIFT), 15.0, 1e-6)] {
        let mut h = harness(BoxState::from_rect(RECT), |config| {
            handle(config);
            config.mods = mods;
        });
        drag(&mut h, knob, end);
        let state = h.state();
        assert!((state.angle() - angle).abs() < within, "{mods:?}: turned {}, expected {angle}", state.angle());
        assert!(state.pivot.distance(centre) < 1e-9, "about the reference point: {:?}", state.pivot);
        assert_eq!(h.host.borrow().events.last(), Some(&Seen::Commit));
    }

    // Elsewhere outside: nothing of the box.
    let mut h = harness(BoxState::from_rect(RECT), handle);
    let away = Point::new(340.0, 260.0);
    hover(&mut h, away);
    assert_eq!(h.host.borrow().hovered, None, "the rest of the outside is the tool's");
    drag(&mut h, away, Point::new(360.0, 200.0));
    assert!(h.host.borrow().events.is_empty(), "a drag there turns nothing: {:?}", h.host.borrow().events);
    assert_eq!(h.state(), BoxState::from_rect(RECT));
    for at in [away, knob] {
        let at = h.screen(at);
        frame(&mut h, vec![pointer::moved(at)]);
        frame(&mut h, vec![pointer::press(at)]);
        frame(&mut h, vec![pointer::release(at)]);
        frame(&mut h, Vec::new());
    }
    assert_eq!(h.host.borrow().applied, 0, "a click outside or on the handle applies nothing");

    let mut h = harness(BoxState::from_rect(RECT), |config| {
        handle(config);
        config.rules = BoxRules::default().with_allow(BoxAllow::new(false, true, true));
    });
    assert!(handle_reach(&mut h).is_empty(), "no handle where rotating is refused");
}

/// With Alt as the snap suspend key, Alt suspends the snapping and is never the drag's: a corner does not scale about
/// the reference point and an Alt press inside moves the box, not the reference point. Control no longer suspends.
#[trial]
fn with_alt_as_the_snap_key_alt_suspends_snapping_and_no_alt_press_moves_the_reference_point() {
    // (os, the keys held, whether the corner snaps onto 310)
    let cases = [
        (Platform::Mac, None, true),
        (Platform::Mac, Some(Mods::ALT), false),
        (Platform::Mac, Some(Mods::CTRL), true),
        (Platform::Windows, Some(Mods::ALT), false),
    ];
    for (os, held, snaps) in cases {
        let mut h = harness(BoxState::from_rect(RECT), |config| {
            config.snap = true;
            config.suspend = SnapSuspend::Alt;
            config.mods = held;
            config.rules = BoxRules::default().with_aspect_linked(false);
        });
        h.fixture.os(os);
        let from = h.screen(Point::new(300.0, 220.0));
        frame(&mut h, vec![pointer::moved(from)]);
        frame(&mut h, vec![pointer::press(from)]);
        frame(&mut h, vec![pointer::moved(from + by(4.0, 0.0))]);
        frame(&mut h, vec![pointer::moved(from + by(7.0, 0.0))]);
        frame(&mut h, vec![pointer::release(from + by(7.0, 0.0))]);
        let quad = h.state().quad;
        let moved = quad[2].x - RECT.x1;
        let expected = if snaps { 10.0 } else { 7.0 };
        assert!((moved - expected).abs() < 1e-3, "{os:?} {held:?}: moved {moved}, expected {expected}");
        assert!(quad[0].distance(Point::new(RECT.x0, RECT.y0)) < 1e-6, "{os:?} {held:?}: no scaling about the reference point: {quad:?}");
    }

    let mut h = harness(BoxState::from_rect(RECT), |config| {
        config.snap = true;
        config.suspend = SnapSuspend::Alt;
        config.mods = Some(Mods::ALT);
    });
    // 7 right brings the right edge within reach of the guide at 310; the Alt drag stays unsnapped.
    drag(&mut h, Point::new(150.0, 120.0), Point::new(157.0, 120.0));
    let state = h.state();
    assert!(state.quad[0].distance(Point::new(107.0, RECT.y0)) < 1e-3, "the Alt drag moves the box, unsnapped: {:?}", state.quad);
    assert!(state.pivot.distance(Point::new(207.0, 150.0)) < 1e-3, "the reference point rides along: {:?}", state.pivot);
}

/// The resize arrows follow the box's own axes: corner 2's along its bisector, the right edge's along the box's x
/// axis, so a box turned by 30° shows other arrows than a level one.
#[trial]
fn a_turned_box_shows_resize_arrows_along_its_own_axes() {
    use craft_behavior::{Cursor, CursorIcon};
    let cursor_over = |h: &mut Harness, at: Point| {
        hover(h, at);
        craft_ui::runtime::cursor::current(h.fixture.ctx())
    };
    let level = BoxState::from_rect(RECT);
    let turned = level.turned_about_pivot(30f64.to_radians());
    // Level: the corner's arrow at 45°, the edge's at 0°. Turned by 30°: 75° and 30°.
    for (state, corner, edge) in
        [(level, CursorIcon::ResizeNwSe, CursorIcon::ResizeHorizontal), (turned, CursorIcon::ResizeVertical, CursorIcon::ResizeNwSe)]
    {
        let mut h = harness(state, |_| {});
        let at_corner = state.quad[2];
        let at_edge = state.quad[1].midpoint(state.quad[2]);
        assert_eq!(cursor_over(&mut h, at_corner), Some(Cursor::from(corner)), "corner 2 of {:?}", state.quad);
        assert_eq!(cursor_over(&mut h, at_edge), Some(Cursor::from(edge)), "the right edge of {:?}", state.quad);
    }
}

/// Free line ends: a drag of an end reports Begin, Update and Commit for that end only, at any angle, snapped through
/// the canvas's snap unless Control suspends it, and leaves the box value alone; Escape cancels the drag.
#[trial]
fn free_line_ends_move_one_end_and_report_the_drag_for_the_host_to_commit() {
    let (from, to) = (Point::new(RECT.x0, RECT.y1), Point::new(RECT.x1, RECT.y0));
    let line = LineLook::new(from, to).ends(LineEnds::Free);
    let free = |held: Option<Mods>| {
        let mut h = harness(BoxState::from_rect(RECT), |config| {
            config.snap = true;
            config.mods = held;
            config.reference_point = false;
            config.zone = RotateZone::Band;
            config.line = Some(line);
        });
        h.fixture.os(Platform::Mac);
        h
    };
    // 7 right and 30 up, at a slant: the end lands on the guide at 310.
    let dragged_to = to + kurbo_vec(7.0, -30.0);
    for (held, x) in [(None, 310.0), (Some(Mods::CTRL), 307.0)] {
        let mut h = free(held);
        drag(&mut h, to, dragged_to);
        let drags = h.host.borrow().line_ends.clone();
        let phases: Vec<GesturePhase> = drags.iter().map(|drag| drag.phase).collect();
        assert_eq!(phases.first(), Some(&GesturePhase::Begin), "{held:?}: {phases:?}");
        assert_eq!(phases.last(), Some(&GesturePhase::Commit), "{held:?}: {phases:?}");
        assert!(phases[1..phases.len() - 1].iter().all(|phase| *phase == GesturePhase::Update), "{held:?}: {phases:?}");
        assert!(drags.iter().all(|drag| drag.end == LineEnd::To), "only the dragged end moves: {drags:?}");
        let Some(last) = drags.last() else { panic!("no drag reported") };
        assert!(last.at.distance(Point::new(x, dragged_to.y)) < 1e-3, "{held:?}: the end at {:?}, expected x {x}", last.at);
        assert_eq!(h.state(), BoxState::from_rect(RECT), "the box value is unchanged");
        assert!(h.host.borrow().events.is_empty(), "no edit of the box: {:?}", h.host.borrow().events);
    }

    let mut h = free(None);
    let (start, end) = (h.screen(to), h.screen(dragged_to));
    frame(&mut h, vec![pointer::moved(start)]);
    frame(&mut h, vec![pointer::press(start)]);
    frame(&mut h, vec![pointer::moved(start + (end - start) * 0.5)]);
    frame(&mut h, vec![pointer::moved(end)]);
    h.fixture.key(egui::Key::Escape, egui::Modifiers::NONE);
    frame(&mut h, vec![pointer::release(end)]);
    frame(&mut h, Vec::new());
    let phases: Vec<GesturePhase> = h.host.borrow().line_ends.iter().map(|drag| drag.phase).collect();
    assert_eq!(phases.last(), Some(&GesturePhase::Cancel), "Escape cancels the end drag: {phases:?}");
    assert!(!phases.contains(&GesturePhase::Commit), "nothing is committed: {phases:?}");
    assert_eq!(h.host.borrow().discarded, 0, "Escape cancels the drag and does not close the box");
}

/// The reference point snaps to the turned box's nine points and to nothing else, with the canvas's snapping on or
/// off: 2 units off the guide at 160 it stays put.
#[trial]
fn the_reference_point_snaps_to_the_turned_boxs_points_only() {
    let turned = BoxState::from_rect(RECT).turned_about_pivot(30f64.to_radians());
    for snap in [true, false] {
        let mut h = harness(turned, |config| config.snap = snap);
        let centre = turned.pivot;
        drag(&mut h, centre, Point::new(158.0, 150.0));
        let pivot = h.state().pivot;
        assert!(pivot.distance(Point::new(158.0, 150.0)) < 1e-6, "snap {snap}: no pull to the guide: {pivot:?}");
        let corner = turned.quad[1];
        drag(&mut h, pivot, corner + kurbo_vec(-2.0, 1.0));
        let pivot = h.state().pivot;
        assert!(pivot.distance(corner) < 1e-6, "snap {snap}: onto the corner: {pivot:?} vs {corner:?}");
        assert_eq!(h.state().quad, turned.quad, "the box never moved");
    }
}

/// An edge handle snaps only along its drag (measured on the reference): a ⇧ side drag on a box in perspective, whose
/// handle starts 1 px off a snap line along the side, lands where it would with snapping off. Without that, the line
/// would pull the handle along the side, and on a box in perspective that turns the dragged side.
#[trial]
fn an_edge_handle_snaps_only_along_its_drag() {
    let quad = [Point::new(130.0, 80.0), Point::new(270.0, 80.0), Point::new(300.0, 220.0), Point::new(100.0, 220.0)];
    let shift = Mods { shift: true, ..Mods::NONE };
    let run = |snap: bool| {
        let mut h = harness(BoxState::new(quad, Point::new(200.0, 150.0)), |config| {
            config.snap = snap;
            config.mods = Some(shift);
            config.targets = vec![SnapTarget::new(craft_behavior::Axis::X, 149.0, SnapKind::Guide)];
        });
        let from = h.screen(Point::new(285.0, 150.0));
        frame(&mut h, vec![pointer::moved(from)]);
        frame(&mut h, vec![pointer::press(from)]);
        frame(&mut h, vec![pointer::moved(from + by(4.0, 0.0))]);
        frame(&mut h, vec![pointer::moved(from + by(50.0, 0.0))]);
        frame(&mut h, vec![pointer::release(from + by(50.0, 0.0))]);
        h.state().quad
    };
    let (off, on) = (run(false), run(true));
    assert!(off.iter().zip(on).all(|(a, b)| a.distance(b) < 1e-6), "snapping off {off:?}, on {on:?}");
}

/// An ⌥ click inside the box puts the reference point half a unit up and left of the click (measured on the
/// reference: a click at (250, 200) puts it at (249.5, 199.5), where its ⌥ drags then scale about).
#[trial]
fn an_alt_click_puts_the_reference_point_half_a_unit_up_and_left() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.mods = Some(Mods::ALT));
    let at = h.screen(Point::new(150.0, 120.0));
    frame(&mut h, vec![pointer::moved(at)]);
    frame(&mut h, vec![pointer::press(at)]);
    frame(&mut h, vec![pointer::release(at)]);
    let pivot = h.state().pivot;
    assert!(pivot.distance(Point::new(149.5, 119.5)) < 1e-6, "{pivot:?}");
}

/// With the reference point hidden, an ⌥ press away from the handles draws no point while the button is down, neither at
/// the old place nor at the new one, as in the reference application; the click moves it, still hidden, and an ⌥ drag
/// draws it where the drag has it.
#[trial]
fn a_held_alt_press_draws_no_hidden_reference_point_until_it_drags() {
    let mut h = harness(BoxState::from_rect(RECT), |config| {
        config.reference_point = false;
        config.mods = Some(Mods::ALT);
    });
    let glyphs = |h: &mut Harness| {
        let mut found = Vec::new();
        circles(&h.fixture.frame_shapes(), &mut found);
        found
    };
    let at = h.screen(Point::new(150.0, 120.0));
    frame(&mut h, vec![pointer::moved(at)]);
    frame(&mut h, vec![pointer::press(at)]);
    frame(&mut h, Vec::new());
    assert_eq!(glyphs(&mut h), Vec::new(), "no point while the press is held, the old one included");
    frame(&mut h, vec![pointer::release(at)]);
    assert_eq!(glyphs(&mut h), Vec::new(), "the hidden point stays hidden after the click");
    assert!(h.state().pivot.distance(Point::new(149.5, 119.5)) < 1e-6, "the click moved it: {:?}", h.state().pivot);

    let to = h.screen(Point::new(180.0, 140.0));
    frame(&mut h, vec![pointer::press(at)]);
    frame(&mut h, vec![pointer::moved(at + by(6.0, 4.0))]);
    frame(&mut h, vec![pointer::moved(to)]);
    let drawn = glyphs(&mut h);
    let pivot = h.screen(h.state().pivot);
    assert!(!drawn.is_empty() && drawn.iter().all(|c| (*c - pivot).length() < 0.5), "drawn at the dragged point {pivot:?}: {drawn:?}");
    frame(&mut h, vec![pointer::release(to)]);
}

fn circles(shapes: &[egui::Shape], out: &mut Vec<egui::Pos2>) {
    for shape in shapes {
        match shape {
            egui::Shape::Vec(inner) => circles(inner, out),
            egui::Shape::Circle(circle) => out.push(circle.center),
            _ => {}
        }
    }
}

/// A corner that keeps the box's proportions stops where the drag puts it, as in the reference application (a corner
/// dragged to 2 px from the document's centre, or 3 px from its edge, is not drawn to them); one that stretches
/// freely still snaps.
#[trial]
fn a_proportional_corner_does_not_snap_and_a_free_one_does() {
    for linked in [true, false] {
        let mut h = harness(BoxState::from_rect(RECT), |config| {
            config.snap = true;
            config.rules = BoxRules::default().with_aspect_linked(linked);
        });
        let from = h.screen(Point::new(300.0, 220.0));
        frame(&mut h, vec![pointer::moved(from)]);
        frame(&mut h, vec![pointer::press(from)]);
        frame(&mut h, vec![pointer::moved(from + by(4.0, 0.0))]);
        frame(&mut h, vec![pointer::moved(from + by(7.0, 0.0))]);
        frame(&mut h, vec![pointer::release(from + by(7.0, 0.0))]);
        let x = h.state().quad[2].x;
        // Linked, the corner follows the drag's projection on the diagonal: 200 × 140 scaled by 1 + 1400/59600. Free,
        // it snaps to the guide at 310.
        let expected = if linked { 100.0 + 200.0 * (1.0 + 7.0 * 200.0 / (200.0 * 200.0 + 140.0 * 140.0)) } else { 310.0 };
        assert!((x - expected).abs() < 1e-6, "linked {linked}: the corner at x {x}, not {expected}");
    }
}

/// An edge that keeps the box's proportions stops where the drag puts it too, as in the reference application (an
/// edge dragged to 2 px from the document's centre column, or 6 px from its edge, is not drawn to them); a free (⇧)
/// one still snaps.
#[trial]
fn a_proportional_edge_does_not_snap_and_a_free_one_does() {
    for free in [false, true] {
        let mut h = harness(BoxState::from_rect(RECT), |config| {
            config.snap = true;
            config.mods = free.then_some(Mods::SHIFT);
        });
        let from = h.screen(Point::new(300.0, 150.0));
        frame(&mut h, vec![pointer::moved(from)]);
        frame(&mut h, vec![pointer::press(from)]);
        frame(&mut h, vec![pointer::moved(from + by(4.0, 0.0))]);
        frame(&mut h, vec![pointer::moved(from + by(7.0, 0.0))]);
        frame(&mut h, vec![pointer::release(from + by(7.0, 0.0))]);
        let x = h.state().quad[1].x;
        let expected = if free { 310.0 } else { 307.0 };
        assert!((x - expected).abs() < 1e-6, "free {free}: the edge at x {x}, not {expected}");
    }
}

#[trial]
fn a_shown_box_takes_no_press_and_no_key_and_leaves_both_to_the_tool() {
    let reported = Rc::new(Cell::new(0usize));
    let sink = Rc::clone(&reported);
    // The view shows the room's centre as the document point of the same numbers, so document and screen points
    // coincide.
    let view = |room: egui::Rect| CanvasView::new(room, Point::new(f64::from(room.center().x), f64::from(room.center().y)), 1.0);
    let mut fixture = ToolFixture::with_view("Document", Presses::default(), view, move |ui, body| {
        let shown = TransformBox::new("Free Transform", Bind::given(BoxState::from_rect(RECT)), RECT, 0.0)
            .reach(TransformReach::Shown)
            .show_at(ui, body.rect);
        sink.set(sink.get() + usize::from(shown.apply.is_some()) + usize::from(shown.discard) + shown.edit.events.len());
    });
    // A handle, the inside and the outside of the box.
    for at in [Pos2::new(300.0, 220.0), Pos2::new(200.0, 150.0), Pos2::new(350.0, 270.0)] {
        fixture.press_at(at).release_at(at);
    }
    assert_eq!(fixture.tool().count, 3, "every press reaches the tool");
    for key in [Key::ArrowRight, Key::Enter, Key::Escape] {
        fixture.fixture().key(key, Modifiers::NONE);
    }
    assert_eq!(reported.get(), 0, "the box reported no edit, apply or discard");
    assert!(fixture.fixture().by_label("Bottom right corner").is_none(), "a shown box has no part in the accessibility tree");
    let framed = |shape: &Shape| matches!(shape, Shape::Path(path) if path.closed && path.points.len() == 4);
    assert!(fixture.fixture().frame_shapes().iter().any(framed), "the box still paints its frame");
}

/// The host of a distort pad in a dialog: the box it keeps, what the box reported and how the dialog closed.
struct Pad {
    host: Host,
    closed: Option<CloseReason>,
}

/// A Free Distort pad as the effects dialogs show it, with `session_commit` off, in an open dialog whose Enter is OK
/// and whose Escape cancels. The pad's view is kept in the host.
fn pad() -> (Fixture, Rc<RefCell<Pad>>) {
    let state = BoxState::from_rect(RECT);
    let host = Host {
        state,
        shown: None,
        before: None,
        events: Vec::new(),
        applied: 0,
        discarded: 0,
        picked: None,
        hovered: None,
        clicks: Vec::new(),
        id: egui::Id::NULL,
        view: CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0),
        guide_events: 0,
        snapped: 0,
        line_ends: Vec::new(),
    };
    let pad = Rc::new(RefCell::new(Pad { host, closed: None }));
    let shown = Rc::clone(&pad);
    let mut fixture = Fixture::new(move |ui| {
        let mut open = true;
        let dialog = Dialog::new("Free Distort")
            .opened(Bind::bound(&mut open))
            .close_on_enter(true)
            .buttons([FooterButton::new("OK", DialogRole::Accept), FooterButton::new("Cancel", DialogRole::Cancel)])
            .show(ui, |ui, _| {
                let (room, _) = ui.allocate_exact_size(egui::vec2(400.0, 300.0), egui::Sense::hover());
                let view = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
                let state = {
                    let mut pad = shown.borrow_mut();
                    pad.host.view = view;
                    pad.host.state
                };
                let canvas = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
                    TransformBox::new("Free Transform", Bind::given(state), RECT, 0.0)
                        .rules(BoxRules::in_mode(TransformMode::Distort))
                        .reference_point(false)
                        .inside_moves(false)
                        .session_commit(false)
                        .show_at(ui, body.rect)
                });
                if let Some(output) = canvas.inner {
                    absorb(&mut shown.borrow_mut().host, &output, false);
                }
            });
        if dialog.output.closed.is_some() {
            shown.borrow_mut().closed = dialog.output.closed;
        }
    });
    fixture.settle();
    (fixture, pad)
}

#[trial]
fn a_box_without_session_commit_leaves_enter_and_escape_to_the_dialog() {
    for (key, closed) in [(Key::Enter, CloseReason::Activated), (Key::Escape, CloseReason::Escape)] {
        let (mut fixture, pad) = pad();
        // A click on a handle gives the box the keyboard, where the session's Enter and Escape would be its own.
        let corner = pad.borrow().host.view.to_screen(Point::new(300.0, 220.0));
        fixture.frame(vec![pointer::moved(corner)]);
        fixture.frame(vec![pointer::press(corner)]);
        fixture.frame(vec![pointer::release(corner)]);
        fixture.key(Key::ArrowRight, Modifiers::NONE);
        let moved = pad.borrow().host.state.quad[0].x - RECT.x0;
        assert!((moved - 1.0).abs() < 1e-6, "an arrow still moves the box by one unit: {moved}");
        fixture.key(key, Modifiers::NONE);
        fixture.settle();
        let pad = pad.borrow();
        assert_eq!((pad.host.applied, pad.host.discarded), (0, 0), "{key:?}: the box takes neither Enter nor Escape");
        assert_eq!(pad.closed, Some(closed), "{key:?}: the dialog around the canvas gets it");
    }
}

#[trial]
fn a_drags_escape_still_cancels_it_in_a_box_without_session_commit() {
    let (mut fixture, pad) = pad();
    let start = pad.borrow().host.view.to_screen(Point::new(300.0, 220.0));
    fixture.frame(vec![pointer::moved(start)]);
    fixture.frame(vec![pointer::press(start)]);
    fixture.frame(vec![pointer::moved(start + by(8.0, 6.0))]);
    fixture.frame(vec![pointer::moved(start + by(16.0, 12.0))]);
    assert!(pad.borrow().host.state != BoxState::from_rect(RECT), "the drag moved the corner");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.frame(vec![pointer::release(start + by(16.0, 12.0))]);
    fixture.settle();
    let pad = pad.borrow();
    assert_eq!(pad.host.state, BoxState::from_rect(RECT), "Escape put the box back");
    assert!(pad.host.events.contains(&Seen::Cancel), "the drag was cancelled: {:?}", pad.host.events);
}

/// A box whose handle set shows only some handles takes no press, key or drag on the others: a sides-only box (a
/// paragraph frame's width) takes its right and left edges only, a corners-only box no edge.
#[trial]
fn a_sides_only_box_takes_presses_on_its_side_edges_only() {
    // Rotation refused: no outside parts and no knob, so a press off the shown handles reaches nothing of the box.
    let no_turn = || free_rules().with_allow(BoxAllow::new(false, true, true));
    let sides = |config: &mut Config| {
        config.rules = no_turn().with_handles(BoxHandles::Sides);
        config.inside_moves = false;
        config.reference_point = false;
    };
    // Just off the bottom right corner: within the corner's reach, outside both side bands that meet there.
    let off_corner = Point::new(305.0, 225.0);
    let mut all = harness(BoxState::from_rect(RECT), |config| {
        sides(config);
        config.rules = no_turn();
    });
    hover(&mut all, off_corner);
    assert_eq!(all.host.borrow().hovered, Some(BoxPart::Corner(2)), "with every handle the corner takes the point");

    let mut h = harness(BoxState::from_rect(RECT), sides);
    hover(&mut h, off_corner);
    assert_eq!(h.host.borrow().hovered, None, "a hidden corner takes no press");
    drag(&mut h, off_corner, Point::new(325.0, 245.0));
    assert_eq!(h.state(), BoxState::from_rect(RECT), "a drag from a hidden corner moves nothing");
    assert!(h.host.borrow().events.is_empty(), "and starts no gesture: {:?}", h.host.borrow().events);
    for hidden in [Point::new(200.0, 80.0), Point::new(200.0, 220.0)] {
        hover(&mut h, hidden);
        assert_eq!(h.host.borrow().hovered, None, "the hidden edge at {hidden:?} takes no press");
    }

    drag(&mut h, Point::new(300.0, 110.0), Point::new(320.0, 110.0));
    let quad = h.state().quad;
    assert!((quad[1].x - 320.0).abs() < 1e-6 && (quad[2].x - 320.0).abs() < 1e-6, "the right side moved: {quad:?}");
    drag(&mut h, Point::new(100.0, 110.0), Point::new(80.0, 110.0));
    let quad = h.state().quad;
    assert!((quad[0].x - 80.0).abs() < 1e-6 && (quad[3].x - 80.0).abs() < 1e-6, "the left side moved: {quad:?}");

    let mut picks = Vec::new();
    h.fixture.key(egui::Key::Tab, egui::Modifiers::NONE).settle();
    picks.extend(h.host.borrow().picked);
    for _ in 0..3 {
        h.fixture.key(egui::Key::PageDown, egui::Modifiers::NONE).settle();
        picks.extend(h.host.borrow().picked);
    }
    assert!(picks.iter().all(|part| matches!(part, BoxPart::Edge(1) | BoxPart::Edge(3))), "the keys reach the shown sides only: {picks:?}");
    assert!(picks.contains(&BoxPart::Edge(1)) && picks.contains(&BoxPart::Edge(3)), "and both of them: {picks:?}");

    let mut corners = harness(BoxState::from_rect(RECT), |config| {
        sides(config);
        config.rules = no_turn().with_handles(BoxHandles::Corners);
    });
    for hidden in [Point::new(200.0, 80.0), Point::new(200.0, 220.0), Point::new(300.0, 150.0)] {
        hover(&mut corners, hidden);
        assert_eq!(corners.host.borrow().hovered, None, "a corners-only box's edge at {hidden:?} takes no press");
    }
    hover(&mut corners, off_corner);
    assert_eq!(corners.host.borrow().hovered, Some(BoxPart::Corner(2)), "its corners still do");
}

/// The rotate knob stands above the box, or below it at the canvas's top edge; a ⇧ drag on it turns the box by whole
/// rotate steps under the grab pointer, and a click on it applies nothing. The knob is found as the app finds it, by
/// its access-tree name.
#[trial]
fn a_knob_sits_below_a_box_at_the_top_turns_in_shift_steps_and_a_click_on_it_applies_nothing() {
    let knob_of = |h: &Harness| h.fixture.by_label("Rotate").map(|target| target.center());
    let knob_zone = |config: &mut Config| {
        config.zone = RotateZone::Knob;
        config.reference_point = false;
    };

    // The harness's view puts the document's origin at the canvas's top left: a box at y 0 has no room above.
    let at_top = Rect::new(100.0, 0.0, 300.0, 140.0);
    let h = harness(BoxState::from_rect(at_top), knob_zone);
    let knob = knob_of(&h).expect("the knob has a node named by the box's words");
    let bottom = h.screen(Point::new(200.0, 140.0));
    assert!((knob.x - bottom.x).abs() < 0.5 && knob.y > bottom.y, "at the top edge the knob stands below: {knob:?} vs {bottom:?}");

    let mut h = harness(BoxState::from_rect(RECT), knob_zone);
    let knob = knob_of(&h).expect("the knob has a node");
    let top = h.screen(Point::new(200.0, 80.0));
    assert!((knob.x - top.x).abs() < 0.5 && knob.y < top.y, "with room above the knob stands above: {knob:?} vs {top:?}");

    frame(&mut h, vec![pointer::moved(knob)]);
    frame(&mut h, vec![pointer::press(knob)]);
    frame(&mut h, vec![pointer::release(knob)]);
    frame(&mut h, Vec::new());
    assert_eq!(h.host.borrow().applied, 0, "a click on the knob is not a click outside");
    assert_eq!(h.state(), BoxState::from_rect(RECT), "and turns nothing");

    h.config.borrow_mut().mods = Some(Mods::SHIFT);
    let centre = h.screen(RECT.center());
    let around = |degrees: f32| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let v = knob - centre;
        centre + by(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
    };
    frame(&mut h, vec![pointer::moved(knob)]);
    assert_eq!(h.fixture.cursor_icon(), egui::CursorIcon::Grab, "the knob offers to be taken hold of");
    frame(&mut h, vec![pointer::press(knob)]);
    frame(&mut h, vec![pointer::moved(around(10.0))]);
    frame(&mut h, vec![pointer::moved(around(25.0))]);
    assert_eq!(h.fixture.cursor_icon(), egui::CursorIcon::Grabbing, "and holds it while it turns");
    frame(&mut h, vec![pointer::release(around(25.0))]);
    frame(&mut h, Vec::new());
    let angle = h.state().angle().rem_euclid(360.0);
    let step = BoxRules::default().rotate_step;
    let steps = angle / step;
    assert!(steps.round() >= 1.0 && (steps - steps.round()).abs() < 1e-6, "a ⇧ drag turns the box by whole rotate steps: {angle}");
    assert!((angle - 25.0).abs() > 1.0, "not by the unstepped 25° of the drag: {angle}");
    assert_eq!(h.host.borrow().events.last(), Some(&Seen::Commit));
    assert_eq!(h.host.borrow().applied, 0, "a drag on the knob applies nothing");
}

/// A linked corner dragged inward along one axis, the other barely moving, still shrinks both sides in proportion
/// (#4477): the box does not freeze because one axis did not shrink.
#[trial]
fn a_linked_corner_dragged_inward_along_one_axis_shrinks_both_sides() {
    let mut h = upright();
    drag(&mut h, Point::new(300.0, 220.0), Point::new(220.0, 222.0));
    let quad = h.state().quad;
    let (width, height) = (quad[2].x - quad[0].x, quad[2].y - quad[0].y);
    assert_eq!(quad[0], Point::new(RECT.x0, RECT.y0), "the opposite corner stays: {quad:?}");
    assert!(width < RECT.width() - 1.0 && height < RECT.height() - 1.0, "both sides shrink: {width} by {height}");
    assert!((width / height - RECT.width() / RECT.height()).abs() < 1e-6, "in proportion: {width} by {height}");
    // PLAN: the framework scales a linked corner by the drag's projection on the anchor–corner diagonal (here 0.736 of
    // the box); production scaled by the larger axis share (0.6, the 40 % the app test asserted). The rule is left as it is.
}

/// Round handles draw the eight corner and side handles as circles, where the default draws squares.
#[trial]
fn round_handles_are_circles() {
    let circles = |round: bool| {
        let mut h = harness(BoxState::from_rect(RECT), |config| config.round_handles = round);
        h.fixture.frame_shapes().iter().filter(|shape| matches!(shape, egui::Shape::Circle(_))).count()
    };
    // Each handle is a filled circle and its stroke.
    assert_eq!(circles(true), circles(false) + 16, "the four corners and four sides are circles");
}

/// A host's handle side draws the corner and side handles at that side: a larger side draws a larger square.
#[trial]
fn a_host_handle_side_sets_the_drawn_handle_square() {
    let corner_side = |side: Option<f32>| {
        let mut h = harness(BoxState::from_rect(RECT), |config| config.handle_side = side);
        let corner = h.screen(Point::new(RECT.x1, RECT.y1));
        let sides: Vec<f32> = h
            .fixture
            .frame_shapes()
            .iter()
            .filter_map(|shape| match shape {
                Shape::Rect(square) if (square.rect.center() - corner).length() < 0.5 => Some(square.rect.width()),
                _ => None,
            })
            .collect();
        sides.into_iter().fold(0.0, f32::max)
    };
    let (small, large) = (corner_side(Some(4.0)), corner_side(Some(12.0)));
    assert!(small > 0.0, "the corner handle is drawn as a square");
    assert!(large > small, "a larger side draws a larger square: {large} against {small}");
}

/// A host handle side that is zero, negative or not finite draws the theme's handle and keeps the theme's reach.
#[trial]
fn an_unusable_host_handle_side_uses_the_themes() {
    let corner_side = |side: Option<f32>| {
        let mut h = harness(BoxState::from_rect(RECT), |config| config.handle_side = side);
        let corner = h.screen(Point::new(RECT.x1, RECT.y1));
        h.fixture
            .frame_shapes()
            .iter()
            .filter_map(|shape| match shape {
                Shape::Rect(square) if (square.rect.center() - corner).length() < 0.5 => Some(square.rect.width()),
                _ => None,
            })
            .collect::<Vec<f32>>()
    };
    let theme = corner_side(None);
    assert!(!theme.is_empty(), "the theme's corner handle is drawn");
    // 3 points in from the bottom right corner on both axes: inside the theme's handle reach.
    let near = Point::new(RECT.x1 - 3.0, RECT.y1 - 3.0);
    for side in [0.0, -6.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(corner_side(Some(side)), theme, "a side of {side} draws the theme's corner handle");
        let mut h = harness(BoxState::from_rect(RECT), |config| {
            config.rules = free_rules();
            config.handle_side = Some(side);
        });
        hover(&mut h, near);
        assert_eq!(h.host.borrow().hovered, Some(BoxPart::Corner(2)), "a side of {side} keeps the theme's reach");
        // Far outside the theme's reach: an infinite side must not widen the reach to the whole canvas.
        let mut far = harness(BoxState::from_rect(RECT), |config| {
            config.rules = free_rules();
            config.handle_side = Some(side);
        });
        hover(&mut far, Point::new(RECT.x1 + 60.0, RECT.y1 + 60.0));
        assert_ne!(far.host.borrow().hovered, Some(BoxPart::Corner(2)), "a side of {side} does not widen the reach");
    }
}

/// A large host handle takes a press anywhere on its square, also past the theme's reach, and the press drags it.
#[trial]
fn a_press_just_inside_a_large_handles_edge_grabs_it() {
    // 10 points in from the bottom right corner on both axes: inside a 24 point square, about 14.1 from its centre,
    // past the theme's 11 point corner reach.
    let near_edge = Point::new(RECT.x1 - 10.0, RECT.y1 - 10.0);
    let mut theme = harness(BoxState::from_rect(RECT), |config| config.rules = free_rules());
    hover(&mut theme, near_edge);
    assert_ne!(theme.host.borrow().hovered, Some(BoxPart::Corner(2)), "past the theme's handle reach");

    let mut h = harness(BoxState::from_rect(RECT), |config| {
        config.rules = free_rules();
        config.handle_side = Some(24.0);
    });
    hover(&mut h, near_edge);
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Corner(2)), "on the large handle's square");
    drag(&mut h, near_edge, Point::new(RECT.x1 + 13.0, RECT.y1 + 13.0));
    let quad = h.state().quad;
    assert!(quad[2].x > RECT.x1 + 1.0 && quad[2].y > RECT.y1 + 1.0, "the corner moved with the drag: {quad:?}");
    assert_eq!(quad[0], Point::new(RECT.x0, RECT.y0), "the opposite corner stayed: {quad:?}");
}

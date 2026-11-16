//! `TransformBox`, `TransformOverlay` and `TransformTool`: the parts the pointer takes, what each drag does, the
//! keys, the readout and the overlay's edits.

use craft_behavior::geometry::quad::{BoxPart, BoxRules, BoxState, ScaleRule};
use craft_behavior::geometry::{SnapKind, SnapOptions, SnapTarget, SnapTargets};
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{CanvasView, EditEvent, RulerUnits, Ticks};
use craft_draw::ui::transform::{LineLook, RotateZone, TransformBox, TransformEdit, TransformOverlay, TransformSession, TransformSettings};
use craft_keymap::{Mods, Platform};
use craft_ui::canvas::{Canvas, RulerText, Snap};
use craft_ui::canvas::{Guide, GuideKey};
use craft_ui::canvas_controls::Guides;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Pos2, Vec2};
use std::cell::RefCell;
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
    overflow: bool,
    /// The canvas's rulers on both axes.
    rulers: Option<RulerText>,
    /// The host keeps the box in f32, as a text layer keeps its box, and gives it back with its centre as the pivot.
    f32_host: bool,
    centre_mark: bool,
    line: Option<LineLook>,
}

/// What the host keeps of the box and what the passes reported.
struct Host {
    state: BoxState,
    before: Option<BoxState>,
    events: Vec<Seen>,
    applied: usize,
    discarded: usize,
    picked: Option<BoxPart>,
    hovered: Option<BoxPart>,
    id: egui::Id,
    view: CanvasView,
    guide_events: usize,
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
        overflow: false,
        rulers: None,
        f32_host: false,
        centre_mark: false,
        line: None,
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
        before: None,
        events: Vec::new(),
        applied: 0,
        discarded: 0,
        picked: None,
        hovered: None,
        id: egui::Id::NULL,
        view: CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0),
        guide_events: 0,
    }));
    let shown_host = Rc::clone(&host);
    let mut guides = vec![Guide::new(GuideKey::new(1), craft_behavior::Axis::Y, 300.0)];
    let mut targets = SnapTargets::new();
    for x in SNAP_XS {
        targets.add(SnapTarget::new(craft_behavior::Axis::Y, x, SnapKind::Guide));
    }
    let snap = Snap::new(Arc::new(targets), SnapOptions::default());
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let config = shown_config.borrow().clone();
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
        if config.snap {
            canvas = canvas.snap(snap.clone());
        }
        if let Some(ruler) = config.rulers {
            canvas = canvas.rulers(ruler, ruler, Point::ZERO);
        }
        let canvas = canvas.show(ui, |ui, body| {
            let mut guide_edits = 0;
            if config.guides {
                let output = Guides::new("Guides", canvas_id, Bind::bound(&mut guides)).interactive(true).show_at(ui, body.rect);
                guide_edits = usize::from(!output.edit.events.is_empty());
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
            }
            .pixel_grid(config.pixel_grid);
            if let Some(mods) = config.mods {
                control = control.mods(mods);
            }
            if let Some(line) = config.line {
                control = control.line(line);
            }
            (control.show_at(ui, body.rect), guide_edits)
        });
        if let Some((output, guide_edits)) = canvas.inner {
            let mut host = shown_host.borrow_mut();
            host.guide_events += guide_edits;
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
                        host.state = if config.f32_host { kept_in_f32(value) } else { *value };
                    }
                    EditEvent::Commit { after, .. } => {
                        host.events.push(Seen::Commit);
                        host.state = if config.f32_host { kept_in_f32(after) } else { *after };
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
    });
    fixture.settle();
    Harness { fixture, host, config: kept_config }
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

#[trial]
fn an_edge_takes_a_press_along_its_side_and_a_corner_wins_where_both_reach() {
    let mut h = harness(BoxState::from_rect(RECT), |config| config.rules = free_rules());
    // The right edge runs from (300, 80) to (300, 220); its middle is (300, 150).
    hover(&mut h, Point::new(300.0, 110.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(1)), "a press along the side, away from its middle and from the corners");
    hover(&mut h, Point::new(300.0, 84.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Corner(1)), "where a corner and the side both reach, the corner");
    hover(&mut h, Point::new(300.0, 150.0));
    assert_eq!(h.host.borrow().hovered, Some(BoxPart::Edge(1)), "the middle still takes the edge");
    drag(&mut h, Point::new(300.0, 110.0), Point::new(320.0, 110.0));
    let quad = h.state().quad;
    assert!((quad[1].x - 320.0).abs() < 1e-6 && (quad[2].x - 320.0).abs() < 1e-6, "the dragged side moved: {quad:?}");
    assert!((quad[0].x - RECT.x0).abs() < 1e-6 && (quad[3].x - RECT.x0).abs() < 1e-6, "the far side stayed: {quad:?}");
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
    assert!((pivot.x - 170.0).abs() < 0.5 && (pivot.y - 130.0).abs() < 0.5, "the pivot follows the Alt drag: {pivot:?}");
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

#[trial]
fn a_press_in_the_move_reach_moves_the_box_and_a_handle_still_scales() {
    // The overlay's own zone is `RotateZone::Outside`; the band's order against the ring is not reachable from here
    // (`TransformBox::move_reach` is crate-private). The reach is the type tool's (38 points): a side's segment takes a
    // press up to half the hit target (at most 16 points) from it, and the ring the rest of the reach.
    let session = Rc::new(RefCell::new(TransformSession::<()>::new(RECT)));
    let view = Rc::new(RefCell::new(CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0)));
    let (shown_session, shown_view) = (Rc::clone(&session), Rc::clone(&view));
    let settings = TransformSettings::default();
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let at = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
        *shown_view.borrow_mut() = at;
        let edits = Canvas::new("Document").view(at).show(ui, |ui, body| {
            let session = shown_session.borrow();
            TransformOverlay::new("Free Transform", &session, &settings).move_reach(38.0).show_at(ui, body.rect).edits
        });
        let mut session = shown_session.borrow_mut();
        for edit in edits.inner.unwrap_or_default() {
            let edit: TransformEdit<()> = edit;
            session.apply(edit, 0.0);
        }
    });
    fixture.settle();
    let screen = |p: Point| view.borrow().to_screen(p);
    let mut drag = |from: Point, to: Point| {
        let (start, end) = (screen(from), screen(to));
        fixture.frame(vec![pointer::moved(start)]);
        fixture.frame(vec![pointer::press(start)]);
        fixture.frame(vec![pointer::moved(start + (end - start) * 0.5)]);
        fixture.frame(vec![pointer::moved(end)]);
        fixture.frame(vec![pointer::release(end)]);
        fixture.frame(Vec::new());
    };
    let size = |quad: [Point; 4]| (quad[1] - quad[0]).hypot() * (quad[3] - quad[0]).hypot();

    // 24 points outside the right side, past the side's segment and away from its handles: the box moves and keeps its
    // size.
    let before = session.borrow().quad();
    drag(Point::new(324.0, 100.0), Point::new(354.0, 120.0));
    let after = session.borrow().quad();
    assert!(
        ((after[0].x - before[0].x) - 30.0).abs() < 1e-6 && ((after[0].y - before[0].y) - 20.0).abs() < 1e-6,
        "the box translates: {before:?} -> {after:?}"
    );
    assert!((size(after) - size(before)).abs() < 1e-6, "the move keeps the size");

    // A corner handle inside the same reach still scales.
    let corner = after[2];
    drag(corner, Point::new(corner.x + 40.0, corner.y + 28.0));
    let scaled = session.borrow().quad();
    assert!(size(scaled) > size(after) + 1.0, "a corner drag scales: {after:?} -> {scaled:?}");
}

/// The snap lines of a session box drag reach the tool host through `TransformTool`, as the crop frame's do: the host
/// reports them while a snapped corner drag runs and none once it ends.
#[trial]
fn the_session_boxs_snap_lines_reach_the_tool_host() {
    use craft_draw::ui::transform::TransformTool;
    use craft_ui::canvas::ToolHost;
    let session = Rc::new(RefCell::new(TransformSession::<()>::new(RECT)));
    let view = Rc::new(RefCell::new(CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0)));
    let lines = Rc::new(RefCell::new(Vec::new()));
    let (shown_session, shown_view, shown_lines) = (Rc::clone(&session), Rc::clone(&view), Rc::clone(&lines));
    let mut targets = SnapTargets::new();
    for x in SNAP_XS {
        targets.add(SnapTarget::new(craft_behavior::Axis::Y, x, SnapKind::Guide));
    }
    let snap = Snap::new(Arc::new(targets), SnapOptions::default());
    let settings = TransformSettings::default();
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let at = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
        *shown_view.borrow_mut() = at;
        let canvas = Canvas::new("Document").view(at).snap(snap.clone()).show(ui, |ui, body| {
            let session = shown_session.borrow();
            TransformOverlay::new("Free Transform", &session, &settings).show_at(ui, body.rect)
        });
        let Some(shown) = canvas.inner else { return };
        let response = {
            let session = shown_session.borrow();
            let mut tool = TransformTool::new(&session, shown.snapped);
            ToolHost::new().run(ui, &canvas.output, &mut tool)
        };
        shown_lines.borrow_mut().push(response.snapped.len());
        let mut session = shown_session.borrow_mut();
        for edit in shown.edits {
            session.apply(edit, 0.0);
        }
    });
    fixture.settle();
    let from = view.borrow().to_screen(Point::new(300.0, 220.0));
    let mut step = |events: Vec<egui::Event>| {
        lines.borrow_mut().clear();
        fixture.frame(events);
        lines.borrow().iter().copied().max().unwrap_or(0)
    };
    step(vec![pointer::moved(from)]);
    step(vec![pointer::press(from)]);
    step(vec![pointer::moved(from + by(4.0, 0.0))]);
    assert!(step(vec![pointer::moved(from + by(7.0, 0.0))]) > 0, "the corner snapped to x 310: the host reports the line");
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
fn the_box_moves_from_the_first_pointer_move_after_the_press_and_keeps_up_with_no_jump() {
    let mut h = upright();
    let slop = craft_ui::testing::input::slop(h.fixture.ctx());
    // Inside, away from the reference point at the box's centre.
    let from = h.screen(Point::new(150.0, 120.0));
    frame(&mut h, vec![pointer::moved(from)]);
    frame(&mut h, vec![pointer::press(from)]);
    frame(&mut h, Vec::new());
    assert!(h.host.borrow().events.is_empty(), "a press alone moves nothing: {:?}", h.host.borrow().events);
    // One step per frame, a third of the slop each, through the slop and past it.
    for step in 1..=9 {
        let offset = by(slop / 3.0 * step as f32, slop / 6.0 * step as f32);
        frame(&mut h, vec![pointer::moved(from + offset)]);
        assert!((moved_by(&h) - offset).length() < 1e-3, "step {step}: the box keeps the pointer's offset {offset:?}, moved {:?}", moved_by(&h));
    }
    let end = from + by(slop * 3.0, slop * 1.5);
    frame(&mut h, vec![pointer::release(end)]);
    frame(&mut h, Vec::new());
    let events = h.host.borrow().events.clone();
    assert_eq!(events.iter().filter(|seen| **seen == Seen::Begin).count(), 1, "one gesture: {events:?}");
    assert_eq!(events.iter().filter(|seen| **seen == Seen::Commit).count(), 1, "one commit: {events:?}");

    // A move inside the slop and a release there still commit the move the user saw.
    let mut h = upright();
    let from = h.screen(Point::new(150.0, 120.0));
    frame(&mut h, vec![pointer::moved(from)]);
    frame(&mut h, vec![pointer::press(from)]);
    frame(&mut h, vec![pointer::moved(from + by(slop / 2.0, 0.0))]);
    frame(&mut h, vec![pointer::release(from + by(slop / 2.0, 0.0))]);
    frame(&mut h, Vec::new());
    assert!((moved_by(&h) - by(slop / 2.0, 0.0)).length() < 1e-3, "the box stays where it followed to: {:?}", moved_by(&h));
    assert_eq!(h.host.borrow().events.last(), Some(&Seen::Commit), "{:?}", h.host.borrow().events);
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

// A double click inside whose presses wobble within the slop and come back applies the box as it was: the wobble the
// box followed is cancelled, never committed as a move before the apply.
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
    assert!(events.contains(&Seen::Begin), "the box followed the wobble: {events:?}");
    assert!(!events.contains(&Seen::Commit), "no move is committed: {events:?}");
    assert_eq!(h.state(), BoxState::from_rect(RECT), "the box stays as it was");
    assert_eq!(h.host.borrow().applied, 1, "the double click applies");
}

/// A secondary click on the session's box opens the mode menu (its rows are on the page); with the menu closed again, a
/// secondary click away from the box opens none.
#[trial]
fn a_secondary_click_on_the_box_opens_the_mode_menu() {
    let session = TransformSession::<()>::new(RECT);
    let settings = TransformSettings::default();
    let view = Rc::new(RefCell::new(CanvasView::new(egui::Rect::NOTHING, Point::ZERO, 1.0)));
    let shown_view = Rc::clone(&view);
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let at = CanvasView::new(room, Point::new(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0);
        *shown_view.borrow_mut() = at;
        Canvas::new("Document").view(at).show(ui, |ui, body| {
            TransformOverlay::new("Free Transform", &session, &settings).show_at(ui, body.rect);
        });
    });
    fixture.settle();
    let mut right_click = |fixture: &mut Fixture, at: Pos2| {
        fixture.frame(vec![pointer::moved(at)]);
        fixture.frame(vec![pointer::secondary(at, true)]);
        fixture.frame(vec![pointer::secondary(at, false)]);
        fixture.settle();
    };
    let screen = |p: Point| view.borrow().to_screen(p);

    right_click(&mut fixture, screen(Point::new(1000.0, 1000.0)));
    assert!(fixture.by_label("Distort").is_none(), "a secondary click away from the box opens no mode menu");

    right_click(&mut fixture, screen(Point::new(200.0, 150.0)));
    assert!(fixture.by_label("Distort").is_some(), "a secondary click on the box opens the mode menu");
}

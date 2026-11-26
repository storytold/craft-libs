//! `CropOverlay`: the frame's gestures, keys and look through real input.

use craft_behavior::geometry::{
    AspectHold, BoxPart, CropBox, CropGuide, GuideShow, SNAP_PX, SnapKind, SnapLine, SnapOptions, SnapTarget, SnapTargets, crop_view,
};
use craft_behavior::kurbo::{Point, Rect};
use craft_behavior::{Axis, CancelReason, CanvasView, EditEvent};
use craft_ui::canvas::{
    Canvas, CanvasTool, CropApply, CropOutside, CropOverlay, CropScope, Snap, ToolAction, ToolContext, ToolHost, ToolPath, ToolPointer, tool_pass,
};
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::primitives::Draw;
use craft_ui::runtime::access::Role;
use craft_ui::testing::{Fixture, Frames, focus, input, layout, pointer, trial};
use egui::{Event, Modifiers, Pos2, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

// Real input through the canvas: a frame per event batch, the view and the shown options held by the rig.

/// The room the rig shows the canvas in.
const ROOM: Vec2 = Vec2::new(400.0, 300.0);

/// The frame the drags start from, centred in the room's default view.
const RECT: Rect = Rect::new(100.0, 80.0, 300.0, 220.0);

/// The shortest painted line that counts as a guide line: longer than any corner bracket or edge bar.
const GUIDE_MIN: f32 = 40.0;

#[derive(Clone)]
struct Setup {
    ratio: Option<f64>,
    default_frame: bool,
    image_mode: bool,
    auto_center: bool,
    fills_uncovered: bool,
    hidden: bool,
    bounds: Option<Rect>,
    snap_x: Option<f64>,
    snap_y: Option<f64>,
    straighten: bool,
    guide: Option<(CropGuide, u8)>,
    guide_show: GuideShow,
    outside: CropOutside,
    area: Option<[Point; 4]>,
    min_side: Option<f64>,
    view: CanvasView,
    /// The overlay crops one placed picture with this source.
    object: Option<Rect>,
}

impl Setup {
    fn new() -> Self {
        Setup {
            ratio: None,
            default_frame: false,
            image_mode: false,
            auto_center: false,
            fills_uncovered: false,
            hidden: false,
            bounds: None,
            snap_x: None,
            snap_y: None,
            straighten: false,
            guide: None,
            guide_show: GuideShow::Auto,
            outside: CropOutside::default(),
            area: None,
            min_side: None,
            view: CanvasView::new(egui::Rect::ZERO, Point::new(200.0, 150.0), 1.0),
            object: None,
        }
    }

    /// Image mode: the view shows `angle`'s frame upright.
    fn image(angle: f64, auto_center: bool) -> Self {
        let mut setup = Setup::new();
        setup.image_mode = true;
        setup.auto_center = auto_center;
        setup.view = crop_view(&CropBox::new(RECT, angle), &setup.view);
        setup
    }

    /// The default mode's untouched default frame, its thirds always shown.
    fn untouched() -> Self {
        let mut setup = Setup::image(0.0, false);
        setup.default_frame = true;
        setup.guide = Some((CropGuide::Thirds, 0));
        setup.guide_show = GuideShow::Always;
        setup
    }
}

/// What the overlay reported across the passes.
#[derive(Default)]
struct Seen {
    applied: Option<CropApply>,
    discarded: bool,
    straightened: Option<f64>,
    snapped: Vec<SnapLine>,
    picked: Option<BoxPart>,
    hovered: Option<BoxPart>,
    sizing: Option<(f64, f64)>,
    turning: Option<f64>,
    events: Vec<&'static str>,
    /// `scrolls_at` of every pass, in order.
    scrolls_at: Vec<Option<Pos2>>,
    /// The passes that reported `straighten_ended`.
    straighten_ended: usize,
}

struct Rig {
    frames: Frames,
    show: Box<dyn FnMut(&mut egui::Ui)>,
    crop: Rc<RefCell<CropBox>>,
    setup: Rc<RefCell<Setup>>,
    seen: Rc<RefCell<Seen>>,
    /// While set, the first pass of each frame asks egui to run the frame again.
    rerun: Rc<Cell<bool>>,
}

#[trial]
fn crop_body_shows_grab_and_grabbing_during_move() {
    use craft_behavior::{Cursor, CursorIcon};
    let mut rig = Rig::upright();
    let at = rig.at(200.0, 150.0);
    rig.move_to(at);
    assert_eq!(craft_ui::runtime::cursor::current(&rig.frames.ctx), Some(Cursor::System(CursorIcon::Grab)));
    rig.press(at);
    rig.move_to(at + Vec2::new(30.0, 0.0));
    assert_eq!(craft_ui::runtime::cursor::current(&rig.frames.ctx), Some(Cursor::System(CursorIcon::Grabbing)));
    rig.release(at + Vec2::new(30.0, 0.0));
}

impl Rig {
    fn new(crop: CropBox, setup: Setup) -> Self {
        let (crop, setup, seen) = (Rc::new(RefCell::new(crop)), Rc::new(RefCell::new(setup)), Rc::new(RefCell::new(Seen::default())));
        let rerun = Rc::new(Cell::new(false));
        let (c, st, sn, again) = (Rc::clone(&crop), Rc::clone(&setup), Rc::clone(&seen), Rc::clone(&rerun));
        let show = move |ui: &mut egui::Ui| {
            let mut s = st.borrow().clone();
            s.view.rect = ui.available_rect_before_wrap();
            st.borrow_mut().view.rect = s.view.rect;
            let mut targets = SnapTargets::new();
            if let Some(x) = s.snap_x {
                targets.add(SnapTarget::new(Axis::Y, x, SnapKind::Guide));
            }
            if let Some(y) = s.snap_y {
                targets.add(SnapTarget::new(Axis::X, y, SnapKind::Guide));
            }
            let snap = Snap::new(Arc::new(targets), SnapOptions::default());
            let _ = Canvas::new("Document").view(s.view).snap(snap).show(ui, |ui, body| {
                let mut value = c.borrow_mut();
                let mut overlay = CropOverlay::new("Crop", Bind::bound(&mut *value)).guide_show(s.guide_show).outside(s.outside);
                if let Some(area) = s.area {
                    overlay = overlay.area(area);
                }
                if let Some(min_side) = s.min_side {
                    overlay = overlay.min_side(min_side);
                }
                if let Some(ratio) = s.ratio {
                    overlay = overlay.ratio(ratio);
                }
                if let Some(bounds) = s.bounds {
                    overlay = overlay.bounds(bounds);
                }
                if let Some(guide) = s.guide {
                    overlay = overlay.guide(guide);
                }
                if s.default_frame {
                    overlay = overlay.default_frame();
                }
                if s.image_mode {
                    overlay = overlay.image_mode();
                }
                if s.auto_center {
                    overlay = overlay.auto_center();
                }
                if s.straighten {
                    overlay = overlay.straighten();
                }
                if s.fills_uncovered {
                    overlay = overlay.fills_uncovered();
                }
                if s.hidden {
                    overlay = overlay.hidden();
                }
                if let Some(source) = s.object {
                    overlay = overlay.scope(CropScope::object(source));
                }
                let out = overlay.show_at(ui, body.rect);
                // A measuring run of the canvas body (a sizing ui) senses nothing: only the real run of a pass reports.
                if ui.is_sizing_pass() {
                    return;
                }
                let mut seen = sn.borrow_mut();
                if let Some(view) = out.view {
                    st.borrow_mut().view = view;
                }
                for event in &out.edit.events {
                    let kind = match event {
                        EditEvent::Begin { .. } => "begin",
                        EditEvent::Update { .. } => "update",
                        EditEvent::Commit { .. } => "commit",
                        EditEvent::Cancel { .. } => "cancel",
                        EditEvent::Set { .. } => "set",
                        _ => "other",
                    };
                    if seen.events.last() != Some(&kind) {
                        seen.events.push(kind);
                    }
                }
                seen.applied = out.apply.or(seen.applied);
                seen.discarded |= out.discard;
                seen.straightened = out.straightened.or(seen.straightened);
                seen.snapped = out.snapped;
                seen.picked = out.picked;
                seen.hovered = out.hovered;
                seen.sizing = out.sizing;
                seen.turning = out.turning;
                seen.scrolls_at.push(out.scrolls_at);
                seen.straighten_ended += usize::from(out.straighten_ended);
            });
            if again.get() && ui.ctx().current_pass_index() == 0 {
                ui.ctx().request_discard("the trial runs the frame twice");
            }
        };
        let mut rig = Rig { frames: Frames::new(), show: Box::new(show), crop, setup, seen, rerun };
        for _ in 0..4 {
            rig.frame(Vec::new());
        }
        rig
    }

    fn upright() -> Self {
        Rig::new(CropBox::upright(RECT), Setup::new())
    }

    /// One frame with `events`; returns how many passes it ran.
    fn frame(&mut self, events: Vec<Event>) -> usize {
        let room = egui::Rect::from_min_size(Pos2::ZERO, ROOM);
        let show = &mut self.show;
        self.frames.frame(events, |ui| layout::in_region(ui, room, |ui| show(ui)))
    }

    /// One frame with `events` that runs twice: its second pass has no input of its own.
    fn frame_twice(&mut self, events: Vec<Event>) {
        self.rerun.set(true);
        let passes = self.frame(events);
        self.rerun.set(false);
        assert_eq!(passes, 2, "the frame ran twice");
    }

    /// What one frame with no input paints.
    fn look(&mut self) -> Look {
        let room = egui::Rect::from_min_size(Pos2::ZERO, ROOM);
        let show = &mut self.show;
        Look::of(&self.frames.frame_shapes(|ui| layout::in_region(ui, room, |ui| show(ui))))
    }

    /// The screen rect the translucent shield meshes of one frame cover together; `None` when there are none.
    fn shield_extent(&mut self) -> Option<egui::Rect> {
        let room = egui::Rect::from_min_size(Pos2::ZERO, ROOM);
        let show = &mut self.show;
        let shapes = self.frames.frame_shapes(|ui| layout::in_region(ui, room, |ui| show(ui)));
        let points: Vec<Pos2> = shapes
            .iter()
            .filter_map(|shape| match shape {
                egui::Shape::Mesh(mesh) if mesh.vertices.iter().any(|v| (1..255).contains(&v.color.a())) => Some(mesh),
                _ => None,
            })
            .flat_map(|mesh| mesh.vertices.iter().map(|v| v.pos))
            .collect();
        (!points.is_empty()).then(|| egui::Rect::from_points(&points))
    }

    /// The shield's meshes this frame: those with a translucent vertex.
    fn shield_meshes(&mut self) -> Vec<egui::Mesh> {
        let room = egui::Rect::from_min_size(Pos2::ZERO, ROOM);
        let show = &mut self.show;
        let shapes = self.frames.frame_shapes(|ui| layout::in_region(ui, room, |ui| show(ui)));
        shapes
            .into_iter()
            .filter_map(|shape| match shape {
                egui::Shape::Mesh(mesh) if mesh.vertices.iter().any(|v| (1..255).contains(&v.color.a())) => Some((*mesh).clone()),
                _ => None,
            })
            .collect()
    }

    fn view(&self) -> CanvasView {
        self.setup.borrow().view
    }

    /// The screen point of document point `(x, y)`.
    fn at(&self, x: f64, y: f64) -> Pos2 {
        self.view().to_screen(Point::new(x, y))
    }

    fn crop(&self) -> CropBox {
        *self.crop.borrow()
    }

    fn rect(&self) -> Rect {
        self.crop().rect
    }

    fn events(&self) -> Vec<&'static str> {
        self.seen.borrow().events.clone()
    }

    fn press(&mut self, at: Pos2) {
        self.frame(vec![pointer::moved(at)]);
        self.frame(vec![pointer::press(at)]);
    }

    fn move_to(&mut self, to: Pos2) {
        self.frame(vec![pointer::moved(to)]);
    }

    fn release(&mut self, at: Pos2) {
        self.frame(vec![pointer::release(at)]);
        self.frame(Vec::new());
    }

    /// Press and release at `at` without moving.
    fn click(&mut self, at: Pos2) {
        self.press(at);
        self.release(at);
    }

    /// Press at `from`, move through the middle to `to`, release, with `mods` held.
    fn drag(&mut self, from: Pos2, to: Pos2, mods: Modifiers) {
        self.frames.hold(mods);
        self.press(from);
        self.move_to(from.lerp(to, 0.5));
        self.move_to(to);
        self.release(to);
        self.frames.hold(Modifiers::NONE);
    }
}

/// What a frame painted, as the overlay's parts tell apart: the shield fills translucent meshes, the untouched outline
/// is dashes, the started outline one closed path, and the guides open paths from edge to edge of the outline.
struct Look {
    shield: usize,
    dashes: usize,
    outline: Option<[Pos2; 4]>,
    guides: Vec<(Pos2, Pos2)>,
}

impl Look {
    fn of(shapes: &[egui::Shape]) -> Look {
        let (mut shield, mut dashes, mut outline, mut open) = (0, 0, None, Vec::new());
        for shape in shapes {
            match shape {
                egui::Shape::Mesh(mesh) if mesh.vertices.iter().any(|v| (1..255).contains(&v.color.a())) => shield += 1,
                egui::Shape::LineSegment { .. } => dashes += 1,
                egui::Shape::Path(path) if path.closed && path.points.len() == 4 && outline.is_none() => {
                    outline = <[Pos2; 4]>::try_from(path.points.as_slice()).ok();
                }
                egui::Shape::Path(path) if !path.closed && path.points.len() >= 2 => {
                    if let (Some(first), Some(last)) = (path.points.first(), path.points.last()) {
                        open.push((*first, *last));
                    }
                }
                _ => {}
            }
        }
        let on_outline = |p: Pos2| outline.is_none_or(|q: [Pos2; 4]| (0..4).any(|i| to_segment(p, q[i], q[(i + 1) % 4]) < 0.75));
        let guides = open.into_iter().filter(|(a, b)| a.distance(*b) > GUIDE_MIN && on_outline(*a) && on_outline(*b)).collect();
        Look { shield, dashes, outline, guides }
    }

    /// No shield, a dashed outline and no guide (#10930).
    fn untouched(&self) -> bool {
        self.shield == 0 && self.dashes > 0 && self.outline.is_none() && self.guides.is_empty()
    }

    /// The shield and a solid outline.
    fn started(&self) -> bool {
        self.shield > 0 && self.dashes == 0 && self.outline.is_some()
    }

    /// A guide line runs through the middle of the frame: the turn grid has one, thirds do not.
    fn through_centre(&self) -> bool {
        let Some(q) = self.outline else { return false };
        let centre = Pos2::new((q[0].x + q[2].x) / 2.0, (q[0].y + q[2].y) / 2.0);
        self.guides.iter().any(|(a, b)| a.lerp(*b, 0.5).distance(centre) < 1.0)
    }
}

/// How far `p` is from the segment from `a` to `b`.
fn to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let d = b - a;
    let t = if d.length_sq() > 0.0 { ((p - a).dot(d) / d.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + d * t)
}

fn near(a: &CropBox, b: &CropBox) -> bool {
    a.corners().iter().zip(b.corners()).all(|(p, q)| p.distance(q) < 1e-6) && (a.angle - b.angle).abs() < 1e-6
}

/// Whether the turned frame lies inside `bounds`, rounding aside.
fn inside(b: &CropBox, bounds: Rect) -> bool {
    let t = b.turned_bounds();
    t.x0 >= bounds.x0 - 1e-6 && t.y0 >= bounds.y0 - 1e-6 && t.x1 <= bounds.x1 + 1e-6 && t.y1 <= bounds.y1 + 1e-6
}

#[trial]
fn a_corner_drag_resizes_along_the_frame_axes_in_a_turned_view() {
    let b = CropBox::new(RECT, 20.0);
    let mut rig = Rig::new(b, {
        let mut s = Setup::new();
        s.view.rotation = 40.0;
        s.view.flip_x = true;
        s
    });
    let view = rig.view();
    let corner = view.to_screen(b.corners()[2]);
    let to = corner + Vec2::new(14.0, -9.0);
    let want = b.resized(BoxPart::Corner(2), view.to_doc(to) - view.to_doc(corner), AspectHold::Free, false);
    rig.drag(corner, to, Modifiers::NONE);
    assert!(near(&rig.crop(), &want), "{:?} vs {want:?}", rig.crop());
    assert_eq!(rig.seen.borrow().picked, Some(BoxPart::Corner(2)));

    // Shift keeps the proportions; Alt resizes about the centre.
    let mut rig = Rig::upright();
    let (from, to) = (rig.at(300.0, 220.0), rig.at(340.0, 230.0));
    rig.drag(from, to, Modifiers::SHIFT);
    let r = rig.rect();
    assert!((r.width() / r.height() - 200.0 / 140.0).abs() < 1e-9, "{r:?}");
    let mut rig = Rig::upright();
    rig.drag(from, to, Modifiers::ALT);
    let c = rig.rect().center();
    assert!((c.x - 200.0).abs() < 1e-9 && (c.y - 150.0).abs() < 1e-9, "{:?}", rig.rect());
}

#[trial]
fn a_quick_drag_in_one_frame_starts_at_the_press() {
    let mut rig = Rig::upright();
    let from = rig.at(300.0, 220.0);
    let to = from + Vec2::new(40.0, 10.0);
    rig.frame(vec![pointer::moved(from)]);
    rig.frame(vec![pointer::press(from), pointer::moved(from + Vec2::new(20.0, 5.0)), pointer::moved(to)]);
    rig.frame(vec![pointer::release(to)]);
    rig.frame(Vec::new());
    assert_eq!(rig.rect(), Rect::new(100.0, 80.0, 340.0, 230.0));
    let events = rig.events();
    assert_eq!((events.first(), events.last()), (Some(&"begin"), Some(&"commit")), "{events:?}");
}

/// Control is pressed once the resize has started: on macOS a press with Control held is a secondary click.
#[trial]
fn the_control_key_held_during_a_resize_turns_snapping_off() {
    for (held, right) in [(Modifiers::NONE, 310.0), (Modifiers::CTRL, 307.0)] {
        let mut rig = Rig::new(CropBox::upright(RECT), {
            let mut s = Setup::new();
            s.snap_x = Some(310.0);
            s
        });
        let (from, to) = (rig.at(300.0, 220.0), rig.at(307.0, 220.0));
        rig.press(from);
        rig.frames.hold(held);
        rig.move_to(from.lerp(to, 0.5));
        rig.move_to(to);
        rig.release(to);
        rig.frames.hold(Modifiers::NONE);
        assert!((rig.rect().x1 - right).abs() < 1e-6, "{held:?}: {:?}", rig.rect());
    }
}

#[trial]
fn a_click_with_snapping_on_changes_nothing() {
    for angle in [0.0, 20.0] {
        let b = CropBox::new(RECT, angle);
        let mut setup = Setup::new();
        // Guides just past the right and bottom edges, in reach of the edges and the corner between them.
        setup.snap_x = Some(RECT.x1 + 3.0);
        setup.snap_y = Some(RECT.y1 + 3.0);
        for part in [BoxPart::Inside, BoxPart::Edge(1), BoxPart::Corner(2)] {
            let mut rig = Rig::new(b, setup.clone());
            let Some(at) = b.part_point(part) else { continue };
            rig.click(rig.view().to_screen(at));
            assert_eq!(rig.crop(), b, "a click on {part:?} at {angle} degrees");
            assert!(rig.events().is_empty(), "{part:?} at {angle} degrees: {:?}", rig.events());
        }
    }
}

#[trial]
fn space_moves_the_frame_being_drawn_at_its_size() {
    let mut setup = Setup::new();
    setup.default_frame = true;
    let mut rig = Rig::new(CropBox::upright(RECT), setup);
    rig.press(rig.at(150.0, 120.0));
    rig.move_to(rig.at(200.0, 160.0));
    rig.move_to(rig.at(250.0, 200.0));
    assert!(near(&rig.crop(), &CropBox::upright(Rect::new(150.0, 120.0, 250.0, 200.0))), "drawn: {:?}", rig.rect());
    // Space held: the pointer's step moves the whole frame.
    rig.frame(vec![input::key_press(egui::Key::Space, Modifiers::NONE)]);
    rig.move_to(rig.at(270.0, 210.0));
    assert!(near(&rig.crop(), &CropBox::upright(Rect::new(170.0, 130.0, 270.0, 210.0))), "moved at its size: {:?}", rig.rect());
    // Space let go: the corner follows the pointer again from where the frame went.
    rig.frame(vec![focus::key_release(egui::Key::Space, Modifiers::NONE)]);
    rig.move_to(rig.at(290.0, 220.0));
    rig.release(rig.at(290.0, 220.0));
    assert!(near(&rig.crop(), &CropBox::upright(Rect::new(170.0, 130.0, 290.0, 220.0))), "drawn on: {:?}", rig.rect());
}

#[trial]
fn an_untouched_default_frame_shows_no_shield_or_guide_until_a_drag_changes_it() {
    let mut rig = Rig::new(CropBox::upright(RECT), Setup::untouched());
    assert!(rig.look().untouched(), "the default frame before anything happens");
    // A press that has not moved, and clicks inside and on a handle, keep the untouched look.
    let corner = rig.at(RECT.x1, RECT.y1);
    rig.press(corner);
    assert!(rig.look().untouched(), "a press that has not moved");
    rig.release(corner);
    rig.click(rig.at(200.0, 150.0));
    assert!(rig.look().untouched(), "after clicks on a handle and inside");
    // A drag that changes the frame starts it, and keeps it started when it brings the frame back, a second pass of
    // the frame included.
    rig.press(corner);
    rig.move_to(corner + Vec2::new(30.0, 20.0));
    assert!(rig.look().started(), "the drag changed the frame");
    rig.frame_twice(vec![pointer::moved(corner)]);
    assert_eq!(rig.rect(), RECT, "the frame came back");
    let back = rig.look();
    assert!(back.started() && !back.guides.is_empty(), "the frame drawn again stays started, its guide shown");
    rig.release(corner);
    assert!(rig.look().untouched(), "the drag over, the app still says the frame is the default");
    // A turn shows its guides from its start.
    let outside = rig.at(40.0, 150.0);
    rig.press(outside);
    rig.move_to(outside + Vec2::new(0.0, -2.0 * input::slop(&rig.frames.ctx)));
    let turning = rig.look();
    assert!(turning.started() && !turning.guides.is_empty(), "a turn starts the crop");
    rig.release(outside);
    // Classic Mode is always started.
    let mut classic = Setup::untouched();
    classic.image_mode = false;
    let mut rig = Rig::new(CropBox::upright(RECT), classic);
    assert!(rig.look().started(), "Classic Mode");
}

#[trial]
fn a_handle_held_past_the_edge_scrolls_and_a_turn_does_not() {
    let past = Pos2::new(ROOM.x + 20.0, 150.0);
    // A resize past the edge asks to scroll at the pointer, in every pass of the frames while it is held.
    let mut rig = Rig::upright();
    rig.press(rig.at(RECT.x1, 150.0));
    rig.move_to(past);
    rig.seen.borrow_mut().scrolls_at.clear();
    rig.frame_twice(Vec::new());
    assert_eq!(rig.seen.borrow().scrolls_at, [Some(past), Some(past)], "both passes ask");
    rig.release(past);
    assert_eq!(rig.seen.borrow().scrolls_at.last(), Some(&None), "the release ends it");
    // What each drag asks for, held past the edge.
    let cases = [
        ("a classic move", Setup::new(), Point::new(200.0, 150.0), true),
        ("a turn", Setup::new(), Point::new(40.0, 150.0), false),
        ("an image-mode move", Setup::image(0.0, false), Point::new(200.0, 150.0), false),
        ("an image-mode resize", Setup::image(0.0, false), Point::new(RECT.x1, 150.0), true),
        ("a live-centred resize", Setup::image(0.0, true), Point::new(RECT.x1, 150.0), false),
    ];
    for (what, setup, from, scrolls) in cases {
        let mut rig = Rig::new(CropBox::upright(RECT), setup);
        rig.press(rig.at(from.x, from.y));
        rig.move_to(rig.at(from.x, from.y) + Vec2::new(20.0, 0.0));
        rig.move_to(past);
        assert_eq!(rig.seen.borrow().scrolls_at.last().copied().flatten().is_some(), scrolls, "{what}");
        rig.release(past);
    }

    // Through a tool host, the tool's `autoscroll_at` makes the pass report how far past the edge the pointer is.
    let (mut fixture, past_edge) = scrolling_host();
    let canvas = fixture.by_role(Role::CANVAS)[0].rect;
    let corner = fixture.by_label("Crop corner, bottom right").expect("the crop shows its corners").center();
    let beyond = Pos2::new(canvas.right() + 20.0, corner.y);
    fixture.frame(vec![pointer::moved(corner)]).frame(vec![pointer::press(corner)]);
    fixture.frame(vec![pointer::moved(corner + Vec2::new(20.0, 0.0))]).frame(vec![pointer::moved(beyond)]);
    assert!(past_edge.get().is_some_and(|by| by.x > 0.0), "past the right edge: {:?}", past_edge.get());
    fixture.frame(vec![pointer::release(beyond)]).frame(Vec::new());
    assert_eq!(past_edge.get(), None, "released");
    let turn = canvas.left_top() + Vec2::splat(10.0);
    fixture.frame(vec![pointer::moved(turn)]).frame(vec![pointer::press(turn)]);
    fixture.frame(vec![pointer::moved(turn + Vec2::new(0.0, 20.0))]).frame(vec![pointer::moved(Pos2::new(canvas.left() - 20.0, turn.y))]);
    assert_eq!(past_edge.get(), None, "a turn never scrolls");
}

/// A tool that only hands on what the crop overlay asks for, as the crop tool does.
struct Scrolling {
    asked: Rc<Cell<Option<Pos2>>>,
}

impl CanvasTool for Scrolling {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        if let Some(at) = self.asked.get() {
            cx.autoscroll_at(at);
        }
    }

    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Command("the options bar's size fields")
    }
}

/// A canvas with the crop overlay in its body and [`Scrolling`] as its tool; returns the fixture and the host's
/// `past_edge` of the last pass.
fn scrolling_host() -> (Fixture, Rc<Cell<Option<Vec2>>>) {
    let crop = Rc::new(RefCell::new(CropBox::upright(RECT)));
    let asked = Rc::new(Cell::new(None::<Pos2>));
    let past_edge = Rc::new(Cell::new(None::<Vec2>));
    let tool = Rc::new(RefCell::new(Scrolling { asked: Rc::clone(&asked) }));
    let (seen, reported) = (Rc::clone(&asked), Rc::clone(&past_edge));
    let mut fixture = Fixture::new(move |ui| {
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, Point::new(200.0, 150.0), 1.0);
        let mut reply = None;
        let _ = Canvas::new("Document").view(view).show(ui, |ui, body| {
            let out = CropOverlay::new("Crop", Bind::bound(&mut *crop.borrow_mut())).show_at(ui, body.rect);
            seen.set(out.scrolls_at);
            if let Some(pass) = tool_pass(&body.env) {
                reply = Some(ToolHost::new().run(ui, &pass, &mut *tool.borrow_mut()));
            }
        });
        reported.set(reply.unwrap_or_default().past_edge);
    });
    fixture.settle();
    (fixture, past_edge)
}

#[trial]
fn a_default_turn_shows_the_turn_grid_and_release_restores_the_guide() {
    let thirds = |image: bool, show: GuideShow| {
        let mut s = if image { Setup::image(0.0, false) } else { Setup::new() };
        s.guide = Some((CropGuide::Thirds, 0));
        s.guide_show = show;
        s
    };
    let outside = Point::new(40.0, 150.0);
    let turn = |rig: &mut Rig| {
        let from = rig.at(outside.x, outside.y);
        rig.press(from);
        rig.move_to(from + Vec2::new(0.0, -12.0));
        rig.move_to(from + Vec2::new(0.0, -24.0));
        from
    };
    // An image-mode turn: the turn grid in place of the thirds; after it, the thirds again.
    let mut rig = Rig::new(CropBox::upright(RECT), thirds(true, GuideShow::Always));
    let before = rig.look();
    assert!(!before.guides.is_empty() && !before.through_centre(), "the thirds before the turn");
    let from = turn(&mut rig);
    let turning = rig.look();
    assert!(turning.through_centre(), "the turn grid while the image turns: {:?}", turning.guides);
    rig.release(from);
    let after = rig.look();
    assert!(!after.guides.is_empty() && !after.through_centre(), "the thirds back after the release");
    // Never shows nothing, the turn grid included.
    let mut rig = Rig::new(CropBox::upright(RECT), thirds(true, GuideShow::Never));
    let from = turn(&mut rig);
    assert!(rig.look().guides.is_empty(), "Never");
    rig.release(from);
    // A Classic Mode turn keeps the thirds.
    let mut rig = Rig::new(CropBox::upright(RECT), thirds(false, GuideShow::Auto));
    let from = turn(&mut rig);
    let classic = rig.look();
    assert!(!classic.guides.is_empty() && !classic.through_centre(), "a Classic Mode turn: {:?}", classic.guides);
    rig.release(from);
    // A Straighten line keeps the thirds, and its release ends Straighten once, a second pass of the frame aside.
    let mut setup = thirds(true, GuideShow::Auto);
    setup.straighten = true;
    let mut rig = Rig::new(CropBox::upright(RECT), setup);
    let (from, to) = (rig.at(150.0, 120.0), rig.at(260.0, 140.0));
    rig.press(from);
    rig.move_to(from.lerp(to, 0.5));
    rig.move_to(to);
    let line = rig.look();
    assert!(!line.guides.is_empty() && !line.through_centre(), "a Straighten line: {:?}", line.guides);
    rig.frame_twice(vec![pointer::release(to)]);
    rig.frame(Vec::new());
    assert_eq!(rig.seen.borrow().straighten_ended, 1, "the release frame's two passes end Straighten once");
}

// Escape ends an armed Straighten and leaves the crop; with none armed it discards the started crop.
#[trial]
fn escape_ends_an_armed_straighten_before_it_discards_the_crop() {
    let mut setup = Setup::new();
    setup.straighten = true;
    let mut rig = Rig::new(CropBox::upright(RECT), setup);
    rig.frame(vec![input::key_press(egui::Key::Escape, Modifiers::NONE)]);
    assert!(rig.seen.borrow().straighten_ended >= 1, "Escape ends Straighten");
    assert!(!rig.seen.borrow().discarded, "and keeps the crop");
    let mut rig = Rig::new(CropBox::upright(RECT), Setup::new());
    rig.frame(vec![input::key_press(egui::Key::Escape, Modifiers::NONE)]);
    assert!(rig.seen.borrow().discarded, "with no Straighten armed Escape discards the started crop");
    assert_eq!(rig.seen.borrow().straighten_ended, 0);
}

#[trial]
fn a_default_turn_keeps_the_frame_inside_the_turned_image() {
    let image = Rect::new(0.0, 0.0, 400.0, 300.0);
    // The image-mode rig over `frame`, the view showing it upright, the image as the bounds.
    let rig_for = |frame: CropBox, image_mode: bool| {
        let mut s = Setup::new();
        s.image_mode = image_mode;
        s.bounds = Some(image);
        if image_mode {
            s.view = crop_view(&frame, &s.view);
        }
        Rig::new(frame, s)
    };
    // Presses above the frame's centre and moves round it by `degrees`, `radius` screen points out.
    let turn_by = |rig: &mut Rig, degrees: &[f64], radius: f32, mods: Modifiers| {
        let centre = rig.view().to_screen(rig.crop().center());
        let at = |deg: f64| {
            let (s, c) = (deg.to_radians().sin() as f32, deg.to_radians().cos() as f32);
            centre + Vec2::new(radius * s, -radius * c)
        };
        rig.frames.hold(mods);
        rig.press(at(0.0));
        for deg in degrees {
            rig.move_to(at(*deg));
        }
        (centre, at(degrees.last().copied().unwrap_or(0.0)))
    };

    // A frame that fits keeps its size and place.
    let fits = CropBox::upright(Rect::new(40.0, 40.0, 360.0, 260.0));
    let mut rig = rig_for(fits, true);
    let (centre, end) = turn_by(&mut rig, &[3.0, 5.0], 140.0, Modifiers::NONE);
    assert_eq!(rig.rect(), fits.rect, "a small turn fits");
    assert!(rig.crop().angle.abs() > 1.0, "it turned: {:?}", rig.crop());
    // Turned further it would leave the image: shrunk, its proportions kept; turned back, the frame comes back.
    let (s, c) = (30f64.to_radians().sin() as f32, 30f64.to_radians().cos() as f32);
    rig.move_to(centre + Vec2::new(140.0 * s, -140.0 * c));
    let shrunk = rig.crop();
    assert!(shrunk.rect.width() < fits.rect.width() - 1.0 && inside(&shrunk, image), "{shrunk:?}");
    assert!((shrunk.rect.width() / shrunk.rect.height() - fits.rect.width() / fits.rect.height()).abs() < 1e-9, "{shrunk:?}");
    rig.move_to(centre + Vec2::new(0.0, -140.0));
    assert_eq!(rig.rect(), fits.rect, "turning back gives the frame back");
    rig.release(end);
    rig.frames.hold(Modifiers::NONE);

    // A frame near the edge moves in at its size, and the view follows it: the box stays put on screen.
    let edge = CropBox::upright(Rect::new(150.0, 100.0, 390.0, 260.0));
    let mut rig = rig_for(edge, true);
    let (centre, end) = turn_by(&mut rig, &[10.0, 20.0], 120.0, Modifiers::NONE);
    let moved = rig.crop();
    assert!(inside(&moved, image), "{moved:?}");
    assert!((moved.rect.width() - edge.rect.width()).abs() < 1e-9 && (moved.rect.height() - edge.rect.height()).abs() < 1e-9, "{moved:?}");
    assert!(moved.center().x < edge.center().x - 1.0, "moved in from the right edge: {moved:?}");
    assert!(rig.view().to_screen(moved.center()).distance(centre) < 1e-2, "the box stays put on screen");
    rig.release(end);

    // Command keeps the press frame.
    let mut rig = rig_for(edge, true);
    let (_, end) = turn_by(&mut rig, &[10.0, 20.0], 120.0, Modifiers::COMMAND);
    assert_eq!(rig.rect(), edge.rect, "Command held");
    rig.release(end);
    rig.frames.hold(Modifiers::NONE);

    // A press frame already past the image is kept.
    let past = CropBox::new(Rect::new(150.0, 100.0, 390.0, 260.0), 20.0);
    let mut rig = rig_for(past, true);
    let (_, end) = turn_by(&mut rig, &[5.0, 10.0], 120.0, Modifiers::NONE);
    assert_eq!(rig.rect(), past.rect, "the frame was past the image at the press");
    rig.release(end);

    // A crop that fills what the turn uncovers keeps the press frame.
    let mut s = Setup::new();
    s.image_mode = true;
    s.bounds = Some(image);
    s.fills_uncovered = true;
    s.view = crop_view(&edge, &s.view);
    let mut rig = Rig::new(edge, s);
    let (_, end) = turn_by(&mut rig, &[10.0, 20.0], 120.0, Modifiers::NONE);
    assert!(rig.crop().angle.abs() > 1.0, "it turned: {:?}", rig.crop());
    assert_eq!(rig.rect(), edge.rect, "the fill covers what the turn uncovers");
    rig.release(end);

    // A Classic Mode turn that would leave the image is refused: both steps leave it.
    let mut rig = rig_for(fits, false);
    let (_, end) = turn_by(&mut rig, &[30.0, 40.0], 140.0, Modifiers::NONE);
    assert_eq!(rig.crop(), fits, "a classic turn out of the image");
    rig.release(end);
}

#[trial]
fn a_placed_picture_crop_applies_on_a_click_away_keeps_the_cut_area_and_stops_at_its_minimum_side() {
    // An 80 x 60 frame at 100 %: its corners and edges reach at most a quarter of its short side, 15 points, past it.
    let start = CropBox::upright(Rect::new(160.0, 120.0, 240.0, 180.0));
    let mut setup = Setup::new();
    setup.outside = CropOutside::Apply;
    // The placed picture, turned 8 degrees about the frame's centre.
    setup.area = Some(CropBox::new(Rect::new(100.0, 70.0, 300.0, 230.0), 8.0).corners());
    setup.min_side = Some(6.0);
    let mut rig = Rig::new(start, setup);
    // Between the frame and the picture's outline: a click and a drag are taken, and nothing applies, turns or moves.
    let cut = rig.at(200.0, 95.0);
    rig.click(cut);
    rig.drag(cut, cut + Vec2::new(30.0, 0.0), Modifiers::NONE);
    assert_eq!(rig.seen.borrow().applied, None, "a press in the cut-away area does not apply");
    assert_eq!(rig.crop(), start, "nothing turned or moved");
    assert!(rig.events().is_empty(), "{:?}", rig.events());
    // Away from the picture: a click applies, the frame as it was.
    rig.click(rig.at(40.0, 40.0));
    assert_eq!(rig.seen.borrow().applied, Some(CropApply::Away), "a click away from the picture applies and says it was an away click");
    assert_eq!(rig.crop(), start);
    // A corner dragged past the minimum side stops at the last step that kept it: narrower than it was, not under the
    // minimum, the opposite corner where it was.
    rig.drag(rig.at(240.0, 180.0), rig.at(164.0, 190.0), Modifiers::NONE);
    let kept = rig.rect();
    assert!(kept.width() >= 6.0 && kept.width() < 80.0, "{kept:?}");
    assert!((kept.x0 - 160.0).abs() < 1e-9 && (kept.y0 - 120.0).abs() < 1e-9, "the opposite corner stayed: {kept:?}");
    assert_eq!(rig.events().last(), Some(&"commit"), "{:?}", rig.events());
}

#[trial]
fn a_resize_and_a_move_toward_the_outside_of_a_turned_area_stop_at_its_edge() {
    let start = CropBox::upright(Rect::new(160.0, 120.0, 240.0, 180.0));
    let mut setup = Setup::new();
    setup.outside = CropOutside::Apply;
    setup.area = Some(CropBox::new(Rect::new(100.0, 70.0, 300.0, 230.0), 8.0).corners());
    let mut rig = Rig::new(start, setup);
    // A move far past the picture keeps the frame where it was.
    rig.drag(rig.at(200.0, 150.0), rig.at(400.0, 150.0), Modifiers::NONE);
    assert_eq!(rig.crop(), start, "a move out of the area");
    // A resize inside the area follows the pointer; one past it keeps the frame as the last step left it.
    rig.drag(rig.at(240.0, 150.0), rig.at(270.0, 150.0), Modifiers::NONE);
    let grown = rig.rect();
    assert!(grown.x1 > 240.0 && (grown.x0 - 160.0).abs() < 1e-9, "a resize inside the area: {grown:?}");
    let edge = rig.at(grown.x1, 150.0);
    rig.drag(edge, rig.at(500.0, 150.0), Modifiers::NONE);
    assert_eq!(rig.rect(), grown, "a resize out of the area");
}

#[trial]
fn a_press_outside_a_new_frame_crop_draws_a_new_frame_inside_the_bounds() {
    let start = CropBox::upright(RECT);
    let mut setup = Setup::new();
    setup.outside = CropOutside::NewFrame;
    setup.bounds = Some(Rect::new(50.0, 30.0, 350.0, 270.0));
    let mut rig = Rig::new(start, setup);
    // Outside the frame, past the reach of its corners and edges.
    let from = rig.at(60.0, 40.0);
    rig.click(from);
    assert_eq!(rig.crop(), start, "a click with no drag changes nothing");
    assert!(rig.events().is_empty(), "{:?}", rig.events());
    // A drag draws a new frame from the press, cut to the bounds; nothing turns.
    rig.drag(from, rig.at(380.0, 290.0), Modifiers::NONE);
    let drawn = CropBox::upright(Rect::new(60.0, 40.0, 350.0, 270.0));
    assert!(near(&rig.crop(), &drawn), "{:?} vs {drawn:?}", rig.crop());
    assert_eq!(rig.crop().angle, start.angle, "no turn");
    let events = rig.events();
    assert_eq!((events.first(), events.last()), (Some(&"begin"), Some(&"commit")), "{events:?}");
}

#[trial]
fn a_corner_drag_on_an_area_smaller_than_the_minimum_side_changes_nothing_and_does_not_panic() {
    // A 5 x 5 placed picture cropped to itself, under the 6-unit minimum, zoomed in so its grips show.
    let tiny = Rect::new(100.0, 100.0, 105.0, 105.0);
    let start = CropBox::upright(tiny);
    let mut setup = Setup::new();
    setup.outside = CropOutside::Apply;
    setup.area = Some(start.corners());
    setup.min_side = Some(6.0);
    setup.view = CanvasView::new(egui::Rect::ZERO, tiny.center(), 8.0);
    let mut rig = Rig::new(start, setup);
    // The bottom right grip across the frame to the opposite corner.
    rig.drag(rig.at(105.0, 105.0), rig.at(100.0, 100.0), Modifiers::NONE);
    assert_eq!(rig.crop(), start, "the frame is as it was");
    let events = rig.events();
    assert_eq!(events.first(), Some(&"begin"), "the grip was taken: {events:?}");
    assert_eq!(events.last(), Some(&"cancel"), "a release under the minimum cancels: {events:?}");
    assert!(!events.contains(&"commit"), "{events:?}");
    assert_eq!(rig.seen.borrow().applied, None, "a grip drag does not apply");
}

#[trial]
fn a_shift_corner_follows_the_diagonal_and_snaps_along_it() {
    // An 800 x 500 frame at 100 %, its bottom right corner on screen.
    let whole = CropBox::upright(Rect::new(0.0, 0.0, 800.0, 500.0));
    let setup = |ratio: Option<f64>, snap_x: Option<f64>, snap_y: Option<f64>, at: Point| {
        let mut s = Setup::new();
        s.ratio = ratio;
        (s.snap_x, s.snap_y) = (snap_x, snap_y);
        s.view.center = at;
        s
    };
    let drag = |rig: &mut Rig, corner: Point, by: Vec2| {
        let from = rig.at(corner.x, corner.y);
        rig.drag(from, from + by, Modifiers::SHIFT);
    };
    // Measured: (-100, -50) on the corner gives 706 x 441.
    let mut rig = Rig::new(whole, setup(None, None, None, Point::new(700.0, 400.0)));
    drag(&mut rig, Point::new(800.0, 500.0), Vec2::new(-100.0, -50.0));
    let (w, h) = rig.crop().crop_size().unwrap();
    assert!((w - 706.0).abs() <= 1.0 && (h - 441.0).abs() <= 1.0, "{w} x {h}");
    // A ratio preset keeps the larger relative extent: a 1 : 1 frame of 500 gives 450 square.
    let square = CropBox::upright(Rect::new(150.0, 0.0, 650.0, 500.0));
    let mut rig = Rig::new(square, setup(Some(1.0), None, None, Point::new(550.0, 400.0)));
    drag(&mut rig, Point::new(650.0, 500.0), Vec2::new(-100.0, -50.0));
    let r = rig.rect();
    assert!((r.width() - 450.0).abs() < 1e-3 && (r.height() - 450.0).abs() < 1e-3, "{r:?}");
    // A guide in reach: the handle slides along the diagonal onto it, the proportions kept.
    let free = 800.0 * (1.0 - 105_000.0 / 890_000.0);
    let guide = free - 0.7 * SNAP_PX;
    let mut rig = Rig::new(whole, setup(None, Some(guide), None, Point::new(700.0, 400.0)));
    drag(&mut rig, Point::new(800.0, 500.0), Vec2::new(-100.0, -50.0));
    let r = rig.rect();
    assert!((r.x1 - guide).abs() < 1e-6, "the handle on the guide: {r:?}");
    assert!((r.width() / r.height() - 1.6).abs() < 1e-9, "8 : 5 kept: {r:?}");
    // Turned -31 degrees, the diagonal runs a degree off level: a level guide 3 points from the handle crosses it
    // far outside the reach and is ignored.
    let turned = CropBox::new(Rect::new(100.0, 60.0, 500.0, 310.0), -31.0);
    let corner = turned.corners()[2];
    let by = craft_behavior::geometry::crop::turn(Point::new(-50.0, -25.0), Point::ZERO, -31.0);
    let by = Vec2::new(by.x as f32, by.y as f32);
    let at = corner - craft_behavior::kurbo::Vec2::new(100.0, 80.0);
    let mut rig = Rig::new(turned, setup(None, None, None, at));
    drag(&mut rig, corner, by);
    let want = rig.crop();
    let handle = want.corners()[2];
    let mut rig = Rig::new(turned, setup(None, None, Some(handle.y + 3.0), at));
    drag(&mut rig, corner, by);
    assert!(near(&rig.crop(), &want), "as with no guide: {:?} vs {want:?}", rig.crop());
    assert!(rig.seen.borrow().snapped.is_empty(), "no snap line reported");
}

#[trial]
fn a_document_crop_grows_past_its_bounds_on_an_edge_or_corner_drag() {
    // The canvas as the bounds, as the Crop tool sets them with Delete Cropped Pixels on.
    let canvas = Rect::new(50.0, 50.0, 350.0, 250.0);
    for (what, base) in [("classic", Setup::new()), ("image mode", Setup::image(0.0, false))] {
        let mut setup = base;
        setup.bounds = Some(canvas);
        // An edge dragged past the canvas takes the frame with it: the crop grows the canvas there.
        let mut rig = Rig::new(CropBox::upright(RECT), setup.clone());
        rig.drag(rig.at(RECT.x1, 150.0), rig.at(380.0, 150.0), Modifiers::NONE);
        let r = rig.rect();
        assert!((r.x1 - 380.0).abs() < 1e-6 && r.x0 == RECT.x0, "{what}: the right edge follows past the canvas: {r:?}");
        // So does a corner.
        let mut rig = Rig::new(CropBox::upright(RECT), setup.clone());
        rig.drag(rig.at(RECT.x1, RECT.y1), rig.at(380.0, 280.0), Modifiers::NONE);
        let r = rig.rect();
        assert!(r.x1 > canvas.x1 + 1.0 && r.y1 > canvas.y1 + 1.0, "{what}: the corner follows past the canvas: {r:?}");
    }
}

/// photocraft#16404: moving an upright document crop past the canvas takes in the area there, as an edge drag does; a
/// turned frame's move still stays inside the canvas (#10869).
#[trial]
fn a_document_crop_moved_past_its_bounds_takes_in_the_area_there() {
    let canvas = Rect::new(50.0, 50.0, 350.0, 250.0);
    // Classic Mode moves the frame with the pointer: to the right, past the canvas's right edge.
    let mut setup = Setup::new();
    setup.bounds = Some(canvas);
    let mut rig = Rig::new(CropBox::upright(RECT), setup);
    rig.drag(rig.at(200.0, 150.0), rig.at(300.0, 150.0), Modifiers::NONE);
    let r = rig.rect();
    assert!(r.x1 > canvas.x1 + 1.0 && (r.width() - RECT.width()).abs() < 1e-6, "classic: the frame moves past the canvas: {r:?}");
    // Image mode moves the image under the frame: a drag to the right takes the frame past the canvas's left edge, with
    // Auto Center Preview off and on (the view then follows the frame).
    for auto_center in [false, true] {
        let mut setup = Setup::image(0.0, auto_center);
        setup.bounds = Some(canvas);
        let mut rig = Rig::new(CropBox::upright(RECT), setup);
        rig.drag(rig.at(200.0, 150.0), rig.at(300.0, 150.0), Modifiers::NONE);
        let r = rig.rect();
        assert!(
            r.x0 < canvas.x0 - 1.0 && (r.width() - RECT.width()).abs() < 1e-6,
            "image mode, auto centre {auto_center}: the frame moves past the canvas: {r:?}"
        );
    }
    // A turned frame moves past the canvas too (photocraft#16460): in Classic Mode past the right edge, in image mode
    // (the image turned under an upright frame on screen) past the left edge.
    let mut setup = Setup::new();
    setup.bounds = Some(canvas);
    let mut rig = Rig::new(CropBox::new(RECT, 10.0), setup);
    rig.drag(rig.at(200.0, 150.0), rig.at(300.0, 150.0), Modifiers::NONE);
    let b = rig.crop().turned_bounds();
    assert!(b.x1 > canvas.x1 + 1.0, "classic: a turned frame moves past the canvas: {b:?}");
    assert!((rig.rect().width() - RECT.width()).abs() < 1e-6, "classic: a turned frame keeps its size: {:?}", rig.rect());
    for auto_center in [false, true] {
        let mut setup = Setup::image(10.0, auto_center);
        setup.bounds = Some(canvas);
        let mut rig = Rig::new(CropBox::new(RECT, 10.0), setup);
        rig.drag(rig.at(200.0, 150.0), rig.at(300.0, 150.0), Modifiers::NONE);
        let b = rig.crop().turned_bounds();
        assert!(b.x0 < canvas.x0 - 1.0, "image mode, auto centre {auto_center}: a turned frame moves past the canvas: {b:?}");
        assert!(
            (rig.rect().width() - RECT.width()).abs() < 1e-6,
            "image mode, auto centre {auto_center}: a turned frame keeps its size: {:?}",
            rig.rect()
        );
    }
}

#[trial]
fn an_object_crop_is_bounded_by_its_source_and_a_press_outside_it_reaches_what_lies_beneath() {
    let source = Rect::new(50.0, 50.0, 350.0, 250.0);
    let mut setup = Setup::new();
    setup.object = Some(source);
    let mut rig = Rig::new(CropBox::upright(RECT), setup);
    // The shield dims the source minus the frame: its extent is the source on screen, not the canvas.
    let (low, high) = (rig.at(source.x0, source.y0), rig.at(source.x1, source.y1));
    let shield = rig.shield_extent().expect("a started crop shows a shield");
    for (got, want) in [(shield.min.x, low.x), (shield.min.y, low.y), (shield.max.x, high.x), (shield.max.y, high.y)] {
        assert!((got - want).abs() < 1.0, "the shield stops at the source: {shield:?} against {low:?} {high:?}");
    }
    // A corner dragged past the source stops at it.
    let corner = rig.at(RECT.x1, RECT.y1);
    let past = rig.at(390.0, 290.0);
    rig.drag(corner, past, Modifiers::NONE);
    let r = rig.rect();
    assert!(r.x1 <= source.x1 + 1e-6 && r.y1 <= source.y1 + 1e-6, "the frame stays inside its source: {r:?}");
    assert!(r.x1 > RECT.x1 && r.y1 > RECT.y1, "and the corner still grew toward it: {r:?}");
    // A press outside the source is not taken: no turn part under it, and a drag from there changes nothing.
    let (before, events) = (rig.crop(), rig.events());
    let outside = rig.at(20.0, 20.0);
    rig.move_to(outside);
    assert_eq!(rig.seen.borrow().hovered, None, "nothing outside the source is hovered");
    rig.drag(outside, rig.at(60.0, 60.0), Modifiers::NONE);
    assert_eq!(rig.crop(), before, "a drag from outside the source leaves the frame");
    assert_eq!(rig.events(), events, "and sends no edit");
    // A click inside is taken but does not move the frame.
    let middle = rig.at(before.rect.center().x, before.rect.center().y);
    rig.click(middle);
    assert_eq!(rig.crop(), before, "a click inside leaves the frame");
    assert_eq!(rig.events(), events, "and sends no edit");
    // Enter applies and Escape discards.
    rig.frame(vec![input::key_press(egui::Key::Enter, Modifiers::NONE)]);
    assert_eq!(rig.seen.borrow().applied, Some(CropApply::Enter), "Enter applies");
    rig.frame(vec![input::key_press(egui::Key::Escape, Modifiers::NONE)]);
    assert!(rig.seen.borrow().discarded, "Escape discards");
}

/// photocraft#16410: the shield is one unfeathered mesh. Its pieces used to be feathered one by one, so each faded
/// out along the edges it shares with the next and the canvas showed through as a line under the translucent shield,
/// from the frame's corners to the canvas's edges.
#[trial]
fn the_shield_is_one_unfeathered_mesh_with_no_seam_between_its_pieces() {
    for (what, setup) in [("classic", Setup::new()), ("image mode", Setup::image(0.0, false)), ("turned", Setup::image(10.0, false))] {
        let angle = if what == "turned" { 10.0 } else { 0.0 };
        let mut rig = Rig::new(CropBox::new(RECT, angle), setup);
        let meshes = rig.shield_meshes();
        assert_eq!(meshes.len(), 1, "{what}: the shield is one mesh: {} meshes", meshes.len());
        let alpha = meshes[0].vertices[0].color.a();
        assert!(meshes[0].vertices.iter().all(|v| v.color.a() == alpha), "{what}: no vertex of the shield fades out (a feathered edge)");
    }
}

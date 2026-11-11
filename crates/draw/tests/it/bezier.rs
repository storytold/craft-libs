//! `BezierOverlay`: the pure path operations, and the overlay's pointer, keyboard and tools through the public API.

use craft_behavior::kurbo::{ParamCurve as _, Point, Vec2};
use craft_behavior::{CanvasView, EditEvent};
use craft_draw::ui::{
    AnchorsShown, BezierHit, BezierOverlay, BezierTool, EditPath, EditSubpath, HandleSide, HandlesShown, Knot, KnotRef, PathEdit, PathSelection,
    SegmentMark,
};
use craft_ui::canvas::{Canvas, Guide};
use craft_ui::canvas_controls::Guides;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Id, Pos2, Vec2 as ScreenVec, vec2};
use std::cell::RefCell;
use std::rc::Rc;

fn p(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn kr(knot: usize) -> KnotRef {
    KnotRef::new(0, knot)
}

fn near(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-9
}

// ---- the pure operations ----

fn ops_path(sub: EditSubpath) -> EditPath {
    EditPath::new(7, vec![sub])
}

// A curve split at t = 0.3 draws the same curve (sampled); a straight segment gets a corner on the cubic's point at
// t; t of 0, 1 or NaN is refused and changes nothing.
#[trial]
fn edit_subpath_insert_at_splits_a_curve_without_changing_it() {
    let mut sp = EditSubpath::new(vec![Knot::smooth(p(0.0, 0.0), p(30.0, 60.0)), Knot::smooth(p(100.0, 0.0), p(130.0, -20.0))], false);
    let before = sp.segment(0).unwrap();
    assert_eq!(sp.insert_at(0, 0.3), Some(1));
    assert_eq!(sp.knots.len(), 3);
    assert!(near(sp.knots[1].anchor, before.eval(0.3)));
    let (a, b) = (sp.segment(0).unwrap(), sp.segment(1).unwrap());
    for i in 0..=10 {
        let s = f64::from(i) / 10.0;
        assert!(near(a.eval(s), before.eval(0.3 * s)));
        assert!(near(b.eval(s), before.eval(0.3 + 0.7 * s)));
    }

    let mut line = EditSubpath::new(vec![Knot::corner(p(0.0, 0.0)), Knot::corner(p(100.0, 0.0))], false);
    let at = line.segment(0).unwrap().eval(0.3);
    assert_eq!(line.insert_at(0, 0.3), Some(1));
    assert!(near(line.knots[1].anchor, at));
    assert!(!line.knots[1].smooth);

    let snapshot = sp.clone();
    for t in [0.0, 1.0, f64::NAN] {
        assert_eq!(sp.insert_at(0, t), None);
    }
    assert_eq!(sp.insert_at(5, 0.5), None);
    assert_eq!(sp, snapshot);
}

// Removing an inner knot joins its neighbours with the previous out and next in handles; a closed subpath stays
// closed; removing the only knot leaves an empty subpath.

// A corner square: toggling a knot makes it smooth and the segments around it curve; toggling again retracts its
// handles.

// `InsertCurvature` on a straight segment adds a smooth knot moved by `by` and reflows.

// MoveAnchors, MoveHandle (smooth and alone), BendSegment (straight and curved), Convert give PhotoCraft's kernel
// results; a wrong key or index changes nothing and returns false.
#[trial]
fn path_edit_apply_matches_the_kernel_for_every_existing_edit() {
    let base = || {
        ops_path(EditSubpath::new(
            vec![Knot::smooth(p(0.0, 0.0), p(20.0, 0.0)), Knot::smooth(p(100.0, 0.0), p(120.0, 0.0)), Knot::corner(p(100.0, 80.0))],
            false,
        ))
    };
    let mut ep = base();
    let by = Vec2::new(5.0, 7.0);
    assert!(PathEdit::move_anchors(7, vec![kr(1), kr(1)], by).apply(&mut ep));
    assert_eq!(ep.subpaths[0].knots[1].anchor, p(105.0, 7.0));
    assert_eq!(ep.subpaths[0].knots[1].out_ctrl, p(125.0, 7.0));

    let mut ep = base();
    let to = p(0.0, 30.0);
    let h = PathEdit::move_handle(7, kr(0), HandleSide::Out, to, false);
    assert!(h.apply(&mut ep));
    let k = ep.subpaths[0].knots[0];
    assert_eq!(k.out_ctrl, to);
    assert!(near(k.in_ctrl, p(0.0, -20.0)));
    let mut ep = base();
    let h = PathEdit::move_handle(7, kr(0), HandleSide::Out, to, true);
    assert!(h.apply(&mut ep));
    assert!(!ep.subpaths[0].knots[0].smooth);
    assert_eq!(ep.subpaths[0].knots[0].in_ctrl, p(-20.0, 0.0));

    let mut ep = base();
    let before = ep.subpaths[0].segment(0).unwrap().eval(0.5);
    let bend = PathEdit::bend_segment(7, kr(0), 0.5, Vec2::new(0.0, 10.0));
    assert!(bend.apply(&mut ep));
    assert!(near(ep.subpaths[0].segment(0).unwrap().eval(0.5), before + Vec2::new(0.0, 10.0)));
    let mut ep = base();
    let bend = PathEdit::bend_segment(7, kr(1), 0.5, Vec2::new(0.0, 10.0));
    ep.subpaths[0].knots[1].out_ctrl = ep.subpaths[0].knots[1].anchor;
    ep.subpaths[0].knots[1].in_ctrl = ep.subpaths[0].knots[1].anchor;
    assert!(bend.apply(&mut ep));
    assert_eq!(ep.subpaths[0].knots[1].anchor, p(100.0, 10.0));
    assert_eq!(ep.subpaths[0].knots[2].anchor, p(100.0, 90.0));

    let mut ep = base();
    assert!(PathEdit::convert(7, kr(0), None).apply(&mut ep));
    assert_eq!(ep.subpaths[0].knots[0], Knot::corner(p(0.0, 0.0)));
    assert!(PathEdit::convert(7, kr(2), Some(p(110.0, 80.0))).apply(&mut ep));
    assert_eq!(ep.subpaths[0].knots[2], Knot::smooth(p(100.0, 80.0), p(110.0, 80.0)));

    let mut ep = base();
    let untouched = base();
    let edits = [
        PathEdit::move_anchors(8, vec![kr(0)], by),
        PathEdit::move_anchors(7, vec![kr(0), kr(9)], by),
        PathEdit::move_handle(7, KnotRef::new(3, 0), HandleSide::In, to, false),
        PathEdit::bend_segment(7, kr(2), 0.5, by),
        PathEdit::convert(7, kr(5), None),
        PathEdit::toggle_curvature(7, kr(5)),
        PathEdit::insert_curvature(7, kr(0), 0.0, by),
    ];
    for e in edits {
        assert!(!e.apply(&mut ep));
        assert_eq!(ep, untouched);
    }
}

// ---- the overlay on a canvas ----

#[derive(Debug, Clone, Copy, PartialEq)]
enum Seen {
    Begin,
    Update,
    Commit,
    Cancel,
    Set,
}

struct State {
    paths: Vec<EditPath>,
    selection: PathSelection,
    guides: Vec<Guide>,
    tool: BezierTool,
    handles: HandlesShown,
    anchors: AnchorsShown,
    segment_mark: SegmentMark,
    rotation: f64,
    flip: bool,
    view: Option<CanvasView>,
    id: Id,
    seen: Vec<Seen>,
    changes: Vec<PathEdit>,
    inserts: Vec<(u64, KnotRef, f64)>,
    deletes: Vec<(u64, Vec<KnotRef>)>,
    hover: Option<BezierHit>,
    /// Passes that reported a selection and no edit event.
    lone_selects: usize,
    /// Presses the overlay left to the canvas's tool.
    tool_clicks: usize,
}

struct Harness {
    fixture: Fixture,
    state: Rc<RefCell<State>>,
}

/// Path 1: an open subpath of four anchors: a straight segment, then two curved ones.
fn path_a() -> EditPath {
    let knots = vec![
        Knot::corner(p(60.0, 60.0)),
        Knot::corner(p(160.0, 60.0)),
        Knot::new(p(260.0, 60.0), p(230.0, 20.0), p(290.0, 100.0), true),
        Knot::corner(p(340.0, 160.0)),
    ];
    EditPath::new(1, vec![EditSubpath::new(knots, false)])
}

/// Path 2, drawn after path 1: a closed square and an open line.
fn path_b() -> EditPath {
    let square = [(60.0, 200.0), (140.0, 200.0), (140.0, 260.0), (60.0, 260.0)].map(|(x, y)| Knot::corner(p(x, y)));
    let line = [(200.0, 220.0), (260.0, 260.0)].map(|(x, y)| Knot::corner(p(x, y)));
    let subpaths = vec![EditSubpath::new(square.to_vec(), true), EditSubpath::new(line.to_vec(), false)];
    EditPath::new(2, subpaths).colored(Some(craft_theme::Srgba::rgb(0.9, 0.43, 0.08)))
}

fn kind(event: &EditEvent<Vec<EditPath>>) -> Seen {
    match event {
        EditEvent::Begin { .. } => Seen::Begin,
        EditEvent::Update { .. } => Seen::Update,
        EditEvent::Commit { .. } => Seen::Commit,
        EditEvent::Cancel { .. } => Seen::Cancel,
        _ => Seen::Set,
    }
}

fn harness(paths: Vec<EditPath>) -> Harness {
    let state = Rc::new(RefCell::new(State {
        paths,
        selection: PathSelection::default(),
        guides: Vec::new(),
        tool: BezierTool::Direct,
        handles: HandlesShown::Selected,
        anchors: AnchorsShown::All,
        segment_mark: SegmentMark::HandlesOnly,
        rotation: 0.0,
        flip: false,
        view: None,
        id: Id::NULL,
        seen: Vec::new(),
        changes: Vec::new(),
        inserts: Vec::new(),
        deletes: Vec::new(),
        hover: None,
        lone_selects: 0,
        tool_clicks: 0,
    }));
    let shown = Rc::clone(&state);
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let st = &mut *held;
        let room = ui.available_rect_before_wrap();
        let view = CanvasView::new(room, p(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0).rotation(st.rotation).flip_x(st.flip);
        st.view = Some(view);
        let selection = st.selection.clone();
        let canvas = Canvas::new("Document").view(view).show(ui, |ui, body| {
            // The guides lie under the paths: a stack position hides the levels of the ones below it.
            if !st.guides.is_empty() {
                Guides::new("Guides", Bind::bound(&mut st.guides)).interactive(true).show_at(ui, body.rect);
            }
            BezierOverlay::new("Paths", Bind::bound(&mut st.paths), &selection)
                .tool(st.tool)
                .handles(st.handles)
                .anchors(st.anchors)
                .segment_mark(st.segment_mark)
                .show_at(ui, body.rect)
        });
        if canvas.output.click_at().is_some() {
            st.tool_clicks += 1;
        }
        let Some(output) = canvas.inner else { return };
        st.id = output.edit.response.id;
        st.seen.extend(output.edit.events.iter().map(kind));
        st.hover = output.hover;
        st.changes.extend(output.change);
        if let Some(select) = output.select {
            st.lone_selects += usize::from(output.edit.events.is_empty());
            st.selection = select;
        }
        if let Some((path, segment, t)) = output.insert {
            st.inserts.push((path, segment, t));
            if let Some(sub) = st.paths.iter_mut().find(|e| e.key == path).and_then(|e| e.subpaths.get_mut(segment.subpath)) {
                sub.insert_at(segment.knot, t);
            }
        }
        if let Some((path, mut knots)) = output.delete {
            st.deletes.push((path, knots.clone()));
            knots.sort_by(|a, b| b.cmp(a));
            if let Some(entry) = st.paths.iter_mut().find(|e| e.key == path) {
                for k in knots {
                    if let Some(sub) = entry.subpaths.get_mut(k.subpath) {
                        sub.remove_knot(k.knot);
                    }
                }
            }
        }
    });
    fixture.settle();
    Harness { fixture, state }
}

fn two_paths() -> Harness {
    harness(vec![path_a(), path_b()])
}

impl Harness {
    /// Changes the state and shows a settled frame.
    fn update(&mut self, change: impl FnOnce(&mut State)) {
        change(&mut self.state.borrow_mut());
        self.fixture.settle();
    }

    fn screen(&self, at: Point) -> Pos2 {
        self.state.borrow().view.expect("shown").to_screen(at)
    }

    fn last_change(&self) -> Option<PathEdit> {
        self.state.borrow().changes.last().cloned()
    }

    fn press_at(&mut self, at: Pos2) {
        self.fixture.frame(vec![pointer::moved(at)]);
        self.fixture.frame(vec![pointer::press(at)]);
    }

    fn move_to(&mut self, at: Pos2) {
        self.fixture.frame(vec![pointer::moved(at)]);
    }

    fn release_at(&mut self, at: Pos2) {
        self.fixture.frame(vec![pointer::release(at)]);
        self.fixture.frame(Vec::new());
    }

    /// A drag over several frames: press, two moves, release.
    fn drag(&mut self, from: Pos2, by: ScreenVec) {
        self.press_at(from);
        self.move_to(from + by * 0.5);
        self.move_to(from + by);
        self.release_at(from + by);
    }
}

fn sel(path: u64, anchors: &[(usize, usize)]) -> PathSelection {
    PathSelection::new(Some(path), anchors.iter().map(|&(s, k)| KnotRef::new(s, k)).collect(), None)
}

/// A distance past any click slop.
const FAR: f32 = 20.0;

// `hit_test` answers what a press takes, without a pass: handles, then anchors, then segments, in a rotated and
// flipped view, as the overlay's own press does.
#[trial]
fn hit_test_matches_the_overlays_press_priority() {
    for (rotation, flip) in [(0.0, false), (40.0, true)] {
        let mut h = two_paths();
        h.update(|st| (st.rotation, st.flip, st.selection) = (rotation, flip, sel(1, &[(0, 2)])));
        let view = h.state.borrow().view.expect("shown");
        let test = |at: Pos2| {
            let st = h.state.borrow();
            BezierOverlay::hit_test(&st.paths, &st.selection, HandlesShown::Selected, &view, at, 6.0)
        };
        let handle = h.screen(p(290.0, 100.0));
        assert!(matches!(test(handle), Some(BezierHit::Handle { path: 1, knot, side: HandleSide::Out, .. }) if knot == kr(2)));
        assert!(matches!(test(h.screen(p(160.0, 60.0))), Some(BezierHit::Anchor { path: 1, knot, .. }) if knot == kr(1)));
        assert!(matches!(test(h.screen(p(110.0, 60.0))), Some(BezierHit::Segment { path: 1, segment, .. }) if segment == kr(0)));
        assert_eq!(test(h.screen(p(10.0, 10.0))), None);
        // The press on the handle drags it, as the test says.
        h.drag(handle, vec2(0.0, FAR));
        assert!(matches!(h.last_change(), Some(PathEdit::MoveHandle { side: HandleSide::Out, .. })), "{:?}", h.last_change());
    }
}

// Hovering a segment under Add reports the segment with its parameter, which the preview anchor stands at.

// `AnchorsShown::Hidden` draws outlines only: no anchor, no handle, no part, no AccessKit object.

// ---- the anchor tools and the Curvature tool ----

// Arrows, select all and Escape belong to Direct, Convert and Curvature; Add, Delete and Auto Add/Delete have none.

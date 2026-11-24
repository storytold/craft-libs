//! `BezierOverlay`: the pure path operations, and the overlay's pointer, keyboard and tools through the public API.

use craft_behavior::kurbo::{ParamCurve as _, Point, Size, Vec2};
use craft_behavior::{CanvasView, EditEvent};
use craft_draw::ui::{
    AnchorDelete, AnchorTool, AnchorsShown, BezierHit, BezierOverlay, BezierTool, EditPath, EditSubpath, HandleSide, HandlesShown, Knot, KnotRef,
    PathEdit, PathSelection, PathTool, SegmentMark,
};
use craft_ui::canvas::{Canvas, Guide, ToolHost, tool_pass};
use craft_ui::canvas_controls::Guides;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::primitives::cancel_press_of;
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
fn path_edit_pull_anchor_inserts_and_pulls_out_the_handles() {
    let straight = || ops_path(EditSubpath::new(vec![Knot::corner(p(0.0, 0.0)), Knot::corner(p(100.0, 0.0))], false));
    let mut path = straight();
    assert!(PathEdit::pull_anchor(7, kr(0), 0.5, p(50.0, 30.0), None).apply(&mut path));
    let k = path.subpaths[0].knots[1];
    assert!(near(k.anchor, p(50.0, 0.0)) && near(k.out_ctrl, p(50.0, 30.0)) && near(k.in_ctrl, p(50.0, -30.0)) && k.smooth, "{k:?}");
    let mut path = straight();
    assert!(PathEdit::pull_anchor(7, kr(0), 0.5, p(50.0, 30.0), Some(p(80.0, 0.0))).apply(&mut path));
    let k = path.subpaths[0].knots[1];
    assert!(near(k.in_ctrl, p(80.0, 0.0)) && !k.smooth, "{k:?}");
    for bad in [
        PathEdit::pull_anchor(7, kr(0), 0.5, p(f64::NAN, 0.0), None),
        PathEdit::pull_anchor(7, kr(0), 0.5, p(1.0, 1.0), Some(p(0.0, f64::INFINITY))),
        PathEdit::pull_anchor(7, kr(0), 1.5, p(1.0, 1.0), None),
        PathEdit::pull_anchor(7, kr(4), 0.5, p(1.0, 1.0), None),
        PathEdit::pull_anchor(7, KnotRef::new(3, 0), 0.5, p(1.0, 1.0), None),
    ] {
        let mut path = straight();
        assert!(!bad.apply(&mut path), "{bad:?}");
        assert_eq!(path, straight());
    }
}

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
        PathEdit::insert_anchor(7, kr(5), 0.5, by),
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
    /// The canvas shows a document and keeps the view it reports, so page keys move it.
    pans: bool,
    /// The canvas's precise cursors (Preferences > Cursors > Other Cursors).
    precise: bool,
    view: Option<CanvasView>,
    id: Id,
    seen: Vec<Seen>,
    changes: Vec<PathEdit>,
    inserts: Vec<(u64, KnotRef, f64)>,
    deletes: Vec<AnchorDelete>,
    hover: Option<BezierHit>,
    /// Passes that reported a selection and no edit event.
    lone_selects: usize,
    /// Presses the overlay left to the canvas's tool.
    tool_clicks: usize,
    /// The anchor tool the canvas runs under the overlay, if any.
    canvas_tool: Option<PathTool>,
    /// Passes in which that tool ran a gesture.
    tool_gestures: usize,
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
    harness_with(paths, false)
}

fn harness_with(paths: Vec<EditPath>, pans: bool) -> Harness {
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
        pans,
        precise: false,
        view: None,
        id: Id::NULL,
        seen: Vec::new(),
        changes: Vec::new(),
        inserts: Vec::new(),
        deletes: Vec::new(),
        hover: None,
        lone_selects: 0,
        tool_clicks: 0,
        canvas_tool: None,
        tool_gestures: 0,
    }));
    let shown = Rc::clone(&state);
    let mut fixture = Fixture::new(move |ui| {
        let mut held = shown.borrow_mut();
        let st = &mut *held;
        let room = ui.available_rect_before_wrap();
        let view = match st.view.filter(|_| st.pans) {
            Some(mut kept) => {
                kept.rect = room;
                kept
            }
            None => {
                CanvasView::new(room, p(f64::from(room.width()) / 2.0, f64::from(room.height()) / 2.0), 1.0).rotation(st.rotation).flip_x(st.flip)
            }
        };
        st.view = Some(view);
        let selection = st.selection.clone();
        let canvas_id = ui.scope_id().with("document canvas");
        let canvas = Canvas::new("Document").id(canvas_id).view(view).precise_cursors(st.precise);
        let canvas = if st.pans { canvas.document(Size::new(4000.0, 4000.0)) } else { canvas };
        let mut reply = None;
        let canvas = canvas.show(ui, |ui, body| {
            // The guides lie under the paths: a stack position hides the levels of the ones below it.
            if !st.guides.is_empty() {
                Guides::new("Guides", canvas_id, Bind::bound(&mut st.guides)).interactive(true).show_at(ui, body.rect);
            }
            let output = BezierOverlay::new("Paths", Bind::bound(&mut st.paths), &selection)
                .tool(st.tool)
                .handles(st.handles)
                .anchors(st.anchors)
                .segment_mark(st.segment_mark)
                .show_at(ui, body.rect);
            if let (Some(tool), Some(pass)) = (st.canvas_tool, tool_pass(&body.env)) {
                reply = Some(ToolHost::new().run(ui, &pass, &mut AnchorTool::new(tool)));
            }
            output
        });
        if canvas.output.click_at().is_some() {
            st.tool_clicks += 1;
        }
        if let Some(reply) = reply {
            st.tool_gestures += usize::from(reply.gesture.is_some());
        }
        if st.pans {
            st.view = canvas.output.view.or(st.view);
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
        if let Some(delete) = output.delete {
            let (path, mut knots) = (delete.path, delete.anchors.clone());
            st.deletes.push(delete);
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
        self.fixture.frame_held(vec![pointer::press(at)]);
    }

    fn move_to(&mut self, at: Pos2) {
        self.fixture.frame(vec![pointer::moved(at)]);
    }

    fn release_at(&mut self, at: Pos2) {
        self.fixture.frame_held(vec![pointer::release(at)]);
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

/// The paint of `selection` over both paths, as text.
fn painted(selection: PathSelection) -> String {
    let mut h = two_paths();
    h.update(|st| st.selection = selection);
    format!("{:?}", h.fixture.frame_shapes())
}

// An anchor listed in `others` draws selected but is no part of the edit: no handle of it answers a hit, and a press
// on it edits only that anchor, never the edited path's selected knots.
#[trial]
fn anchors_of_other_paths_draw_selected_and_stay_out_of_the_edit() {
    let mut other = path_b();
    other.subpaths[0].knots[0] = Knot::smooth(p(60.0, 200.0), p(90.0, 170.0));
    let mut h = harness(vec![path_a(), other]);
    h.update(|st| st.selection = sel(1, &[(0, 1)]).also(2, vec![kr(0)]));
    let view = h.state.borrow().view.expect("shown");
    {
        let st = h.state.borrow();
        let at = h.screen(p(90.0, 170.0));
        let hit = BezierOverlay::hit_test(&st.paths, &st.selection, HandlesShown::Selected, &view, at, 6.0);
        assert!(!matches!(hit, Some(BezierHit::Handle { .. })), "{hit:?}");
    }
    let anchor = h.screen(p(60.0, 200.0));
    h.drag(anchor, vec2(0.0, FAR));
    let change = h.last_change();
    assert!(matches!(&change, Some(PathEdit::MoveAnchors { path: 2, anchors, .. }) if *anchors == vec![kr(0)]), "{change:?}");

    let listed = painted(sel(1, &[]).also(2, vec![kr(0)]));
    assert_eq!(listed, painted(sel(2, &[(0, 0)])), "a listed anchor fills as a selected one");
    assert_ne!(listed, painted(sel(1, &[])), "and differs from an unselected one");
}

// One anchor selected over all paths shows its handles; one in each path is two, so none.
#[trial]
fn one_anchor_shows_handles_only_while_one_anchor_is_selected_over_all_paths() {
    let mut h = two_paths();
    h.update(|st| st.handles = HandlesShown::OneAnchor);
    let view = h.state.borrow().view.expect("shown");
    let handle = h.screen(p(290.0, 100.0));
    let test = |selection: PathSelection| {
        let st = h.state.borrow();
        BezierOverlay::hit_test(&st.paths, &selection, HandlesShown::OneAnchor, &view, handle, 6.0)
    };
    assert!(matches!(test(sel(1, &[(0, 2)])), Some(BezierHit::Handle { path: 1, .. })));
    let two = test(sel(1, &[(0, 2)]).also(2, vec![kr(0)]));
    assert!(!matches!(two, Some(BezierHit::Handle { .. })), "{two:?}");
    let none = test(PathSelection::default());
    assert!(!matches!(none, Some(BezierHit::Handle { .. })), "{none:?}");
}

// Hovering a segment under Add reports the segment with its parameter, which the preview anchor stands at.

// `AnchorsShown::Hidden` draws outlines only: no anchor, no handle, no part, no AccessKit object.

// ---- the anchor tools and the Curvature tool ----

// Arrows and select all belong to Direct, Convert and Curvature; Add, Delete and Auto Add/Delete have none.

#[trial]
fn cmd_or_ctrl_with_page_down_on_a_focused_anchor_scrolls_the_canvas_and_picks_no_part() {
    let mut h = harness_with(vec![path_a(), path_b()], true);
    // The canvas, then the overlay; Home picks the first anchor of path 1.
    h.fixture
        .key(egui::Key::Tab, egui::Modifiers::NONE)
        .key(egui::Key::Tab, egui::Modifiers::NONE)
        .key(egui::Key::Home, egui::Modifiers::NONE)
        .settle();
    let picked = h.state.borrow().selection.clone();
    assert!(!picked.anchors.is_empty(), "Home picks an anchor");
    let before = h.state.borrow().view.expect("the canvas showed").center;
    h.fixture.key(egui::Key::PageDown, egui::Modifiers::COMMAND).settle();
    let after = h.state.borrow().view.expect("the canvas showed").center;
    assert!(after.x > before.x && after.y == before.y, "the overlay leaves Cmd/Ctrl+Page Down to the canvas: {before:?} -> {after:?}");
    assert_eq!(h.state.borrow().selection, picked, "the page key picks no part");
}

// The Pen's Auto Add/Delete on a segment: a press, a drag and a release are one gesture that inserts the anchor and
// moves it, with the new anchor selected. Add Anchor's click inserts it where the segment was pressed and moves
// nothing (its drag pulls out handles: `an_add_anchor_drag_pulls_out_the_new_anchors_handles`).
#[trial]
fn an_add_anchor_drag_inserts_and_moves_the_new_anchor_in_one_gesture() {
    let mut h = two_paths();
    h.update(|st| st.tool = BezierTool::AutoAddDelete);
    let on = h.screen(p(110.0, 60.0));
    h.drag(on, vec2(0.0, FAR));
    {
        let st = h.state.borrow();
        assert_eq!(st.seen.iter().filter(|s| **s == Seen::Begin).count(), 1, "{:?}", st.seen);
        assert_eq!(st.seen.first(), Some(&Seen::Begin));
        assert_eq!(st.seen.last(), Some(&Seen::Commit));
        assert!(!st.seen.contains(&Seen::Set), "{:?}", st.seen);
        let Some(PathEdit::InsertAnchor { path: 1, segment, by, .. }) = st.changes.last().cloned() else { panic!("{:?}", st.changes) };
        assert_eq!(segment, kr(0));
        assert!(by.y > 10.0 && by.x.abs() < 1e-6, "{by:?}");
        let knots = &st.paths[0].subpaths[0].knots;
        assert_eq!(knots.len(), 5);
        assert!((knots[1].anchor.y - (60.0 + by.y)).abs() < 1e-6, "the new anchor moved with the pointer: {:?}", knots[1].anchor);
        assert_eq!(st.selection, sel(1, &[(0, 1)]));
        assert!(st.inserts.is_empty(), "the Insert key's report is not used");
    }

    // A click inserts without moving.
    let mut h = two_paths();
    h.update(|st| st.tool = BezierTool::AddAnchor);
    let on = h.screen(p(110.0, 60.0));
    h.press_at(on);
    h.release_at(on);
    let st = h.state.borrow();
    assert!(matches!(st.changes.last(), Some(PathEdit::InsertAnchor { by, .. }) if *by == Vec2::ZERO), "{:?}", st.changes);
    assert_eq!(st.paths[0].subpaths[0].knots.len(), 5);
    assert_eq!(st.selection, sel(1, &[(0, 1)]));
}

// Add Anchor's drag from a segment inserts the anchor where the segment was pressed and pulls out its handles, as the
// reference application does (PhotoCraft #12707): the out handle follows the pointer and the in handle mirrors it, in
// one gesture, the new anchor selected and smooth; ⇧ turns the handle to the nearest 45°, an axis keeping the
// drag's offset along it and a diagonal the shorter offset; ⌥ pressed during the drag breaks the pair, so only the
// out handle follows from then on.
#[trial]
fn an_add_anchor_drag_pulls_out_the_new_anchors_handles() {
    let add_anchor = || {
        let mut h = two_paths();
        h.update(|st| st.tool = BezierTool::AddAnchor);
        h
    };
    let close = |a: Vec2, b: Vec2| (a - b).hypot() < 1e-6;
    let on = p(110.0, 60.0);

    let mut h = add_anchor();
    let at = h.screen(on);
    h.drag(at, vec2(0.0, 60.0));
    {
        let st = h.state.borrow();
        assert_eq!(st.seen.iter().filter(|s| **s == Seen::Begin).count(), 1, "{:?}", st.seen);
        assert_eq!((st.seen.first(), st.seen.last()), (Some(&Seen::Begin), Some(&Seen::Commit)), "{:?}", st.seen);
        assert!(
            matches!(st.changes.last(), Some(PathEdit::PullAnchor { path: 1, segment, in_ctrl: None, .. }) if *segment == kr(0)),
            "{:?}",
            st.changes
        );
        let knots = &st.paths[0].subpaths[0].knots;
        assert_eq!(knots.len(), 5);
        let k = knots[1];
        assert!(k.anchor.distance(on) < 1e-3, "the anchor stays where the segment was pressed: {k:?}");
        assert!(close(k.out_ctrl - k.anchor, Vec2::new(0.0, 60.0)) && close(k.in_ctrl - k.anchor, Vec2::new(0.0, -60.0)), "{k:?}");
        assert!(k.smooth);
        assert_eq!(st.selection, sel(1, &[(0, 1)]));
    }

    // ⇧: the measured drags.
    for (by, out) in [((30.0, 80.0), (0.0, 80.0)), ((70.0, 40.0), (40.0, 40.0)), ((-50.0, -60.0), (-50.0, -50.0)), ((60.0, -20.0), (60.0, 0.0))] {
        let mut h = add_anchor();
        let at = h.screen(on);
        h.fixture.hold(egui::Modifiers::SHIFT);
        h.drag(at, vec2(by.0, by.1));
        h.fixture.hold(egui::Modifiers::NONE);
        let st = h.state.borrow();
        let k = st.paths[0].subpaths[0].knots[1];
        let out = Vec2::new(out.0, out.1);
        assert!(close(k.out_ctrl - k.anchor, out) && close(k.in_ctrl - k.anchor, -out), "{by:?}: {k:?}");
    }

    // ⌥ during the drag: the in handle stays where the pair had it, the out handle follows on.
    let mut h = add_anchor();
    let at = h.screen(on);
    h.press_at(at);
    h.move_to(at + vec2(-30.0, 0.0));
    h.move_to(at + vec2(-60.0, 0.0));
    h.fixture.hold(egui::Modifiers::ALT);
    h.move_to(at + vec2(-60.0, 20.0));
    h.move_to(at + vec2(-60.0, 40.0));
    h.release_at(at + vec2(-60.0, 40.0));
    h.fixture.hold(egui::Modifiers::NONE);
    let st = h.state.borrow();
    let k = st.paths[0].subpaths[0].knots[1];
    assert!(close(k.out_ctrl - k.anchor, Vec2::new(-60.0, 40.0)) && close(k.in_ctrl - k.anchor, Vec2::new(60.0, 0.0)), "{k:?}");
    assert!(!k.smooth, "a broken pair is a corner");
    assert_eq!(st.seen.iter().filter(|s| **s == Seen::Begin).count(), 1, "{:?}", st.seen);
}

// Convert on a segment: a drag on a straight one bends it when nothing of the path is selected and moves the path when
// every anchor is, a partial selection and a curved segment do nothing; handles within the straightness tolerance of
// their anchors still make the segment straight.
#[trial]
fn convert_bends_a_straight_segment_and_moves_a_fully_selected_path() {
    let mut h = two_paths();
    h.update(|st| st.tool = BezierTool::Convert);
    let straight = h.screen(p(110.0, 60.0));
    h.drag(straight, vec2(0.0, FAR));
    assert!(matches!(h.last_change(), Some(PathEdit::BendSegment { path: 1, .. })), "{:?}", h.last_change());
    let moved = h.state.borrow().paths[0].subpaths[0].knots.iter().take(2).map(|k| k.anchor.y).collect::<Vec<_>>();
    assert!(moved.iter().all(|y| *y > 70.0), "both ends of a straight segment move: {moved:?}");

    // A curved segment stays untaken.
    let mut h = two_paths();
    h.update(|st| st.tool = BezierTool::Convert);
    let curved_at = h.state.borrow().paths[0].subpaths[0].segment(1).expect("segment").eval(0.5);
    let curved = h.screen(curved_at);
    let before = h.state.borrow().paths.clone();
    h.drag(curved, vec2(0.0, FAR));
    assert!(h.state.borrow().changes.is_empty(), "{:?}", h.last_change());
    assert_eq!(h.state.borrow().paths, before);

    // A part of the path selected: nothing; every anchor selected: the whole path moves.
    let mut h = two_paths();
    let straight = h.screen(p(110.0, 60.0));
    h.update(|st| (st.tool, st.selection) = (BezierTool::Convert, sel(1, &[(0, 0)])));
    h.drag(straight, vec2(0.0, FAR));
    assert!(h.state.borrow().changes.is_empty(), "{:?}", h.last_change());
    h.update(|st| st.selection = sel(1, &[(0, 0), (0, 1), (0, 2), (0, 3)]));
    h.drag(straight, vec2(0.0, FAR));
    assert!(matches!(h.last_change(), Some(PathEdit::MoveAnchors { path: 1, anchors, .. }) if anchors.len() == 4), "{:?}", h.last_change());

    // Handles a hair off their anchors: the segment is straight all the same.
    let mut h = two_paths();
    h.update(|st| {
        st.tool = BezierTool::Convert;
        let knots = &mut st.paths[0].subpaths[0].knots;
        knots[0].out_ctrl.x += 1e-12;
        knots[1].in_ctrl.x -= 1e-12;
    });
    h.drag(straight, vec2(0.0, FAR));
    assert!(matches!(h.last_change(), Some(PathEdit::BendSegment { path: 1, .. })), "{:?}", h.last_change());
}

// Delete reports the selected anchors with `segments` true under the tools that delete anchors; under Anchors and Auto
// Add/Delete it stays with the pen draft. A Delete Anchor click reports one anchor with `segments` false.
#[trial]
fn delete_keys_report_segments_and_only_deleting_tools_take_them() {
    for (tool, takes) in [(BezierTool::Direct, true), (BezierTool::AutoAddDelete, false), (BezierTool::Anchors, false)] {
        let mut h = two_paths();
        h.update(|st| (st.tool, st.selection) = (tool, sel(1, &[(0, 1)])));
        h.fixture.key(egui::Key::Tab, egui::Modifiers::NONE).key(egui::Key::Tab, egui::Modifiers::NONE);
        h.fixture.key(egui::Key::Delete, egui::Modifiers::NONE).settle();
        let st = h.state.borrow();
        if takes {
            assert_eq!(st.deletes, [AnchorDelete::new(1, vec![kr(1)], true)], "{tool:?}");
        } else {
            assert!(st.deletes.is_empty(), "{tool:?} leaves Delete to the pen draft: {:?}", st.deletes);
        }
    }

    let mut h = two_paths();
    h.update(|st| st.tool = BezierTool::DeleteAnchor);
    let anchor = h.screen(p(160.0, 60.0));
    h.press_at(anchor);
    h.release_at(anchor);
    assert_eq!(h.state.borrow().deletes, [AnchorDelete::new(1, vec![kr(1)], false)]);
}

// Delete Anchor on a segment: a drag bends it as Direct does and deletes nothing.
#[trial]
fn a_delete_anchor_drag_on_a_segment_bends_it() {
    let mut h = two_paths();
    h.update(|st| st.tool = BezierTool::DeleteAnchor);
    let segment = h.screen(p(110.0, 60.0));
    h.drag(segment, vec2(0.0, FAR));
    let st = h.state.borrow();
    assert!(matches!(st.changes.last(), Some(PathEdit::BendSegment { path: 1, .. })), "{:?}", st.changes);
    assert!(st.deletes.is_empty(), "{:?}", st.deletes);
}

// ⌥ with Delete Anchor on an anchor: a click picks the anchor's whole subpath and edits nothing; a drag selects
// nothing.
#[trial]
fn alt_under_delete_anchor_picks_the_subpath_on_a_click_and_nothing_on_a_drag() {
    let mut h = two_paths();
    h.update(|st| st.tool = BezierTool::DeleteAnchor);
    let anchor = h.screen(p(160.0, 60.0));
    h.fixture.hold(egui::Modifiers::ALT);
    h.press_at(anchor);
    h.release_at(anchor);
    {
        let st = h.state.borrow();
        assert_eq!(st.selection, sel(1, &[(0, 0), (0, 1), (0, 2), (0, 3)]));
        assert!(st.deletes.is_empty() && st.changes.is_empty() && st.seen.is_empty(), "{:?} {:?} {:?}", st.deletes, st.changes, st.seen);
    }

    h.update(|st| st.selection = PathSelection::default());
    h.drag(anchor, vec2(FAR, FAR));
    let st = h.state.borrow();
    assert_eq!(st.selection, PathSelection::default());
    assert!(st.deletes.is_empty() && st.changes.is_empty() && st.seen.is_empty());
}

#[trial]
fn insert_on_a_picked_anchor_adds_one_halfway_along_the_segment_leaving_it() {
    let mut h = two_paths();
    // The canvas, then the overlay; Home picks the first anchor of path 1.
    h.fixture.key(egui::Key::Tab, egui::Modifiers::NONE).key(egui::Key::Tab, egui::Modifiers::NONE).key(egui::Key::Home, egui::Modifiers::NONE);
    h.fixture.key(egui::Key::Insert, egui::Modifiers::NONE).settle();
    assert_eq!(h.state.borrow().inserts, [(1, kr(0), 0.5)]);
    // End picks the last anchor of path 2's open line: no segment leaves it.
    h.fixture.key(egui::Key::End, egui::Modifiers::NONE).key(egui::Key::Insert, egui::Modifiers::NONE).settle();
    assert_eq!(h.state.borrow().inserts.len(), 1, "nothing to insert after the last anchor of an open subpath");
}

// ⇧ during an anchor drag: a 5:12 move stays on the horizontal, also with the f32 noise of the screen mapping, a 12:5
// one on the vertical, a 45° move goes on the diagonal; the drag proposes what it commits (#13684).
#[trial]
fn shift_keeps_an_anchor_drag_on_the_axis_or_the_diagonal() {
    let cases =
        [((60.0, -25.0), p(220.0, 60.0)), ((60.0, -25.00005), p(220.0, 60.0)), ((25.0, 60.0), p(160.0, 120.0)), ((50.0, -50.0), p(210.0, 10.0))];
    for ((dx, dy), want) in cases {
        let mut h = two_paths();
        let anchor = h.screen(p(160.0, 60.0));
        let to = anchor + vec2(dx, dy);
        h.press_at(anchor);
        // ⇧ at the press would add the anchor to the selection: it comes once the drag is under way.
        h.move_to(anchor + vec2(dx, dy) * 0.5);
        h.fixture.hold(egui::Modifiers::SHIFT);
        h.move_to(to);
        let Some(PathEdit::MoveAnchors { by, .. }) = h.last_change() else { panic!("an anchor move: {:?}", h.last_change()) };
        let proposed = p(160.0, 60.0) + by;
        h.release_at(to);
        h.fixture.hold(egui::Modifiers::NONE);
        let st = h.state.borrow();
        let committed = st.paths[0].subpaths[0].knots[1].anchor;
        assert!(proposed.distance(want) < 1e-3, "({dx}, {dy}) proposes {proposed:?}, wants {want:?}");
        assert!(committed.distance(proposed) < 1e-9, "({dx}, {dy}) commits {committed:?}, proposed {proposed:?}");
        assert_eq!(st.seen.last(), Some(&Seen::Commit));
    }
}

// ⇧ during a handle drag turns the handle to the nearest 45° about its anchor, also when pressed mid-drag: about 20°
// lands on the axis, about 40° on the diagonal, and photocraft#15760's measured (-78, +140) on (-78, +78); without ⇧
// the handle follows the pointer.
#[trial]
fn shift_locks_a_handle_drag_to_the_nearest_45_degrees() {
    let anchor = p(260.0, 60.0);
    let cases =
        [(p(360.0, 96.0), p(360.0, 60.0)), (p(340.0, 127.0), p(327.0, 127.0)), (anchor + Vec2::new(-78.0, 140.0), anchor + Vec2::new(-78.0, 78.0))];
    for (shift, (target, locked)) in cases.iter().flat_map(|c| [(true, *c), (false, *c)]) {
        let mut h = two_paths();
        h.update(|st| st.selection = sel(1, &[(0, 2)]));
        let handle = h.screen(p(290.0, 100.0));
        let to = h.screen(target);
        h.press_at(handle);
        h.move_to(handle + (to - handle) * 0.5);
        if shift {
            h.fixture.hold(egui::Modifiers::SHIFT);
        }
        h.move_to(to);
        let Some(PathEdit::MoveHandle { to: got, .. }) = h.last_change() else { panic!("a handle move: {:?}", h.last_change()) };
        h.release_at(to);
        h.fixture.hold(egui::Modifiers::NONE);
        let want = if shift { locked } else { target };
        assert!(got.distance(want) < 1e-3, "shift {shift}: {target:?} gives {got:?}, wants {want:?}");
    }
}

// Delete Anchor with only some subpaths' anchors shown (#12769): a click on a hidden anchor deletes nothing and
// edits nothing, selects that anchor and shows only its subpath; the next click on it deletes it.
#[trial]
fn a_delete_anchor_click_on_a_hidden_anchor_shows_its_subpath_and_deletes_nothing() {
    let mut h = harness(vec![path_b()]);
    h.update(|st| (st.tool, st.selection) = (BezierTool::DeleteAnchor, PathSelection::new(Some(2), Vec::new(), None).shown(Some(vec![0]))));
    let hidden = h.screen(p(200.0, 220.0));
    h.press_at(hidden);
    h.release_at(hidden);
    {
        let st = h.state.borrow();
        assert!(st.deletes.is_empty() && st.changes.is_empty() && st.seen.is_empty(), "{:?} {:?} {:?}", st.deletes, st.changes, st.seen);
        assert_eq!(st.selection, sel(2, &[(1, 0)]).shown(Some(vec![1])));
    }
    h.press_at(hidden);
    h.release_at(hidden);
    assert_eq!(h.state.borrow().deletes, [AnchorDelete::new(2, vec![KnotRef::new(1, 0)], false)]);
}

// The keyboard reaches only the anchors shown: with the square hidden, Home picks the line's first anchor, and the
// selection keeps showing only the line.
#[trial]
fn hidden_anchors_take_no_keyboard_stop() {
    let mut h = harness(vec![path_b()]);
    h.update(|st| st.selection = PathSelection::new(Some(2), Vec::new(), None).shown(Some(vec![1])));
    h.fixture
        .key(egui::Key::Tab, egui::Modifiers::NONE)
        .key(egui::Key::Tab, egui::Modifiers::NONE)
        .key(egui::Key::Home, egui::Modifiers::NONE)
        .settle();
    assert_eq!(h.state.borrow().selection, sel(2, &[(1, 0)]).shown(Some(vec![1])));
}

// After the host deleted an anchor of a Delete Anchor click: the anchor that followed is selected with only its
// subpath shown, the first of a closed subpath after its last, none after an open subpath's last; when the delete
// removed a subpath nothing is selected and every subpath shows.
#[trial]
fn after_a_delete_the_following_anchor_is_selected_and_its_subpath_shown() {
    let deleted = |subpath: usize, knot: usize| {
        let mut path = path_b();
        assert!(path.subpaths[subpath].remove_knot(knot));
        PathSelection::after_delete(&path, KnotRef::new(subpath, knot), 2)
    };
    assert_eq!(deleted(0, 1), sel(2, &[(0, 1)]).shown(Some(vec![0])));
    assert_eq!(deleted(0, 3), sel(2, &[(0, 0)]).shown(Some(vec![0])), "a closed subpath wraps to its first anchor");
    assert_eq!(deleted(1, 1), sel(2, &[]).shown(Some(vec![1])), "nothing follows an open subpath's last anchor");
    let mut path = path_b();
    path.subpaths.remove(1);
    assert_eq!(PathSelection::after_delete(&path, KnotRef::new(1, 0), 2), sel(2, &[]));
}

// Direct Selection's ⌥ drag on a segment drags a copy of its subpath (#13756): the copy goes last, moved by the drag,
// the source stays, and the copy ends selected, its anchors shown beside those already shown. ⇧ during the drag takes
// the anchor ⇧ rule. A ⌥ click selects the subpath and edits nothing; on a path that takes no copies the drag moves
// the subpath.
#[trial]
fn an_alt_drag_on_a_segment_drags_a_copy_of_its_subpath() {
    let on_square = |h: &Harness| h.screen(p(100.0, 200.0));
    let square = || path_b().subpaths[0].clone();

    let mut h = harness(vec![path_b()]);
    h.update(|st| st.selection = PathSelection::new(Some(2), Vec::new(), None).shown(Some(vec![1])));
    let on = on_square(&h);
    h.fixture.hold(egui::Modifiers::ALT);
    h.drag(on, vec2(0.0, 80.0));
    h.fixture.hold(egui::Modifiers::NONE);
    {
        let st = h.state.borrow();
        assert!(matches!(st.changes.last(), Some(PathEdit::DuplicateSubpath { path: 2, subpath: 0, .. })), "{:?}", st.changes);
        let subpaths = &st.paths[0].subpaths;
        assert_eq!(subpaths.len(), 3);
        assert_eq!(subpaths[..2], path_b().subpaths[..], "the source stays");
        let moved =
            subpaths[2].knots.iter().zip(&square().knots).all(|(copy, source)| copy.anchor.distance(source.anchor + Vec2::new(0.0, 80.0)) < 1e-3);
        assert!(moved && subpaths[2].closed, "{:?}", subpaths[2]);
        assert_eq!(st.selection, sel(2, &[(2, 0), (2, 1), (2, 2), (2, 3)]).shown(Some(vec![1, 0, 2])));
        assert_eq!((st.seen.first(), st.seen.last()), (Some(&Seen::Begin), Some(&Seen::Commit)), "{:?}", st.seen);
    }

    // ⇧ once the drag is under way: a 5:12 move stays on the horizontal.
    let mut h = harness(vec![path_b()]);
    let on = on_square(&h);
    h.fixture.hold(egui::Modifiers::ALT);
    h.press_at(on);
    h.move_to(on + vec2(30.0, -12.5));
    h.fixture.hold(egui::Modifiers::ALT | egui::Modifiers::SHIFT);
    h.move_to(on + vec2(60.0, -25.0));
    h.release_at(on + vec2(60.0, -25.0));
    h.fixture.hold(egui::Modifiers::NONE);
    let first = h.state.borrow().paths[0].subpaths[2].knots[0].anchor;
    assert!(first.distance(p(120.0, 200.0)) < 1e-3, "{first:?}");

    // A ⌥ click selects the subpath.
    let mut h = harness(vec![path_b()]);
    let on = on_square(&h);
    h.fixture.hold(egui::Modifiers::ALT);
    h.press_at(on);
    h.release_at(on);
    h.fixture.hold(egui::Modifiers::NONE);
    {
        let st = h.state.borrow();
        assert_eq!(st.selection, sel(2, &[(0, 0), (0, 1), (0, 2), (0, 3)]));
        assert!(st.changes.is_empty() && st.seen.is_empty(), "{:?} {:?}", st.changes, st.seen);
        assert_eq!(st.paths, [path_b()]);
    }

    // A path that takes no copies: the subpath moves.
    let mut h = harness(vec![path_b().copies(false)]);
    let on = on_square(&h);
    h.fixture.hold(egui::Modifiers::ALT);
    h.drag(on, vec2(0.0, 80.0));
    h.fixture.hold(egui::Modifiers::NONE);
    let st = h.state.borrow();
    assert!(matches!(st.changes.last(), Some(PathEdit::MoveAnchors { path: 2, anchors, .. }) if anchors.len() == 4), "{:?}", st.changes);
    assert_eq!(st.paths[0].subpaths.len(), 2);
}

// Add Anchor's cursor says what a click does (#13292): the pen with "+" over a segment a click splits, the arrow over
// an anchor; under ⌘ the cursor is the host tool's; the canvas's precise cursor makes it the crosshair.
#[trial]
fn add_anchor_shows_the_pen_with_plus_over_a_segment_and_the_arrow_over_an_anchor() {
    use craft_behavior::{Cursor, CursorBadge, PaintedCursor};
    let cursor_at = |h: &mut Harness, at: Point, mods: egui::Modifiers| {
        let at = h.screen(at);
        h.fixture.hold(mods);
        h.fixture.frame(vec![pointer::moved(at)]);
        h.fixture.frame(Vec::new());
        h.fixture.hold(egui::Modifiers::NONE);
        craft_ui::runtime::cursor::current(h.fixture.ctx())
    };
    let pen_plus = |cursor: &Option<Cursor>| matches!(cursor, Some(Cursor::Painted(PaintedCursor::Tool { badge: Some(CursorBadge::Add), .. })));
    let (segment, anchor) = (p(110.0, 60.0), p(160.0, 60.0));

    let mut h = two_paths();
    h.update(|st| st.tool = BezierTool::AddAnchor);
    let over_segment = cursor_at(&mut h, segment, egui::Modifiers::NONE);
    assert!(pen_plus(&over_segment), "{over_segment:?}");
    assert_eq!(cursor_at(&mut h, anchor, egui::Modifiers::NONE), Some(Cursor::Painted(PaintedCursor::arrow(false))));
    let with_cmd = cursor_at(&mut h, segment, egui::Modifiers::COMMAND);
    assert!(!pen_plus(&with_cmd), "{with_cmd:?}");

    h.update(|st| st.precise = true);
    for at in [segment, anchor] {
        assert_eq!(cursor_at(&mut h, at, egui::Modifiers::NONE), Some(Cursor::Painted(PaintedCursor::Crosshair)), "{at:?}");
    }
}

// Add Anchor with ⌥ takes the whole subpath, as the reference application does (PhotoCraft #12709), instead of
// turning into Delete Anchor: a click on a segment selects every anchor of its subpath and adds nothing; a drag moves
// a copy of the subpath, which ends up selected, the source staying; a press on an anchor deletes and selects nothing;
// there is no hover mark and the cursor is the arrow with "+" over the parts. Delete Anchor with ⌥ still adds.
#[trial]
fn alt_under_add_anchor_selects_or_copies_the_whole_subpath() {
    use craft_behavior::{Cursor, CursorBadge, PaintedCursor};
    let add_anchor = || {
        let mut h = two_paths();
        h.update(|st| st.tool = BezierTool::AddAnchor);
        h.fixture.hold(egui::Modifiers::ALT);
        h
    };
    let click = |h: &mut Harness, at: Pos2| {
        h.press_at(at);
        h.release_at(at);
    };
    let segment = p(110.0, 60.0);

    let mut h = add_anchor();
    let at = h.screen(segment);
    h.move_to(at);
    h.fixture.frame(Vec::new());
    let arrow_plus = Some(Cursor::Painted(PaintedCursor::badged(craft_behavior::CursorIcon::Default, CursorBadge::Add)));
    assert_eq!(craft_ui::runtime::cursor::current(h.fixture.ctx()), arrow_plus);
    assert_eq!(h.state.borrow().hover.map(|hit| hit.path()), Some(1));
    click(&mut h, at);
    {
        let st = h.state.borrow();
        assert_eq!(st.selection, sel(1, &[(0, 0), (0, 1), (0, 2), (0, 3)]));
        assert!(st.changes.is_empty() && st.seen.is_empty() && st.deletes.is_empty(), "{:?} {:?} {:?}", st.changes, st.seen, st.deletes);
        assert_eq!(st.paths, [path_a(), path_b()]);
    }

    let mut h = add_anchor();
    let at = h.screen(segment);
    h.drag(at, vec2(0.0, 50.0));
    {
        let st = h.state.borrow();
        assert!(matches!(st.changes.last(), Some(PathEdit::DuplicateSubpath { path: 1, subpath: 0, .. })), "{:?}", st.changes);
        assert_eq!((st.seen.first(), st.seen.last()), (Some(&Seen::Begin), Some(&Seen::Commit)), "{:?}", st.seen);
        let subs = &st.paths[0].subpaths;
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0], path_a().subpaths[0], "the source stays");
        assert!(subs[1].knots.iter().zip(&subs[0].knots).all(|(copy, source)| near(copy.anchor, source.anchor + Vec2::new(0.0, 50.0))));
        assert_eq!(st.selection, sel(1, &[(1, 0), (1, 1), (1, 2), (1, 3)]), "the copy is selected");
    }

    let mut h = add_anchor();
    let at = h.screen(p(160.0, 60.0));
    click(&mut h, at);
    {
        let st = h.state.borrow();
        assert!(st.deletes.is_empty() && st.changes.is_empty(), "{:?} {:?}", st.deletes, st.changes);
        assert_eq!(st.selection, PathSelection::default());
    }

    let mut h = two_paths();
    h.update(|st| st.tool = BezierTool::DeleteAnchor);
    h.fixture.hold(egui::Modifiers::ALT);
    let at = h.screen(segment);
    click(&mut h, at);
    assert!(matches!(h.last_change(), Some(PathEdit::InsertAnchor { path: 1, .. })), "{:?}", h.last_change());
}

// Add Anchor selects and edits the anchors and handles a path has, as Direct Selection does and as the reference
// application measures (PhotoCraft #12708): a click on an anchor selects it and ⇧ adds one, with no edit; a drag moves
// the anchor in one gesture; dragging a smooth anchor's handle turns the other one, ⌥ moves it alone; a click on empty
// canvas deselects the anchors and keeps the path; a click on a segment still adds an anchor; ⌥ on an anchor deletes
// nothing (PhotoCraft #12709).
#[trial]
fn add_anchor_selects_and_edits_existing_anchors_and_handles() {
    let add_anchor = || {
        let mut h = two_paths();
        h.update(|st| st.tool = BezierTool::AddAnchor);
        h
    };
    let click = |h: &mut Harness, at: Pos2| {
        h.press_at(at);
        h.release_at(at);
    };
    let (corner, smooth) = (p(160.0, 60.0), p(260.0, 60.0));

    let mut h = add_anchor();
    let at = h.screen(corner);
    click(&mut h, at);
    assert_eq!(h.state.borrow().selection, sel(1, &[(0, 1)]));
    let at = h.screen(smooth);
    h.fixture.hold(egui::Modifiers::SHIFT);
    click(&mut h, at);
    h.fixture.hold(egui::Modifiers::NONE);
    {
        let st = h.state.borrow();
        assert_eq!(st.selection, sel(1, &[(0, 1), (0, 2)]));
        assert!(st.changes.is_empty() && st.seen.is_empty(), "a click edits nothing: {:?} {:?}", st.changes, st.seen);
        assert_eq!(st.paths, [path_a(), path_b()]);
    }

    // A drag on an anchor moves it with its handles, in one gesture, and adds nothing.
    let mut h = add_anchor();
    let at = h.screen(corner);
    h.drag(at, vec2(30.0, -20.0));
    {
        let st = h.state.borrow();
        assert!(
            matches!(st.changes.last(), Some(PathEdit::MoveAnchors { path: 1, anchors, by }) if anchors == &[kr(1)] && near(Point::ZERO + *by, p(30.0, -20.0))),
            "{:?}",
            st.changes
        );
        assert_eq!((st.seen.first(), st.seen.last()), (Some(&Seen::Begin), Some(&Seen::Commit)), "{:?}", st.seen);
        assert_eq!(st.paths[0].subpaths[0].knots.len(), 4, "no anchor added");
        assert!(near(st.paths[0].subpaths[0].knots[1].anchor, p(190.0, 40.0)));
    }

    // The smooth anchor's out handle, shown once the anchor is selected: a drag turns the in handle opposite, with its
    // length; ⌥ moves the dragged handle alone.
    for alt in [false, true] {
        let mut h = add_anchor();
        let at = h.screen(smooth);
        click(&mut h, at);
        let handle = h.screen(p(290.0, 100.0));
        h.fixture.hold(if alt { egui::Modifiers::ALT } else { egui::Modifiers::NONE });
        h.drag(handle, vec2(0.0, 40.0));
        h.fixture.hold(egui::Modifiers::NONE);
        let st = h.state.borrow();
        assert!(
            matches!(st.changes.last(), Some(PathEdit::MoveHandle { path: 1, knot, side: HandleSide::Out, alone, .. }) if *knot == kr(2) && *alone == alt),
            "{alt}: {:?}",
            st.changes
        );
        let k = st.paths[0].subpaths[0].knots[2];
        assert!(near(k.out_ctrl, p(290.0, 140.0)), "{alt}: {k:?}");
        if alt {
            assert!(near(k.in_ctrl, p(230.0, 20.0)), "the in handle stays: {k:?}");
        } else {
            let (was, is) = (p(230.0, 20.0).distance(p(260.0, 60.0)), k.in_ctrl.distance(k.anchor));
            assert!((was - is).abs() < 1e-6, "the in handle keeps its length: {was} {is}");
            let (o, i) = (k.out_ctrl - k.anchor, k.in_ctrl - k.anchor);
            assert!((o.x * i.y - o.y * i.x).abs() < 1e-6 && o.dot(i) < 0.0, "the in handle turns opposite: {k:?}");
        }
    }

    // A click on empty canvas deselects the anchors and keeps the path, with no edit.
    let mut h = add_anchor();
    h.update(|st| st.selection = sel(1, &[(0, 1)]));
    let empty = h.screen(p(320.0, 250.0));
    click(&mut h, empty);
    {
        let st = h.state.borrow();
        assert_eq!(st.selection, sel(1, &[]));
        assert!(st.changes.is_empty() && st.seen.is_empty(), "{:?} {:?}", st.changes, st.seen);
    }

    // A click on a segment still adds an anchor, the only one selected.
    let at = h.screen(p(110.0, 60.0));
    click(&mut h, at);
    {
        let st = h.state.borrow();
        assert!(matches!(st.changes.last(), Some(PathEdit::InsertAnchor { path: 1, .. })), "{:?}", st.changes);
        assert_eq!(st.selection, sel(1, &[(0, 1)]));
    }

    // ⌥ on an anchor deletes nothing and edits nothing (PhotoCraft #12709).
    let mut h = add_anchor();
    let at = h.screen(corner);
    h.fixture.hold(egui::Modifiers::ALT);
    click(&mut h, at);
    h.fixture.hold(egui::Modifiers::NONE);
    {
        let st = h.state.borrow();
        assert!(st.deletes.is_empty() && st.changes.is_empty(), "{:?} {:?}", st.deletes, st.changes);
        assert_eq!(st.paths, [path_a(), path_b()]);
    }
}

// Add Anchor reaches a segment within 5 screen points, as the reference application does (PhotoCraft #12732): a click
// 2 to 5 points above or below a straight segment adds an anchor and shows the pen with "+", one 5.5 or 6 points away
// adds nothing; Delete Anchor and Direct Selection keep a line's reach.
#[trial]
fn add_anchor_reaches_a_segment_within_5_points() {
    use craft_behavior::{Cursor, CursorBadge, PaintedCursor};
    let segment = p(110.0, 60.0);
    let at_tool = |tool: BezierTool| {
        let mut h = two_paths();
        h.update(|st| st.tool = tool);
        h
    };
    let click = |h: &mut Harness, at: Pos2| {
        h.press_at(at);
        h.release_at(at);
    };
    let pen_plus = |h: &Harness| {
        matches!(
            craft_ui::runtime::cursor::current(h.fixture.ctx()),
            Some(Cursor::Painted(PaintedCursor::Tool { badge: Some(CursorBadge::Add), .. }))
        )
    };
    for off in [2.0_f32, 3.0, 4.5, 5.0, -2.0, -5.0] {
        let mut h = at_tool(BezierTool::AddAnchor);
        let at = h.screen(segment) + vec2(0.0, off);
        h.move_to(at);
        h.fixture.frame(Vec::new());
        assert!(pen_plus(&h), "{off}: the pen with + shows where a click adds");
        click(&mut h, at);
        let st = h.state.borrow();
        assert!(matches!(st.changes.last(), Some(PathEdit::InsertAnchor { path: 1, segment, .. }) if *segment == kr(0)), "{off}: {:?}", st.changes);
        assert_eq!(st.paths[0].subpaths[0].knots.len(), 5, "{off}");
    }
    for off in [5.5_f32, 6.0, -6.0] {
        let mut h = at_tool(BezierTool::AddAnchor);
        let at = h.screen(segment) + vec2(0.0, off);
        h.move_to(at);
        h.fixture.frame(Vec::new());
        assert!(!pen_plus(&h), "{off}: no + past the reach");
        click(&mut h, at);
        let st = h.state.borrow();
        assert!(st.changes.is_empty() && st.seen.is_empty(), "{off}: {:?} {:?}", st.changes, st.seen);
        assert_eq!(st.paths, [path_a(), path_b()], "{off}");
    }
    // The other tools keep a line's reach: 4.5 points from the segment misses it, 3 points hits it.
    for tool in [BezierTool::DeleteAnchor, BezierTool::Direct] {
        let mut h = at_tool(tool);
        let near = h.screen(segment) + vec2(0.0, 3.0);
        h.drag(near, vec2(0.0, FAR));
        assert!(matches!(h.last_change(), Some(PathEdit::BendSegment { path: 1, .. })), "{tool:?}: {:?}", h.last_change());
        let mut h = at_tool(tool);
        let past = h.screen(segment) + vec2(0.0, 4.5);
        h.drag(past, vec2(0.0, FAR));
        assert!(!matches!(h.last_change(), Some(PathEdit::BendSegment { .. })), "{tool:?}: {:?}", h.last_change());
    }
}

// The anchor tools' canvas tool: a press off the paths begins nothing (it never starts a path), and off the paths the
// cursor says what a click does: the arrow for Add Anchor (with "+" under ⌥, PhotoCraft #12709), the pen
// with "-" for Delete Anchor, the Convert Point icon, the arrow for Direct Selection and under ⌘, the crosshair when
// the canvas's cursor is precise.
#[trial]
fn the_anchor_tools_take_no_press_off_the_paths_and_show_their_own_cursor() {
    use craft_behavior::{Cursor, CursorBadge, PaintedCursor};
    let empty = p(320.0, 250.0);
    let cursor_at = |h: &mut Harness, mods: egui::Modifiers| {
        let at = h.screen(empty);
        h.fixture.hold(mods);
        h.fixture.frame(vec![pointer::moved(at)]);
        h.fixture.frame(Vec::new());
        h.fixture.hold(egui::Modifiers::NONE);
        craft_ui::runtime::cursor::current(h.fixture.ctx())
    };
    let pen = |badge: CursorBadge, cursor: &Option<Cursor>| matches!(cursor, Some(Cursor::Painted(PaintedCursor::Tool { badge: b, .. })) if *b == Some(badge));
    let arrow = Some(Cursor::Painted(PaintedCursor::arrow(false)));

    let mut h = two_paths();
    h.update(|st| {
        st.tool = BezierTool::AddAnchor;
        st.canvas_tool = Some(PathTool::AddAnchor);
    });
    let before = h.state.borrow().paths.clone();
    let at = h.screen(empty);
    h.press_at(at);
    h.fixture.frame(vec![pointer::release(at)]);
    h.fixture.settle();
    assert_eq!(h.state.borrow().tool_gestures, 0, "a press off the paths begins no gesture");
    assert_eq!(h.state.borrow().paths, before);

    assert_eq!(cursor_at(&mut h, egui::Modifiers::NONE), arrow);
    let with_alt = cursor_at(&mut h, egui::Modifiers::ALT);
    assert_eq!(with_alt, Some(Cursor::Painted(PaintedCursor::badged(craft_behavior::CursorIcon::Default, CursorBadge::Add))));
    assert_eq!(cursor_at(&mut h, egui::Modifiers::COMMAND), arrow);

    h.update(|st| {
        st.tool = BezierTool::DeleteAnchor;
        st.canvas_tool = Some(PathTool::DeleteAnchor);
    });
    let delete = cursor_at(&mut h, egui::Modifiers::NONE);
    assert!(pen(CursorBadge::Subtract, &delete), "{delete:?}");

    h.update(|st| {
        st.tool = BezierTool::Convert;
        st.canvas_tool = Some(PathTool::ConvertPoint);
    });
    let convert = cursor_at(&mut h, egui::Modifiers::NONE);
    assert!(matches!(convert, Some(Cursor::Painted(PaintedCursor::Tool { badge: None, .. }))), "{convert:?}");

    h.update(|st| {
        st.tool = BezierTool::AddAnchor;
        st.canvas_tool = Some(PathTool::AddAnchor);
        st.precise = true;
    });
    assert_eq!(cursor_at(&mut h, egui::Modifiers::NONE), Some(Cursor::Painted(PaintedCursor::Crosshair)));
}

// A Direct Selection marquee is a gesture like the other drags: it begins with its drag, a release commits it with the
// anchors inside selected, and Escape cancels it and selects nothing.
#[trial]
fn a_marquee_is_one_gesture_that_escape_cancels() {
    let from = |h: &Harness| h.screen(p(30.0, 30.0));
    let to = |h: &Harness| h.screen(p(180.0, 80.0));

    let mut h = two_paths();
    let (start, end) = (from(&h), to(&h));
    h.drag(start, end - start);
    {
        let st = h.state.borrow();
        assert_eq!(st.selection, sel(1, &[(0, 0), (0, 1)]));
        assert_eq!((st.seen.first(), st.seen.last()), (Some(&Seen::Begin), Some(&Seen::Commit)), "{:?}", st.seen);
    }

    let mut h = two_paths();
    h.update(|st| st.selection = sel(2, &[(0, 0)]));
    let (start, end) = (from(&h), to(&h));
    h.press_at(start);
    h.move_to(start + (end - start) * 0.5);
    h.move_to(end);
    h.fixture.key(egui::Key::Escape, egui::Modifiers::NONE).settle();
    h.release_at(end);
    let st = h.state.borrow();
    assert!(st.seen.contains(&Seen::Cancel) && !st.seen.contains(&Seen::Commit), "{:?}", st.seen);
    assert_eq!(st.selection, sel(2, &[(0, 0)]), "the cancelled marquee selects nothing");
}

// A click on empty canvas deselects the anchors and keeps the path they belong to as the one being edited.
#[trial]
fn a_click_on_empty_canvas_deselects_the_anchors_and_keeps_the_path() {
    let mut h = two_paths();
    h.update(|st| st.selection = sel(1, &[(0, 1)]));
    let empty = h.screen(p(30.0, 30.0));
    h.press_at(empty);
    h.release_at(empty);
    assert_eq!(h.state.borrow().selection, sel(1, &[]));
}

// Convert Point on an anchor that already is a corner still reports the click's edit: the host decides what it makes
// of it.
#[trial]
fn a_convert_click_on_a_corner_reports_its_edit() {
    let mut h = two_paths();
    h.update(|st| st.tool = BezierTool::Convert);
    let corner = h.screen(p(60.0, 60.0));
    h.press_at(corner);
    h.release_at(corner);
    let st = h.state.borrow();
    assert!(matches!(st.changes.last(), Some(PathEdit::Convert { path: 1, out: None, .. })), "{:?}", st.changes);
    assert!(st.seen.contains(&Seen::Set), "{:?}", st.seen);
}

// The arrow keys nudge the selected anchors, ⇧ by a larger step; each press is one edit, and with the path targeted
// but no anchor selected they move nothing.
#[trial]
fn arrow_keys_nudge_the_selected_anchors_and_shift_nudges_farther() {
    let mut h = two_paths();
    // The canvas, then the overlay. Focus arriving at an anchor selects it, so the selection is emptied after.
    h.fixture.key(egui::Key::Tab, egui::Modifiers::NONE).key(egui::Key::Tab, egui::Modifiers::NONE).settle();
    h.update(|st| st.selection = sel(1, &[]));
    h.fixture.key(egui::Key::ArrowRight, egui::Modifiers::NONE).settle();
    assert!(h.state.borrow().changes.is_empty(), "nothing selected, nothing moves: {:?}", h.state.borrow().changes);
    h.update(|st| st.selection = sel(1, &[(0, 1), (0, 2)]));
    h.fixture.key(egui::Key::ArrowRight, egui::Modifiers::NONE).settle();
    let Some(PathEdit::MoveAnchors { anchors, by: small, .. }) = h.last_change() else { panic!("{:?}", h.last_change()) };
    assert_eq!(anchors, vec![kr(1), kr(2)]);
    assert!(small.x > 0.0 && small.y == 0.0, "{small:?}");
    h.fixture.key(egui::Key::ArrowRight, egui::Modifiers::SHIFT).settle();
    let Some(PathEdit::MoveAnchors { by: far, .. }) = h.last_change() else { panic!("{:?}", h.last_change()) };
    assert!(far.x > small.x, "{small:?} {far:?}");
    assert_eq!(h.state.borrow().changes.len(), 2, "one edit per press");
}

// Escape on a focused anchor leaves the anchor selection and the targeted path under Direct, Convert and Curvature; it
// goes on to the window. A key still reaches the overlay afterwards.
#[trial]
fn escape_on_a_focused_anchor_keeps_the_selection() {
    for tool in [BezierTool::Direct, BezierTool::Convert, BezierTool::Curvature] {
        let mut h = two_paths();
        h.update(|st| (st.tool, st.selection) = (tool, sel(1, &[(0, 1)])));
        h.fixture.key(egui::Key::Tab, egui::Modifiers::NONE).key(egui::Key::Tab, egui::Modifiers::NONE).settle();
        let before = h.state.borrow().selection.clone();
        assert!(before != PathSelection::default(), "{tool:?}: an anchor is selected");
        h.fixture.key(egui::Key::Escape, egui::Modifiers::NONE).settle();
        assert_eq!(h.state.borrow().selection, before, "{tool:?}: Escape keeps the selection");
        h.fixture.key(egui::Key::ArrowRight, egui::Modifiers::NONE).settle();
        assert!(h.last_change().is_some(), "{tool:?}: the overlay still has the keys");
    }
}

// A press belongs to the tool it began under: switching the host's tool in the middle of a drag leaves the drag its
// own, and it commits as that tool's edit.
#[trial]
fn a_press_keeps_the_tool_it_began_under_across_a_tool_switch() {
    let mut h = two_paths();
    let anchor = h.screen(p(160.0, 60.0));
    h.press_at(anchor);
    h.move_to(anchor + vec2(FAR * 0.5, FAR * 0.5));
    h.update(|st| st.tool = BezierTool::Anchors);
    h.move_to(anchor + vec2(FAR, FAR));
    h.release_at(anchor + vec2(FAR, FAR));
    let st = h.state.borrow();
    assert_eq!(st.seen.last(), Some(&Seen::Commit), "{:?}", st.seen);
    assert!(matches!(st.changes.last(), Some(PathEdit::MoveAnchors { path: 1, .. })), "{:?}", st.changes);
}

// The parts follow the press's tool too: a marquee begun under Direct Selection keeps its part when the host switches
// to a tool that declares none, and its release selects the anchors it encloses.
#[trial]
fn a_tool_switch_mid_press_keeps_the_parts_the_press_began_with() {
    let mut h = two_paths();
    let (start, end) = (h.screen(p(30.0, 30.0)), h.screen(p(180.0, 80.0)));
    h.press_at(start);
    h.move_to(start + (end - start) * 0.5);
    h.update(|st| st.tool = BezierTool::Anchors);
    h.move_to(end);
    h.release_at(end);
    let st = h.state.borrow();
    assert_eq!(st.seen.last(), Some(&Seen::Commit), "{:?}", st.seen);
    assert_eq!(st.selection, sel(1, &[(0, 0), (0, 1)]));
}

// The parts follow the paths the press began on: a host that binds other paths after the press (its tool follows a
// modifier let go before the drag) leaves the press its part, and the drag commits as an edit of the pressed path.
#[trial]
fn a_press_keeps_the_paths_it_began_on_when_the_host_binds_others() {
    let mut h = two_paths();
    let anchor = h.screen(p(60.0, 200.0));
    h.press_at(anchor);
    h.update(|st| {
        st.paths.remove(0);
    });
    h.move_to(anchor + vec2(FAR * 0.5, FAR * 0.5));
    h.move_to(anchor + vec2(FAR, FAR));
    h.release_at(anchor + vec2(FAR, FAR));
    let st = h.state.borrow();
    assert_eq!(st.seen.first(), Some(&Seen::Begin), "{:?}", st.seen);
    assert_eq!(st.seen.last(), Some(&Seen::Commit), "{:?}", st.seen);
    assert!(matches!(st.changes.last(), Some(PathEdit::MoveAnchors { path: 2, .. })), "{:?}", st.changes);
}

// A press the host cancels (its target went away) ends as Escape ends one: the gesture is cancelled and its release
// reports nothing. The cancel restores nothing (the host's state moved under the press), so the paths stay where the
// drag left them and the drag goes no further.
#[trial]
fn a_cancelled_press_reports_nothing_on_release() {
    let mut h = two_paths();
    let anchor = h.screen(p(160.0, 60.0));
    h.press_at(anchor);
    h.move_to(anchor + vec2(FAR * 0.5, FAR * 0.5));
    let (before, changes) = (h.state.borrow().paths.clone(), h.state.borrow().changes.len());
    assert!(changes > 0, "the drag moved the anchor before the cancel");
    let overlay = h.state.borrow().id;
    cancel_press_of(h.fixture.ctx(), overlay);
    h.move_to(anchor + vec2(FAR, FAR));
    h.release_at(anchor + vec2(FAR, FAR));
    let st = h.state.borrow();
    assert!(st.seen.contains(&Seen::Cancel), "{:?}", st.seen);
    assert!(!st.seen.contains(&Seen::Commit) && !st.seen.contains(&Seen::Set), "{:?}", st.seen);
    assert_eq!(st.changes.len(), changes, "{:?}", st.changes);
    assert_eq!(st.paths, before);
}

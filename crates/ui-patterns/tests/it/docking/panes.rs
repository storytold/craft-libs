//! `PaneArea`: a pane docks only at the edges its spec allows, and Escape during a pane's title drag cancels the drag.

use craft_behavior::Intent;
use craft_ui::contract::{BodyCx, PaneEdge, Role};
use craft_ui::docking::GroupDrag;
use craft_ui::patterns::dock::{PaneArea, PaneCommands, PaneContent, PaneEdges, PanePlace, PaneRequest, PaneSpec, PaneState};
use craft_ui::runtime::commands::{FireSource, fire};
use craft_ui::runtime::{drag, keys};
use craft_ui::testing::pointer::{moved, press, release};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Pos2, Rect, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const ROOM: Vec2 = Vec2::new(900.0, 600.0);
/// The floating frame a pane starts in, clear of every edge band.
const FLOAT: Rect = Rect { min: Pos2::new(300.0, 200.0), max: Pos2::new(540.0, 360.0) };
/// A point on a floating frame's title row, clear of the action parts at its end.
const TITLE_AT: Vec2 = Vec2::new(24.0, 10.0);

/// The app's one pane: it docks at `edges` and the app keeps its state by applying the area's requests.
struct Panes {
    edges: PaneEdges,
    state: Rc<Cell<PaneState>>,
}

impl PaneContent<u32> for Panes {
    fn spec(&self, pane: &u32) -> PaneSpec {
        let mut spec = PaneSpec::new(format!("Pane {pane}"), PaneState::floating(FLOAT), FLOAT.size());
        spec.edges = self.edges;
        spec
    }

    fn state(&self, _: &u32) -> Option<PaneState> {
        Some(self.state.get())
    }

    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

/// A pane area of one pane starting at `start`, with what the app saw.
struct Bed {
    fixture: Fixture,
    state: Rc<Cell<PaneState>>,
    requests: Rc<RefCell<Vec<PaneRequest<u32>>>>,
    /// The window Cancels no component took, read last in the UI.
    window_cancels: Rc<Cell<usize>>,
}

fn bed(edges: PaneEdges, start: PaneState) -> Bed {
    let state = Rc::new(Cell::new(start));
    let requests = Rc::new(RefCell::new(Vec::new()));
    let window_cancels = Rc::new(Cell::new(0));
    let (kept, seen, cancels) = (Rc::clone(&state), Rc::clone(&requests), Rc::clone(&window_cancels));
    let mut fixture = Fixture::new(move |ui| {
        let mut content = Panes { edges, state: Rc::clone(&kept) };
        let out = PaneArea::new("Panes", &[0_u32], &mut content).show(ui, |_, _| {});
        for request in out.requests {
            let mut next = kept.get();
            next.apply(&request);
            kept.set(next);
            seen.borrow_mut().push(request);
        }
        if keys::with_window_intents(ui.ctx(), false, |window| window.take(Intent::Cancel)) {
            cancels.set(cancels.get() + 1);
        }
    });
    fixture.room(ROOM).settle();
    Bed { fixture, state, requests, window_cancels }
}

impl Bed {
    /// The requests since the last call.
    fn take(&self) -> Vec<PaneRequest<u32>> {
        std::mem::take(&mut *self.requests.borrow_mut())
    }

    /// The places the requests since the last call asked for.
    fn places(&self) -> Vec<PanePlace> {
        self.take()
            .into_iter()
            .filter_map(|request| match request {
                PaneRequest::Place(_, place) => Some(place),
                _ => None,
            })
            .collect()
    }

    /// The point on the floating pane's title row.
    fn title(&self) -> Pos2 {
        self.state.get().float_rect.unwrap_or(FLOAT).min + TITLE_AT
    }

    /// Presses the floating pane's title and moves it to `to` in steps, leaving the button down.
    fn hold_title_at(&mut self, to: Pos2) {
        let from = self.title();
        self.fixture.frame(vec![moved(from)]);
        self.fixture.frame(vec![press(from)]);
        for step in 1..=4 {
            self.fixture.frame(vec![moved(from + (to - from) * (step as f32 / 4.0))]);
        }
        self.fixture.idle(1);
    }

    /// Whether the pane menu shows a row labelled `label`.
    fn menu_has(&self, label: &str) -> bool {
        self.fixture.by_label(label).is_some()
    }
}

/// `PaneEdges::SIDES`: the pane menu lists no Move to Top or Bottom, a run of those commands does nothing, and a drop
/// in the top edge band is refused with no drop mark; Start and End still dock.
#[trial]
fn a_pane_offers_and_takes_only_its_dock_edges() {
    let mut start = PaneState::docked(PaneEdge::Start, 200.0);
    start.float_rect = Some(FLOAT);
    let mut bed = bed(PaneEdges::SIDES, start);
    let menu = bed.fixture.by_label("More actions").expect("the docked pane's menu button");
    bed.fixture.click(&menu).idle(2);
    assert!(bed.menu_has("Move to Start") && bed.menu_has("Move to End") && bed.menu_has("Float"), "the side moves and Float are listed");
    // Start, End, Float and Close: a row for Top or Bottom, under any label, makes the count larger.
    assert_eq!(bed.fixture.by_role(Role::MENU_ITEM).len(), 4, "no move to an edge the pane may not dock at");
    bed.fixture.key(Key::Escape, Modifiers::NONE).idle(2);
    bed.take();

    for refused in [PaneCommands::MoveToTop, PaneCommands::MoveToBottom] {
        assert!(!fire(bed.fixture.ctx(), refused, FireSource::Key), "{refused:?} is disabled for the pane");
        bed.fixture.idle(2);
        assert_eq!(bed.places(), Vec::new(), "{refused:?} moves nothing");
    }
    assert!(fire(bed.fixture.ctx(), PaneCommands::MoveToEnd, FireSource::Key), "Move to End runs");
    bed.fixture.idle(2);
    assert_eq!(bed.places(), vec![PanePlace::Docked(PaneEdge::End)], "the pane docks at the end edge");

    assert!(fire(bed.fixture.ctx(), PaneCommands::Float, FireSource::Key), "Float runs");
    bed.fixture.idle(2);
    assert_eq!(bed.places(), vec![PanePlace::Floating]);
    let area = bed.fixture.shown();
    let top = Pos2::new(area.center().x, area.top() + 2.0);
    bed.hold_title_at(top);
    assert!(drag::active::<GroupDrag<u32>>(bed.fixture.ctx()).is_some(), "the title move started a window drag");
    assert!(drag::mark(bed.fixture.ctx()).is_none(), "the top edge band takes no drop and draws no mark");
    bed.fixture.frame(vec![release(top)]).idle(2);
    assert_eq!(bed.places(), Vec::new(), "the release over the top edge docks nothing");
    assert_eq!(bed.state.get().place, PanePlace::Floating);

    let side = Pos2::new(area.left() + 2.0, area.center().y);
    bed.hold_title_at(side);
    assert!(drag::mark(bed.fixture.ctx()).is_some(), "the start edge band takes the drop and marks it");
    bed.fixture.frame(vec![release(side)]).idle(2);
    assert_eq!(bed.places(), vec![PanePlace::Docked(PaneEdge::Start)], "the release over the start edge docks the pane there");
}

/// Escape during a floating pane's title drag cancels the drag: no drop runs, the pane is back at the rect it had, and
/// neither the window nor the app sees the Escape.
#[trial]
fn escape_during_a_pane_title_drag_cancels_it_and_reaches_nothing_else() {
    let mut bed = bed(PaneEdges::ALL, PaneState::floating(FLOAT));
    bed.take();
    let area = bed.fixture.shown();
    let start = Pos2::new(area.left() + 2.0, area.center().y);
    bed.hold_title_at(start);
    assert!(drag::active::<GroupDrag<u32>>(bed.fixture.ctx()).is_some(), "the title move started a window drag");
    assert!(drag::mark(bed.fixture.ctx()).is_some(), "over the start edge band a release would dock the pane");
    bed.window_cancels.set(0);
    bed.fixture.key(Key::Escape, Modifiers::NONE);
    assert!(drag::active::<GroupDrag<u32>>(bed.fixture.ctx()).is_none(), "Escape cancelled the drag");
    bed.fixture.frame(vec![release(start)]).idle(2);
    assert_eq!(bed.places(), Vec::new(), "the release after Escape docks nothing");
    assert_eq!(bed.state.get().place, PanePlace::Floating, "the pane stays floating");
    assert_eq!(bed.state.get().float_rect, Some(FLOAT), "the pane is back where the drag began");
    assert_eq!(bed.window_cancels.get(), 0, "the Escape went to the drag alone");
}

/// The float rects a pane's requests since the last call reported.
fn float_rects(requests: &[PaneRequest<u32>]) -> Vec<Rect> {
    requests
        .iter()
        .filter_map(|request| match request {
            PaneRequest::FloatRect(_, rect) => Some(*rect),
            _ => None,
        })
        .collect()
}

/// Float reports the rect the pane floats in with its place, so an app that keeps the float rect (as WordCraft keeps
/// the Styles pane's) has it from the Float on, not only after a move or resize: a pane with no kept rect reports
/// where the area draws it, one with a kept rect reports that rect.
#[trial]
fn float_reports_the_rect_the_pane_floats_in() {
    let mut bed = bed(PaneEdges::ALL, PaneState::docked(PaneEdge::Start, 200.0));
    bed.take();
    assert!(fire(bed.fixture.ctx(), PaneCommands::Float, FireSource::Key), "Float runs");
    bed.fixture.idle(2);
    let requests = bed.take();
    assert!(requests.contains(&PaneRequest::Place(0, PanePlace::Floating)), "{requests:?}");
    let reported = float_rects(&requests);
    assert_eq!(reported.len(), 1, "the Float reports one rect: {requests:?}");
    assert_eq!(reported[0].size(), FLOAT.size(), "at the pane's float size");
    assert_eq!(bed.state.get().float_rect, Some(reported[0]), "the app kept it by applying the requests");
    let frame = bed.fixture.by_label("Pane 0").expect("the floating pane").rect;
    assert!((frame.min - reported[0].min).length() < 1.0, "the pane floats where the rect says: {frame:?} against {:?}", reported[0]);

    // A docked pane that keeps a float rect reports that rect.
    let mut start = PaneState::docked(PaneEdge::Start, 200.0);
    start.float_rect = Some(FLOAT);
    let mut kept = self::bed(PaneEdges::ALL, start);
    kept.take();
    assert!(fire(kept.fixture.ctx(), PaneCommands::Float, FireSource::Key), "Float runs");
    kept.fixture.idle(2);
    assert_eq!(float_rects(&kept.take()), vec![FLOAT], "the kept rect is reported");
    let frame = kept.fixture.by_label("Pane 0").expect("the floating pane").rect;
    assert!((frame.min - FLOAT.min).length() < 1.0, "the pane floats in its kept rect: {frame:?}");
}

//! Caller-owned placement: a layer whose overlay is `placed` shows at the caller's rect and reports the user's moves
//! and resizes as `PlacementChange`s, within the overlay's bounds and size limits; a viewport mount hands the move
//! and resize to the OS window.

use craft_behavior::GesturePhase;
use craft_ui::contract::overlay::{PlacementChange, PlacementKind};
use craft_ui::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, DragBand, HeaderRegion, Mount, Overlay, Props as _, Role, Show as _,
    ShowContainer as _, Sizing,
};
use craft_ui::runtime::commands::{CommandSet, WindowCommand};
use craft_ui::style::StyleCx;
use craft_ui::testing::layout::allocate;
use craft_ui::testing::{Frames, pointer, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use egui::{Pos2, Rect, Response, Ui, Vec2};
use std::cell::{Cell, RefCell};

const SLACK: f32 = 1.5;
const MOVE: Vec2 = Vec2::new(40.0, 30.0);
const BODY: Vec2 = Vec2::new(120.0, 60.0);
const HEADER: Vec2 = Vec2::new(100.0, 24.0);
/// Where the caller places the layer, and the screen rect it keeps the layer in.
const START: Rect = Rect { min: Pos2::new(60.0, 70.0), max: Pos2::new(280.0, 210.0) };

/// A container that reports the placement changes it is given.
#[derive(craft_ui::contract::Component)]
#[component(container)]
struct Pane<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    log: &'a RefCell<Vec<PlacementChange>>,
}

impl Component for Pane<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PaneStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

impl Container for Pane<'_> {
    fn header(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        allocate(ui, HEADER);
    }

    fn placement_changed(&mut self, change: &PlacementChange) {
        self.log.borrow_mut().push(*change);
    }
}

/// The caller of a `Pane`: it places the layer at the rect of the last change it was told of.
struct Caller {
    placed: Cell<Rect>,
    controlled: Cell<bool>,
    rect: Cell<Rect>,
    history: RefCell<Vec<PlacementChange>>,
    probe: RefCell<Seen>,
}

impl Caller {
    fn new() -> Self {
        Caller {
            placed: Cell::new(START),
            controlled: Cell::new(true),
            rect: Cell::new(Rect::NOTHING),
            history: RefCell::new(Vec::new()),
            probe: RefCell::new(Seen::default()),
        }
    }

    fn show(&self, overlay: &Overlay) -> impl FnMut(&mut Ui) + '_ {
        let base = overlay.clone();
        move |ui| {
            let overlay = if self.controlled.get() { base.clone().placed(self.placed.get()) } else { base.clone() };
            let props = ContainerProps::new()
                .mount(Mount::Layer(overlay))
                .sizing(Sizing::Content)
                .header(HeaderRegion::new().band(DragBand::Move))
                .command_scope(<WindowCommand as CommandSet>::SCOPE);
            let log = RefCell::new(Vec::new());
            let out = Pane { common: Common::new(), props, log: &log }.id_salt("pane").show(ui, |ui, _| {
                Probe::new(&self.probe).size(BODY).show(ui);
            });
            self.rect.set(out.output.rect);
            for change in log.into_inner() {
                self.placed.set(change.rect);
                self.history.borrow_mut().push(change);
            }
        }
    }

    fn phases(&self) -> Vec<GesturePhase> {
        self.history.borrow().iter().map(|change| change.phase).collect()
    }

    fn last(&self) -> PlacementChange {
        *self.history.borrow().last().expect("a change was reported")
    }
}

fn near(a: Rect, b: Rect) -> bool {
    (a.min - b.min).abs().max_elem() < SLACK && (a.size() - b.size()).abs().max_elem() < SLACK
}

fn header_of(rect: Rect) -> Pos2 {
    Pos2::new(rect.center().x, rect.min.y + 8.0)
}

#[trial]
fn a_placed_layer_reports_its_move_and_keeps_no_placement_of_its_own() {
    let (caller, mut frames) = (Caller::new(), Frames::new());
    let overlay = Overlay::floating().movable(true);
    let mut show = caller.show(&overlay);
    frames.idle(4, &mut show);
    assert!(near(caller.rect.get(), START), "the layer shows at the placed rect: {:?}", caller.rect.get());
    pointer::drag(&mut frames, header_of(START), MOVE, &mut show);
    frames.idle(2, &mut show);
    let phases = caller.phases();
    assert_eq!(phases.first(), Some(&GesturePhase::Begin), "the drag began: {phases:?}");
    assert_eq!(phases.iter().filter(|phase| **phase == GesturePhase::Commit).count(), 1, "one commit: {phases:?}");
    assert_eq!(phases.last(), Some(&GesturePhase::Commit), "the commit ends the gesture: {phases:?}");
    assert!(phases.contains(&GesturePhase::Update), "the drag updated in between: {phases:?}");
    let commit = caller.last();
    assert_eq!(commit.kind, PlacementKind::Move);
    assert!(commit.pointer.is_some(), "a pointer gesture reports the pointer");
    assert!((commit.rect.min - (START.min + MOVE)).abs().max_elem() < SLACK, "the commit carries the moved rect: {:?}", commit.rect);
    assert!(near(caller.rect.get(), commit.rect), "the layer follows the caller's rect: {:?}", caller.rect.get());
    // Without the caller's rect the container has no placement of its own to fall back on.
    caller.controlled.set(false);
    frames.idle(3, &mut show);
    assert!((caller.rect.get().min - commit.rect.min).abs().max_elem() > SLACK, "no placement was stored: {:?}", caller.rect.get());
}

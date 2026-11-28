//! Caller-owned placement: a layer whose overlay is `placed` shows at the caller's rect and reports the user's moves
//! and resizes as `PlacementChange`s, within the overlay's bounds and size limits; a viewport mount hands the move
//! and resize to the OS window.

use craft_behavior::overlay::{Align, Side};
use craft_behavior::{GesturePhase, Sense};
use craft_ui::contract::overlay::{Anchor, FloatPlace, Place, PlacementChange, PlacementKind};
use craft_ui::contract::{
    BodyCx, Common, Component, Container, ContainerProps, Cx, DragBand, HeaderRegion, MemoProps, MemoView, Mount, Overlay, Props as _, Role,
    Show as _, ShowContainer as _, Sizing,
};
use craft_ui::layout::BodyLayout;
use craft_ui::runtime::commands::{self, CommandSet, FireSource, WindowCommand};
use craft_ui::runtime::env::{Direction, with_env};
use craft_ui::style::StyleCx;
use craft_ui::testing::input::key_press;
use craft_ui::testing::layout::allocate;
use craft_ui::testing::{Frames, MemoRun, audit_replays, last_frame_memo_runs, pointer, trial};
use craft_ui_test_kit::contract::{PlainStyle, Probe, Seen};
use egui::{Align2, Key, Modifiers, Pos2, Rect, Response, Ui, Vec2};
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

/// Where the layers of the Size trial hang from: with room above it and before it.
const ANCHOR: Rect = Rect { min: Pos2::new(400.0, 300.0), max: Pos2::new(430.0, 320.0) };

/// Keyboard Size mode on a resizable layer hung from its anchor: the arrows move the edges its placement leaves free
/// and never the edge that faces the anchor or the one aligned with it; Escape puts the layer back.
#[trial]
fn keyboard_size_mode_moves_the_free_edges() {
    // Above its anchor the bottom faces it and the start (left) edge is aligned with it; beside it at the end under
    // right to left (to its left) the right edge faces it and the top is aligned with it.
    for (rtl, side, grow) in [(false, Side::Above, [Key::ArrowRight, Key::ArrowUp]), (true, Side::End, [Key::ArrowLeft, Key::ArrowDown])] {
        let (shown, log, seen) = (Cell::new(Rect::NOTHING), RefCell::new(Vec::new()), RefCell::new(Seen::default()));
        let pane = |ui: &mut Ui| {
            let overlay = Overlay::popover().place(Place::anchored(Anchor::Rect(ANCHOR), side, Align::Start)).resizable(true);
            let props =
                ContainerProps::new().mount(Mount::Layer(overlay)).sizing(Sizing::Content).command_scope(<WindowCommand as CommandSet>::SCOPE);
            let out = Pane { common: Common::new(), props, log: &log }.id_salt("sized").show(ui, |ui, _| {
                Probe::new(&seen).size(BODY).sense(Sense::CLICK | Sense::FOCUS).show(ui);
            });
            shown.set(out.output.rect);
        };
        let mut show = |ui: &mut Ui| {
            if rtl {
                with_env(ui, |o| o.direction = Some(Direction::Rtl), pane);
            } else {
                pane(ui);
            }
        };
        let mut frames = Frames::new();
        frames.idle(4, &mut show);
        let start = shown.get();
        assert!(start.is_positive(), "rtl={rtl}: the layer shows: {start:?}");
        if rtl {
            assert!(start.right() <= ANCHOR.left() + SLACK, "the layer lies before its anchor: {start:?}");
        } else {
            assert!(start.bottom() <= ANCHOR.top() + SLACK, "the layer lies above its anchor: {start:?}");
        }
        // The focus goes into the layer, where the mode needs it.
        pointer::click(&mut frames, start.center(), &mut show);
        assert!(commands::fire(&frames.ctx, WindowCommand::Size, FireSource::Menu), "rtl={rtl}: the layer is a window instance");
        frames.idle(2, &mut show);
        for key in grow.iter().flat_map(|key| [*key, *key]) {
            frames.frame(vec![key_press(key, Modifiers::NONE)], &mut show);
            frames.idle(1, &mut show);
        }
        frames.idle(2, &mut show);
        let grown = shown.get();
        // The edges the placement holds: the one facing the anchor and the one aligned with it.
        let held = |rect: Rect| if rtl { Vec2::new(rect.right(), rect.top()) } else { Vec2::new(rect.left(), rect.bottom()) };
        assert!((held(grown) - held(start)).abs().max_elem() < SLACK, "rtl={rtl}: the held edges stay: {start:?} -> {grown:?}");
        assert!(grown.width() > start.width() + SLACK, "rtl={rtl}: the free side edge moved out: {start:?} -> {grown:?}");
        assert!(grown.height() > start.height() + SLACK, "rtl={rtl}: the free end edge moved out: {start:?} -> {grown:?}");
        let changes = log.borrow().clone();
        assert!(changes.iter().all(|change| change.kind == PlacementKind::Keyboard), "rtl={rtl}: keyboard steps only: {changes:?}");
        assert!(changes.iter().any(|change| change.phase == GesturePhase::Update), "rtl={rtl}: each arrow is a step: {changes:?}");
        frames.frame(vec![key_press(Key::Escape, Modifiers::NONE)], &mut show);
        frames.idle(3, &mut show);
        assert!(near(shown.get(), start), "rtl={rtl}: Escape puts the layer back: {start:?} -> {:?}", shown.get());
        assert_eq!(log.borrow().last().map(|change| change.phase), Some(GesturePhase::Cancel), "rtl={rtl}: Escape cancels the mode");
    }
}

/// The rect the layers of the bounds trial are kept inside, and an anchor at its bottom end corner.
const BOUNDS: Rect = Rect { min: Pos2::new(100.0, 100.0), max: Pos2::new(400.0, 360.0) };
const EDGE: Rect = Rect { min: Pos2::new(370.0, 320.0), max: Pos2::new(400.0, 340.0) };

/// A layer with bounds shows inside them however it is placed: hung from an anchor at their edge, it slides inside
/// them as it slides inside the screen; placed at a position outside them, it is kept inside.
#[trial]
fn a_layer_with_bounds_shows_inside_them_hung_or_placed() {
    let hung = Overlay::popover().place(Place::anchored(Anchor::Rect(EDGE), Side::Below, Align::Start));
    let placed_at = Overlay::floating().placed_at(Pos2::new(BOUNDS.right() - 20.0, BOUNDS.top() - 50.0));
    for (name, overlay) in [("hung", hung), ("placed_at", placed_at)] {
        let (shown, log, seen) = (Cell::new(Rect::NOTHING), RefCell::new(Vec::new()), RefCell::new(Seen::default()));
        let mut show = |ui: &mut Ui| {
            let props = ContainerProps::new().mount(Mount::Layer(overlay.clone().bounds(BOUNDS))).sizing(Sizing::Content);
            let out = Pane { common: Common::new(), props, log: &log }.id_salt(name).show(ui, |ui, _| {
                Probe::new(&seen).size(BODY).show(ui);
            });
            shown.set(out.output.rect);
        };
        let mut frames = Frames::new();
        frames.idle(4, &mut show);
        let rect = shown.get();
        assert!(rect.is_positive(), "{name}: the layer shows: {rect:?}");
        assert!(BOUNDS.expand(SLACK).contains_rect(rect), "{name}: the layer lies inside {BOUNDS:?}: {rect:?}");
    }
}

/// A layer anchored `Parent` hangs from the placed panel of the layer whose body opened it, not from its own cursor.
#[trial]
fn a_layer_anchored_parent_hangs_on_the_parent_panel() {
    let (parent_rect, child_rect) = (Cell::new(Rect::NOTHING), Cell::new(Rect::NOTHING));
    let (log, seen, kid) = (RefCell::new(Vec::new()), RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let mut show = |ui: &mut Ui| {
        let props = ContainerProps::new().mount(Mount::Layer(Overlay::floating().placed(START))).sizing(Sizing::Content);
        let out = Pane { common: Common::new(), props, log: &log }.id_salt("parent").show(ui, |ui, _| {
            Probe::new(&seen).size(BODY).show(ui);
            let overlay = Overlay::popover().place(Place::anchored(Anchor::Parent, Side::Below, Align::Start));
            let props = ContainerProps::new().mount(Mount::Layer(overlay)).sizing(Sizing::Content);
            let inner = Pane { common: Common::new(), props, log: &log }.id_salt("child").show(ui, |ui, _| {
                Probe::new(&kid).size(Vec2::new(60.0, 20.0)).show(ui);
            });
            child_rect.set(inner.output.rect);
        });
        parent_rect.set(out.output.rect);
    };
    let mut frames = Frames::new();
    frames.idle(6, &mut show);
    let (parent, child) = (parent_rect.get(), child_rect.get());
    assert!(parent.is_positive() && child.is_positive(), "both layers show: {parent:?} {child:?}");
    assert!((child.left() - parent.left()).abs() < SLACK, "the child lines up with the parent panel's start: {parent:?} {child:?}");
    assert!((child.top() - parent.bottom()).abs() < SLACK, "the child hangs below the parent panel: {parent:?} {child:?}");
}

/// Under a right-to-left interface, a row inside a container whose axis keeps its left-to-right order (`ltr_axis`)
/// flows from the left, and a popover that container opens flows from the right: its first member is right of its
/// second.
#[trial]
fn a_layer_opened_inside_an_ltr_axis_flows_in_the_interface_direction() {
    let anchor = Rect::from_min_size(Pos2::new(200.0, 120.0), Vec2::new(40.0, 20.0));
    let log = RefCell::new(Vec::new());
    let seen = [(); 4].map(|()| RefCell::new(Seen::default()));
    let mut show = |ui: &mut Ui| {
        with_env(
            ui,
            |o| o.direction = Some(Direction::Rtl),
            |ui| {
                let axis = ContainerProps::new().layout(BodyLayout::row()).sizing(Sizing::Content);
                Pane { common: Common::new(), props: axis, log: &log }.ltr_axis().id_salt("axis").show(ui, |ui, _| {
                    Probe::new(&seen[0]).id_salt("axis first").show(ui);
                    Probe::new(&seen[1]).id_salt("axis second").show(ui);
                    let overlay = Overlay::popover().place(Place::anchored(Anchor::Rect(anchor), Side::Below, Align::Start));
                    let props = ContainerProps::new().mount(Mount::Layer(overlay)).layout(BodyLayout::row()).sizing(Sizing::Content);
                    Pane { common: Common::new(), props, log: &log }.id_salt("popover").show(ui, |ui, _| {
                        Probe::new(&seen[2]).id_salt("layer first").show(ui);
                        Probe::new(&seen[3]).id_salt("layer second").show(ui);
                    });
                });
            },
        );
    };
    let mut frames = Frames::new();
    frames.idle(4, &mut show);
    let [axis_first, axis_second, first, second] = seen.each_ref().map(|seen| seen.borrow().rect);
    assert!(axis_first.is_positive() && first.is_positive(), "the axis and the popover show: {axis_first:?} {first:?}");
    assert!(axis_first.center().x < axis_second.center().x, "the axis keeps its order: {axis_first:?} {axis_second:?}");
    assert!(first.center().x > second.center().x, "the popover flows from the right: {first:?} {second:?}");
}

/// Under a right-to-left interface, a start-aligned popover lines its right edge up with its anchor's right edge, and
/// one opened from inside a container whose axis keeps its left-to-right order (`ltr_axis`) is placed the same way.
#[trial]
fn a_layer_opened_inside_an_ltr_axis_aligns_to_its_anchor_in_the_interface_direction() {
    let anchor = Rect::from_min_size(Pos2::new(200.0, 120.0), Vec2::new(40.0, 20.0));
    let placed = |inside: bool| {
        let log = RefCell::new(Vec::new());
        let rect = Cell::new(Rect::NOTHING);
        let seen = RefCell::new(Seen::default());
        let popover = |ui: &mut Ui| {
            let overlay = Overlay::popover().place(Place::anchored(Anchor::Rect(anchor), Side::Below, Align::Start));
            let props = ContainerProps::new().mount(Mount::Layer(overlay)).layout(BodyLayout::row()).sizing(Sizing::Content);
            let out = Pane { common: Common::new(), props, log: &log }.id_salt("popover").show(ui, |ui, _| {
                Probe::new(&seen).size(Vec2::new(120.0, 30.0)).show(ui);
            });
            rect.set(out.output.rect);
        };
        let mut show = |ui: &mut Ui| {
            with_env(
                ui,
                |o| o.direction = Some(Direction::Rtl),
                |ui| {
                    if inside {
                        let axis = ContainerProps::new().layout(BodyLayout::row()).sizing(Sizing::Content);
                        Pane { common: Common::new(), props: axis, log: &log }.ltr_axis().id_salt("axis").show(ui, |ui, _| popover(ui));
                    } else {
                        popover(ui);
                    }
                },
            );
        };
        let mut frames = Frames::new();
        frames.idle(4, &mut show);
        rect.get()
    };
    let (outside, inside) = (placed(false), placed(true));
    assert!(outside.is_positive() && inside.is_positive(), "both popovers show: {outside:?} {inside:?}");
    assert!((outside.right() - anchor.right()).abs() < SLACK, "the popover's right edge is the anchor's: {outside:?} {anchor:?}");
    assert!(near(inside, outside), "the popover opened inside the axis is placed as the one outside it: {inside:?} {outside:?}");
}

/// A float placed by its right-top corner has that corner at the position, its width reaching back from it; near the
/// start edge of its bounds, where the corner's place would put the layer outside them, it is kept inside.
#[trial]
fn a_float_places_its_corner_at_the_position() {
    let free = Pos2::new(500.0, 120.0);
    let near_edge = Pos2::new(BOUNDS.left() + 20.0, BOUNDS.top() + 20.0);
    for (name, at, bounds) in [("free", free, None), ("kept", near_edge, Some(BOUNDS))] {
        let (shown, log, seen) = (Cell::new(Rect::NOTHING), RefCell::new(Vec::new()), RefCell::new(Seen::default()));
        let mut show = |ui: &mut Ui| {
            let overlay = Overlay::floating().place(Place::Float(FloatPlace::corner_at(at, Align2::RIGHT_TOP)));
            let overlay = match bounds {
                Some(bounds) => overlay.bounds(bounds),
                None => overlay,
            };
            let props = ContainerProps::new().mount(Mount::Layer(overlay)).sizing(Sizing::Content);
            let out = Pane { common: Common::new(), props, log: &log }.id_salt(name).show(ui, |ui, _| {
                Probe::new(&seen).size(BODY).show(ui);
            });
            shown.set(out.output.rect);
        };
        let mut frames = Frames::new();
        frames.idle(4, &mut show);
        let rect = shown.get();
        assert!(rect.width() >= BODY.x - SLACK, "{name}: the layer shows its body: {rect:?}");
        match bounds {
            None => {
                assert!((rect.right_top() - at).abs().max_elem() < SLACK, "{name}: the right-top corner sits at {at:?}: {rect:?}");
                assert!((rect.left() - (at.x - rect.width())).abs() < SLACK, "{name}: the layer reaches back its width: {rect:?}");
            }
            Some(bounds) => {
                assert!(bounds.expand(SLACK).contains_rect(rect), "{name}: the layer lies inside {bounds:?}: {rect:?}");
                assert!((rect.left() - bounds.left()).abs() < SLACK, "{name}: the layer is kept at the start edge: {rect:?}");
                assert!((rect.top() - at.y).abs() < SLACK, "{name}: the corner's top stays at {at:?}: {rect:?}");
            }
        }
    }
}

/// An anchored layer that stays open takes the side its host asks for now: asked for End after Above, it hangs at the
/// anchor's end; asked for Above again, it sits above. Guards against a layer that kept the side an earlier pass took
/// while it stayed open, whatever side the host asked for since.
#[trial]
fn an_open_layer_takes_the_side_its_host_asks_for_now() {
    let anchor = Rect::from_min_size(Pos2::new(200.0, 300.0), Vec2::new(40.0, 20.0));
    let (shown, asked, log, seen) = (Cell::new(Rect::NOTHING), Cell::new(Side::Above), RefCell::new(Vec::new()), RefCell::new(Seen::default()));
    let mut show = |ui: &mut Ui| {
        let overlay = Overlay::popover().place(Place::anchored(Anchor::Rect(anchor), asked.get(), Align::Start));
        let props = ContainerProps::new().mount(Mount::Layer(overlay)).sizing(Sizing::Content);
        let out = Pane { common: Common::new(), props, log: &log }.id_salt("bar").show(ui, |ui, _| {
            Probe::new(&seen).size(Vec2::new(60.0, 20.0)).show(ui);
        });
        shown.set(out.output.rect);
    };
    let mut frames = Frames::new();
    frames.idle(4, &mut show);
    assert!(shown.get().bottom() <= anchor.top() + SLACK, "asked for Above, the layer sits above: {:?}", shown.get());
    asked.set(Side::End);
    frames.idle(4, &mut show);
    assert!(shown.get().left() >= anchor.right() - SLACK, "asked for End, the layer hangs at the anchor's end: {:?}", shown.get());
    asked.set(Side::Above);
    frames.idle(4, &mut show);
    assert!(shown.get().bottom() <= anchor.top() + SLACK, "asked for Above again, the layer sits above: {:?}", shown.get());
}

/// A memoized view holding a layer whose body is `width` points wide.
#[derive(MemoProps)]
struct HeldLayer {
    width: f32,
}

fn held_layer(held: &mut HeldLayer, ui: &mut Ui) {
    let log = RefCell::new(Vec::new());
    let props = ContainerProps::new().mount(Mount::Layer(Overlay::floating().modeless().placed_at(START.min))).sizing(Sizing::Content);
    let width = held.width;
    Pane { common: Common::new(), props, log: &log }.id_salt("held").show(ui, |ui, _| allocate(ui, Vec2::new(width, BODY.y)));
}

/// A layer that grows inside a memoized view senses its background over the rect it shows from the run that grows it,
/// so that run's record is what the view replays: the replay audit finds the replay the same as a run. egui's `Area`
/// started each pass at the size it kept from the last one, so the run that grew the layer kept its background at the
/// old size (a canvas task bar showing new members).
#[trial]
fn a_memoized_layer_that_grows_replays_as_it_runs() {
    let width = Cell::new(BODY.x);
    let mut frames = Frames::new();
    audit_replays(&frames.ctx, true);
    let mut show = |ui: &mut Ui| {
        MemoView::new("held layer", HeldLayer { width: width.get() }, held_layer).show(ui);
    };
    frames.idle(4, &mut show);
    width.set(2.0 * BODY.x);
    frames.idle(4, &mut show);
    let runs = last_frame_memo_runs(&frames.ctx);
    assert!(runs.iter().any(|(name, _, run)| name.contains("HeldLayer") && *run == MemoRun::Replayed), "the grown layer's view replays: {runs:?}");
}

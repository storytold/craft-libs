//! `FloatingGroup`: a window unit drag sees the drop targets under its own group, and a window shows within its
//! panels' size limits whatever rect it is given.

use craft_behavior::dock::DockEdge;
use craft_ui::contract::SizeLimits;
use craft_ui::contract::{Common, Component, Cx, DropMark, DropOperation, DropTarget, MeasureCx, Pointer as _, Props as _, Role, Show as _, Sizing};
use craft_ui::docking::{FloatingGroup, FloatingGroupOutput, GroupDrag, WindowColumn};
use craft_ui::runtime::drag;
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::layout::allocate;
use craft_ui::testing::snapshot::layer_rects;
use craft_ui::testing::{Frames, pointer, trial};
use egui::{Pos2, Rect, Response, Ui, Vec2};
use std::cell::{Cell, RefCell};

const START: Rect = Rect { min: Pos2::new(60.0, 70.0), max: Pos2::new(300.0, 250.0) };
const BOUNDS: Rect = Rect { min: Pos2::ZERO, max: Pos2::new(400.0, 320.0) };
const LEAST: Vec2 = Vec2::new(150.0, 90.0);
const MOVE: Vec2 = Vec2::new(30.0, 20.0);
/// A point on the title row, clear of the action parts at its end.
const TITLE_AT: Vec2 = Vec2::new(24.0, 10.0);

/// The caller of a group: it keeps the rect the way the dock model does.
struct Caller {
    sizing: Sizing,
    rect: Cell<Rect>,
    shown: Cell<Rect>,
    body: Cell<Vec2>,
    anchor: Cell<Option<&'static str>>,
    history: RefCell<Vec<FloatingGroupOutput>>,
}

impl Caller {
    fn new() -> Self {
        Caller {
            sizing: Sizing::Fill,
            rect: Cell::new(START),
            shown: Cell::new(Rect::NOTHING),
            body: Cell::new(LEAST),
            anchor: Cell::new(None),
            history: RefCell::new(Vec::new()),
        }
    }

    fn show(&self) -> impl FnMut(&mut Ui) + '_ {
        move |ui| {
            let mut group = FloatingGroup::new("Layers", self.rect.get()).sized(self.sizing).bounds(BOUNDS).limits(SizeLimits::at_least(LEAST));
            if let Some(anchor) = self.anchor.get() {
                group = group.window_of(anchor);
            }
            let out = group.show_group(ui, |ui, _| {
                allocate(ui, self.body.get());
            });
            self.shown.set(out.output.response.rect);
            if let Some(change) = out.output.placed {
                self.rect.set(change.rect);
            }
            self.history.borrow_mut().push(out.output);
        }
    }
}

fn title_of(rect: Rect) -> Pos2 {
    rect.min + TITLE_AT
}

#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct PadStyle {
    #[style(metrics)]
    metrics: craft_behavior::Metrics,
}

/// The rect of the pad, under the window group's start rect and the pointer of the window drag.
const PAD: Rect = Rect { min: Pos2::new(40.0, 60.0), max: Pos2::new(200.0, 200.0) };

struct TakesGroups;

impl DropTarget<GroupDrag<&'static str>> for TakesGroups {
    type Drop = ();

    fn accept(&self, _: &GroupDrag<&'static str>, _: Pos2, _: DropOperation) -> Option<()> {
        Some(())
    }

    fn mark(&self, _: &()) -> DropMark {
        DropMark::Into(PAD)
    }
}

/// A target for dragged groups that lies under the floating group.
#[derive(Component)]
struct Pad {
    #[common]
    common: Common,
}

impl Component for Pad {
    const ROLE: Role = Role::GROUP;
    type Style = PadStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: PadStyleKey) -> PadStyle {
        PadStyle::default()
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        PAD.size()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.drop_target(&TakesGroups);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

#[trial]
fn a_window_unit_drag_sees_the_target_under_its_own_group() {
    let (caller, mut frames) = (Caller::new(), Frames::new());
    caller.anchor.set(Some("Layers"));
    let mut group = caller.show();
    let mut show = |ui: &mut Ui| {
        Pad { common: Common::new() }.id_salt("pad").show_at(ui, PAD);
        group(ui);
    };
    frames.idle(4, &mut show);
    let from = title_of(START);
    for events in [vec![pointer::moved(from)], vec![pointer::press(from)], vec![pointer::moved(from + MOVE / 2.0)], vec![pointer::moved(from + MOVE)]]
    {
        frames.frame(events, &mut show);
    }
    frames.idle(2, &mut show);
    assert!(drag::active::<GroupDrag<&'static str>>(&frames.ctx).is_some(), "the move started a window drag");
    assert!(drag::mark(&frames.ctx).is_some(), "the target under the moving group takes the drop");
}

/// The rect the layer of a group that fills `rect` shows at, with `limits` on its body, after it settles.
fn shown_at(rect: Rect, limits: SizeLimits, column: Option<WindowColumn>) -> Rect {
    let mut frames = Frames::new();
    let mut show = |ui: &mut Ui| {
        let mut group = FloatingGroup::new("Layers", rect).sized(Sizing::Fill).bounds(BOUNDS).limits(limits);
        if let Some(column) = column {
            group = group.column(column);
        }
        let _ = group.show_group(ui, |ui, _| {
            allocate(ui, LEAST);
        });
    };
    frames.idle(4, &mut show);
    let layers = layer_rects(&frames.ctx);
    assert_eq!(layers.len(), 1, "the group shows one layer: {layers:?}");
    layers[0]
}

// A window placed or restored from a saved layout at a size outside its panels' limits shows at the nearest size
// within them, at the place it was given; a window already within them keeps its rect.
#[trial]
fn a_window_placed_outside_its_limits_shows_within_them() {
    let least = SizeLimits::at_least(LEAST);
    let narrow = Rect::from_min_size(START.min, Vec2::new(LEAST.x - 50.0, LEAST.y - 40.0));
    let shown = shown_at(narrow, least, None);
    assert!(shown.width() >= LEAST.x - 0.5, "a window placed narrower than its limits widens to them: {shown:?}");
    assert!(shown.height() > LEAST.y, "a window placed shorter than its body's limits grows past them: {shown:?}");
    assert!((shown.min - narrow.min).length() < 0.5, "the window keeps its place: {shown:?}");
    let within = shown_at(START, least, None);
    assert!((within.min - START.min).length() < 0.5 && (within.size() - START.size()).length() < 0.5, "{within:?} keeps {START:?}");
    let most = Vec2::new(START.width() - 60.0, BOUNDS.height());
    let shown = shown_at(START, SizeLimits::new(LEAST, most), None);
    assert!(shown.width() <= most.x + 0.5, "a window placed wider than its limits narrows to them: {shown:?}");
}

// A window placed narrower than its limits is dragged at the size it shows, so a float of the drag does not bring the
// narrower size back.
#[trial]
fn a_window_placed_narrower_than_its_limits_is_dragged_at_the_size_it_shows() {
    let (caller, mut frames) = (Caller::new(), Frames::new());
    let narrow = Rect::from_min_size(START.min, Vec2::new(LEAST.x - 50.0, START.height()));
    caller.rect.set(narrow);
    caller.anchor.set(Some("Layers"));
    let mut show = caller.show();
    frames.idle(4, &mut show);
    let from = title_of(narrow);
    for events in [vec![pointer::moved(from)], vec![pointer::press(from)], vec![pointer::moved(from + MOVE / 2.0)], vec![pointer::moved(from + MOVE)]]
    {
        frames.frame(events, &mut show);
    }
    let drag = drag::active::<GroupDrag<&'static str>>(&frames.ctx);
    let size = drag.as_ref().and_then(|drag| drag.source);
    assert!(size.is_some_and(|size| size.x >= LEAST.x - 0.5), "the window drag carries the shown width: {size:?}");
}

// A window whose column's groups are all minimized shows at the rect's width, within the panels' limits too.
#[trial]
fn a_window_of_minimized_groups_placed_narrower_than_its_limits_shows_at_them() {
    let narrow = Rect::from_min_size(START.min, Vec2::new(LEAST.x - 50.0, START.height()));
    let column = WindowColumn::minimized(DockEdge::Start, 24.0);
    let held = shown_at(narrow, SizeLimits::at_least(LEAST), Some(column));
    assert!(held.width() >= LEAST.x - 0.5, "a column of minimized groups widens to its limits: {held:?}");
}

//! `FloatingGroup`: a window unit drag sees the drop targets under its own group.

use craft_ui::contract::SizeLimits;
use craft_ui::contract::{Common, Component, Cx, DropMark, DropOperation, DropTarget, MeasureCx, Pointer as _, Props as _, Role, Show as _, Sizing};
use craft_ui::docking::{FloatingGroup, FloatingGroupOutput, GroupDrag};
use craft_ui::runtime::drag;
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::testing::layout::allocate;
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

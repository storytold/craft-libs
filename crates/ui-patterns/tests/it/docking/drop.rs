//! Drop resolution: a spot under the pointer becomes the model action that moves the dragged panel or group.

use craft_behavior::dock::{Action, Column, DockEdge, DockPlacement, Floating, Group, Layout, Node, Permissions, SplitSize};
use craft_behavior::{Axis, DockSide};
use craft_ui::docking::{DockDrag, DockSpot, DockTargets, DragUnit, resolve_drop};
use craft_ui::testing::memory::global;
use craft_ui::testing::{Frames, trial};
use egui::{Context, Id, Pos2, Rect, Vec2};

const TOP_GROUP: u32 = 1;
const TOP_SECOND_TAB: u32 = 2;
const LOWER_GROUP: u32 = 3;
const CANVAS: u32 = 9;
const WINDOW: u32 = 7;
const COLUMN_WIDTH: f32 = 200.0;

/// A dock column of two groups, `[1, 2]` above `[3]`, beside a canvas.
fn layout() -> Layout<u32> {
    let column = Column::new(vec![Group::new(vec![TOP_GROUP, TOP_SECOND_TAB], 0), Group::new(vec![LOWER_GROUP], 0)]).with_width(COLUMN_WIDTH);
    let dock = Node::dock(DockEdge::Start, vec![column]);
    let window = Floating::new(Node::tabs(vec![WINDOW], 0), Rect::from_min_size(at(500.0, 0.0), Vec2::new(200.0, 150.0)));
    Layout::new(Some(Node::split(Axis::X, SplitSize::FixedFirst(COLUMN_WIDTH), dock, Node::tabs(vec![CANVAS], 0))), vec![window])
}

fn dock_id() -> Id {
    global("drop registry", "dock")
}

fn at(x: f32, y: f32) -> Pos2 {
    Pos2::new(x, y)
}

/// The spots a dock column registers: the top group's strip (its tabs are 100 wide) and body sides, and the lower
/// group's strip.
fn register(ctx: &Context) {
    let dock = dock_id();
    let rect = |min: Pos2, size: Vec2| Rect::from_min_size(min, size);
    DockTargets::register(ctx, dock, DockSpot::group(TOP_GROUP, DockSide::Center), rect(at(0.0, 24.0), Vec2::new(COLUMN_WIDTH, 100.0)));
    DockTargets::register(ctx, dock, DockSpot::group(TOP_GROUP, DockSide::Bottom), rect(at(0.0, 124.0), Vec2::new(COLUMN_WIDTH, 24.0)));
    DockTargets::register(ctx, dock, DockSpot::group(TOP_GROUP, DockSide::Start), rect(at(0.0, 24.0), Vec2::new(24.0, 100.0)));
    DockTargets::register(ctx, dock, DockSpot::tab(TOP_GROUP, 1), rect(at(100.0, 0.0), Vec2::new(100.0, 24.0)));
    DockTargets::register(ctx, dock, DockSpot::tab(TOP_GROUP, 2), rect(at(0.0, 0.0), Vec2::new(24.0, 24.0)));
    DockTargets::register(ctx, dock, DockSpot::group(LOWER_GROUP, DockSide::Center), rect(at(0.0, 200.0), Vec2::new(COLUMN_WIDTH, 100.0)));
    DockTargets::register(ctx, dock, DockSpot::<u32>::NewDock(DockSide::End), rect(at(380.0, 0.0), Vec2::new(20.0, 300.0)));
}

/// Resolves a drop of `unit` at `pointer` in the pass after the spots were registered.
fn drop_of(unit: DragUnit<u32>, pointer: Pos2, permissions: impl Fn(&u32) -> Permissions) -> Option<Action<u32>> {
    struct Carried(DragUnit<u32>);
    impl DockDrag<u32> for Carried {
        fn unit(&self, _layout: &Layout<u32>) -> Option<DragUnit<u32>> {
            Some(self.0.clone())
        }
    }
    let mut frames = Frames::new();
    frames.frame(Vec::new(), |ui| register(ui.ctx()));
    let mut resolved = None;
    frames.frame(Vec::new(), |ui| {
        register(ui.ctx());
        resolved = resolve_drop(ui.ctx(), dock_id(), &layout(), pointer, &Carried(unit.clone()), &permissions);
    });
    resolved
}

fn open(_: &u32) -> Permissions {
    Permissions::default()
}

#[trial]
fn a_tab_dropped_after_its_own_neighbour_reorders_its_group() {
    let action = drop_of(DragUnit::Panel(TOP_GROUP), at(10.0, 10.0), open);
    assert_eq!(
        action,
        Some(Action::move_beside(TOP_GROUP, TOP_SECOND_TAB, DockPlacement::tab(None))),
        "the destination is a panel that stays, not the dragged one"
    );
}

//! A group the host takes with `group_view` lays out in the group's rect, not in the room the dock area has left; a
//! tool rail the host takes has the dock's grip strip on top.

use craft_behavior::Axis;
use craft_behavior::dock::{Column, DockEdge, Group, Layout, Node, SplitSize, Workspace};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, BodyCx, PlainData, Scroll, Show as _, ShowContainer as _, Tracked};
use craft_ui::display::Text;
use craft_ui::docking::{DockConfig, DockGrid, DockPanels, DockSizes, DragUnit, GroupAt, GroupDrag, PanelSpec, PanelSpecs};
use craft_ui::layout_components::Stack;
use craft_ui::patterns::dock::DockArea;
use craft_ui::rails::{LastUsed, RailTool, ToolRail, ToolSlot};
use craft_ui::runtime::commands::EditCommands;
use craft_ui::runtime::drag;
use craft_ui::testing::{Fixture, last_pass_runs, pointer, trial};
use egui::{Rect, Shape, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

/// Panel 0 is a canvas the host draws in its group's place; it records the group's rect and the `Ui` it was given.
struct Panels {
    seen: Rc<Cell<Option<(Rect, Rect, Rect)>>>,
}

/// Panel 0 is a canvas.
#[derive(PlainData)]
struct Specs;

impl PanelSpecs<usize> for Specs {
    fn spec(&self, panel: &usize) -> PanelSpec {
        let spec = PanelSpec::new(format!("Panel {panel}"));
        if *panel == 0 { spec.canvas() } else { spec }
    }
}

impl DockPanels<usize> for Panels {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn group_view(&mut self, panels: &[usize], _: usize, ui: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if !panels.contains(&0) {
            return None;
        }
        match at {
            GroupAt::Measure(_) => Some(0.0),
            GroupAt::Show(rect) => {
                self.seen.set(Some((rect, ui.max_rect(), ui.clip_rect())));
                Some(rect.height())
            }
        }
    }
}

#[trial]
fn a_group_view_lays_out_and_clips_to_its_group_rect() {
    let seen = Rc::new(Cell::new(None));
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0], 0), Node::tabs(vec![1], 0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let (mut panels, specs) = (Panels { seen: seen.clone() }, Tracked::new(Specs));
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    let (rect, room, clip) = seen.get().expect("the canvas group was shown");
    assert!(rect.width() < 1000.0, "the canvas group takes part of the area: {rect:?}");
    assert!(rect.contains_rect(room), "the view's room {room:?} lies in its group's rect {rect:?}");
    assert!(rect.contains_rect(clip), "the view's clip {clip:?} lies in its group's rect {rect:?}");
}

/// The tool rail of the grip trial: a panel with a rail grid, which the host shows itself.
const RAIL: usize = 7;
/// The height the host's rail measures at.
const RAIL_HEIGHT: f32 = 200.0;

/// The rail is a tool rail; the rest are plain panels.
#[derive(PlainData)]
struct RailSpecs;

impl PanelSpecs<usize> for RailSpecs {
    fn spec(&self, panel: &usize) -> PanelSpec {
        let spec = PanelSpec::new(format!("Panel {panel}"));
        if *panel == RAIL { spec.grid(DockGrid::rail(4)) } else { spec }
    }
}

/// The host shows the rail itself and records the rect it was given.
struct RailPanels {
    seen: Rc<Cell<Option<Rect>>>,
}

impl DockPanels<usize> for RailPanels {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn group_view(&mut self, panels: &[usize], _: usize, _: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if !panels.contains(&RAIL) {
            return None;
        }
        match at {
            GroupAt::Measure(_) => Some(RAIL_HEIGHT),
            GroupAt::Show(rect) => {
                self.seen.set(Some(rect));
                Some(rect.height())
            }
        }
    }
}

/// A tool rail the host shows itself has the dock's grip under its column bar, as the references' toolbar: a strip of
/// the theme's upright ticks in its grip ink, above the host's view, which takes the docked column's room down to its
/// foot as the references' docked toolbar does; a drag on the strip moves the rail's group.
#[trial]
fn a_tool_rail_shown_by_its_host_has_a_grip_strip_that_drags_it() {
    let seen = Rc::new(Cell::new(None));
    let dock = Node::dock(DockEdge::Start, vec![Column::new(vec![Group::new(vec![RAIL], 0)]).with_steps(1)]);
    let root = Node::split(Axis::X, SplitSize::FixedFirst(120.0), dock, Node::tabs(vec![1], 0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let (mut panels, specs) = (RailPanels { seen: seen.clone() }, Tracked::new(RailSpecs));
    let columns: Rc<RefCell<Vec<(usize, Rect)>>> = Rc::default();
    let placed = Rc::clone(&columns);
    let mut fixture = Fixture::new(move |ui| {
        let out = DockArea::new("Rail dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
        *placed.borrow_mut() = out.columns;
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    let sizes = DockSizes::of(fixture.ctx());
    let view = seen.get().expect("the host showed the rail");
    let column = columns.borrow().iter().find(|(panel, _)| *panel == RAIL).map(|(_, rect)| *rect).expect("the rail's column shows");
    let strip = Rect::from_min_max(egui::pos2(view.left(), view.top() - sizes.grip), egui::pos2(view.right(), view.top()));
    assert!(sizes.grip > 0.0 && column.top() < strip.top(), "the strip lies under the column bar: {strip:?} in {column:?}");
    assert!(
        (view.bottom() - column.bottom()).abs() < 0.5 && view.height() > RAIL_HEIGHT,
        "the host's view reaches the docked column's foot (photocraft#16461): {view:?} in {column:?}"
    );

    // The strip's ticks: one mesh inside it of the theme's upright marks.
    let shapes = fixture.frame_shapes();
    let ticks: Vec<Rect> = shapes
        .iter()
        .find_map(|shape| {
            let Shape::Mesh(mesh) = shape else { return None };
            let quads: Vec<Rect> = mesh.vertices.chunks(4).map(|quad| Rect::from_points(&quad.iter().map(|v| v.pos).collect::<Vec<_>>())).collect();
            (!quads.is_empty() && quads.iter().all(|quad| strip.expand(0.01).contains_rect(*quad))).then_some(quads)
        })
        .expect("the strip paints its ticks");
    assert_eq!(ticks.len(), usize::from(craft_theme::GripShape::DEFAULT.count), "the theme's number of ticks: {ticks:?}");
    assert!(ticks.iter().all(|tick| tick.height() > tick.width()), "the ticks stand upright: {ticks:?}");
    let run = ticks.iter().fold(Rect::NOTHING, |run, tick| run.union(*tick));
    assert!((run.center().x - strip.center().x).abs() <= 0.5, "the ticks are centred across the rail: {run:?} in {strip:?}");

    fixture.drag_hold_at(strip.center(), Vec2::new(200.0, 0.0));
    let unit = drag::active::<GroupDrag<usize>>(fixture.ctx()).map(|drag| drag.unit.clone());
    assert_eq!(unit, Some(DragUnit::Group(RAIL)), "a drag on the grip moves the rail's group");
    fixture.release_at(strip.center() + Vec2::new(200.0, 0.0)).settle();
}

/// The rail's grip strip is a memoized view: a pointer moving elsewhere in the window replays it, as it replays the
/// column bars and the tab rows, and runs no grip (photocraft#16333).
#[trial]
fn a_tool_rails_grip_strip_replays_while_the_pointer_moves_elsewhere() {
    let seen = Rc::new(Cell::new(None));
    let dock = Node::dock(DockEdge::Start, vec![Column::new(vec![Group::new(vec![RAIL], 0)]).with_steps(1)]);
    let root = Node::split(Axis::X, SplitSize::FixedFirst(120.0), dock, Node::tabs(vec![1], 0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let (mut panels, specs) = (RailPanels { seen: seen.clone() }, Tracked::new(RailSpecs));
    let mut fixture = Fixture::new(move |ui| {
        let _ = DockArea::new("Rail dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    let grips =
        |fixture: &Fixture| last_pass_runs(fixture.ctx()).into_iter().filter(|(ty, _)| ty.contains("GroupGrip<")).map(|(_, runs)| runs).sum::<u32>();
    for step in 1..=3 {
        fixture.frame(vec![pointer::moved(egui::pos2(500.0 + step as f32, 300.0))]);
        assert_eq!(grips(&fixture), 0, "pointer step {step}: the rail's grip replays its last run");
    }
}

/// A tool rail floating in a window of its own is moved by its grip strip, as its title row moves it: a drag on the
/// grip released over empty room moves the window by the pointer's travel, the point grabbed staying under the pointer
/// (photocraft#16426). Before, the drag tried to float
/// the group again, which the model refuses for a floating group, and the window stayed.
#[trial]
fn a_floating_tool_rails_grip_moves_its_window() {
    let seen = Rc::new(Cell::new(None));
    let at = Rect::from_min_size(egui::pos2(300.0, 100.0), Vec2::new(60.0, RAIL_HEIGHT + 60.0));
    let window = craft_behavior::dock::Floating::new(100, Column::new(vec![Group::new(vec![RAIL], 0)]).with_steps(1), at);
    let mut workspace = Arc::new(Workspace::new(Layout::new(None, vec![window])));
    let (mut panels, specs) = (RailPanels { seen: seen.clone() }, Tracked::new(RailSpecs));
    let layout: Rc<RefCell<Option<Layout<usize>>>> = Rc::default();
    let kept = Rc::clone(&layout);
    let mut fixture = Fixture::new(move |ui| {
        let _ = DockArea::new("Rail dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
        *kept.borrow_mut() = Some(workspace.layout().clone());
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    let sizes = DockSizes::of(fixture.ctx());
    let view = seen.get().expect("the host showed the floating rail");
    let grip = egui::pos2(view.center().x, view.top() - sizes.grip * 0.5);
    let before = layout.borrow().as_ref().and_then(|l| l.floating.first().map(|w| w.rect)).expect("the rail floats");
    fixture.drag_hold_at(grip, Vec2::new(-200.0, 150.0));
    assert_eq!(
        drag::active::<GroupDrag<usize>>(fixture.ctx()).map(|drag| drag.unit.clone()),
        Some(DragUnit::Group(RAIL)),
        "a drag on the floating rail's grip drags its group"
    );
    let to = grip + Vec2::new(-200.0, 150.0);
    fixture.release_at(to).settle();
    let after = layout.borrow().as_ref().and_then(|l| l.floating.first().map(|w| w.rect)).expect("the rail still floats");
    let travel = to - grip;
    assert!(
        (after.min - (before.min + travel)).length() < 0.5 && (after.size() - before.size()).length() < 0.5,
        "the window moved by the pointer's travel {travel:?}, keeping the grabbed point under it: {before:?} -> {after:?}"
    );
    assert_eq!(layout.borrow().as_ref().map(|l| l.floating.len()), Some(1), "still one floating window");
}

/// The host shows a real tool rail with its Edit Toolbar button, as an app's Tools panel does, and records whether each
/// shown pass reported the button and whether a click on it was reported.
struct ToolRailPanels {
    slots: Tracked<Vec<ToolSlot<u8>>>,
    last: Tracked<LastUsed<u8>>,
    reported: Rc<RefCell<Vec<bool>>>,
    clicked: Rc<Cell<bool>>,
}

impl DockPanels<usize> for ToolRailPanels {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn group_view(&mut self, panels: &[usize], _: usize, ui: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if !panels.contains(&RAIL) {
            return None;
        }
        let rail = ToolRail::new("Tools", &self.slots, &0, &self.last).tear_off().edit_toolbar("Edit Toolbar");
        match at {
            GroupAt::Measure(width) => Some(rail.measure_rail(ui, width, |_, _| {}).y),
            GroupAt::Show(rect) => {
                let out = rail.show_rail(ui, |_, _| {});
                self.reported.borrow_mut().push(out.edit_toolbar.is_some());
                self.clicked.set(self.clicked.get() || out.edit_toolbar.as_ref().is_some_and(craft_ui::contract::clicked));
                Some(rect.height())
            }
        }
    }
}

/// A tool rail its host shows in a docked column reports its Edit Toolbar button in every pass it shows, settled or
/// not, and the click on it, so the app can open its drawer.
#[trial]
fn a_docked_tool_rail_reports_its_edit_toolbar_button() {
    let tools: Vec<ToolSlot<u8>> = [EditCommands::Undo, EditCommands::Redo]
        .into_iter()
        .zip(0u8..)
        .map(|(command, id)| ToolSlot::new(u32::from(id), vec![RailTool::new(id, command)]))
        .collect();
    let (reported, clicked) = (Rc::new(RefCell::new(Vec::new())), Rc::new(Cell::new(false)));
    let mut panels =
        ToolRailPanels { slots: Tracked::new(tools), last: Tracked::new(LastUsed::default()), reported: reported.clone(), clicked: clicked.clone() };
    let dock = Node::dock(DockEdge::Start, vec![Column::new(vec![Group::new(vec![RAIL], 0)]).with_steps(1)]);
    let root = Node::split(Axis::X, SplitSize::FixedFirst(120.0), dock, Node::tabs(vec![1], 0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let specs = Tracked::new(RailSpecs);
    let mut fixture = Fixture::new(move |ui| {
        let _ = DockArea::new("Rail dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    assert!(
        !reported.borrow().is_empty() && reported.borrow().iter().all(|seen| *seen),
        "every shown pass reports the button: {:?}",
        reported.borrow()
    );
    let button = fixture.by_label("Edit Toolbar").expect("the docked rail shows its Edit Toolbar button");
    reported.borrow_mut().clear();
    fixture.click(&button).idle(3);
    assert!(reported.borrow().iter().all(|seen| *seen), "the passes of the click report the button: {:?}", reported.borrow());
    assert!(clicked.get(), "the click on the button is reported");
}

/// A panel whose body lists more rows than its group shows, with a footer of one action.
#[derive(PlainData)]
struct Listed;

impl PanelSpecs<usize> for Listed {
    fn spec(&self, _: &usize) -> PanelSpec {
        PanelSpec::new("Findings").footer()
    }
}

impl DockPanels<usize> for Listed {
    fn body(&mut self, _: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        Stack::new().fill().scroll(Scroll::vertical()).show(ui, |ui, _| {
            for row in 0..60 {
                Text::new(format!("Row {row}")).show(ui);
            }
        });
    }

    fn footer(&mut self, _: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        Button::new("Apply all").show(ui);
    }
}

/// A panel's footer is pinned under its body: it shows inside the group while the body is taller than the group, and
/// stays where it is when the body scrolls.
#[trial]
fn a_panels_footer_stays_in_view_under_a_scrolling_body() {
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(Node::tabs(vec![1], 0)), Vec::new())));
    let specs = Tracked::new(Listed);
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut Listed, DockConfig::default()).show(ui);
    });
    let room = Vec2::new(400.0, 300.0);
    fixture.room(room).settle();
    let footer = fixture.by_label("Apply all").expect("the footer shows its action").rect;
    assert!(footer.bottom() <= room.y, "the footer lies inside the group: {footer:?}");
    let row = |fixture: &Fixture| fixture.by_label("Row 0").expect("the first row").rect;
    let before = row(&fixture);
    assert!(before.bottom() <= footer.top(), "the body lies above the footer: {before:?}, footer {footer:?}");

    fixture.frame(vec![egui::Event::PointerMoved(before.center())]);
    fixture.frame(vec![pointer::wheel(-120.0)]).settle();
    assert!(row(&fixture).top() < before.top(), "the body scrolls: {before:?}, then {:?}", row(&fixture));
    assert_eq!(fixture.by_label("Apply all").map(|action| action.rect), Some(footer), "the footer stays where it was");
}

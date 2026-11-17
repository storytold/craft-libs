//! Stories of the dock area.

use super::DockArea;
use crate::contract::SizeLimits;
use crate::contract::{Bind, BodyCx, Show as _};
use crate::display::Text;
use crate::docking::{DockConfig, DockGrid, DockPanels, GroupAt, PanelSpec};
use crate::menu::PanelRow;
use crate::patterns::presets::{PresetPanelMenu, PresetView};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::dock::{Column, DockEdge, Floating, Group, Layout, Node, Permissions, SplitSize, Workspace};
use craft_behavior::{Axis, Proposal};
use egui::{Pos2, Rect, Ui};

const LAYERS: usize = 0;
const CHANNELS: usize = 1;
const PROPERTIES: usize = 2;
const HISTORY: usize = 3;
const CANVAS: usize = 4;
const SWATCHES: usize = 5;
/// A toolbar: its content lays out in a grid, and its column never collapses or holds another group.
const TOOLS: usize = 6;
const NAVIGATOR: usize = 7;
const SIDE: f32 = 220.0;
const WINDOW: Rect = Rect { min: Pos2::new(320.0, 80.0), max: Pos2::new(520.0, 220.0) };
/// Where the collapsed floating window keeps its expanded size.
const COLLAPSED_WINDOW: Rect = Rect { min: Pos2::new(320.0, 240.0), max: Pos2::new(520.0, 380.0) };

/// Panels that show their own name.
struct Demo {
    /// The canvas group is drawn by the host.
    canvas_view: bool,
    /// The Swatches menu holds the host's preset rows above the dock's rows.
    host_rows: bool,
}

fn name(panel: usize) -> &'static str {
    match panel {
        LAYERS => "Layers",
        CHANNELS => "Channels",
        PROPERTIES => "Properties",
        HISTORY => "History",
        SWATCHES => "Swatches",
        TOOLS => "Tools",
        NAVIGATOR => "Navigator",
        _ => "Canvas",
    }
}

impl DockPanels<usize> for Demo {
    fn spec(&self, panel: &usize) -> PanelSpec {
        let spec = PanelSpec::new(name(*panel)).limits(SizeLimits::default());
        match *panel {
            CANVAS => spec.canvas(),
            TOOLS => spec.grid(DockGrid::new(40.0, 32.0, 4)),
            _ => spec,
        }
    }

    fn permissions(&self, panel: &usize) -> Permissions {
        match *panel {
            CANVAS => Permissions::PROTECTED,
            TOOLS => Permissions::ALL.collapses(false).shares_column(false),
            _ => Permissions::ALL,
        }
    }

    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        Text::new(name(*panel)).show(ui);
    }

    fn panel_rows(&mut self, panel: &usize) -> Vec<PanelRow> {
        if self.host_rows && *panel == SWATCHES { PresetPanelMenu::<usize>::new(PresetView::Grid).new_group(true).rows() } else { Vec::new() }
    }

    fn group_view(&mut self, panels: &[usize], _: usize, ui: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if !(self.canvas_view && panels.contains(&CANVAS)) {
            return None;
        }
        let mut document = Text::new("Document");
        Some(match at {
            GroupAt::Measure(width) => document.measure_in(ui, Proposal::Width(width)).y,
            GroupAt::Show(rect) => {
                document.show(ui);
                rect.height()
            }
        })
    }
}

/// A canvas between a start dock, a toolbar column one step wide and a column of panels, and an iconic column at the
/// end, with one floating group and one floating window collapsed to icons.
fn workspace() -> Workspace<usize> {
    let tools = Column::new(vec![Group::new(vec![TOOLS], 0)]).with_steps(1);
    let side = Column::new(vec![Group::new(vec![LAYERS, CHANNELS], 0), Group::new(vec![PROPERTIES], 0)]).with_width(SIDE);
    let mut icons = Column::new(vec![Group::new(vec![SWATCHES], 0)]).with_width(SIDE);
    icons.iconic = true;
    let centre = Node::split(Axis::X, SplitSize::Ratio(0.8), Node::tabs(vec![CANVAS], 0), Node::dock(DockEdge::End, vec![icons]));
    let root = Node::split(Axis::X, SplitSize::FixedFirst(SIDE), Node::dock(DockEdge::Start, vec![tools, side]), centre);
    let window = Floating::single(vec![HISTORY], 0, WINDOW).with_id(1);
    let mut collapsed = Floating::single(vec![NAVIGATOR], 0, COLLAPSED_WINDOW).with_id(2);
    collapsed.column.iconic = true;
    Workspace::new(Layout::new(Some(root), vec![window, collapsed]))
}

fn dock(ui: &mut Ui, data: &mut StoryData, demo: &mut Demo) -> StoryOutput {
    let workspace = data.value("workspace", workspace());
    DockArea::new("Dock", Bind::bound(&mut *workspace), demo, DockConfig::default()).show(ui).into()
}

#[story(DockArea<'static, usize>, covers = [Default], drive = Drive::Never)]
fn docked_columns_a_floating_group_and_a_flyout(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dock(ui, data, &mut Demo { canvas_view: false, host_rows: false })
}

#[story(DockArea<'static, usize>, covers = [Default], drive = Drive::Never)]
fn a_canvas_group_the_host_draws(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dock(ui, data, &mut Demo { canvas_view: true, host_rows: false })
}

#[story(DockArea<'static, usize>, covers = [Default], drive = Drive::Never)]
fn host_rows_above_the_dock_rows_in_a_panel_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dock(ui, data, &mut Demo { canvas_view: false, host_rows: true })
}

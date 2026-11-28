//! Stories of the dock area.

use super::DockArea;
use crate::buttons::IconButton;
use crate::contract::SizeLimits;
use crate::contract::{BodyCx, Props as _, Show as _, Tracked};
use crate::display::Text;
use crate::docking::{ColumnCards, DockConfig, DockGrid, DockPanels, GroupAt, IconicMode, PanelSpec, PanelSpecs};
use crate::menu::PanelRow;
use crate::patterns::presets::{PresetPanelMenu, PresetView};
use crate::tabs::PanelTabLook;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::dock::{Column, DockEdge, Floating, Group, Layout, Node, Permissions, SplitSize, Workspace};
use craft_behavior::{Axis, IconId, Proposal};
use egui::{Pos2, Rect, Ui};
use std::sync::{Arc, LazyLock};

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

/// The demo panels' specs: fixed by panel.
#[derive(Clone, Debug, Default, PartialEq, crate::contract::PlainData)]
struct DemoSpecs;

impl PanelSpecs<usize> for DemoSpecs {
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
}

impl DockPanels<usize> for Demo {
    fn body(&mut self, panel: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        Text::new(name(*panel)).show(ui);
    }

    fn rail_footer(&mut self, _: &usize, ui: &mut Ui, _: &mut BodyCx<'_>) {
        IconButton::new(IconId::new("chevron_up")).label("Previous").show(ui);
        IconButton::new(IconId::new("chevron_down")).label("Next").show(ui);
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
/// end, with one floating group, `minimized` or not, and one floating window collapsed to icons.
fn workspace(minimized: bool) -> Arc<Workspace<usize>> {
    let tools = Column::new(vec![Group::new(vec![TOOLS], 0)]).with_steps(1);
    let side = Column::new(vec![Group::new(vec![LAYERS, CHANNELS], 0), Group::new(vec![PROPERTIES], 0)]).with_width(SIDE);
    let mut icons = Column::new(vec![Group::new(vec![SWATCHES], 0)]).with_width(SIDE);
    icons.iconic = true;
    let centre = Node::split(Axis::X, SplitSize::Ratio(0.8), Node::tabs(vec![CANVAS], 0), Node::dock(DockEdge::End, vec![icons]));
    let root = Node::split(Axis::X, SplitSize::FixedFirst(SIDE), Node::dock(DockEdge::Start, vec![tools, side]), centre);
    let mut window = Floating::single(1, vec![HISTORY], 0, WINDOW);
    window.column.groups[0].minimized = minimized;
    let mut collapsed = Floating::single(2, vec![NAVIGATOR], 0, COLLAPSED_WINDOW);
    collapsed.column.iconic = true;
    Arc::new(Workspace::new(Layout::new(Some(root), vec![window, collapsed])))
}

/// One spec model for every story, so its revision stays put across frames.
static SPECS: LazyLock<Tracked<DemoSpecs>> = LazyLock::new(|| Tracked::new(DemoSpecs));

fn dock(ui: &mut Ui, data: &mut StoryData, demo: &mut Demo, minimized: bool) -> StoryOutput {
    dock_with(ui, data, demo, minimized, DockConfig::default())
}

fn dock_with(ui: &mut Ui, data: &mut StoryData, demo: &mut Demo, minimized: bool, config: DockConfig) -> StoryOutput {
    let (workspace, workspace_at) = data.value_at("workspace", workspace(minimized));
    DockArea::new("Dock", workspace_at.bind(&mut *workspace), &*SPECS, demo, config).show(ui).into()
}

#[story(DockArea<'static, usize>, covers = [Default], drive = Drive::Never)]
fn docked_columns_a_floating_group_and_a_flyout(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dock(ui, data, &mut Demo { canvas_view: false, host_rows: false }, false)
}

#[story(DockArea<'static, usize>, covers = [Default], drive = Drive::Never)]
fn a_canvas_group_the_host_draws(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dock(ui, data, &mut Demo { canvas_view: true, host_rows: false }, false)
}

#[story(DockArea<'static, usize>, covers = [Default], drive = Drive::Never)]
fn host_rows_above_the_dock_rows_in_a_panel_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dock(ui, data, &mut Demo { canvas_view: false, host_rows: true }, false)
}

/// Every group shows its panel menu button, the three-line mark, whether its shown panel has rows of its own or only
/// the dock's (`DockConfig::panel_menu_always`, `DockConfig::panel_menu_icon`): VectorCraft's dock.
#[story(DockArea<'static, usize>, covers = [Default], drive = Drive::Never)]
fn a_menu_button_on_every_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let config = DockConfig::default().panel_menu_always(true).panel_menu_icon(IconId::new("menu"));
    dock_with(ui, data, &mut Demo { canvas_view: false, host_rows: false }, false, config)
}

/// The floating group minimized: its window shows the title row and the tab strip at its width.
#[story(DockArea<'static, usize>, covers = [Default], drive = Drive::Never)]
fn a_minimized_floating_window(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    dock(ui, data, &mut Demo { canvas_view: false, host_rows: false }, true)
}

#[story(DockArea<'static, usize>, covers = [Default], drive = Drive::Never, title = "Rail with fixed controls")]
fn rail_with_fixed_controls(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (workspace, workspace_at) = data.value_at("rail workspace", {
        let side = Column::new(vec![Group::new(vec![LAYERS, CHANNELS, PROPERTIES, HISTORY, SWATCHES, NAVIGATOR], 0)]).with_iconic(true);
        let root = Node::split(Axis::X, SplitSize::Ratio(0.8), Node::tabs(vec![CANVAS], 0), Node::dock(DockEdge::End, vec![side]));
        Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))
    });
    let mut demo = Demo { canvas_view: false, host_rows: false };
    let config = DockConfig::default().iconic_mode(IconicMode::Rail);
    DockArea::new("Rail dock", workspace_at.bind(&mut *workspace), &*SPECS, &mut demo, config).rail_footer(LAYERS).show(ui).into()
}

/// VectorCraft 0.24's dock frame: only the stepped tool column is a card (`ColumnCards::Stepped`); the docks stand flush
/// against the canvas, the column of icons framed as the panel column is (`DockConfig::flush_docks`); the panel tabs and
/// the column bars in the block look (`PanelTabLook::Block`).
#[story(DockArea<'static, usize>, covers = [Default], drive = Drive::Never)]
fn a_tool_card_beside_flush_panels(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (workspace, workspace_at) = data.value_at("stepped cards workspace", {
        let tools = Column::new(vec![Group::new(vec![TOOLS], 0)]).with_steps(1);
        let side = Column::new(vec![Group::new(vec![PROPERTIES, LAYERS], 0), Group::new(vec![HISTORY], 0)]);
        let icons = Column::new(vec![Group::new(vec![SWATCHES], 0), Group::new(vec![NAVIGATOR], 0)]).with_iconic(true);
        let docked = Node::split(Axis::X, SplitSize::Ratio(0.6), Node::tabs(vec![CANVAS], 0), Node::dock(DockEdge::End, vec![side, icons]));
        let root = Node::split(Axis::X, SplitSize::FixedFirst(60.0), Node::dock(DockEdge::Start, vec![tools]), docked);
        Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())))
    });
    let mut demo = Demo { canvas_view: false, host_rows: false };
    let config = DockConfig::default().column_cards(ColumnCards::Stepped).flush_docks(true).tab_look(PanelTabLook::Block);
    DockArea::new("Stepped cards dock", workspace_at.bind(&mut *workspace), &*SPECS, &mut demo, config).show(ui).into()
}

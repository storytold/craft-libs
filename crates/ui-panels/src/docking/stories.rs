//! Stories of the docking family.

use super::SplitFramesCx as _;
use super::stack::{StackSection, StackSections};
use super::tree::FrameRules;
use super::{DockGroup, DockPanels, DragUnit, GroupGrip, HANDLE, PanelKey, PanelSpec, PanelTitle, SplitArea, Splitter, WindowMenuRows};
use crate::contract::SizeLimits;
use crate::contract::{Bind, BodyCx, Props as _, Show as _, ShowContainer as _, Validity};
use crate::menu::Menu;
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::dock::{Layout, Node, SplitSize};
use craft_behavior::{Axis, Mixed};
use egui::{Ui, Vec2};

const LAYERS: usize = 0;
const CHANNELS: usize = 1;
const PROPERTIES: usize = 10;

fn two_columns() -> Layout<usize> {
    let root = Node::split(Axis::X, SplitSize::Ratio(0.33), Node::tabs(vec![LAYERS, CHANNELS], 0), Node::tabs(vec![PROPERTIES], 0));
    Layout::new(Some(root), Vec::new())
}

fn title_of(panel: usize) -> PanelTitle {
    let name = match panel {
        LAYERS => "Layers",
        CHANNELS => "Channels",
        _ => "Properties",
    };
    PanelTitle::new(PanelKey::of(&panel), name)
}

fn titles(panels: &[usize]) -> Vec<PanelTitle> {
    panels.iter().map(|panel| title_of(*panel)).collect()
}

fn dock(ui: &mut Ui, bind: Bind<'_, Layout<usize>>, shown: &Layout<usize>) -> StoryOutput {
    let limits = |_: &usize| SizeLimits::default();
    let rules = FrameRules::new(&limits);
    let area = SplitArea::new("Panels", bind, rules).show(ui, |ui, body| {
        for (anchor, rect) in body.group_frames::<usize>() {
            let Some(Node::Tabs { panels, .. }) = shown.place(&anchor).and_then(|place| shown.node_of(&place)) else { continue };
            let panels = titles(panels);
            let active = panels.first().map_or(PanelKey::new(0), |panel| panel.key);
            DockGroup::new(anchor, &panels, active).show_at(ui, rect, |_, _| ());
        }
    });
    area.output.edit.into()
}

fn drag_handle(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(grip) = fixture.part(id, HANDLE.at(0)) else { return };
    let travel = 2.0 * crate::testing::input::max_click_dist(fixture.ctx());
    fixture.drag(&grip, Vec2::new(travel, 0.0));
}

#[story(SplitArea<'static, usize>, covers = [Default], drive = Drive::Custom(drag_handle))]
fn columns(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let layout = data.value("dock", two_columns());
    let shown = layout.clone();
    dock(ui, Bind::bound(layout), &shown)
}

#[story(SplitArea<'static, usize>, covers = [Mixed], drive = Drive::Custom(drag_handle))]
fn layouts_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let layout = data.value("dock", Mixed::<Layout<usize>>::Mixed);
    dock(ui, Bind::bound_mixed(layout), &two_columns())
}

#[story(SplitArea<'static, usize>, covers = [Invalid], drive = Drive::Custom(drag_handle))]
fn a_dock_of_one_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let layout = data.value("dock", Layout::new(Some(Node::tabs(vec![LAYERS], 0)), Vec::new()));
    let shown = layout.clone();
    let needs_two = |layout: &Layout<usize>| {
        if layout.panels().len() < 2 { Validity::Refuse(Message::new("A dock needs two panels")) } else { Validity::Ok }
    };
    dock(ui, Bind::bound(layout).check(needs_two), &shown)
}

#[story(DockGroup<'static, usize>, covers = [Default])]
fn two_panels(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let panels = titles(&[LAYERS, CHANNELS]);
    DockGroup::new(LAYERS, &panels, panels[0].key).show(ui, |_, _| ()).output.into()
}

#[story(GroupGrip<usize>, covers = [Default])]
fn a_group_grip(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    GroupGrip::new(DragUnit::Group(LAYERS), PanelKey::of(&LAYERS)).label("Move tab group").show(ui).into()
}

#[story(Splitter, covers = [Default])]
fn between_columns(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Splitter::new(Axis::X).label("Resize panels").show(ui).into()
}

#[story(Splitter, covers = [Default])]
fn between_rows(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Splitter::new(Axis::Y).label("Resize panels").show(ui).into()
}

#[story(StackSections, covers = [Default])]
fn between_sections(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    StackSections::new(vec![StackSection::new(true), StackSection::new(true)]).label("Sections").show(ui, |_, _| ()).output.into()
}

/// The panels of the Window menu story: each labelled by its number.
struct NumberedPanels;

impl DockPanels<u32> for NumberedPanels {
    fn spec(&self, panel: &u32) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }

    fn body(&mut self, _: &u32, _: &mut Ui, _: &mut BodyCx<'_>) {}
}

#[story(WindowMenuRows<'static, u32>, covers = [Default], drive = Drive::Never)]
fn window_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let layout = Layout::new(Some(Node::tabs(vec![1], 0)), Vec::new());
    let menu = Menu::new("Window").open(Bind::bound(data.value("open", true))).show(ui, |ui, _| {
        WindowMenuRows::new(&layout, &NumberedPanels, &[1, 2]).show(ui);
    });
    StoryOutput::from(menu.output)
}

//! Stories of the docking family.

use super::SplitFramesCx as _;
use super::group::MENU;
use super::stack::{StackSection, StackSections};
use super::tree::FrameRules;
use super::{
    DockCommands, DockGroup, DragUnit, GroupGrip, HANDLE, PanelDocument, PanelKey, PanelSpec, PanelSpecs, PanelTitle, SplitArea, Splitter, Titles,
    WindowMenuRows,
};
use crate::buttons::Button;
use crate::contract::SizeLimits;
use crate::contract::{Bind, MemoKey, Props as _, Show as _, ShowContainer as _, Tracked, Validity};
use crate::menu::{Menu, PanelRow};
use crate::runtime::commands::CommandRef;
use crate::runtime::env::Message;
use crate::tabs::PanelTabLook;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::dock::{Layout, Node, SplitSize};
use craft_behavior::{Axis, IconId, Mixed};
use craft_theme::Srgba;
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

/// The story's split area over `bind`, whose layout and rules the story data's `revision` covers.
fn dock(ui: &mut Ui, bind: Bind<'_, Layout<usize>>, shown: &Layout<usize>, revision: u128) -> StoryOutput {
    let limits = |_: &usize| SizeLimits::default();
    let rules = FrameRules::new(&limits);
    let area = SplitArea::new("Panels", bind, rules, revision).show(ui, |ui, body| {
        for (anchor, rect) in body.group_frames::<usize>() {
            let Some(Node::Tabs { panels, .. }) = shown.place(&anchor).and_then(|place| shown.node_of(&place)) else { continue };
            let panels = titles(panels);
            let active = panels.first().map_or(PanelKey::new(0), |panel| panel.key);
            DockGroup::new(anchor, Titles::from_content(panels.clone()), active).show_at(ui, rect, |_, _| ());
        }
    });
    area.output.edit.into()
}

fn drag_handle(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(grip) = fixture.part(id, HANDLE.at(0)) else { return };
    let travel = 2.0 * crate::testing::input::slop(fixture.ctx());
    fixture.drag(&grip, Vec2::new(travel, 0.0));
}

#[story(SplitArea<'static, usize>, covers = [Default], drive = Drive::Custom(drag_handle))]
fn columns(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let revision = MemoKey::of(&data.revision());
    let (layout, layout_at) = data.value_at("dock", two_columns());
    let shown = layout.clone();
    dock(ui, layout_at.bind(layout), &shown, revision)
}

#[story(SplitArea<'static, usize>, covers = [Mixed], drive = Drive::Custom(drag_handle))]
fn layouts_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let revision = MemoKey::of(&data.revision());
    let (layout, layout_at) = data.value_at("dock", Mixed::<Layout<usize>>::Mixed);
    dock(ui, layout_at.bind_mixed(layout), &two_columns(), revision)
}

#[story(SplitArea<'static, usize>, covers = [Invalid], drive = Drive::Custom(drag_handle))]
fn a_dock_of_one_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let revision = MemoKey::of(&data.revision());
    let (layout, layout_at) = data.value_at("dock", Layout::new(Some(Node::tabs(vec![LAYERS], 0)), Vec::new()));
    let shown = layout.clone();
    let needs_two = |layout: &Layout<usize>| {
        if layout.panels().len() < 2 { Validity::Refuse(Message::new("A dock needs two panels")) } else { Validity::Ok }
    };
    dock(ui, layout_at.bind(layout).check(needs_two), &shown, revision)
}

#[story(DockGroup<'static, usize>, covers = [Default])]
fn two_panels(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let panels = titles(&[LAYERS, CHANNELS]);
    DockGroup::new(LAYERS, Titles::from_content(panels.clone()), panels[0].key).show(ui, |_, _| ()).output.into()
}

/// A group in a floating window: nothing goes beside it, so a drop on its body joins it or stacks above or below.
#[story(DockGroup<'static, usize>, covers = [Default])]
fn a_group_in_a_window(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let panels = titles(&[LAYERS, CHANNELS]);
    DockGroup::new(LAYERS, Titles::from_content(panels.clone()), panels[0].key).in_window().show(ui, |_, _| ()).output.into()
}

/// A group whose panel has a footer of actions pinned under its body, which stays in view while the body scrolls.
#[story(DockGroup<'static, usize>, covers = [Default])]
fn a_group_with_a_footer(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let panels = titles(&[LAYERS, CHANNELS]);
    DockGroup::new(LAYERS, Titles::from_content(panels.clone()), panels[0].key)
        .footer(|ui, _| {
            Button::new("Apply All").show(ui);
        })
        .show(ui, |_, _| ())
        .output
        .into()
}

/// A group whose shown panel counts what it lists after its name and says more in a caption under the header.
#[story(DockGroup<'static, usize>, covers = [Default])]
fn a_panel_with_a_count_and_a_caption(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let panels = vec![title_of(LAYERS).count(Some(3)).caption(Some("2 hidden".into())), title_of(CHANNELS).count(Some(4))];
    DockGroup::new(LAYERS, Titles::from_content(panels.clone()), panels[0].key).show(ui, |_, _| ()).output.into()
}

/// A group whose Timeline lists its open compositions as tabs after the panel tabs, each with its label colour and a
/// close button; the shown composition's tab is selected while the Timeline shows, a pick shows another and a close
/// closes it.
#[story(DockGroup<'static, usize>, covers = [Default])]
fn a_panel_with_documents(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = *data.value("shown", 1_u64);
    let open = data.value("open", vec![1_u64, 2]).clone();
    let comp = |id: u64| {
        let color = (id == 1).then(|| Srgba::rgb(0.8, 0.3, 0.3));
        PanelDocument::new(id, format!("Comp {id}")).swatch(color).closable(true).shown(id == shown)
    };
    let panels = vec![title_of(LAYERS).documents(open.iter().map(|id| comp(*id)).collect()), title_of(CHANNELS)];
    let output = DockGroup::new(LAYERS, Titles::from_content(panels.clone()), panels[0].key).closable().show(ui, |_, _| ()).output;
    if let Some((_, id)) = output.picked_document {
        *data.value("shown", 1_u64) = id;
    }
    data.value("open", vec![1_u64, 2]).retain(|id| !output.closed_documents.iter().any(|(_, closed)| closed == id));
    output.into()
}

/// A click on the panel menu button opens the group's panel menu.
fn open_panel_menu(fixture: &mut Fixture) {
    if let Some(button) = fixture.root_id().and_then(|id| fixture.part(id, MENU)) {
        fixture.click(&button);
        fixture.settle();
    }
}

#[story(DockGroup<'static, usize>, covers = [Open], drive = Drive::Custom(open_panel_menu))]
fn panel_menu(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let panels = titles(&[LAYERS, CHANNELS]);
    let rows = [DockCommands::FloatPanel, DockCommands::FloatGroup].map(|command| PanelRow::Command(CommandRef::from(command))).to_vec();
    DockGroup::new(LAYERS, Titles::from_content(panels.clone()), panels[0].key).menu(move || rows.clone()).show(ui, |_, _| ()).output.into()
}

/// VectorCraft's group header (`DockConfig::tab_look(PanelTabLook::Pill)`): the shown tab on a filled pill in the
/// strip's middle, and the panel menu button, the three-line mark, level with it at the strip's end.
#[story(DockGroup<'static, usize>, covers = [Default])]
fn pill_tabs_and_the_panel_menu(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let panels = titles(&[LAYERS, CHANNELS]);
    let rows = [DockCommands::FloatPanel, DockCommands::FloatGroup].map(|command| PanelRow::Command(CommandRef::from(command))).to_vec();
    DockGroup::new(LAYERS, Titles::from_content(panels.clone()), panels[0].key)
        .menu(move || rows.clone())
        .panel_menu_icon(IconId::new("menu"))
        .tab_look(PanelTabLook::Pill)
        .show(ui, |_, _| ())
        .output
        .into()
}

/// VectorCraft 0.24's group header (`DockConfig::tab_look(PanelTabLook::Block)`): the shown tab a square
/// `surface.control` block as tall as the strip, the strip on `surface.canvas`, and the panel menu button in the
/// strip's middle at its end.
#[story(DockGroup<'static, usize>, covers = [Default])]
fn block_tabs_and_the_panel_menu(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let panels = titles(&[LAYERS, CHANNELS]);
    let rows = [DockCommands::FloatPanel, DockCommands::FloatGroup].map(|command| PanelRow::Command(CommandRef::from(command))).to_vec();
    DockGroup::new(LAYERS, Titles::from_content(panels.clone()), panels[0].key)
        .menu(move || rows.clone())
        .panel_menu_icon(IconId::new("menu"))
        .tab_look(PanelTabLook::Block)
        .show(ui, |_, _| ())
        .output
        .into()
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
#[derive(crate::contract::PlainData)]
struct NumberedPanels;

impl PanelSpecs<u32> for NumberedPanels {
    fn spec(&self, panel: &u32) -> PanelSpec {
        PanelSpec::new(format!("Panel {panel}"))
    }
}

#[story(WindowMenuRows<'static, u32>, covers = [Default], drive = Drive::Never)]
fn window_menu(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let layout = Layout::new(Some(Node::tabs(vec![1], 0)), Vec::new());
    let menu = Menu::new("Window").open(data.bind("open", true)).show(ui, |ui, _| {
        WindowMenuRows::new(&layout, &Tracked::new(NumberedPanels), &[1, 2]).show(ui);
    });
    StoryOutput::from(menu.output)
}

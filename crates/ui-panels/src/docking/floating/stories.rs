//! Stories of `FloatingGroup`.

use super::{FloatingGroup, WindowColumn};
use crate::buttons::Button;
use crate::contract::Show as _;
use crate::docking::IconicSizes;
use crate::menu::MenuItem;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::dock::DockEdge;
use egui::{Pos2, Rect, Ui, Vec2};

const START: Rect = Rect { min: Pos2::new(40.0, 40.0), max: Pos2::new(280.0, 220.0) };

/// The icons a collapsed story column shows: three panels.
const ICON_CELLS: f32 = 3.0;

/// A collapsed column of `ICON_CELLS` icon cells, `width` wide when the user resized it, else the rail's width: it
/// resizes from the rail's width up to the width of the expanded story window.
fn icon_strip(ui: &Ui, edge: DockEdge, width: Option<f32>) -> WindowColumn {
    let sizes = IconicSizes::of(ui.ctx());
    let widths = (sizes.column.iconic_least, START.width());
    let width = width.unwrap_or(widths.0).clamp(widths.0, widths.1);
    WindowColumn::collapsed(edge, Vec2::new(width, sizes.cell * ICON_CELLS), widths)
}

#[story(FloatingGroup<'static>, covers = [Default], drive = Drive::Never)]
fn floating_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = *data.value("rect", START);
    let shown = FloatingGroup::new("Layers", rect)
        .menu(|ui, _| {
            MenuItem::new("New Layer").show(ui);
        })
        .show_group(ui, |ui, _| {
            Button::new("Add layer").show(ui);
        });
    if let Some(change) = shown.output.placed {
        *data.value("rect", START) = change.rect;
    }
    shown.output.into()
}

/// A dock window: its title row is its column's bar, with the double arrow at the end the window keeps to. The
/// arrow and a double click on the band collapse the column to a strip of icons against that edge.
#[story(FloatingGroup<'static>, covers = [Open], drive = Drive::Never)]
fn floating_dock_window(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = *data.value("rect", START);
    let collapsed = *data.value("collapsed", false);
    let width = *data.value("width", None::<f32>);
    let column = if collapsed { icon_strip(ui, DockEdge::Start, width) } else { WindowColumn::expanded(DockEdge::Start) };
    let shown = FloatingGroup::new("Layers", rect).column(column).sized(crate::contract::Sizing::Fill).show_group(ui, |ui, _| {
        Button::new(if collapsed { "L" } else { "Add layer" }).show(ui);
    });
    if let Some(change) = shown.output.placed {
        *data.value("rect", START) = change.rect;
    }
    if let Some(iconic) = shown.output.iconic {
        *data.value("collapsed", false) = iconic;
    }
    if let Some(width) = shown.output.collapsed_width {
        *data.value("width", None::<f32>) = Some(width);
    }
    shown.output.into()
}

/// A dock window whose column is collapsed to icons, against the float area's end edge.
#[story(FloatingGroup<'static>, covers = [Default], drive = Drive::Never)]
fn floating_collapsed_window(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rect = *data.value("rect", START);
    let width = *data.value("width", None::<f32>);
    let column = icon_strip(ui, DockEdge::End, width);
    let shown = FloatingGroup::new("Layers", rect).column(column).sized(crate::contract::Sizing::Fill).show_group(ui, |ui, _| {
        Button::new("L").show(ui);
    });
    if let Some(change) = shown.output.placed {
        *data.value("rect", START) = change.rect;
    }
    if let Some(width) = shown.output.collapsed_width {
        *data.value("width", None::<f32>) = Some(width);
    }
    shown.output.into()
}

//! Stories of `UnitSelect`.

use super::UnitSelect;
use crate::contract::{Bind, Entry, Show as _, Validity};
use crate::runtime::env::{Message, UnitKind};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use egui::Ui;

fn units() -> [Entry<UnitKind>; 3] {
    [Entry::new(UnitKind::Millimetres, "mm"), Entry::new(UnitKind::Inches, "in"), Entry::new(UnitKind::Points, "pt")]
}

#[story(UnitSelect<'static>, covers = [Default], drive = Drive::Type("42"))]
fn unit_select(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let units = units();
    let width = data.value("width_and_unit", (Some(210.0), UnitKind::Millimetres));
    let (width, unit) = (&mut width.0, &mut width.1);
    UnitSelect::new("Width", Bind::bound(width), Bind::bound(unit), &units).show(ui).into()
}

#[story(UnitSelect<'static>, covers = [Mixed], drive = Drive::Never)]
fn unit_select_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let units = units();
    let (width, unit) = data.value("mixed_width_and_unit", (Mixed::<Option<f64>>::Mixed, UnitKind::Millimetres));
    UnitSelect::new("Width", Bind::bound_mixed(width), Bind::bound(unit), &units).show(ui).into()
}

#[story(UnitSelect<'static>, covers = [Invalid], drive = Drive::Never)]
fn unit_select_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let units = units();
    let refuse_negative = |width: &Option<f64>| match width {
        Some(width) if *width < 0.0 => Validity::Refuse(Message::new("Width cannot be negative")),
        _ => Validity::Ok,
    };
    let (width, unit) = data.value("negative_width_and_unit", (Some(-5.0), UnitKind::Millimetres));
    UnitSelect::new("Width", Bind::bound(width).check(refuse_negative), Bind::bound(unit), &units).show(ui).into()
}

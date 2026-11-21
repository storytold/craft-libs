//! Stories of `UnitSelect`.

use super::UnitSelect;
use crate::contract::{Bind, Entry, Props as _, Show as _, Sizing, Validity};
use crate::runtime::env::{Message, UnitKind};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use egui::Ui;

fn units() -> [Entry<UnitKind>; 3] {
    [Entry::new(UnitKind::Millimetres, "Millimetres"), Entry::new(UnitKind::Inches, "Inches"), Entry::new(UnitKind::Points, "Points")]
}

#[story(UnitSelect<'static>, covers = [Default], drive = Drive::Type("42"))]
fn unit_select(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let units = units();
    let (width, width_at) = data.value_at("width_and_unit", (Some(210.0), UnitKind::Millimetres));
    let (width, unit) = (&mut width.0, &mut width.1);
    UnitSelect::new("Width", width_at.bind(width), Bind::bound(unit), &units).show(ui).into()
}

/// Content-sized, as a compact form's rows show it: the field one number cell wide, the select as wide as its
/// longest unit.
#[story(UnitSelect<'static>, covers = [], drive = Drive::Type("42"))]
fn unit_select_content_sized(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let units = units();
    let (width, width_at) = data.value_at("compact_width_and_unit", (Some(210.0), UnitKind::Millimetres));
    let (width, unit) = (&mut width.0, &mut width.1);
    UnitSelect::new("Width", width_at.bind(width), Bind::bound(unit), &units).sizing(Sizing::Content).show(ui).into()
}

#[story(UnitSelect<'static>, covers = [Mixed], drive = Drive::Never)]
fn unit_select_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let units = units();
    let ((width, unit), mixed_width_and_unit_at) = data.value_at("mixed_width_and_unit", (Mixed::<Option<f64>>::Mixed, UnitKind::Millimetres));
    UnitSelect::new("Width", mixed_width_and_unit_at.bind_mixed(width), mixed_width_and_unit_at.bind(unit), &units).show(ui).into()
}

#[story(UnitSelect<'static>, covers = [Invalid], drive = Drive::Never)]
fn unit_select_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let units = units();
    let refuse_negative = |width: &Option<f64>| match width {
        Some(width) if *width < 0.0 => Validity::Refuse(Message::new("Width cannot be negative")),
        _ => Validity::Ok,
    };
    let ((width, unit), negative_width_and_unit_at) = data.value_at("negative_width_and_unit", (Some(-5.0), UnitKind::Millimetres));
    UnitSelect::new("Width", negative_width_and_unit_at.bind(width).check(refuse_negative), negative_width_and_unit_at.bind(unit), &units)
        .show(ui)
        .into()
}

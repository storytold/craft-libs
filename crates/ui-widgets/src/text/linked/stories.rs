//! Stories of `LinkedFields`.

use super::{LinkMode, LinkedFields, LinkedLayout};
use crate::contract::{Bind, Props as _, Show as _, SideLabel, Validity};
use crate::runtime::env::{Message, UnitKind};
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::Mixed;
use craft_theme::ControlSize;
use egui::Ui;

#[story(LinkedFields<'static, 2>, covers = [Default], drive = Drive::Type("120"))]
fn linked_fields(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = *data.choice("size", ControlSize::Md, &[("Sm", ControlSize::Sm), ("Md", ControlSize::Md)]);
    let disabled = *data.value("disabled", false);
    let linked = *data.value("linked", true);
    LinkedFields::new("Size", data.bind("size", Mixed::each([Some(100.0), Some(50.0)])), Bind::given(linked))
        .captions(["W", "H"])
        .range(1.0..=1000.0)
        .size(size)
        .enabled(!disabled)
        .show(ui)
        .into()
}

#[story(LinkedFields<'static, 2>, covers = [], drive = Drive::Never)]
fn linked_fields_equal_columns(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let linked = *data.value("linked", true);
    LinkedFields::new("Margins", data.bind("margins", Mixed::each([Some(12.0); 2])), Bind::given(linked))
        .captions(["Inside", "Outside"])
        .mode(LinkMode::Equal)
        .layout(LinkedLayout::Columns)
        .show(ui)
        .into()
}

#[story(LinkedFields<'static, 2>, covers = [], drive = Drive::Never)]
fn linked_fields_percent(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let linked = *data.value("linked", true);
    LinkedFields::new("Scale", data.bind("scale", Mixed::each([Some(100.0); 2])), Bind::given(linked))
        .captions(["W", "H"])
        .range(1.0..=1000.0)
        .unit_kind(UnitKind::Percent)
        .show(ui)
        .into()
}

#[story(LinkedFields<'static, 2>, covers = [], drive = Drive::Never)]
fn linked_fields_link_caption(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let linked = *data.value("linked", true);
    LinkedFields::new("Margins", data.bind("link caption", Mixed::each([Some(12.0); 2])), Bind::given(linked))
        .captions(["Inside", "Outside"])
        .caption_with(1, SideLabel::new().link())
        .layout(LinkedLayout::Column)
        .show(ui)
        .into()
}

#[story(LinkedFields<'static, 2>, covers = [Mixed], drive = Drive::Never)]
fn linked_fields_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    LinkedFields::new("Size", data.bind_mixed("mixed", Mixed::<[Mixed<Option<f64>>; 2]>::Mixed), Bind::given(false)).show(ui).into()
}

#[story(LinkedFields<'static, 2>, covers = [], drive = Drive::Never)]
fn linked_fields_mixed_height(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let size = [Mixed::Same(Some(100.0)), Mixed::Mixed];
    LinkedFields::new("Size", data.bind("mixed height", size), Bind::given(false)).captions(["W", "H"]).show(ui).into()
}

#[story(LinkedFields<'static, 2>, covers = [Invalid], drive = Drive::Never)]
fn linked_fields_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let too_wide = |size: &[Mixed<Option<f64>>; 2]| match size[0] {
        Mixed::Same(Some(width)) if width > 500.0 => Validity::Refuse(Message::new("Too wide")),
        _ => Validity::Ok,
    };
    LinkedFields::new("Size", data.bind("wide", Mixed::each([Some(900.0), Some(50.0)])).check(too_wide), Bind::given(false)).show(ui).into()
}

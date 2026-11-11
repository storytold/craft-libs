//! Stories of `RadioGroup` and `Radio`.

use super::{Radio, RadioGroup};
use crate::contract::{Bind, FieldLayout, Props as _, Show as _, ShowContainer as _, SideLabel, Validity};
use crate::layout_components::{Row, Stack};
use crate::runtime::env::Message;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{IconId, Mixed};
use egui::{Response, Ui};

/// Members spread over nested layouts, with the group's body between them: one group, one value.
fn spread(ui: &mut Ui, group: RadioGroup<'_, u8>) -> (StoryOutput, Option<Response>) {
    let mut second = None;
    let shown = group.show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            Radio::new(1u8).label("Small").show(ui);
            Row::new().show(ui, |ui, _| {
                second = Some(Radio::new(2u8).label("Medium").show(ui));
                Radio::new(3u8).label("Large").show(ui);
            });
        });
    });
    (shown.into(), second)
}

fn pick_second(fixture: &mut Fixture) {
    if let Some(medium) = fixture.by_label("Medium") {
        fixture.click(&medium);
    }
}

#[story(RadioGroup<'static, u8>, covers = [Default], drive = Drive::Custom(pick_second))]
fn radio_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    spread(ui, RadioGroup::new("Size", Bind::bound(data.value("size", 1u8)))).0
}

#[story(RadioGroup<'static, u8>, covers = [Mixed], drive = Drive::Never)]
fn radio_group_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    spread(ui, RadioGroup::new("Size", Bind::bound_mixed(data.value("mixed", Mixed::Mixed)))).0
}

#[story(RadioGroup<'static, u8>, covers = [Invalid], drive = Drive::Never)]
fn radio_group_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_large = |size: &u8| if *size == 3 { Validity::Refuse(Message::new("Too large")) } else { Validity::Ok };
    spread(ui, RadioGroup::new("Size", Bind::bound(data.value("large", 3u8)).check(refuse_large))).0
}

/// Rows that carry an icon and a count through the body's label step, the label after the button.
#[story(RadioGroup<'static, u8>, covers = [], drive = Drive::Never)]
fn radio_group_with_icons_and_counts(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = [(1u8, "Layers", "list", "12"), (2, "Pages", "grid", "4"), (3, "Masks", "eye", "0")];
    RadioGroup::new("Show", Bind::bound(data.value("show", 1u8)))
        .show(ui, |ui, cx| {
            for (value, label, icon, count) in rows {
                let side = SideLabel::new().layout(FieldLayout::After).icon(IconId::new(icon)).detail(count);
                cx.labelled(ui, value, label.into(), side, Radio::new(value));
            }
        })
        .into()
}

/// Rows with a description under the label: the `SideLabel`'s detail joins the radio's accessible description.
#[story(RadioGroup<'static, u8>, covers = [], drive = Drive::Never)]
fn radio_group_with_descriptions(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows =
        [(1u8, "Fast", "Lower quality, smaller file"), (2, "Balanced", "The default for most photos"), (3, "Best", "Full quality, largest file")];
    RadioGroup::new("Export", Bind::bound(data.value("export", 2u8)))
        .show(ui, |ui, cx| {
            for (value, label, description) in rows {
                let side = SideLabel::new().layout(FieldLayout::After).detail(description);
                cx.labelled(ui, value, label.into(), side, Radio::new(value));
            }
        })
        .into()
}

// The shown member is the selected one, where Tab arrives, so the key drives reach it.
#[story(Radio<u8>, covers = [Default])]
fn radio(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    spread(ui, RadioGroup::new("Size", Bind::bound(data.value("size", 2u8)))).1.map(StoryOutput::from).unwrap_or_default()
}

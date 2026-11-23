//! Stories of `IconPicker`.

use super::{IconPicker, IconPickerGroup, IconPickerGroups, IconPickerItem};
use crate::buttons::Button;
use crate::contract::{Bind, Show as _, ShowContainer as _};
use crate::popover::Popover;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::IconId;
use egui::Ui;

fn groups() -> Vec<IconPickerGroup<u8>> {
    vec![
        IconPickerGroup::new(
            "Selection",
            vec![
                IconPickerItem::new(1, "Move").icon(IconId::new("move")),
                IconPickerItem::new(2, "Marquee").icon(IconId::new("marquee_rect")).keywords(vec!["select".to_owned()]),
            ],
        ),
        IconPickerGroup::new(
            "Drawing",
            vec![
                IconPickerItem::new(3, "Brush").icon(IconId::new("brush")),
                IconPickerItem::new(4, "Eraser").icon(IconId::new("eraser")),
                IconPickerItem::new(5, "Pen").icon(IconId::new("pencil")),
            ],
        ),
    ]
}

/// The picker as an app hosts it: a button opens it in a popover, and a pick closes it.
#[story(IconPicker<'static, u8>, covers = [], drive = Drive::Never)]
fn icon_picker_in_a_popover(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let groups = groups();
    let opener = Button::new("Choose an icon").show(ui);
    let (open, open_at) = data.value_at("open", false);
    let mut picked = None;
    let shown = Popover::new("Icons", open_at.bind(open)).opened_by(&opener).show(ui, |ui, _| {
        picked = IconPicker::new("Tool", Bind::bound(&mut None), &groups).show(ui).picked;
    });
    if picked.is_some() {
        *data.value("open", false) = false;
    }
    let chosen = data.note("chosen", None::<u8>);
    if picked.is_some() {
        *chosen = picked;
    }
    let name = chosen.and_then(|key| groups.iter().flat_map(|group| &group.items).find(|item| item.key == key).map(|item| item.label.clone()));
    let readout = name.map_or_else(|| "Nothing chosen".to_owned(), |name| format!("Chosen: {name}"));
    StoryOutput::from(shown.output).readout(readout)
}

#[story(IconPicker<'static, u8>, covers = [Default], drive = Drive::Never)]
fn icon_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    IconPicker::new("Tool", data.bind("tool", Some(3u8)), &groups()).show(ui).into()
}

/// The groups as a filter row: All first, then each group.
#[story(IconPicker<'static, u8>, covers = [], drive = Drive::Never)]
fn icon_picker_filter(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    IconPicker::new("Tool", data.bind("tool", Some(3u8)), &groups()).group_display(IconPickerGroups::Filter).show(ui).into()
}

/// The filter row with a search that matches nothing, and the app's own empty text.
#[story(IconPicker<'static, u8>, covers = [], drive = Drive::Type("zzz"))]
fn icon_picker_filter_no_match(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    IconPicker::new("Tool", data.bind("none", None), &groups())
        .group_display(IconPickerGroups::Filter)
        .empty("No icons in this category")
        .show(ui)
        .into()
}

#[story(IconPicker<'static, u8>, covers = [Empty], drive = Drive::Type("zzz"))]
fn icon_picker_no_match(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    IconPicker::new("Tool", data.bind("none", None), &groups()).show(ui).into()
}

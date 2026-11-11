//! Stories of `IconPicker`.

use super::{IconPicker, IconPickerGroup, IconPickerItem};
use crate::contract::Bind;
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

#[story(IconPicker<'static, u8>, covers = [Default], drive = Drive::Never)]
fn icon_picker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    IconPicker::new("Tool", Bind::bound(data.value("tool", Some(3u8))), &groups()).show(ui).into()
}

#[story(IconPicker<'static, u8>, covers = [Empty], drive = Drive::Type("zzz"))]
fn icon_picker_no_match(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    IconPicker::new("Tool", Bind::bound(data.value("none", None)), &groups()).show(ui).into()
}

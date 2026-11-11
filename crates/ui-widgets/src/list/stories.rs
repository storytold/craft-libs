//! Stories of the row types of the list family: `ListRow`.

use super::{ListRow, RowInk, TrailingFace, TrailingPart};
use crate::contract::Show as _;
use crate::primitives::IconSpec;
use crate::testing::{Drive, StoryData, StoryOutput, show_as_only_item, story};
use craft_behavior::{ColumnKey, IconId};
use egui::{Pos2, Rect, Ui};

#[story(ListRow<'static>, covers = [Default], drive = Drive::Activate)]
fn list_row(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ListRow::new("Background").show(ui).into()
}

#[story(ListRow<'static>, covers = [LongText], drive = Drive::Never)]
fn list_row_long_text(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let label = "A layer whose name is far longer than the room its row gives the label";
    ListRow::new(label).detail("A second line under it").show(ui).into()
}

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_tree_group(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_as_only_item(ui, ListRow::new("Group 1").depth(1).group(true).icon(IconSpec::svg(craft_behavior::IconId::new("eye")))).into()
}

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_clipped_dimmed(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ListRow::new("Shadow").clipped(true).ink(RowInk::Dimmed).italic(true).lines("Clipped to the layer below, hidden").show(ui).into()
}

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_text_cell(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    const NAME: ColumnKey = ColumnKey::new("name");
    const SIZE: ColumnKey = ColumnKey::new("size");
    let (left, width, height) = (ui.cursor().min.x, ui.available_width(), ui.spacing().interact_size.y);
    let split = left + width * 0.6;
    let cells = [
        (NAME, Rect::from_min_max(Pos2::new(left, 0.0), Pos2::new(split, height))),
        (SIZE, Rect::from_min_max(Pos2::new(split, 0.0), Pos2::new(left + width, height))),
    ];
    show_as_only_item(ui, ListRow::new("report.pdf").cells(&cells).cell(SIZE, "2.4 MB")).into()
}

fn trailing_row() -> ListRow<'static> {
    ListRow::new("Background copy")
        .trailing("100%")
        .trailing_icon(IconId::new("lock"), "Locked")
        .trailing_part(TrailingPart::new(0), TrailingFace::disclosure(false), "Effects")
        .optional("Multiply")
}

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_trailing_items(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_as_only_item(ui, trailing_row()).into()
}

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_trailing_items_narrow(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ui.set_max_width(ui.available_width() * 0.3);
    show_as_only_item(ui, trailing_row()).into()
}

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_clamped_description_with_rail(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let text = "Retouched the sky, then rebuilt the horizon from the second exposure and matched its grain and tone to the foreground";
    let row = ListRow::new("Version 3").lines(text).line_limit(3).lead_painter(|cx, rect| {
        cx.painter.circle_filled(rect.center(), rect.width() / 4.0, cx.ink());
    });
    show_as_only_item(ui, row).into()
}

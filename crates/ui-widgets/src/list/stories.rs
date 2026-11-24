//! Stories of the row types of the list family: `ListRow`.

use super::{CellStatus, HueTarget, ListRow, RowInk, TrailingFace, TrailingPart};
use crate::contract::Show as _;
use crate::primitives::{IconSpec, Span};
use crate::runtime::env::Env;
use crate::style::DataColor;
use crate::testing::{Drive, StoryData, StoryOutput, show_as_only_item, show_as_only_item_across, story};
use craft_behavior::{ColumnKey, IconId};
use craft_theme::variant::Tone;
use egui::{Pos2, Rangef, Rect, Ui};

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
    show_as_only_item_across(ui, ListRow::new("report.pdf").cells(&cells).cell(SIZE, "2.4 MB"), Rangef::new(left, left + width)).into()
}

/// A search result: the match in the label reads in the highlight look.
///
/// The excerpt neither starts nor ends with a neutral character such as "…": in a right-to-left paragraph a neutral
/// at the paragraph's edge next to left-to-right text takes the paragraph's direction (UAX #9 N1/N2), so it is shaped
/// apart from the letter beside it and the run's width changes by direction, which the suite's right-to-left case
/// rejects.
#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_match_highlight(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let spans = vec![Span::new("the light over the "), Span::new("harbour").highlight(), Span::new(" turned grey")];
    let joined: String = spans.iter().map(|span| span.text.as_ref()).collect();
    show_as_only_item(ui, ListRow::new(joined).label_spans(spans).detail("Page 4")).into()
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

/// A colour from the document's data: a bookmark's or a tag's own colour.
const DATA_RED: DataColor = DataColor::new(egui::Color32::from_rgb(0xd0, 0x30, 0x30));

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_icon_hue(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let row = ListRow::new("Chapter 2").icon(IconSpec::svg(IconId::new("bookmark"))).hue(DATA_RED, HueTarget::Icon);
    show_as_only_item(ui, row).into()
}

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_label_hue(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_as_only_item(ui, ListRow::new("Chapter 2").hue(DATA_RED, HueTarget::Label)).into()
}

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_status_cell(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    const NAME: ColumnKey = ColumnKey::new("name");
    const WARNINGS: ColumnKey = ColumnKey::new("warnings");
    let (left, width, height) = (ui.cursor().min.x, ui.available_width(), ui.spacing().interact_size.y);
    let split = left + width * 0.4;
    let mut cells = [
        (NAME, Rect::from_min_max(Pos2::new(left, 0.0), Pos2::new(split, height))),
        (WARNINGS, Rect::from_min_max(Pos2::new(split, 0.0), Pos2::new(left + width, height))),
    ];
    // The columns are screen rects: under right-to-left the name column is at the right.
    if Env::of(ui).view().direction().is_rtl() {
        for (_, cell) in &mut cells {
            *cell =
                Rect::from_min_max(Pos2::new(2.0 * left + width - cell.max.x, cell.min.y), Pos2::new(2.0 * left + width - cell.min.x, cell.max.y));
        }
    }
    let status = CellStatus::new(Tone::Warn, "2 warnings")
        .tip("The file has form fields that will be flattened.\nIts bookmarks point to pages that are not kept.")
        .action("Details");
    // The row spans its columns, as a list's row does: the reflection of its cells is the reflection in the row.
    show_as_only_item_across(ui, ListRow::new("report.pdf").cells(&cells).cell_status(WARNINGS, status), Rangef::new(left, left + width)).into()
}

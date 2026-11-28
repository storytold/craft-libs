//! Stories of the row types of the list family: `ListRow`.

use super::{CellStatus, HueTarget, LabelUnderline, ListRow, RowInk, RowRun, ThumbRows, ThumbSize, TrailingFace, TrailingPart};
use crate::contract::{SetPosition, Show as _, ShowContainer as _, show_item_in_flow};
use crate::layout_components::Stack;
use crate::primitives::{ContentPainterCx, IconSpec, Span, StrokeWidth};
use crate::runtime::env::{Env, with_env};
use crate::style::{Corners, DataColor, SurfaceKind};
use crate::testing::{Drive, StoryData, StoryOutput, show_as_only_item, show_as_only_item_across, story};
use craft_behavior::{ColumnKey, IconId};
use craft_theme::variant::Tone;
use egui::{Color32, Pos2, Rangef, Rect, Ui};

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

/// Shows the rows `rows` builds one under the other, as the items of one set, and reports the last one's output.
fn show_rows(ui: &mut Ui, rows: impl Fn() -> Vec<ListRow<'static>>) -> StoryOutput {
    let mut shown = StoryOutput::default();
    Stack::new().show(ui, |ui, _| {
        let rows = rows();
        let count = rows.len();
        for (at, row) in rows.into_iter().enumerate() {
            shown = show_item_in_flow(ui, row, SetPosition::new(at).of(count)).into();
        }
    });
    shown
}

fn thumb(cx: &ContentPainterCx, rect: Rect) {
    cx.surface(rect, SurfaceKind::Raised, Corners::None);
}

const STRIPE: DataColor = DataColor::new(Color32::from_rgb(0x2e, 0x7d, 0xd1));

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_label_emphasis(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_rows(ui, || {
        vec![
            ListRow::new("Italic").italic(true),
            ListRow::new("Solid underline").underline(LabelUnderline::Solid),
            ListRow::new("Dashed underline").underline(LabelUnderline::Dashed),
            ListRow::new("Italic and dashed").italic(true).underline(LabelUnderline::Dashed),
            ListRow::new("A dashed label far longer than the room its row gives it").underline(LabelUnderline::Dashed),
        ]
    })
}

/// The stripe in the two widths, the current row's corner mark, a stripe on a nested row. The suite shows it in
/// right-to-left too.
#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_stripe_and_corner_mark(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_rows(ui, || {
        vec![
            ListRow::new("Thick").stripe(STRIPE, StrokeWidth::Thick),
            ListRow::new("Heavy").stripe(STRIPE, StrokeWidth::Heavy),
            ListRow::new("Heavy with corner mark").stripe(STRIPE, StrokeWidth::Heavy).corner_mark(true),
            ListRow::new("Nested").depth(1).stripe(STRIPE, StrokeWidth::Heavy),
        ]
    })
}

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_runs(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_rows(ui, || {
        vec![
            ListRow::new("Alone").run(RowRun::Alone),
            ListRow::new("First").run(RowRun::First),
            ListRow::new("Middle").run(RowRun::Middle),
            ListRow::new("Last").run(RowRun::Last),
        ]
    })
}

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_thumbnails_only(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let row =
        ListRow::new("Layer mask").thumbnails_only().thumb(ThumbSize::Medium, thumb).extra_thumb(thumb).thumb_link(true, "Link the mask to {row}");
    row.show(ui).into()
}

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_thumb_height(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let icon = || IconSpec::svg(IconId::new("eye"));
    show_rows(ui, || {
        vec![
            ListRow::new("Small").thumb_height(ThumbSize::Small).icon(icon()),
            ListRow::new("Medium").thumb_height(ThumbSize::Medium).icon(icon()),
            ListRow::new("Large").thumb_height(ThumbSize::Large).icon(icon()),
        ]
    })
}

/// Thumbnail rows of a list under `ThumbRows::Tight` (VectorCraft's Layers panel): the thumbnail with a tight gap, no
/// taller than a control, and a nested row indented by a small icon and a tight gap.
#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_tight_thumb_rows(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    with_env(
        ui,
        |overrides| {
            overrides.values.set(ThumbRows::Tight);
        },
        |ui| {
            show_rows(ui, || {
                vec![
                    ListRow::new("Layer 1").depth(0).thumb(ThumbSize::Medium, thumb),
                    ListRow::new("Ellipse").depth(1).thumb(ThumbSize::Medium, thumb),
                ]
            })
        },
    )
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

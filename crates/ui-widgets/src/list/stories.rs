//! Stories of the row types of the list family: `ListRow`.

use super::{LabelUnderline, ListRow, RowInk, RowRun, ThumbSize, TrailingFace, TrailingPart};
use crate::contract::Show as _;
use crate::primitives::{ContentPainterCx, IconSpec, Span, StrokeWidth};
use crate::style::{Corners, DataColor, SurfaceKind};
use crate::testing::{Drive, StoryData, StoryOutput, show_as_only_item, story};
use craft_behavior::{ColumnKey, IconId};
use egui::{Color32, Pos2, Rect, Ui};

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

/// Shows `rows` one under the other and reports the last one's output.
fn show_rows(ui: &mut Ui, rows: Vec<ListRow<'static>>) -> StoryOutput {
    let mut shown = StoryOutput::default();
    for row in rows {
        shown = row.show(ui).into();
    }
    shown
}

fn thumb(cx: &ContentPainterCx, rect: Rect) {
    cx.surface(rect, SurfaceKind::Raised, Corners::None);
}

const STRIPE: DataColor = DataColor::new(Color32::from_rgb(0x2e, 0x7d, 0xd1));

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_label_emphasis(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_rows(
        ui,
        vec![
            ListRow::new("Italic").italic(true),
            ListRow::new("Solid underline").underline(LabelUnderline::Solid),
            ListRow::new("Dashed underline").underline(LabelUnderline::Dashed),
            ListRow::new("Italic and dashed").italic(true).underline(LabelUnderline::Dashed),
            ListRow::new("A dashed label far longer than the room its row gives it").underline(LabelUnderline::Dashed),
        ],
    )
}

/// The stripe in the two widths, the current row's corner mark, a stripe on a nested row. The suite shows it in
/// right-to-left too.
#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_stripe_and_corner_mark(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_rows(
        ui,
        vec![
            ListRow::new("Thick").stripe(STRIPE, StrokeWidth::Thick),
            ListRow::new("Heavy").stripe(STRIPE, StrokeWidth::Heavy),
            ListRow::new("Heavy with corner mark").stripe(STRIPE, StrokeWidth::Heavy).corner_mark(true),
            ListRow::new("Nested").depth(1).stripe(STRIPE, StrokeWidth::Heavy),
        ],
    )
}

#[story(ListRow<'static>, covers = [], drive = Drive::Never)]
fn list_row_runs(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_rows(
        ui,
        vec![
            ListRow::new("Alone").run(RowRun::Alone),
            ListRow::new("First").run(RowRun::First),
            ListRow::new("Middle").run(RowRun::Middle),
            ListRow::new("Last").run(RowRun::Last),
        ],
    )
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
    show_rows(
        ui,
        vec![
            ListRow::new("Small").thumb_height(ThumbSize::Small).icon(icon()),
            ListRow::new("Medium").thumb_height(ThumbSize::Medium).icon(icon()),
            ListRow::new("Large").thumb_height(ThumbSize::Large).icon(icon()),
        ],
    )
}

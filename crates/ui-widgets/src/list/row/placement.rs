//! Where a row's parts lie: its sizes, bands and the placement of label, cells and trailing items.

use super::ListRow;
use super::show::named;
use super::style::ListRowStyle;
use super::thumb::{Thumb, ThumbSize};
use super::trailing::{TrailingItem, trailing_run};
use crate::contract::{ColumnAlign, PartsCx, WordSetCx as _};
use crate::list::toggle::ToggleState;
use crate::list::words::ListWords;
use crate::primitives::{HAlign, TextRun, Wrap};
use crate::runtime::env::EnvView;
use crate::style::{StyleCx, TextLook};
use craft_behavior::{ColumnKey, Flow};
use craft_theme::TypeRole;
use egui::{Pos2, Rect, Vec2};

/// The label's least width, in control heights, before the indent and the optional items give way.
const MIN_LABEL_HEIGHTS: f32 = 3.0;

/// The sizes a row takes from its control size and the tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Sizes {
    pub(super) row: f32,
    pub(super) icon: f32,
    pad: f32,
    pub(super) gap: f32,
    trailing_pad: f32,
    /// The width of a hosted field.
    field: f32,
    pub(super) min_label: f32,
    thumbs: [f32; 3],
    thumb_pad: f32,
    /// The least height of a row with a thumbnail: `size.thumb_row`.
    thumb_row: f32,
    pub(super) tight: f32,
    /// The room `space_above` adds: `space.gap.row`.
    pub(super) gap_row: f32,
    pub(super) label_line: f32,
    pub(super) detail_line: f32,
    /// The room a text cell keeps free at each side of its text.
    pub(super) cell_inset: f32,
    /// The vertical room around a wrapped cell's text, in total.
    pub(super) cell_pad: f32,
}

impl Sizes {
    pub(super) fn of(cx: &StyleCx<'_>, role: TypeRole) -> Sizes {
        let (control, tokens) = (cx.control(), cx.tokens);
        let thumbs = &tokens.size.thumb;
        Sizes {
            row: control.height,
            icon: control.icon,
            pad: control.pad_x,
            gap: control.gap,
            trailing_pad: tokens.space.pad.regular,
            field: tokens.size.length.short,
            min_label: MIN_LABEL_HEIGHTS * control.height,
            thumbs: [thumbs.sm, thumbs.md, thumbs.lg],
            thumb_pad: tokens.space.gap.stack,
            thumb_row: tokens.size.thumb_row,
            tight: tokens.space.gap.tight,
            gap_row: tokens.space.gap.row,
            label_line: tokens.typography.get(role).line_height,
            detail_line: tokens.typography.get(TypeRole::Caption).line_height,
            cell_inset: control.pad_x,
            cell_pad: control.gap,
        }
    }

    /// The width a text cell of `cell` width lays its text out in.
    pub(super) fn cell_room(&self, cell: f32) -> f32 {
        (cell - 2.0 * self.cell_inset).max(0.0)
    }

    fn side(&self, thumb: ThumbSize) -> f32 {
        thumb.pick(self.thumbs)
    }

    /// The height of a row with a thumbnail of `thumb` and, if `detail`, a detail line: the control height, the
    /// thumbnail with its padding (at least `size.thumb_row`), or the two lines of text, whichever is tallest.
    pub(super) fn height(&self, thumb: ThumbSize, detail: bool) -> f32 {
        let base = match self.side(thumb) {
            0.0 => self.row,
            side => (side + self.thumb_pad).max(self.thumb_row),
        };
        if detail { base.max(self.label_line + self.detail_line + self.gap) } else { base }
    }
}

pub(super) fn halign(align: ColumnAlign) -> HAlign {
    match align {
        ColumnAlign::Start => HAlign::Start,
        ColumnAlign::End => HAlign::End,
    }
}

impl ListRow<'_> {
    /// The height of a row with a thumbnail of `thumb` and, if `detail`, a detail line, for the item layout of a
    /// list of such rows.
    pub fn row_height(cx: &StyleCx<'_>, thumb: ThumbSize, detail: bool) -> f32 {
        Sizes::of(cx, TypeRole::Body).height(thumb, detail)
    }
}

/// The cells of a row that lead and trail its label band.
struct Bands {
    lead: f32,
    trail: f32,
    /// Index into `toggles`, the cell's rect, whether it leads.
    toggles: Vec<(usize, Rect, bool)>,
    /// Index into `texts`, the column's position in the row's cells, the cell's rect, whether it leads.
    texts: Vec<(usize, usize, Rect, bool)>,
}

/// Where the parts of a row lie in its rect.
pub(super) struct Placed {
    pub(super) toggles: Vec<(usize, Rect)>,
    /// The text cells by index into `texts`, with the column's position in the row's cells.
    pub(super) texts: Vec<(usize, usize, Rect)>,
    pub(super) dividers: Vec<f32>,
    pub(super) disclosure: Option<Rect>,
    pub(super) marker: Option<Rect>,
    pub(super) icon: Option<Rect>,
    pub(super) first: Option<Rect>,
    pub(super) extras: Vec<(Rect, Option<Rect>)>,
    pub(super) label: Rect,
    pub(super) items: Vec<Option<Rect>>,
    pub(super) min_width: f32,
}

impl ListRow<'_> {
    /// Whether the detail takes a second line: a heading shows it at the end of the label's line.
    pub(super) fn detail_line(&self) -> bool {
        self.detail.is_some() && !self.heading
    }

    /// Whether the leading part holds thumbnails; without a first thumbnail or a thumbnail side none are placed.
    fn shows_thumbs(&self, sizes: &Sizes) -> bool {
        self.first.is_some() && sizes.side(self.thumb) > 0.0
    }

    /// The width of the leading part: every thumbnail and what is between them, or the icon; `None` for a row with
    /// neither.
    fn leading(&self, sizes: &Sizes) -> Option<f32> {
        let side = sizes.side(self.thumb);
        if self.shows_thumbs(sizes) {
            let first = self.thumb_aspect.of(side);
            let between = |extra: &Thumb<'_>| if extra.link.is_some() { sizes.icon } else { sizes.gap };
            Some(first + self.extras.iter().map(|extra| first + between(extra)).sum::<f32>())
        } else {
            (self.icon.is_some() || self.lead_paint.is_some()).then_some(sizes.icon)
        }
    }

    /// Takes the toggle columns from the facts the collection recorded for this row.
    pub(super) fn adopt_toggle_columns(&mut self, env: &EnvView<'_>, id: egui::Id) {
        if let Some(facts) = env.facts_of(id) {
            self.toggle_columns.clone_from(&facts.toggles);
        }
    }

    /// Hands the collection the names of this row's toggle cells, so its cell nodes read them.
    pub(super) fn record_toggle_names(&self, cx: &PartsCx<'_, Self>) {
        let words = cx.words::<ListWords>();
        let label = cx.resolved.label;
        let names = self
            .toggles
            .iter()
            .map(|toggle| (toggle.column.clone(), named(toggle.name_pattern(&words), label), toggle.state == ToggleState::Mixed))
            .collect();
        crate::contract::put_toggle_names(cx.ui.ctx(), cx.resolved.id, names);
    }

    /// How the wrapped lines fit: all of them, or cut after `line_limit`.
    pub(super) fn lines_wrap(&self) -> Wrap {
        self.line_limit.map_or(Wrap::Wrap, Wrap::Clamp)
    }

    /// The thumbnail width and height of this row.
    pub(super) fn thumb_size(&self, sizes: &Sizes) -> Vec2 {
        let side = sizes.side(self.thumb);
        Vec2::new(self.thumb_aspect.of(side), side)
    }

    /// The height of the progress line, a caption line at the bottom of the label band; 0 without progress.
    pub(super) fn progress_line(&self, sizes: &Sizes) -> f32 {
        if self.progress.is_some() { sizes.detail_line } else { 0.0 }
    }

    /// The part of the label band the progress bar takes: its bottom line.
    pub(super) fn progress_band(&self, label: Rect, sizes: &Sizes) -> Rect {
        Rect::from_x_y_ranges(label.x_range(), (label.bottom() - self.progress_line(sizes))..=label.bottom())
    }

    /// The label band above the progress line, and the top of the label, detail and wrapped lines centred in it
    /// together, for wrapped lines `lines` high with the gap above them.
    pub(super) fn text_block(&self, label: Rect, sizes: &Sizes, lines: f32) -> (Rect, f32) {
        let band = Rect::from_x_y_ranges(label.x_range(), label.top()..=label.bottom() - self.progress_line(sizes));
        let detail = if self.detail_line() { sizes.detail_line } else { 0.0 };
        let block = sizes.label_line + detail + lines;
        (band, band.top() + ((band.height() - block) / 2.0).max(0.0))
    }

    /// The levels of indent: a leaf inside a group indents one more level, past its parent's disclosure column.
    fn steps(&self) -> usize {
        match (self.depth, self.group) {
            (Some(depth), None) if depth > 0 => depth + 1,
            (Some(depth), _) => depth,
            (None, _) => 0,
        }
    }

    /// The widths of the trailing items: as `text_width` measures a text, the icon side for the others.
    pub(super) fn item_widths(&self, sizes: &Sizes, text_width: &dyn Fn(&TextRun<'_>) -> f32) -> Vec<(f32, bool)> {
        let width = |item: &TrailingItem| match item {
            TrailingItem::Text(text) => text_width(&trailing_run(text)),
            TrailingItem::Icon { .. } | TrailingItem::Part { .. } => sizes.icon,
            TrailingItem::Hosted(_) => sizes.field,
        };
        self.trailing.iter().map(|trailing| (width(&trailing.item), trailing.optional)).collect()
    }

    /// The indent a row asks for before it gives way to the label's least width.
    pub(super) fn indent_full(&self, sizes: &Sizes) -> f32 {
        (self.steps() + usize::from(self.clipped)) as f32 * sizes.icon
    }

    /// The column no text or toggle cell claims, which holds the label.
    pub(super) fn label_column<'c>(&self, cells_of: &'c [(ColumnKey, Rect)]) -> Option<&'c ColumnKey> {
        let claimed = |key: &ColumnKey| {
            self.toggle_columns.contains(key)
                || self.toggles.iter().any(|toggle| toggle.column == *key)
                || self.texts.iter().any(|text| text.column == *key)
        };
        cells_of.iter().map(|(key, _)| key).find(|key| !claimed(key))
    }

    /// The offsets from the reading start of the row of the label column's near edge and of what lies past its
    /// far edge, and the toggle cells and text cells with whether each leads. A cell leads when it displays before
    /// the label column. A row without columns keeps the whole width for the label band; a row whose columns are all
    /// claimed has an empty label band and every cell leads.
    fn bands(&self, cells_of: &[(ColumnKey, Rect)], rect: Rect, rtl: bool) -> Bands {
        let start = if rtl { rect.right() } else { rect.left() };
        let span = |cell: Rect| {
            let (a, b) = (if rtl { start - cell.left() } else { cell.left() - start }, if rtl { start - cell.right() } else { cell.right() - start });
            (a.min(b), a.max(b))
        };
        let label_at = self.label_column(cells_of).and_then(|label| cells_of.iter().position(|(key, _)| key == label));
        let (lead, trail) = match label_at {
            Some(at) => {
                let (near, far) = span(cells_of[at].1);
                (near, (rect.width() - far).max(0.0))
            }
            None if cells_of.is_empty() => (0.0, 0.0),
            None => {
                let end = cells_of.iter().map(|(_, cell)| span(*cell).1).fold(0.0, f32::max);
                (end, (rect.width() - end).max(0.0))
            }
        };
        let leads = |position: usize| label_at.is_none_or(|at| position < at);
        let mut toggles = Vec::new();
        for (at, toggle) in self.toggles.iter().enumerate() {
            let Some(position) = cells_of.iter().position(|(column, _)| *column == toggle.column) else { continue };
            toggles.push((at, cells_of[position].1, leads(position)));
        }
        let mut texts = Vec::new();
        for (at, text) in self.texts.iter().enumerate() {
            let Some(position) = cells_of.iter().position(|(column, _)| *column == text.column) else { continue };
            texts.push((at, position, cells_of[position].1, leads(position)));
        }
        Bands { lead, trail, toggles, texts }
    }

    /// The alignment of a column's text and whether its figures are tabular.
    pub(super) fn align_of(&self, column: &ColumnKey) -> (HAlign, bool) {
        self.aligns.iter().find(|(key, ..)| key == column).map_or((HAlign::Start, false), |(_, align, numeric)| (*align, *numeric))
    }

    /// The look of the text cell of `column`.
    pub(super) fn cell_look<'s>(&self, style: &'s ListRowStyle, column: &ColumnKey) -> &'s TextLook {
        if self.muted.contains(column) { &style.cell_muted } else { &style.cell }
    }

    pub(super) fn place(&self, sizes: &Sizes, items: &[(f32, bool)], rect: Rect, rtl: bool) -> Placed {
        if !self.space_above {
            return self.place_in(&self.cells, sizes, items, rect, rtl);
        }
        let below = |rect: Rect| Rect::from_min_max(Pos2::new(rect.left(), (rect.top() + sizes.gap_row).min(rect.bottom())), rect.max);
        let cells: Vec<_> = self.cells.iter().map(|(column, rect)| (column.clone(), below(*rect))).collect();
        self.place_in(&cells, sizes, items, below(rect), rtl)
    }

    pub(super) fn place_in(&self, cells_of: &[(ColumnKey, Rect)], sizes: &Sizes, items: &[(f32, bool)], rect: Rect, rtl: bool) -> Placed {
        let Bands { lead, trail, toggles: cells, texts } = self.bands(cells_of, rect, rtl);
        let flow = Flow::new(rect, rtl);
        let band_width = (rect.width() - lead - trail).max(0.0);
        let band = flow.span(lead, band_width);
        let inner = Flow::new(band, rtl);
        let leading = self.leading(sizes);
        let leading_width = leading.map_or(0.0, |side| side + sizes.gap);
        let disclosure = if self.group.is_some() { sizes.icon } else { 0.0 };
        let marker = if self.clipped { sizes.icon } else { 0.0 };
        let column = disclosure + marker;
        let base = sizes.pad;
        let indent_full = self.indent_full(sizes);
        let end = band_width - sizes.trailing_pad;
        let required: f32 = items.iter().filter(|(_, optional)| !optional).map(|(width, _)| width + sizes.gap).sum();
        let mut used = required;
        let mut kept: Vec<bool> = items.iter().map(|(_, optional)| !optional).collect();
        for (at, (width, optional)) in items.iter().enumerate() {
            if *optional && end - (used + width + sizes.gap) - (base + indent_full + column + leading_width) >= sizes.min_label {
                kept[at] = true;
                used += width + sizes.gap;
            }
        }
        let label_end = end - used;
        let overflow = (base + column + leading_width + sizes.min_label - label_end).max(0.0);
        let label_end = label_end + overflow;
        let indent = indent_full.min((label_end - base - column - leading_width - sizes.min_label).max(0.0));
        let mut from_end = 0.0;
        let items = items
            .iter()
            .zip(&kept)
            .map(|((width, _), kept)| {
                kept.then(|| {
                    let start = end - from_end - width + overflow;
                    from_end += width + sizes.gap;
                    inner.span(start, *width)
                })
            })
            .collect();
        let column_start = base + indent + if self.leading_first { leading_width } else { 0.0 };
        let leading_start = base + indent + if self.leading_first { 0.0 } else { column };
        let label_start = base + indent + column + leading_width;
        let disclosure_rect = (disclosure > 0.0).then(|| inner.span(column_start, disclosure));
        let marker_rect = (marker > 0.0).then(|| inner.span(column_start + disclosure, marker));
        let mut at = leading_start;
        let (mut icon, mut first, mut extras) = (None, None, Vec::new());
        match leading {
            Some(_) if self.shows_thumbs(sizes) => {
                let size = self.thumb_size(sizes);
                let place = |x: f32, width: f32| Rect::from_center_size(inner.span(x, width).center(), Vec2::new(width, size.y));
                first = Some(place(at, size.x));
                at += size.x;
                for extra in &self.extras {
                    let link = extra.link.is_some().then(|| inner.span(at, sizes.icon));
                    at += if link.is_some() { sizes.icon } else { sizes.gap };
                    extras.push((place(at, size.x), link));
                    at += size.x;
                }
            }
            Some(width) => icon = Some(inner.span(at, width)),
            None => {}
        }
        let toggles = cells.iter().map(|(at, cell, _)| (*at, *cell)).collect();
        let texts = texts.iter().map(|(at, position, cell, _)| (*at, *position, *cell)).collect();
        let dividers = cells.iter().filter(|(_, _, leading)| *leading).map(|(_, cell, _)| if rtl { cell.left() } else { cell.right() }).collect();
        let min_width = lead + trail + base + column + leading_width + sizes.min_label + required + sizes.trailing_pad;
        Placed {
            toggles,
            texts,
            dividers,
            disclosure: disclosure_rect,
            marker: marker_rect,
            icon,
            first,
            extras,
            label: inner.span(label_start, (label_end - label_start).max(0.0)),
            items,
            min_width,
        }
    }
}

/// The width of a character of a trailing text under fixed sizes.
#[cfg(feature = "testing")]
const FIXED_ADVANCE: f32 = 10.0;

/// Where the label, the trailing items and the disclosure of `row` lie in a rect of `width` under fixed sizes, where
/// each character of a trailing text is `FIXED_ADVANCE` wide: the geometry trials read it.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn placement_under_fixed_sizes(row: &ListRow<'_>, width: f32, rtl: bool) -> RowPlacement {
    let sizes = Sizes {
        row: 20.0,
        icon: 16.0,
        pad: 6.0,
        gap: 4.0,
        trailing_pad: 8.0,
        field: 64.0,
        min_label: 60.0,
        thumbs: [24.0, 32.0, 48.0],
        thumb_pad: 4.0,
        thumb_row: 28.0,
        tight: 2.0,
        gap_row: 0.0,
        label_line: 16.0,
        detail_line: 12.0,
        cell_inset: 6.0,
        cell_pad: 4.0,
    };
    let items = row.item_widths(&sizes, &|run| FIXED_ADVANCE * run.full_text().chars().count() as f32);
    let placed = row.place(&sizes, &items, Rect::from_min_size(Pos2::ZERO, Vec2::new(width, sizes.row)), rtl);
    RowPlacement { label: placed.label, items: placed.items, disclosure: placed.disclosure, min_label: sizes.min_label }
}

/// The indent steps of `row`.
#[cfg(feature = "testing")]
#[doc(hidden)]
pub fn indent_steps(row: &ListRow<'_>) -> usize {
    row.steps()
}

/// A row's parts placed by [`placement_under_fixed_sizes`].
#[cfg(feature = "testing")]
#[non_exhaustive]
pub struct RowPlacement {
    /// The label and detail.
    pub label: Rect,
    /// The trailing items, `None` for one that dropped out.
    pub items: Vec<Option<Rect>>,
    /// The disclosure column of a group row.
    pub disclosure: Option<Rect>,
    /// The label's least width under the fixed sizes.
    pub min_label: f32,
}

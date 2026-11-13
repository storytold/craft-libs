//! Where a row's parts lie: its sizes, bands and the placement of label, cells and trailing items.

use super::ListRow;
use super::show::named;
use super::style::ListRowStyle;
use super::thumb::{Thumb, ThumbSize};
use super::trailing::{TrailingFace, TrailingItem, trailing_run};
use crate::contract::{ColumnAlign, PartsCx, RowHeight, WordSetCx as _};
use crate::layout::{ShrinkOrder, ShrinkUnit, shrink_widths};
use crate::list::toggle::ToggleState;
use crate::list::words::ListWords;
use crate::primitives::{HAlign, TextRun, Wrap};
use crate::runtime::env::{EnvView, Label};
use crate::style::{StyleCx, TextLook};
use craft_behavior::{ColumnKey, Flow};
use craft_theme::{ControlSize, TypeRole};
use egui::{Pos2, Rect, Vec2};

/// The label's least width, in control heights, before the indent and the optional items give way.
const MIN_LABEL_HEIGHTS: f32 = 3.0;

/// The sizes a row takes from its control size and the tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Sizes {
    pub(super) row: f32,
    /// The slot of an icon, a toggle cell's mark or a step of indent.
    pub(super) icon: f32,
    /// The side the row's icon and its toggle marks draw at in their slots: the control's icon side, the small
    /// control's in a sub-row.
    pub(super) mark: f32,
    pad: f32,
    pub(super) gap: f32,
    trailing_pad: f32,
    pub(super) min_label: f32,
    thumbs: [f32; 3],
    thumb_pad: f32,
    /// The least height of a row with a thumbnail.
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
    pub(super) fn of(cx: &StyleCx<'_>, role: TypeRole, rows: RowHeight, sub_row: bool) -> Sizes {
        let (control, tokens) = (cx.control(), cx.tokens);
        let thumbs = &tokens.size.thumb;
        let small = cx.control_of(ControlSize::Sm);
        Sizes {
            row: if sub_row { small.height } else { rows.of(cx) },
            icon: control.icon,
            mark: if sub_row { small.icon } else { control.icon },
            pad: control.pad_x,
            gap: control.gap,
            trailing_pad: tokens.space.pad.regular,
            min_label: MIN_LABEL_HEIGHTS * control.height,
            thumbs: [thumbs.sm, thumbs.md, thumbs.lg],
            thumb_pad: tokens.space.gap.stack,
            // A standard control and the panel padding, as the old `thumb_row_min`.
            thumb_row: tokens.size.control.md + tokens.space.pad.regular,
            tight: tokens.space.gap.tight,
            gap_row: tokens.space.gap.row,
            label_line: tokens.typography.get(role).line_height,
            detail_line: tokens.typography.get(TypeRole::Caption).line_height,
            cell_inset: control.pad_x,
            // A text row's wrapped cell keeps the room a text row has around its line.
            cell_pad: if rows == RowHeight::Text { tokens.space.gap.tight } else { control.gap },
        }
    }

    /// The width a text cell of `cell` width lays its text out in.
    pub(super) fn cell_room(&self, cell: f32) -> f32 {
        (cell - 2.0 * self.cell_inset).max(0.0)
    }

    fn side(&self, thumb: ThumbSize) -> f32 {
        thumb.pick(self.thumbs)
    }

    /// The height of a row with a thumbnail of `thumb` and, if `detail`, a detail line: the row height, the thumbnail
    /// with its padding (at least `thumb_row`), or the two lines of text, whichever is tallest.
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
    /// The height of a control-high row with a thumbnail of `thumb` and, if `detail`, a detail line, for the item
    /// layout of a list of such rows.
    pub fn row_height(cx: &StyleCx<'_>, thumb: ThumbSize, detail: bool) -> f32 {
        Sizes::of(cx, TypeRole::Body, RowHeight::Control, false).height(thumb, detail)
    }

    /// Where this row's disclosure lies when it fills `rect` under `style`, with texts `text_width` wide: the rect at
    /// which a host that owns the disclosure part (`disclosure_by_host`) declares it. `None` for a row without one.
    pub fn disclosure_at(&self, style: &ListRowStyle, rect: Rect, rtl: bool, text_width: &dyn Fn(&TextRun<'_>) -> f32) -> Option<Rect> {
        let widths = self.widths(&style.sizes, text_width);
        self.place(&style.sizes, &widths, rect, rtl).disclosure
    }
}

/// A trailing item's width where the row has room, the least it narrows to, and whether it drops out for room.
#[derive(Clone, Copy)]
pub(super) struct ItemWidth {
    pub(super) width: f32,
    pub(super) least: f32,
    pub(super) optional: bool,
}

/// The measured widths a row is placed by.
pub(super) struct Widths {
    /// The trailing items' widths.
    pub(super) items: Vec<ItemWidth>,
    /// The leading text's slot.
    pub(super) lead: Option<f32>,
}

/// The run of a leading text, without its colour.
pub(super) fn lead_run(text: &Label, role: TypeRole) -> TextRun<'_> {
    TextRun::new(text.as_str()).role(role)
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
    /// The mark column of an option of a value list.
    pub(super) check: Option<Rect>,
    pub(super) icon: Option<Rect>,
    pub(super) first: Option<Rect>,
    pub(super) extras: Vec<(Rect, Option<Rect>)>,
    pub(super) label: Rect,
    pub(super) items: Vec<Option<Rect>>,
    /// The row's least width: the label at its least, the required items at theirs.
    pub(super) min_width: f32,
    /// The row's least width with every trailing item, optional ones included, at its width where the row has room.
    pub(super) ideal_width: f32,
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

    /// The width of the leading part: every thumbnail and what is between them, the leading text's slot, or the icon;
    /// `None` for a row with none of them.
    fn leading(&self, sizes: &Sizes, widths: &Widths) -> Option<f32> {
        let side = sizes.side(self.thumb);
        if self.shows_thumbs(sizes) {
            let first = self.thumb_aspect.of(side);
            let between = |extra: &Thumb<'_>| if extra.link.is_some() { sizes.icon } else { sizes.gap };
            Some(first + self.extras.iter().map(|extra| first + between(extra)).sum::<f32>())
        } else if let Some(lead) = widths.lead {
            Some(lead)
        } else {
            (self.icon.is_some() || self.lead_paint.is_some()).then_some(sizes.icon)
        }
    }

    /// Takes the toggle columns, their keys, the columns' alignments and the press the collection holds on the row from
    /// the facts it recorded for this row: an end-aligned column's text ends at the cell's end (mirrored in a
    /// right-to-left interface) and a numeric column's figures are tabular.
    pub(super) fn adopt_facts(&mut self, env: &EnvView<'_>, id: egui::Id) {
        if let Some(facts) = env.facts_of(id) {
            self.toggle_columns.clone_from(&facts.toggles);
            self.toggle_keys.clone_from(&facts.toggle_keys);
            self.aligns = facts.aligns.iter().map(|(key, align, tabular)| (key.clone(), halign(*align), *tabular)).collect();
            if self.pressed.is_none() {
                self.pressed.clone_from(&facts.pressed);
            }
            self.menu_open |= facts.menu_open;
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

    /// The height the line of components under the text takes with the gap above it; 0 without one.
    pub(super) fn below_height(&self, sizes: &Sizes) -> f32 {
        self.below.iter().filter_map(|below| below.size).map(|size| sizes.gap + size.ideal.y).fold(0.0, f32::max)
    }

    /// The width of the line of components under the text, with the gaps between them.
    pub(super) fn below_width(&self, sizes: &Sizes) -> f32 {
        let widths: Vec<f32> = self.below.iter().filter_map(|below| below.size).map(|size| size.ideal.x).collect();
        widths.iter().sum::<f32>() + sizes.gap * widths.len().saturating_sub(1) as f32
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
        // A sub-row's indent comes before its toggle cells (`sub_steps`); its label follows them.
        if self.sub_row {
            return 0;
        }
        self.depth.map_or(0, |depth| tree_steps(depth, self.group.is_some()))
    }

    /// The widths of the trailing items and the leading text's slot: as `text_width` measures a text, the icon side for
    /// icons and parts, and a hosted component's measured width.
    pub(super) fn widths(&self, sizes: &Sizes, text_width: &dyn Fn(&TextRun<'_>) -> f32) -> Widths {
        let width = |item: &TrailingItem| match item {
            TrailingItem::Text { text, strong, .. } => text_width(&trailing_run(text, *strong)),
            TrailingItem::Part { face: TrailingFace::Strong(text), .. } => text_width(&trailing_run(text, true)),
            TrailingItem::Icon { .. } | TrailingItem::Part { .. } => sizes.icon,
            TrailingItem::Hosted(hosted) => hosted.size.map_or(0.0, |size| size.ideal.x),
        };
        let least = |item: &TrailingItem, width: f32| match item {
            TrailingItem::Hosted(hosted) => hosted.size.map_or(width, |size| size.least.min(width)),
            _ => width,
        };
        let items = self
            .trailing
            .iter()
            .map(|trailing| {
                let width = width(&trailing.item);
                ItemWidth { width, least: least(&trailing.item, width), optional: trailing.optional }
            })
            .collect();
        let lead = self.lead_text.as_ref().map(|(_, widest)| text_width(&lead_run(widest, self.lead_role)));
        Widths { items, lead }
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

    /// The positions in `cells_of` of the label column and of the first and last column the label band spans: its
    /// own and every toggle column between it and the nearest cell the row fills on each side. A row lays out only
    /// the toggle columns it fills, as a heading row without toggles starts at the row's start; a row `in_label_column`
    /// spans none before its label column.
    pub(super) fn label_span(&self, cells_of: &[(ColumnKey, Rect)]) -> Option<(usize, usize, usize)> {
        let label = self.label_column(cells_of)?;
        let at = cells_of.iter().position(|(key, _)| key == label)?;
        let open = |position: &usize| {
            let key = &cells_of[*position].0;
            self.toggle_columns.contains(key) && !self.toggles.iter().any(|toggle| toggle.column == *key)
        };
        let first = if self.in_label_column { at } else { (0..at).rev().take_while(open).last().unwrap_or(at) };
        let last = (at + 1..cells_of.len()).take_while(open).last().unwrap_or(at);
        Some((at, first, last))
    }

    /// The offsets from the reading start of the row of the label band's near edge and of what lies past its far
    /// edge (the band spans the columns of `label_span`), and the toggle cells and text cells with whether each
    /// leads. A cell leads when it displays before the label column. A row without columns keeps the whole width for
    /// the label band; a row whose columns are all claimed (a list of toggle columns only) has its label band past
    /// the last column, and every cell leads.
    fn bands(&self, cells_of: &[(ColumnKey, Rect)], rect: Rect, rtl: bool) -> Bands {
        let start = if rtl { rect.right() } else { rect.left() };
        let span = |cell: Rect| {
            let (a, b) = (if rtl { start - cell.left() } else { cell.left() - start }, if rtl { start - cell.right() } else { cell.right() - start });
            (a.min(b), a.max(b))
        };
        let label = self.label_span(cells_of);
        let label_at = label.map(|(at, ..)| at);
        let (lead, trail) = match label {
            Some((_, first, last)) => {
                let spans = cells_of[first..=last].iter().map(|(_, cell)| span(*cell));
                let (near, far) = spans.fold((f32::INFINITY, f32::NEG_INFINITY), |(near, far), (a, b)| (near.min(a), far.max(b)));
                (near, (rect.width() - far).max(0.0))
            }
            None => (cells_of.iter().map(|(_, cell)| span(*cell).1).fold(0.0, f32::max), 0.0),
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

    pub(super) fn place(&self, sizes: &Sizes, widths: &Widths, rect: Rect, rtl: bool) -> Placed {
        if !self.space_above {
            return self.place_in(&self.cells, sizes, widths, rect, rtl);
        }
        let below = |rect: Rect| Rect::from_min_max(Pos2::new(rect.left(), (rect.top() + sizes.gap_row).min(rect.bottom())), rect.max);
        let cells: Vec<_> = self.cells.iter().map(|(column, rect)| (column.clone(), below(*rect))).collect();
        self.place_in(&cells, sizes, widths, below(rect), rtl)
    }

    pub(super) fn place_in(&self, columns: &[(ColumnKey, Rect)], sizes: &Sizes, widths: &Widths, rect: Rect, rtl: bool) -> Placed {
        let items = &widths.items;
        let moved;
        let cells_of = if self.sub_row {
            let toggle = |key: &ColumnKey| self.toggle_columns.contains(key) || self.toggles.iter().any(|toggle| toggle.column == *key);
            let mut cells = columns.to_vec();
            craft_behavior::sub_row_cells(&mut cells, toggle, self.sub_steps as f32 * sizes.icon, rtl);
            moved = cells;
            &moved[..]
        } else {
            columns
        };
        let Bands { lead, trail, toggles: cells, texts } = self.bands(cells_of, rect, rtl);
        let flow = Flow::new(rect, rtl);
        let band_width = (rect.width() - lead - trail).max(0.0);
        let band = flow.span(lead, band_width);
        let inner = Flow::new(band, rtl);
        let leading = self.leading(sizes, widths);
        let leading_width = leading.map_or(0.0, |side| side + sizes.gap);
        let disclosure = if self.group.is_some() { sizes.icon } else { 0.0 };
        let marker = if self.clipped { sizes.icon } else { 0.0 };
        let check = if self.check.is_some() { sizes.icon + sizes.gap } else { 0.0 };
        let column = disclosure + marker + check;
        let base = sizes.pad;
        let indent_full = self.indent_full(sizes);
        let end = band_width - sizes.trailing_pad;
        let required: f32 = items.iter().filter(|item| !item.optional).map(|item| item.width + sizes.gap).sum();
        let mut used = required;
        let mut kept: Vec<bool> = items.iter().map(|item| !item.optional).collect();
        for (at, item) in items.iter().enumerate() {
            if item.optional && end - (used + item.width + sizes.gap) - (base + indent_full + column + leading_width) >= sizes.min_label {
                kept[at] = true;
                used += item.width + sizes.gap;
            }
        }
        // A row that lacks room for the label's least width narrows its kept items and its indent by the shrink rule of
        // every row: each toward its least (the indent's is none) by the same share of what it can give.
        let units: Vec<ShrinkUnit> = items
            .iter()
            .zip(&kept)
            .filter(|(_, kept)| **kept)
            .map(|(item, _)| ShrinkUnit { ideal: item.width, least: item.least })
            .chain(std::iter::once(ShrinkUnit { ideal: indent_full, least: 0.0 }))
            .collect();
        let gaps = kept.iter().filter(|kept| **kept).count() as f32 * sizes.gap;
        let mut given = shrink_widths(&units, base + column + leading_width + sizes.min_label + gaps, end, ShrinkOrder::Even);
        let indent_given = given.pop().unwrap_or(0.0);
        let used = gaps + given.iter().sum::<f32>();
        let label_end = end - used;
        // A row without columns keeps its label's least width and runs past its end, where the collection clips it; a
        // label column's cell is the label's hard end, so it elides there instead of running into the next column.
        let in_cell = self.label_span(cells_of).is_some();
        let overflow = if in_cell { 0.0 } else { (base + column + leading_width + sizes.min_label - label_end).max(0.0) };
        let label_end = label_end + overflow;
        let indent = indent_given.min((label_end - base - column - leading_width - sizes.min_label).max(0.0));
        let mut from_end = 0.0;
        let mut given = given.into_iter();
        let items = kept
            .iter()
            .map(|kept| {
                kept.then(|| {
                    let width = given.next().unwrap_or(0.0);
                    let start = end - from_end - width + overflow;
                    from_end += width + sizes.gap;
                    inner.span(start, width)
                })
            })
            .collect();
        let column_start = base + indent + if self.leading_first { leading_width } else { 0.0 };
        let leading_start = base + indent + if self.leading_first { 0.0 } else { column };
        let label_start = base + indent + column + leading_width;
        let disclosure_rect = (disclosure > 0.0).then(|| inner.span(column_start, disclosure));
        let marker_rect = (marker > 0.0).then(|| inner.span(column_start + disclosure, marker));
        let check_rect = (check > 0.0).then(|| inner.span(column_start + disclosure + marker, sizes.icon));
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
        // A column's line stays at the column's own edge where a sub-row's toggle cell moved out of it.
        let dividers = cells
            .iter()
            .filter(|(_, _, leading)| *leading)
            .filter_map(|(at, ..)| columns.iter().find(|(column, _)| *column == self.toggles[*at].column))
            .map(|(_, cell)| if rtl { cell.left() } else { cell.right() })
            .collect();
        let least: f32 = widths.items.iter().filter(|item| !item.optional).map(|item| item.least + sizes.gap).sum();
        let min_width = lead + trail + base + column + leading_width + sizes.min_label + least + sizes.trailing_pad;
        // Every item, optional ones included: at its ideal width the row shows everything.
        let all: f32 = widths.items.iter().map(|item| item.width + sizes.gap).sum();
        let ideal_width = min_width - least + all;
        Placed {
            toggles,
            texts,
            dividers,
            disclosure: disclosure_rect,
            marker: marker_rect,
            check: check_rect,
            icon,
            first,
            extras,
            label: inner.span(label_start, (label_end - label_start).max(0.0)),
            items,
            min_width,
            ideal_width,
        }
    }
}

/// The indent steps of a tree row at `depth`: a leaf inside a group indents one more level, past its parent's
/// disclosure column. A source that indents a row's sub-rows (`ItemSource::sub_row`) counts from it.
pub fn tree_steps(depth: usize, group: bool) -> usize {
    if depth > 0 && !group { depth + 1 } else { depth }
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
        mark: 16.0,
        pad: 6.0,
        gap: 4.0,
        trailing_pad: 8.0,
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
    let widths = row.widths(&sizes, &|run| FIXED_ADVANCE * run.full_text().chars().count() as f32);
    let placed = row.place(&sizes, &widths, Rect::from_min_size(Pos2::ZERO, Vec2::new(width, sizes.row)), rtl);
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

//! Painting a row's trailing items, toggles, thumbnails, text and cells.

use super::placement::{Placed, StatusMeasure, action_link};
use super::show::{LINK, STATUS_ACTION, TEXT_CELL, TOGGLE};
use super::style::{ListRowStyle, RowForm, row_ink_of};
use super::thumb::{RowThumb, target_at};
use super::trailing::{TrailingFace, TrailingItem, cell_key, field_key, trailing_run};
use super::{LabelUnderline, ListRow, RowRun};
use crate::buttons::IconButton;
use crate::choice::Checkbox;
use crate::contract::{Bind, Cx, PaintUi, Pointer as _, Props as _, Role, Styled as _, item_part_open};
use crate::controls::disclosure::Disclosure;
use crate::list::toggle::{ToggleFace, ToggleState};
use crate::notices::tone_icon;
use crate::primitives::{DocText, Draw, IconSpec, PaintShape, RectSide, RuleKind, Span, StrokeWidth, TextRun, Wrap};
use crate::style::{Flags, GlyphRole, Ink, StateMix, StateSet, StyleColor};
use craft_behavior::{Flow, IconId, Mixed, PartRef};
use craft_theme::ControlSize;
use egui::{CornerRadius, Pos2, Rect, Vec2};

impl ListRow<'_> {
    /// Whether the trailing part `key` shows: a `revealed` one only on a hovered or `selected` row, while it has focus
    /// or while the menu it opens is open. A part that does not show is neither painted nor exposed.
    pub(super) fn part_shown(cx: &Cx<'_, Self>, key: PartRef, revealed: bool, selected: bool, menu_open: bool) -> bool {
        !revealed
            || cx.response.hovered()
            || selected
            || menu_open
            || cx.part_state(key).flags.contains(Flags::FOCUSED)
            || item_part_open(&cx.ctx, cx.id, key)
    }

    pub(super) fn paint_trailing(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed) {
        let side = Vec2::splat(cx.style.sizes.icon);
        let selected = cx.resolved.selected == Some(true) || cx.env.facts_of(cx.id).is_some_and(|facts| facts.flags.contains(Flags::SELECTED));
        let drags = &self.drags;
        let handle = |part: &super::TrailingPart| drags.iter().any(|(spot, _)| *spot == super::RowDragSpot::Trailing(*part));
        for (at, (trailing, slot)) in self.trailing.iter_mut().zip(&placed.items).enumerate() {
            let Some(slot) = slot else { continue };
            let shape = Rect::from_center_size(slot.center(), side);
            match &mut trailing.item {
                TrailingItem::Text { text, strong, .. } => {
                    ui.draw().text(*slot, &trailing_run(text, *strong));
                }
                TrailingItem::Icon { icon, .. } => {
                    let mut icon = icon.clone();
                    icon.color.get_or_insert(cx.resolve(&cx.style.trailing_icon.color));
                    ui.draw().icon(shape, &icon);
                }
                TrailingItem::Hosted(hosted) => {
                    if let Some(item) = hosted.item.take() {
                        let height = hosted.size.map_or(slot.height(), |size| size.ideal.y.min(slot.height()));
                        item.show(ui, field_key(at), Rect::from_center_size(slot.center(), Vec2::new(slot.width(), height)));
                    }
                }
                TrailingItem::Part { part, face, enabled, revealed, .. } => {
                    let key = part.part();
                    if !Self::part_shown(cx, key, *revealed, selected, self.menu_open) {
                        continue;
                    }
                    match face {
                        // A drag handle is no button (`access` names it in the row's description): its icon is a mark.
                        TrailingFace::Icon(icon) if handle(part) => {
                            let mut icon = icon.clone();
                            icon.color.get_or_insert(cx.resolve(&cx.style.trailing_icon.color));
                            ui.draw().icon(shape, &icon);
                        }
                        TrailingFace::Icon(icon) => {
                            ui.face(key, IconButton::new(icon.clone()).focusable(false).enabled(*enabled).glyphs(GlyphRole::PanelRail))
                        }
                        TrailingFace::Disclosure { open } => ui.face(key, Disclosure::new(Bind::given(*open)).node(false).enabled(*enabled)),
                        TrailingFace::Strong(text) => {
                            ui.draw().text(*slot, &trailing_run(text, true));
                        }
                    }
                }
            }
        }
    }

    pub(super) fn paint_toggles(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed) {
        let icon = cx.style.sizes.mark;
        for (at, cell) in &placed.toggles {
            let toggle = &self.toggles[*at];
            let part = TOGGLE.at(*at as u64);
            if let Some(fill) = toggle.fill {
                ui.draw().data_fill(&PaintShape::Rect(*cell, CornerRadius::ZERO), fill);
            }
            match &toggle.face {
                ToggleFace::Check => {
                    let value = match toggle.state {
                        ToggleState::Mixed => Mixed::Mixed,
                        state => Mixed::Same(state == ToggleState::On),
                    };
                    ui.face(part, Checkbox::new(Bind::given(value)).size(ControlSize::Sm));
                }
                ToggleFace::Icons { on, off } => {
                    let shown = match toggle.state {
                        ToggleState::On | ToggleState::Mixed => Some(on),
                        ToggleState::Off if toggle.shows_off(cx.part(part).hovered && !self.toggle_closed(toggle)) => Some(off),
                        ToggleState::Off => None,
                    };
                    if let Some(id) = shown {
                        let spec = IconSpec::svg(id.clone());
                        let spec = if toggle.fill.is_some() {
                            spec.ink(Ink::OnDark)
                        } else {
                            spec.color(glyph_ink(cx, part, toggle.state == ToggleState::On))
                        };
                        ui.draw().icon(Rect::from_center_size(cell.center(), Vec2::splat(icon)), &spec);
                    }
                }
            }
        }
    }

    pub(super) fn paint_thumbs(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed, ink: StyleColor) {
        let style = cx.style;
        let frame = style.thumb_frame_width;
        let outset = style.sizes.tight;
        let arm = style.sizes.gap;
        let rects: Vec<Rect> = placed.first.into_iter().chain(placed.extras.iter().map(|(thumb, _)| *thumb)).collect();
        let mut paints = self.first.take().into_iter().map(Some).chain(self.extras.iter_mut().map(|extra| extra.paint.take()));
        for (at, thumb) in rects.iter().enumerate() {
            if let Some(Some(paint)) = paints.next() {
                let _ = ui.child(("thumb", at), thumb.shrink(frame), RowThumb::new(paint));
            }
            // The frame is the slot's square edge, and the picture fills the slot inside it.
            ui.draw().stroke(&PaintShape::Rect(*thumb, CornerRadius::ZERO), StrokeWidth::Thin, style.thumb_frame);
        }
        // Adjacent targets are one target together (a frame and its content): one set of brackets frames them all.
        let mut at = 0;
        while at < rects.len() {
            if !self.targets.contains(target_at(at)) {
                at += 1;
                continue;
            }
            let mut framed = rects[at];
            while at + 1 < rects.len() && self.targets.contains(target_at(at + 1)) {
                at += 1;
                framed = framed.union(rects[at]);
            }
            at += 1;
            let r = framed.expand(outset);
            let draw = ui.draw();
            for (corner, dx, dy) in
                [(r.left_top(), 1.0, 1.0), (r.right_top(), -1.0, 1.0), (r.right_bottom(), -1.0, -1.0), (r.left_bottom(), 1.0, -1.0)]
            {
                let path = vec![corner + Vec2::new(arm * dx, 0.0), corner, corner + Vec2::new(0.0, arm * dy)];
                draw.stroke(&PaintShape::Path(path), StrokeWidth::Medium, ink);
            }
        }
        for (at, (_, link)) in placed.extras.iter().enumerate() {
            let (Some(slot), Some((linked, _))) = (link, &self.extras[at].link) else { continue };
            let id = IconId::new(if *linked { "link" } else { "unlink" });
            let side = Rect::from_center_size(slot.center(), Vec2::splat(style.sizes.icon));
            let color = glyph_ink(cx, LINK.at(at as u64), *linked);
            ui.draw().icon(side, &IconSpec::svg(id).color(color));
        }
    }

    /// Paints the stripe at the start of the content, the corner mark in the trailing top corner (in `ink`) and the
    /// outline of the row's run.
    pub(super) fn paint_marks(&self, draw: &Draw<'_>, cx: &Cx<'_, Self>, placed: &Placed, ink: StyleColor) {
        let rtl = cx.env.direction().is_rtl();
        let rect = cx.rect;
        if let Some((color, width)) = self.stripe {
            let bar = Flow::new(placed.band, rtl).span(0.0, draw.width(width));
            draw.data_fill(&PaintShape::Rect(bar, CornerRadius::ZERO), color);
        }
        if self.corner_mark {
            let side = cx.style.sizes.handle;
            let (edge, inward) = if rtl { (rect.left(), 1.0) } else { (rect.right(), -1.0) };
            let corner = vec![Pos2::new(edge, rect.top()), Pos2::new(edge + inward * side, rect.top()), Pos2::new(edge, rect.top() + side)];
            draw.fill(&PaintShape::Polygon(corner), ink);
        }
        if let Some(run) = self.run {
            // PLAN: `ListRowStyle` (style.rs) holds no outline radius, so the run's ends are square, not `radius.control`.
            let sides: &[RectSide] = match run {
                RowRun::Alone => &[RectSide::Top, RectSide::Bottom, RectSide::Left, RectSide::Right],
                RowRun::First => &[RectSide::Top, RectSide::Left, RectSide::Right],
                RowRun::Middle => &[RectSide::Left, RectSide::Right],
                RowRun::Last => &[RectSide::Bottom, RectSide::Left, RectSide::Right],
            };
            for side in sides {
                draw.edge_rule(RuleKind::Divider, rect, *side);
            }
        }
    }

    /// Paints the label, detail and wrapped lines in the label band; returns the bottom of the text, where the component
    /// under it (`below`) starts after a gap. `hue` is the label's ink when a colour from data inks it; `document` is
    /// the height of the label laid out as document text, for a row of `document_rows`.
    pub(super) fn paint_text(&self, draw: &Draw<'_>, cx: &Cx<'_, Self>, label: Rect, hue: Option<StyleColor>, document: Option<f32>) -> f32 {
        let style = cx.style;
        let sizes = &style.sizes;
        let rtl = cx.env.direction().is_rtl();
        let wrapped = self.lines.as_ref().map(|lines| {
            let run = lines.run().role(style.detail.role).color(cx.resolve(&style.detail.color)).wrap(self.lines_wrap());
            draw.layout(&run, self.text_block(label, sizes, 0.0).0.width())
        });
        let lines = wrapped.as_ref().map_or(0.0, |layout| sizes.gap + layout.size().y);
        let (label, mut top) = self.text_block(label, sizes, lines + self.below_height(sizes));
        // A heading's detail sits at the end of the label's line and the title is cut to what it leaves.
        let beside = match (&self.detail, self.heading) {
            (Some(detail), true) => {
                let run = TextRun::new(detail.as_str()).role(style.detail.role).color(cx.resolve(&style.detail.color)).elide();
                let shown = draw.layout(&run, label.width());
                let x = if rtl { label.left() } else { label.right() - shown.size().x };
                draw.galley(Pos2::new(x, top + (sizes.label_line - shown.size().y) / 2.0), &shown);
                shown.size().x + sizes.gap
            }
            _ => 0.0,
        };
        if !self.renaming && !self.label_has_field() {
            let muted = self.label_column(&self.cells).is_some_and(|column| self.muted.contains(column));
            let look = if muted { &style.cell_muted } else { &style.text };
            let ink = hue.unwrap_or_else(|| cx.resolve(&look.color));
            let width = (label.width() - beside).max(0.0);
            match (self.document_run(cx.resolved.label, ink.egui()), document, &self.document) {
                (Some(run), Some(height), Some(rows)) => {
                    // The document line is centred on the label's line, which is the band's middle in a row of a label
                    // alone; measure made the row tall enough for it.
                    let runs = [run];
                    let text = DocText { align: rows.align, ..DocText::new(&runs).elide() };
                    draw.doc(Rect::from_min_size(Pos2::new(label.left(), top + (sizes.label_line - height) / 2.0), Vec2::new(width, height)), &text);
                }
                _ => {
                    let run = self.label_run(cx.resolved.label).role(look.role).color(ink).elide();
                    let room = Rect::from_min_size(Pos2::new(label.left(), top), Vec2::new(width, sizes.label_line));
                    if self.underline == LabelUnderline::Dashed {
                        // The line runs under the shown (elided) text only.
                        let shown = draw.layout(&run, room.width());
                        let x = draw.text_x(room, &run, &shown);
                        draw.galley(Pos2::new(x, room.top()), &shown);
                        let y = room.top() + shown.size().y;
                        draw.dashed(&[Pos2::new(x, y), Pos2::new(x + shown.size().x, y)], draw.dashes(), StrokeWidth::Thin, ink);
                    } else {
                        draw.text(room, &run);
                    }
                }
            }
        }
        top += sizes.label_line;
        if let Some(detail) = self.detail.as_ref().filter(|_| !self.heading) {
            let run = TextRun::new(detail.as_str()).role(style.detail.role).color(cx.resolve(&style.detail.color)).elide();
            draw.text(Rect::from_min_size(Pos2::new(label.left(), top), Vec2::new(label.width(), sizes.detail_line)), &run);
            top += sizes.detail_line;
        }
        if let Some(layout) = wrapped {
            let x = if rtl { label.right() - layout.size().x } else { label.left() };
            draw.galley(Pos2::new(x, top + sizes.gap), &layout);
        }
        top + lines
    }
}

impl ListRow<'_> {
    /// The label as a run without its role and colour: its styled spans (`label_spans`) when it has them, else `label`
    /// as one span, every span italic under `italic` and underlined under a solid `underline`.
    pub(super) fn label_run<'s>(&'s self, label: &'s str) -> TextRun<'s> {
        let mut spans: Vec<Span<'s>> = if self.label_spans.is_empty() { vec![Span::new(label)] } else { self.label_spans.clone() };
        let underline = self.underline == LabelUnderline::Solid;
        for span in &mut spans {
            span.italic |= self.italic;
            span.underline |= underline;
        }
        TextRun::spans(spans)
    }

    /// The text cells, inset and centred on the row's height, each aligned as its column says.
    /// The text cell `at` as a run in its column's look, without its colour.
    pub(super) fn cell_run(&self, style: &ListRowStyle, at: usize) -> TextRun<'_> {
        let text = &self.texts[at];
        let (align, numeric) = self.align_of(&text.column);
        let mut span = Span::new(text.text.as_str());
        span.tabular = numeric;
        let run = TextRun::spans(vec![span]).role(self.cell_look(style, &text.column).role).align(align);
        if text.wrap { run.wrap(Wrap::Wrap) } else { run.elide() }
    }

    /// Whether the text cell `at` is a selection part: wrapped text in a column `cell_selectable` names.
    pub(super) fn cell_selects(&self, at: usize) -> bool {
        let text = &self.texts[at];
        text.wrap && self.selectable.contains(&text.column)
    }

    pub(super) fn paint_cells(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed) {
        let sizes = &cx.style.sizes;
        let rtl = cx.env.direction().is_rtl();
        for (at, position, cell) in &placed.texts {
            let text = &self.texts[*at];
            let field = self.cell_fields.iter().any(|(column, _)| *column == text.column);
            if field && !self.field_suffixes.contains(&text.column) {
                continue;
            }
            let look = self.cell_look(cx.style, &text.column);
            let cell = Rect::from_x_y_ranges(cell.x_range(), cx.rect.y_range());
            let mut room = if field && self.label_column(&self.cells) == Some(&text.column) {
                placed.label
            } else {
                Flow::new(cell, rtl).span(sizes.cell_inset, sizes.cell_room(cell.width()))
            };
            let run = self.cell_run(cx.style, *at).color(cx.resolve(&look.color));
            if field {
                let width = ui.draw().layout(&run, f32::INFINITY).size().x;
                room = split_field_text(room, rtl, width, sizes.gap).1;
            }
            let layout = ui.draw().layout(&run, room.width());
            let top =
                if text.wrap { room.top() + ((room.height() - layout.size().y) / 2.0).max(0.0) } else { room.center().y - sizes.label_line / 2.0 };
            let x = ui.draw().text_x(room, &run, &layout);
            if self.cell_selects(*at) {
                ui.selectable_text(TEXT_CELL.at(*position as u64), Rect::from_min_size(Pos2::new(x, top), layout.size()), &run);
            } else {
                ui.draw().galley(Pos2::new(x, top), &layout);
            }
        }
    }

    /// The status cells, centred on the row's height: the tone icon and the text in the tone's ink (the selection's
    /// paired ink on a selected row), then the action link in the link look of its own part's state.
    pub(super) fn paint_statuses(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed) {
        let sizes = &cx.style.sizes;
        let rtl = cx.env.direction().is_rtl();
        let role = cx.style.cell.role;
        let form = RowForm::of(self.option, self.command);
        for (at, _, cell) in &placed.statuses {
            let status = &self.statuses[*at].status;
            let ink = cx.resolve(&ui.with_style(cx.resolved.variant, |scx| row_ink_of(scx, scx.tone(status.tone), form)));
            let cell = Rect::from_x_y_ranges(cell.x_range(), cx.rect.y_range());
            let part = STATUS_ACTION.at(*at as u64);
            let action = {
                let draw = ui.draw();
                // The action's slot is its part's rect, which `parts` declared from the link's measured width.
                let slots = self.status_slots(
                    *at,
                    cell,
                    sizes,
                    rtl,
                    role,
                    StatusMeasure { text_width: &|run| draw.layout(run, f32::INFINITY).size().x, action_width: &|_| cx.part(part).rect.width() },
                );
                if let (Some(slot), Some(icon)) = (slots.icon, tone_icon(status.tone)) {
                    draw.icon(Rect::from_center_size(slot.center(), Vec2::splat(sizes.mark)), &IconSpec::svg(IconId::new(icon)).color(ink));
                }
                draw.text(slots.text, &TextRun::new(status.text.as_str()).role(role).color(ink).elide());
                slots.action.and(status.action.as_ref())
            };
            if let Some(action) = action {
                ui.face(part, action_link(action, role));
            }
        }
    }
}

impl ListRow<'_> {
    /// Whether a cell field stands in the label's place.
    pub(super) fn label_has_field(&self) -> bool {
        self.label_column(&self.cells).is_some_and(|label| self.cell_fields.iter().any(|(column, _)| column == label))
    }

    /// Shows each cell field in its column's cell, or in the label's room in the label's column, at its own height up to
    /// the row's, centred on the row's band. A link is as wide as its text, at the room's start, as a text cell's text
    /// is: a press past it acts on the row.
    pub(super) fn paint_cell_fields(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed) {
        let sizes = &cx.style.sizes;
        let rtl = cx.env.direction().is_rtl();
        let label = self.label_column(&self.cells).cloned();
        let band = placed.label.y_range();
        let suffixes: Vec<_> = self
            .texts
            .iter()
            .filter(|text| self.field_suffixes.contains(&text.column))
            .map(|text| {
                let run = TextRun::new(text.text.as_str()).role(cx.style.cell.role);
                (text.column.clone(), ui.draw().layout(&run, f32::INFINITY).size().x)
            })
            .collect();
        for (at, (column, field)) in self.cell_fields.iter_mut().enumerate() {
            let (Some(item), Some(size)) = (field.item.take(), field.size) else { continue };
            let link = item.role() == Role::LINK;
            let room = if label.as_ref() == Some(column) {
                placed.label
            } else {
                let Some((_, cell)) = self.cells.iter().find(|(key, _)| key == column) else { continue };
                let cell = Rect::from_x_y_ranges(cell.x_range(), band);
                Flow::new(cell, rtl).span(sizes.cell_inset, sizes.cell_room(cell.width()))
            };
            let room = suffixes.iter().find(|(key, _)| key == column).map_or(room, |(_, width)| split_field_text(room, rtl, *width, sizes.gap).0);
            let height = size.ideal.y.min(room.height());
            let width = if link { size.ideal.x.min(room.width()) } else { room.width() };
            let slot = Flow::new(room, rtl).take_start(width).0;
            item.show(ui, cell_key(at), Rect::from_center_size(Pos2::new(slot.center().x, room.center().y), Vec2::new(width, height)));
        }
    }
}

/// Divides a cell's inset room into a field and its trailing text, in reading order.
fn split_field_text(room: Rect, rtl: bool, text_width: f32, gap: f32) -> (Rect, Rect) {
    let text = text_width.min(room.width());
    let gap = gap.min((room.width() - text).max(0.0));
    let field = (room.width() - text - gap).max(0.0);
    let flow = Flow::new(room, rtl);
    (flow.span(0.0, field), flow.span(field + gap, text))
}

/// The ink of a toggle or link glyph that is `on` or not: its own part's hover and press, over the row's selection and
/// lead.
fn glyph_ink(cx: &Cx<'_, ListRow<'_>>, part: PartRef, on: bool) -> StyleColor {
    let (row, cell) = (cx.state(), cx.part_state(part));
    let look = if on { &cx.style.toggle } else { &cx.style.toggle_off };
    StateSet::new(cell.state, row.flags, StateMix::new(cell.t.hover, cell.t.press, row.t.on)).resolve(&look.color)
}

/// The down-right clipping mark: a stem down from the top, then a leg toward the reading end with an arrowhead.
pub(super) fn draw_clip_arrow(draw: &Draw<'_>, slot: Rect, rtl: bool, ink: StyleColor) {
    let side = slot.width().min(slot.height());
    let square = Rect::from_center_size(slot.center(), Vec2::splat(side));
    let at = |x: f32, y: f32| Pos2::new(if rtl { square.right() - x * side } else { square.left() + x * side }, square.top() + y * side);
    draw.stroke(&PaintShape::Path(vec![at(0.2, 0.2), at(0.2, 0.7), at(0.8, 0.7)]), StrokeWidth::Medium, ink);
    draw.stroke(&PaintShape::Path(vec![at(0.6, 0.45), at(0.85, 0.7), at(0.6, 0.95)]), StrokeWidth::Medium, ink);
}

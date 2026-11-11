//! Painting a row's trailing items, toggles, thumbnails, text and cells.

use super::ListRow;
use super::placement::Placed;
use super::show::{TOGGLE, TRAILING};
use super::thumb::{RowThumb, target_at};
use super::trailing::{TrailingFace, TrailingItem, trailing_run};
use crate::buttons::IconButton;
use crate::choice::Checkbox;
use crate::contract::{Bind, Cx, PaintUi, Pointer as _, Props as _, Styled as _};
use crate::controls::disclosure::Disclosure;
use crate::list::toggle::{ToggleFace, ToggleState};
use crate::primitives::{Draw, HAlign, IconSpec, PaintShape, Span, StrokeWidth, TextRun, Wrap};
use crate::style::{Ink, StyleColor};
use craft_behavior::{Flow, IconId, Mixed};
use craft_theme::ControlSize;
use egui::{CornerRadius, Pos2, Rect, Vec2};

impl ListRow<'_> {
    pub(super) fn paint_trailing(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed) {
        let side = Vec2::splat(cx.style.sizes.icon);
        for (at, (trailing, slot)) in self.trailing.iter_mut().zip(&placed.items).enumerate() {
            let Some(slot) = slot else { continue };
            let shape = Rect::from_center_size(slot.center(), side);
            match &mut trailing.item {
                TrailingItem::Text(text) => {
                    ui.draw().text(*slot, &trailing_run(text));
                }
                TrailingItem::Icon { icon, .. } => {
                    let mut icon = icon.clone();
                    icon.color.get_or_insert(cx.resolve(&cx.style.trailing_icon.color));
                    ui.draw().icon(shape, &icon);
                }
                TrailingItem::Hosted(show) => {
                    if let Some(show) = show.take() {
                        show(ui, at, Rect::from_center_size(slot.center(), Vec2::new(slot.width(), cx.style.sizes.row)));
                    }
                }
                TrailingItem::Part { part, face, enabled, .. } => {
                    let key = TRAILING.at(u64::from(part.0));
                    match face {
                        TrailingFace::Icon(icon) => ui.face(key, IconButton::new(icon.clone()).focusable(false).enabled(*enabled)),
                        TrailingFace::Disclosure { open } => ui.face(key, Disclosure::new(Bind::given(*open)).node(false).enabled(*enabled)),
                    }
                }
            }
        }
    }

    pub(super) fn paint_toggles(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed) {
        let icon = cx.style.sizes.icon;
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
                        ToggleState::Off if toggle.always_visible || cx.part(part).hovered => Some(off),
                        ToggleState::Off => None,
                    };
                    if let Some(id) = shown {
                        let ink = if toggle.fill.is_some() { Ink::OnDark } else { Ink::OnPanel };
                        ui.draw().icon(Rect::from_center_size(cell.center(), Vec2::splat(icon)), &IconSpec::svg(id.clone()).ink(ink));
                    }
                }
            }
        }
    }

    pub(super) fn paint_thumbs(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, placed: &Placed, ink: StyleColor) {
        let style = cx.style;
        let outset = style.sizes.tight;
        let arm = style.sizes.gap;
        let rects: Vec<Rect> = placed.first.into_iter().chain(placed.extras.iter().map(|(thumb, _)| *thumb)).collect();
        let mut paints = self.first.take().into_iter().map(Some).chain(self.extras.iter_mut().map(|extra| extra.paint.take()));
        for (at, thumb) in rects.iter().enumerate() {
            if let Some(Some(paint)) = paints.next() {
                let _ = ui.child(("thumb", at), *thumb, RowThumb::new(paint));
            }
            let draw = ui.draw();
            let shape = PaintShape::Rect(*thumb, style.thumb_radius);
            draw.stroke(&shape, StrokeWidth::Thin, style.thumb_edge);
            let target = target_at(at);
            if self.target == Some(target) {
                let r = thumb.expand(outset);
                for (corner, dx, dy) in
                    [(r.left_top(), 1.0, 1.0), (r.right_top(), -1.0, 1.0), (r.right_bottom(), -1.0, -1.0), (r.left_bottom(), 1.0, -1.0)]
                {
                    let path = vec![corner + Vec2::new(arm * dx, 0.0), corner, corner + Vec2::new(0.0, arm * dy)];
                    draw.stroke(&PaintShape::Path(path), StrokeWidth::Medium, ink);
                }
            }
        }
        for (at, (_, link)) in placed.extras.iter().enumerate() {
            let (Some(slot), Some((linked, _))) = (link, &self.extras[at].link) else { continue };
            let id = IconId::new(if *linked { "link" } else { "unlink" });
            let side = Rect::from_center_size(slot.center(), Vec2::splat(style.sizes.icon));
            ui.draw().icon(side, &IconSpec::svg(id));
        }
    }

    pub(super) fn paint_text(&self, draw: &Draw<'_>, cx: &Cx<'_, Self>, label: Rect) {
        let style = cx.style;
        let sizes = &style.sizes;
        let rtl = cx.env.direction().is_rtl();
        let wrapped = self.lines.as_ref().map(|text| {
            let run = TextRun::new(text.as_str()).role(style.detail.role).color(cx.resolve(&style.detail.color)).wrap(self.lines_wrap());
            draw.layout(&run, self.text_block(label, sizes, 0.0).0.width())
        });
        let lines = wrapped.as_ref().map_or(0.0, |layout| sizes.gap + layout.size().y);
        let (label, mut top) = self.text_block(label, sizes, lines);
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
        if !self.renaming {
            let muted = self.label_column(&self.cells).is_some_and(|column| self.muted.contains(column));
            let look = if muted { &style.cell_muted } else { &style.text };
            let mut span = Span::new(cx.resolved.label);
            span.italic = self.italic;
            let run = TextRun::spans(vec![span]).role(look.role).color(cx.resolve(&look.color)).elide();
            draw.text(Rect::from_min_size(Pos2::new(label.left(), top), Vec2::new((label.width() - beside).max(0.0), sizes.label_line)), &run);
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
    }
}

impl ListRow<'_> {
    /// The text cells, inset and centred on the row's height, each aligned as its column says.
    pub(super) fn paint_cells(&self, draw: &Draw<'_>, cx: &Cx<'_, Self>, placed: &Placed) {
        let sizes = &cx.style.sizes;
        let rtl = cx.env.direction().is_rtl();
        for (at, _, cell) in &placed.texts {
            let text = &self.texts[*at];
            let (align, numeric) = self.align_of(&text.column);
            let look = self.cell_look(cx.style, &text.column);
            let cell = Rect::from_x_y_ranges(cell.x_range(), cx.rect.y_range());
            let room = Flow::new(cell, rtl).span(sizes.cell_inset, sizes.cell_room(cell.width()));
            let mut span = Span::new(text.text.as_str());
            span.tabular = numeric;
            let run = TextRun::spans(vec![span]).role(look.role).color(cx.resolve(&look.color)).align(align);
            let run = if text.wrap { run.wrap(Wrap::Wrap) } else { run.elide() };
            let layout = draw.layout(&run, room.width());
            let top =
                if text.wrap { room.top() + ((room.height() - layout.size().y) / 2.0).max(0.0) } else { room.center().y - sizes.label_line / 2.0 };
            let free = (room.width() - layout.size().x).max(0.0);
            let inset = match align {
                HAlign::Start => 0.0,
                HAlign::Center => free / 2.0,
                HAlign::End => free,
            };
            let x = if rtl { room.right() - layout.size().x - inset } else { room.left() + inset };
            draw.galley(Pos2::new(x, top), &layout);
        }
    }
}

/// The down-right clipping mark: a stem down from the top, then a leg toward the reading end with an arrowhead.
pub(super) fn draw_clip_arrow(draw: &Draw<'_>, slot: Rect, rtl: bool, ink: StyleColor) {
    let side = slot.width().min(slot.height());
    let square = Rect::from_center_size(slot.center(), Vec2::splat(side));
    let at = |x: f32, y: f32| Pos2::new(if rtl { square.right() - x * side } else { square.left() + x * side }, square.top() + y * side);
    draw.stroke(&PaintShape::Path(vec![at(0.2, 0.2), at(0.2, 0.7), at(0.8, 0.7)]), StrokeWidth::Medium, ink);
    draw.stroke(&PaintShape::Path(vec![at(0.6, 0.45), at(0.85, 0.7), at(0.6, 0.95)]), StrokeWidth::Medium, ink);
}

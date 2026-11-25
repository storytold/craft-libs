//! The ruler's drawing.

use super::geometry::glyph;
use super::items::{RulerItem, SELECTOR};
use super::run::{Held, ruler_text};
use super::{TextRuler, TextRulerStyle};
use craft_ui::canvas::{RulerTick, Rulers, TickRule};
use craft_ui::contract::{Cx, Edit as _, PaintUi, Remember as _, Styled as _};
use craft_ui::primitives::{PaintShape, RulerFace, StrokeWidth, TextRun, Tick, TickEdge, TickRank};
use craft_ui::style::StyleColor;
use egui::{CornerRadius, Pos2, Rect, pos2, vec2};

impl TextRuler<'_> {
    pub(super) fn paint_ruler(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style: &TextRulerStyle = cx.style;
        let layout = self.layout(cx.rect, style, cx.flow.is_rtl());
        let (g, shapes) = (layout.g, layout.shapes);
        let values = cx.value().cloned().unwrap_or_default();
        let held = cx.recall::<Held>();
        let rect = |rect: Rect| PaintShape::Rect(rect, CornerRadius::ZERO);
        let path = |a: Pos2, b: Pos2| PaintShape::Path(vec![a, b]);
        let visible = g.visible();
        let bar = ui.content_painter(cx).ruler_bar(g.strip, TickEdge::Bottom, style.face);
        // The edge face shades the whole strip; the centred face shades its bar on the strip's own surface.
        let across = |part: Rect| Rect::from_x_y_ranges(part.x_range(), bar.y_range());
        let draw = ui.draw();
        if style.face == RulerFace::Centred {
            draw.fill(&rect(g.strip), style.strip);
        }
        draw.fill(&rect(across(visible)), style.margin);
        draw.fill(&rect(across(g.column().intersect(visible))), style.fill);
        for (start, end) in self.gaps {
            let (a, b) = (g.x(*start), g.x(*end));
            let gap = Rect::from_x_y_ranges(a.min(b)..=a.max(b), bar.y_range()).intersect(visible);
            if gap.is_positive() {
                draw.fill(&rect(gap), style.margin);
            }
        }

        let items = RulerItem::all(&values);
        let column_boxes: Vec<Rect> = self.columns.iter().map(|at| shapes.column_marker(g.strip, g.x(*at))).collect();
        let keep_clear: Vec<Rect> = items.iter().map(|item| shapes.rect(&g, *item, &values)).chain(column_boxes.iter().copied()).collect();
        self.paint_ticks(ui, cx, &keep_clear);

        let draw = ui.draw();
        let base = g.strip.max.y;
        for (at, stop) in values.stops.iter().enumerate() {
            let off = held.off && held.now == Some(RulerItem::Stop(at));
            let line = if off { style.marker_off } else { cx.resolve_part(RulerItem::Stop(at).part(), &style.marker) };
            let (segments, dot) = glyph(&shapes, g.x(stop.position), base, stop.align, g.right_to_left);
            for [a, b] in segments {
                draw.stroke(&path(a, b), StrokeWidth::Thin, line);
            }
            if let Some(centre) = dot {
                draw.fill(&PaintShape::Circle(centre, style.dot), line);
            }
        }
        for marker in &column_boxes {
            draw.fill(&rect(*marker), style.margin);
            draw.stroke(&rect(*marker), StrokeWidth::Thin, cx.resolve(&style.marker));
            let middle = marker.center();
            for step in GRIP_LINES {
                let at = pos2(middle.x + step * style.grip_gap, middle.y);
                draw.stroke(&path(at - vec2(0.0, style.grip_reach), at + vec2(0.0, style.grip_reach)), StrokeWidth::Thin, cx.resolve(&style.marker));
            }
        }
        if held.widths {
            let units = cx.env.units();
            let text = ruler_text(self.unit, &units);
            let color: StyleColor = cx.resolve(&style.label.color);
            for pair in self.columns.windows(2) {
                let [left, right] = pair else { continue };
                let run = TextRun::new(text.format_value(&units, f64::from((right - left).abs()), 0.0)).role(style.label.role).color(color);
                let size = draw.layout(&run, f32::INFINITY).size();
                let boxed = Rect::from_center_size(pos2(g.x((left + right) / 2.0), g.strip.center().y), size);
                draw.fill(&rect(boxed), style.fill);
                draw.text(boxed, &run);
            }
        }
        let x = |item: RulerItem| g.x(item.position(&values, g.width));
        let line = |item: RulerItem| cx.resolve_part(item.part(), &style.marker);
        let triangle = |a: Pos2, b: Pos2, c: Pos2, item: RulerItem| {
            let shape = PaintShape::Polygon(vec![a, b, c]);
            draw.fill(&shape, style.marker_fill);
            draw.stroke(&shape, StrokeWidth::Thin, line(item));
        };
        let (first, left, right) = (x(RulerItem::FirstLine), x(RulerItem::LeftBox), x(RulerItem::Right));
        let top = shapes.first_line(g.strip, first);
        triangle(top.left_top(), top.right_top(), pos2(first, top.max.y), RulerItem::FirstLine);
        let hanging = shapes.hanging(g.strip, left);
        triangle(hanging.left_bottom(), hanging.right_bottom(), pos2(left, hanging.min.y), RulerItem::Hanging);
        let boxed = rect(shapes.left_box(g.strip, left));
        draw.fill(&boxed, style.marker_fill);
        draw.stroke(&boxed, StrokeWidth::Thin, line(RulerItem::LeftBox));
        let end = shapes.right(g.strip, right);
        triangle(end.left_bottom(), end.right_bottom(), pos2(right, end.min.y), RulerItem::Right);

        if let (Some(kind), Some(corner)) = (self.kind, layout.corner) {
            // The corner continues the strip round it, the box is the column's colour.
            let surround = if style.face == RulerFace::Centred { style.strip } else { style.margin };
            draw.fill(&rect(corner), surround);
            let side = style.selector_side.min(corner.width()).min(corner.height());
            let boxed = PaintShape::Rect(Rect::from_center_size(corner.center(), vec2(side, side)), style.selector_radius);
            draw.fill(&boxed, cx.resolve_part(SELECTOR, &style.selector));
            draw.stroke(&boxed, StrokeWidth::Thin, style.tick);
            // The glyph is centred on the box by its own bounds: a left or right stop's foot runs to one side of its stem.
            let (segments, dot) = glyph(&shapes, 0.0, 0.0, kind, false);
            let bounds = Rect::from_points(&segments.iter().flatten().copied().chain(dot).collect::<Vec<_>>());
            let shift = corner.center() - bounds.center();
            for [a, b] in segments {
                draw.stroke(&path(a + shift, b + shift), StrokeWidth::Thin, cx.resolve(&style.marker));
            }
            if let Some(centre) = dot {
                draw.fill(&PaintShape::Circle(centre + shift, style.dot), cx.resolve(&style.marker));
            }
        }
    }

    fn paint_ticks(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, keep_clear: &[Rect]) {
        let style: &TextRulerStyle = cx.style;
        let g = self.layout(cx.rect, style, cx.flow.is_rtl()).g;
        let numbers = cx.env.units();
        let text = ruler_text(self.unit, &numbers);
        let per_doc = text.units.per_doc;
        let (lo, hi) = g.shown_span();
        let ticks: Vec<RulerTick> =
            Rulers::ticks(&text.units, TickRule::of_unit(self.unit.kind), f64::from(g.scale), (f64::from(lo) * per_doc)..=(f64::from(hi) * per_doc));
        let bar = ui.content_painter(cx).ruler_bar(g.strip, TickEdge::Bottom, style.face);
        let draw = ui.draw();
        let color: StyleColor = cx.resolve(&style.label.color);
        // Numbers count from the column's start edge, so none stand in the margins.
        let in_column = |at: f32| (0.0..=g.width).contains(&at);
        let mut runs = Vec::new();
        for tick in ticks.iter().filter(|tick| matches!(tick.rank, TickRank::Major)) {
            if !in_column((tick.value / per_doc) as f32) {
                continue;
            }
            let x = g.x((tick.value / per_doc) as f32);
            let label = text.tick_label(&numbers, tick.value, tick.step);
            let run = TextRun::new(label).role(style.label.role).color(color);
            let size = draw.layout(&run, f32::INFINITY).size();
            let boxed = match style.face {
                RulerFace::Edge => {
                    let start = if g.right_to_left { x - style.pad - size.x } else { x + style.pad };
                    Rect::from_min_size(pos2(start, g.strip.top()), vec2(size.x, g.strip.height()))
                }
                RulerFace::Centred => RulerFace::centred_number(bar, TickEdge::Bottom, x, size),
            };
            if g.visible().contains_rect(boxed) && !keep_clear.iter().any(|clear| clear.intersects(boxed)) {
                runs.push((boxed, run));
            }
        }
        // The tick rule adds a labelled step beyond each end; a ruler cut to its extent (a page's) shows none past it.
        let covered = g.visible().x_range();
        let marks: Vec<Tick> = ticks
            .iter()
            .map(|tick| (g.x((tick.value / per_doc) as f32), tick))
            .filter(|(x, _)| covered.contains(*x))
            .map(|(x, tick)| Tick::new(x - bar.min.x, tick.rank))
            .collect();
        let boxes: Vec<Rect> = runs.iter().map(|(boxed, _)| *boxed).collect();
        let marks = style.face.beside_numbers(TickEdge::Bottom, bar.min.x, marks, &boxes, style.pad);
        ui.content_painter(cx).ticks(bar, TickEdge::Bottom, style.face, &marks);
        let draw = ui.draw();
        for (boxed, run) in &runs {
            draw.text(*boxed, run);
        }
    }
}

/// The three grip lines across a table column marker, in marker-gap steps from its middle.
const GRIP_LINES: [f32; 3] = [-1.0, 0.0, 1.0];

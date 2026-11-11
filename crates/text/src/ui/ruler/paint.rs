//! The ruler's drawing.

use super::geometry::glyph;
use super::items::Item;
use super::run::{Held, rule, ruler_text};
use super::{TextRuler, TextRulerStyle};
use craft_ui::canvas::{RulerTick, Rulers};
use craft_ui::contract::{Cx, Edit as _, PaintUi, Remember as _, Styled as _};
use craft_ui::primitives::{PaintShape, StrokeWidth, TextRun, Tick, TickEdge, TickRank};
use craft_ui::style::StyleColor;
use egui::{CornerRadius, Pos2, Rect, pos2, vec2};

impl TextRuler<'_> {
    pub(super) fn paint_ruler(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style: &TextRulerStyle = cx.style;
        let layout = self.layout(cx.rect, style, cx.flow.is_rtl());
        let (g, shapes) = (layout.g, layout.shapes);
        let values = cx.value().cloned().unwrap_or_default();
        let held = cx.recall::<Held>();
        let draw = ui.draw();
        let rect = |rect: Rect| PaintShape::Rect(rect, CornerRadius::ZERO);
        let path = |a: Pos2, b: Pos2| PaintShape::Path(vec![a, b]);
        draw.fill(&rect(g.strip), style.margin);
        draw.fill(&rect(g.column()), style.fill);

        let items = Item::all(&values);
        let column_boxes: Vec<Rect> = self.columns.iter().map(|at| shapes.column_marker(g.strip, g.x(*at))).collect();
        let keep_clear: Vec<Rect> = items.iter().map(|item| shapes.rect(&g, *item, &values)).chain(column_boxes.iter().copied()).collect();
        self.paint_ticks(ui, cx, &keep_clear);

        let draw = ui.draw();
        let base = g.strip.max.y;
        for (at, stop) in values.stops.iter().enumerate() {
            let off = held.off && held.now == Some(Item::Stop(at));
            let line = if off { style.marker_off } else { cx.resolve_part(Item::Stop(at).part(), &style.marker) };
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
        let x = |item: Item| g.x(item.position(&values, g.width));
        let line = |item: Item| cx.resolve_part(item.part(), &style.marker);
        let triangle = |a: Pos2, b: Pos2, c: Pos2, item: Item| {
            let shape = PaintShape::Polygon(vec![a, b, c]);
            draw.fill(&shape, style.marker_fill);
            draw.stroke(&shape, StrokeWidth::Thin, line(item));
        };
        let (first, left, right) = (x(Item::FirstLine), x(Item::LeftBox), x(Item::Right));
        let top = shapes.first_line(g.strip, first);
        triangle(top.left_top(), top.right_top(), pos2(first, top.max.y), Item::FirstLine);
        let hanging = shapes.hanging(g.strip, left);
        triangle(hanging.left_bottom(), hanging.right_bottom(), pos2(left, hanging.min.y), Item::Hanging);
        let boxed = rect(shapes.left_box(g.strip, left));
        draw.fill(&boxed, style.marker_fill);
        draw.stroke(&boxed, StrokeWidth::Thin, line(Item::LeftBox));
        let end = shapes.right(g.strip, right);
        triangle(end.left_bottom(), end.right_bottom(), pos2(right, end.min.y), Item::Right);

        if let Some(kind) = self.kind {
            let corner = layout.corner;
            draw.fill(&rect(corner), style.fill);
            let side = style.thickness.min(corner.width()).min(corner.height());
            let boxed = Rect::from_center_size(corner.center(), vec2(side, side));
            draw.stroke(&rect(boxed), StrokeWidth::Thin, style.tick);
            let (segments, dot) = glyph(&shapes, boxed.center().x, boxed.max.y - (boxed.height() - style.stem) / 2.0, kind, false);
            for [a, b] in segments {
                draw.stroke(&path(a, b), StrokeWidth::Thin, cx.resolve(&style.marker));
            }
            if let Some(centre) = dot {
                draw.fill(&PaintShape::Circle(centre, style.dot), cx.resolve(&style.marker));
            }
        }
    }

    fn paint_ticks(&self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, keep_clear: &[Rect]) {
        let style: &TextRulerStyle = cx.style;
        let g = self.layout(cx.rect, style, cx.flow.is_rtl()).g;
        let numbers = cx.env.units();
        let text = ruler_text(self.unit, &numbers);
        let per_doc = text.units.per_doc;
        let (lo, hi) = g.span();
        let ticks: Vec<RulerTick> =
            Rulers::ticks(&text.units, rule(self.unit), f64::from(g.scale), (f64::from(lo) * per_doc)..=(f64::from(hi) * per_doc));
        let marks: Vec<Tick> = ticks.iter().map(|tick| Tick::new(g.x((tick.value / per_doc) as f32) - g.strip.min.x, tick.rank)).collect();
        ui.content_painter(cx).ticks(g.strip, TickEdge::Bottom, &marks);
        let draw = ui.draw();
        let color: StyleColor = cx.resolve(&style.label.color);
        for tick in ticks.iter().filter(|tick| matches!(tick.rank, TickRank::Major)) {
            let x = g.x((tick.value / per_doc) as f32);
            let label = text.tick_label(&numbers, tick.value, tick.step);
            let run = TextRun::new(label).role(style.label.role).color(color);
            let width = draw.layout(&run, f32::INFINITY).size().x;
            let size = vec2(width, g.strip.height());
            let at = if g.right_to_left { pos2(x - style.pad - width, g.strip.top()) } else { pos2(x + style.pad, g.strip.top()) };
            let boxed = Rect::from_min_size(at, size);
            if g.strip.contains_rect(boxed) && !keep_clear.iter().any(|clear| clear.intersects(boxed)) {
                draw.text(boxed, &run);
            }
        }
    }
}

/// The three grip lines across a table column marker, in marker-gap steps from its middle.
const GRIP_LINES: [f32; 3] = [-1.0, 0.0, 1.0];

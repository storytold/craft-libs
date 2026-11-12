//! Stories of the chart painter.
//!
//! `ChartSheet` is the component the stories paint on: a chart is painted through the `Draw` of a component, and
//! the table widgets have none that shows a bare primitive list. The sheet builds each chart's primitives from the
//! theme's colours at the chart's size, as an app's chart engine builds them from its palette and the chart's frame.

use super::{ChartPrimitive, HAlign, VAlign, paint_chart};
use craft_ui::contract::{Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, Show as _};
use craft_ui::primitives::StrokeWidth;
use craft_ui::style::{ComponentStyle, DataColor, StyleCx};
use craft_ui::testing::{Drive, StoryData, StoryOutput, stage_width, story};
use egui::{Response, Ui, Vec2, vec2};

/// The style of the story's chart sheet: the colours the charts are drawn in.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub struct ChartSheetStyle {
    #[style(metrics)]
    metrics: craft_behavior::Metrics,
    /// The series colours, in order: accent, good, warning, bad.
    series: [DataColor; 4],
    /// Axes and gridlines.
    axis: DataColor,
    /// The line between two wedges.
    gap: DataColor,
}

/// A chart's primitives in the sheet's colours at a size.
type Chart = fn(&ChartSheetStyle, Vec2) -> Vec<ChartPrimitive>;

/// A sheet that paints its charts side by side, each `size`.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct ChartSheet<'a> {
    #[common]
    common: Common,
    charts: &'a [Chart],
    size: Vec2,
}

impl Component for ChartSheet<'_> {
    const ROLE: Role = Role::IMAGE;
    const OVERLAYS: bool = false;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = ChartSheetStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: ChartSheetStyleKey) -> ChartSheetStyle {
        let color = &cx.tokens.color;
        let data = |token| DataColor::new(cx.color(token).egui());
        ChartSheetStyle {
            metrics: craft_behavior::Metrics::new(),
            series: [color.accent.solid, color.status.good.solid, color.status.warn.solid, color.status.bad.solid].map(data),
            axis: data(color.line.border),
            gap: data(color.surface.raised),
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        vec2(self.size.x * self.charts.len() as f32, self.size.y)
    }

    fn sense(&self) -> craft_behavior::Sense {
        craft_behavior::Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        for (at, chart) in self.charts.iter().enumerate() {
            let origin = cx.rect.min + vec2(self.size.x * at as f32, 0.0);
            paint_chart(&draw, origin, &chart(cx.style, self.size), 1.0);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The share of a chart's width and height left empty at each edge.
const CHART_MARGIN: f32 = 0.12;

/// The height of a chart over its width.
const CHART_ASPECT: f32 = 0.7;

/// A column chart: a title, axes, dashed gridlines, and a bar and a label per region.
fn column_chart(style: &ChartSheetStyle, size: Vec2) -> Vec<ChartPrimitive> {
    let regions = ["North", "South", "East", "West"];
    let values = [42.0, 65.0, 38.0, 80.0];
    let (left, right) = (size.x * CHART_MARGIN, size.x * (1.0 - CHART_MARGIN));
    let band = size.y * CHART_MARGIN * 1.5;
    let (top, bottom) = (band, size.y - band);
    let mut out = vec![ChartPrimitive::text(size.x / 2.0, top / 2.0, "Sales by region").anchored(HAlign::Center, VAlign::Center).bold()];
    for step in 1..=3 {
        let y = bottom - (bottom - top) * step as f32 / 4.0;
        out.push(ChartPrimitive::line(vec![[left, y], [right, y]], style.axis, StrokeWidth::Hairline).dashed());
    }
    out.push(ChartPrimitive::line(vec![[left, top], [left, bottom], [right, bottom]], style.axis, StrokeWidth::Hairline));
    let max = values.iter().copied().fold(f32::MIN, f32::max);
    let slot = (right - left) / values.len() as f32;
    for (at, (value, region)) in values.iter().zip(regions).enumerate() {
        let width = slot * 0.6;
        let height = (bottom - top) * value / max;
        let x = left + slot * at as f32 + (slot - width) / 2.0;
        out.push(ChartPrimitive::rect(x, bottom - height, width, height).filled(style.series[0]));
        out.push(ChartPrimitive::text(x + width / 2.0, bottom + band / 2.0, region).anchored(HAlign::Center, VAlign::Center));
    }
    out
}

/// A doughnut: a title and a wedge per share, from 12 o'clock clockwise, in the four series colours.
fn doughnut_chart(style: &ChartSheetStyle, size: Vec2) -> Vec<ChartPrimitive> {
    let shares = [40.0, 30.0, 20.0, 10.0];
    let centre = [size.x / 2.0, size.y * 0.55];
    let outer = size.x.min(size.y) * 0.34;
    let total: f32 = shares.iter().sum();
    let mut out = vec![ChartPrimitive::text(centre[0], size.y * CHART_MARGIN, "Share of sales").anchored(HAlign::Center, VAlign::Center).bold()];
    let mut from = 0.0;
    for (share, fill) in shares.iter().zip(style.series) {
        let to = from + std::f32::consts::TAU * share / total;
        out.push(ChartPrimitive::wedge(centre, (outer * 0.55, outer), (from, to), fill).stroked(style.gap, StrokeWidth::Hairline));
        from = to;
    }
    out
}

/// A column chart and a doughnut side by side, each half the stage wide.
#[story(ChartSheet<'static>, covers = [Default], drive = Drive::Never, title = "Primitives")]
fn primitives(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let charts: [Chart; 2] = [column_chart, doughnut_chart];
    let width = stage_width(ui) / charts.len() as f32;
    ChartSheet { common: Common::new(), charts: &charts, size: vec2(width, width * CHART_ASPECT) }.label("Charts").show(ui).into()
}

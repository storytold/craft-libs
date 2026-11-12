//! [`GradientRamp`]: a gradient drawn across a rect over the checkerboard.

use craft_behavior::Axis;
use craft_theme::Srgba;
use craft_ui::choice::SwatchMetrics;
use craft_ui::primitives::{Checker, ContentPainterCx, PaintShape, StrokeWidth};
use craft_ui::style::{DataColor, StyleColor, StyleCx};
use egui::{Color32, CornerRadius, Rect};

use super::stops::Sampler;
use super::value::{GradientStops, StopRow};

/// Even samples added to a stop bar's own, so a custom sampler's curve shows.
const EVEN_SAMPLES: usize = 64;

/// Paint for a gradient: a mesh across a rect, over the checkerboard where it is translucent.
/// The ramp has no edge; [`GradientSwatch`](super::GradientSwatch) draws a swatch's rings and the
/// stop bar its edge.
#[derive(Clone, Copy, Debug)]
pub struct GradientRamp;

impl GradientRamp {
    /// Width to height of a gradient chip, a swatch or a palette cell: wide enough that the ramp reads along it.
    pub const CHIP_ASPECT: (u16, u16) = (16, 9);

    /// The colours to draw `stops` with at the locations a mesh needs: the ends, every stop and
    /// midpoint of both rows, and even steps between, from `sampler`.
    pub fn colors(stops: &GradientStops, sampler: &Sampler) -> Vec<(f32, Srgba)> {
        let mut at = vec![0.0, 1.0];
        at.extend((1..EVEN_SAMPLES).map(|k| k as f32 / EVEN_SAMPLES as f32));
        let mut rows = vec![(StopRow::Color, stops.color.iter().map(|s| (s.location, s.midpoint)).collect::<Vec<_>>())];
        rows.push((StopRow::Opacity, stops.opacity.iter().map(|s| (s.location, s.midpoint)).collect()));
        for (_, list) in &rows {
            for (k, &(a, m)) in list.iter().enumerate() {
                at.push(a);
                if let Some(&(b, _)) = list.get(k + 1) {
                    at.push(a + m * (b - a));
                }
            }
        }
        at.retain(|x| x.is_finite());
        at.iter_mut().for_each(|x| *x = x.clamp(0.0, 1.0));
        at.sort_by(f32::total_cmp);
        at.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        at.into_iter().map(|x| (x, sampler(stops, x))).collect()
    }

    /// Paints `colors` (location 0..=1, sorted) across `rect` with corners of `radius`, straight lines
    /// between them, over the theme's checkerboard where a colour is translucent; the end colours
    /// continue to the rect's ends.
    pub fn paint(cx: &ContentPainterCx<'_>, rect: Rect, radius: CornerRadius, colors: &[(f32, Srgba)]) {
        if colors.is_empty() || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        if colors.iter().any(|(_, c)| c.a < 1.0) {
            cx.checker_with(&Checker::new(), rect, radius);
        }
        let stops: Vec<(f32, DataColor)> = colors.iter().map(|&(at, c)| (at, DataColor::new(Color32::from(c)))).collect();
        cx.ramp(rect, radius, &stops, Axis::X);
    }

    /// The corners and the ring of a gradient chip: the colour swatch's corners and `line.control_edge`.
    pub fn chip_look(cx: &StyleCx<'_>) -> (CornerRadius, StyleColor) {
        (SwatchMetrics::of(cx).radius, cx.color(cx.tokens.color.line.control_edge))
    }

    /// A gradient chip: `colors` across `rect` in corners of `radius`, inside a thin `edge` ring that separates it
    /// from the panel.
    pub fn paint_chip(cx: &ContentPainterCx<'_>, rect: Rect, radius: CornerRadius, colors: &[(f32, Srgba)], edge: StyleColor) {
        Self::paint(cx, rect, radius, colors);
        cx.with_draw(|draw| draw.stroke(&PaintShape::Rect(rect, radius), StrokeWidth::Thin, edge));
    }
}

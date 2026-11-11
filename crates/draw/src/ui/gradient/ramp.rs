//! [`GradientRamp`]: a gradient drawn across a rect over the checkerboard.

use craft_behavior::Axis;
use craft_theme::Srgba;
use craft_ui::primitives::{Checker, ContentPainterCx};
use craft_ui::style::DataColor;
use egui::{Color32, Rect};

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

    /// Paints `colors` (location 0..=1, sorted) across `rect`, straight lines between them, over the
    /// theme's checkerboard where a colour is translucent; the end colours continue to the rect's
    /// ends.
    pub fn paint(cx: &ContentPainterCx<'_>, rect: Rect, colors: &[(f32, Srgba)]) {
        if colors.is_empty() || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        if colors.iter().any(|(_, c)| c.a < 1.0) {
            cx.checker_with(&Checker::new(), rect);
        }
        let stops: Vec<(f32, DataColor)> = colors.iter().map(|&(at, c)| (at, DataColor::new(Color32::from(c)))).collect();
        cx.ramp(rect, &stops, Axis::X);
    }
}

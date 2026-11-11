//! How a histogram is drawn: the colours of the bars, the plot frame and the pure geometry of the bins, the selection
//! band and the markers. [`paint_bins`] is the one paint path of the bins: the histogram plot calls it, and a curve
//! graph calls it with [`HistogramLook::Backdrop`] behind its curve.

use super::data::{Blend, ChannelColor, HistogramData, HistogramLook, HistogramScale, LevelRange, Plot, color32};
use craft_theme::Stroke as StrokeStep;
use craft_ui::primitives::ContentPainterCx;
use craft_ui::style::StyleCx;
use egui::{Color32, CornerRadius, Mesh, Pos2, Rect, Shape, Stroke, StrokeKind, pos2};

/// The colours of the bars, read in a style hook: the data tokens of the channels, the backdrop behind a curve, the
/// scope the ribbons sit on and the outline's width.
///
/// A field of the `ComponentStyle` of every component that paints bins.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct HistogramInks {
    /// The tone look's colour per [`ChannelColor`], in [`ChannelColor::ALL`] order: the data tokens, `text.muted` for
    /// neutral.
    tones: [Color32; 7],
    /// The colour of each [`ChannelColor`] on the scope, in [`ChannelColor::ALL`] order: its data token, `data.luma`
    /// for neutral.
    inks: [Color32; 7],
    /// The plot behind ribbons: `data.scope`, dark in every theme.
    scope: Color32,
    /// The tone look's outline in the neutral channel: `text.normal`, so it stands out from the `text.muted` neutral
    /// fill.
    outline_neutral: Color32,
    /// The tone look behind a curve, in every channel: `text.muted` at half opacity.
    backdrop: Color32,
    /// Width of the outlines: `stroke.thin`.
    line: f32,
}

impl HistogramInks {
    /// The colours of the tokens of `cx`.
    ///
    /// The ribbons and inks read the ambient tokens although the scope is dark in every theme; they take the dark
    /// mode's colours once a theme scope can resolve its children in a given mode.
    pub fn new(cx: &StyleCx<'_>) -> Self {
        let colors = &cx.tokens.color;
        let muted = color32(colors.text.muted);
        Self {
            tones: ChannelColor::ALL.map(|channel| if channel == ChannelColor::Neutral { muted } else { channel.tone(cx) }),
            inks: ChannelColor::ALL.map(|channel| channel.tone(cx)),
            scope: color32(colors.data.scope),
            outline_neutral: color32(colors.text.normal),
            backdrop: color32(colors.data.backdrop),
            line: cx.tokens.stroke.thin,
        }
    }

    /// The tone look's colour for `channel`.
    pub fn tone(&self, channel: ChannelColor) -> Color32 {
        self.tones[channel as usize]
    }

    /// The ribbon colour of `channel` on the scope.
    pub fn ink(&self, channel: ChannelColor) -> Color32 {
        self.inks[channel as usize]
    }

    /// The red, green and blue ribbon colours.
    pub fn ribbons(&self) -> [Color32; 3] {
        [ChannelColor::Red, ChannelColor::Green, ChannelColor::Blue].map(|channel| self.ink(channel))
    }

    /// The plot behind ribbons.
    pub fn scope(&self) -> Color32 {
        self.scope
    }

    /// The tone look behind a curve.
    pub fn backdrop(&self) -> Color32 {
        self.backdrop
    }

    /// The width of an outline.
    pub fn line(&self) -> f32 {
        self.line
    }
}

/// The frame of a plot: the sunken well and its edge. The histogram, the curve editor and preview and the scope plots
/// draw the same one.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlotFrame {
    fill: Color32,
    edge: Color32,
}

impl PlotFrame {
    /// The frame in the tokens of `cx`: `surface.sunken` and `line.control_edge`, with square corners.
    pub fn new(cx: &StyleCx<'_>) -> Self {
        let colors = &cx.tokens.color;
        Self { fill: color32(colors.surface.sunken), edge: color32(colors.line.control_edge) }
    }

    /// The frame with the plot filled by `fill`, for a plot on the scope.
    #[must_use]
    pub fn on(mut self, fill: Color32) -> Self {
        self.fill = fill;
        self
    }
}

/// Paints the well of a plot over `rect`, and its edge when `bordered`.
pub fn paint_frame(painter: &ContentPainterCx<'_>, rect: Rect, frame: &PlotFrame, bordered: bool) {
    painter.painter.rect_filled(rect, CornerRadius::ZERO, frame.fill);
    if bordered {
        paint_edge(painter, rect, frame);
    }
}

/// Paints the edge of a plot over `rect`, over what the plot drew, as wide as the border of a field.
pub fn paint_edge(painter: &ContentPainterCx<'_>, rect: Rect, frame: &PlotFrame) {
    let width = StrokeStep::W1.width(painter.pixels_per_point());
    painter.painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(width, frame.edge), StrokeKind::Inside);
}

/// Draws `data` in `look` into `rect`: no frame, no layout, no interaction, nothing in the AccessKit tree. `data` that
/// doesn't fit the look (RGB data in the tone look) and a degenerate `rect` draw nothing.
pub fn paint_bins(painter: &ContentPainterCx<'_>, rect: Rect, data: &HistogramData, look: HistogramLook, inks: &HistogramInks) {
    painter.painter.extend(plot_shapes(rect, data, look, inks));
}

/// Whether `rect` is a finite rectangle with room to draw in.
pub fn usable(rect: Rect) -> bool {
    rect.is_finite() && rect.is_positive() && rect.size().is_finite()
}

/// The colour of the area under every channel of `mask` (bit `c` for channel `c`): the channel-wise saturating sum of
/// their colours, black under none. It doesn't depend on drawing order.
pub fn sum(colors: &[Color32], mask: u32) -> Color32 {
    let (mut r, mut g, mut b) = (0u8, 0u8, 0u8);
    for (c, color) in colors.iter().enumerate() {
        if mask & (1 << c) != 0 {
            (r, g, b) = (r.saturating_add(color.r()), g.saturating_add(color.g()), b.saturating_add(color.b()));
        }
    }
    Color32::from_rgb(r, g, b)
}

/// The colour of the area under every channel of `mask`, multiplied channel-wise: white under none.
pub fn product(colors: &[Color32], mask: u32) -> Color32 {
    let mut rgb = [255u16; 3];
    for (c, color) in colors.iter().enumerate() {
        if mask & (1 << c) != 0 {
            for (acc, v) in rgb.iter_mut().zip([color.r(), color.g(), color.b()]) {
                *acc = (*acc * u16::from(v) + 127) / 255;
            }
        }
    }
    Color32::from_rgb(rgb[0] as u8, rgb[1] as u8, rgb[2] as u8)
}

impl Blend {
    /// The colour under the channels of `mask`.
    pub fn color(self, colors: &[Color32], mask: u32) -> Color32 {
        match self {
            Blend::Add => sum(colors, mask),
            Blend::Multiply => product(colors, mask),
        }
    }
}

/// The shapes of `data` in `look` inside `rect`.
pub fn plot_shapes(rect: Rect, data: &HistogramData, look: HistogramLook, inks: &HistogramInks) -> Vec<Shape> {
    if !usable(rect) {
        return Vec::new();
    }
    match (&data.plot, look) {
        (Plot::Tone { bins, channel }, HistogramLook::Tone | HistogramLook::Backdrop) => {
            let color = if look == HistogramLook::Backdrop { inks.backdrop } else { inks.tone(*channel) };
            vec![Shape::mesh(tone_mesh(rect, &tone_heights(bins, data.scale), color))]
        }
        (Plot::Channels { channels, blend }, HistogramLook::Rgb) => {
            let colors: Vec<Color32> = channels.iter().map(|(_, channel)| inks.ink(*channel)).collect();
            let bins: Vec<Vec<f32>> = channels.iter().map(|(bins, _)| bins.clone()).collect();
            let heights = rgb_heights(&bins, data.scale);
            let mut shapes = vec![Shape::mesh(rgb_mesh(rect, &heights, &colors, *blend))];
            for (values, color) in heights.iter().zip(colors) {
                if values.iter().any(|&v| v > 0.0) {
                    shapes.push(Shape::line(outline(rect, values), Stroke::new(inks.line, color)));
                }
            }
            shapes
        }
        _ => Vec::new(),
    }
}

/// The marker tick of `width` by `height` at bin position `level`, standing on the bottom of `rect`, snapped to whole
/// points and clipped to `rect`.
pub fn marker_shapes(rect: Rect, bins: usize, level: f32, (width, height): (f32, f32), color: Color32) -> Vec<Shape> {
    if !usable(rect) || bins == 0 || !level.is_finite() {
        return Vec::new();
    }
    let left = (level_x(rect, bins, level) - width / 2.0).round();
    let band = Rect::from_min_max(pos2(left, rect.bottom() - height), pos2(left + width, rect.bottom())).intersect(rect);
    vec![Shape::rect_filled(band, 0.0, color)]
}

/// The x of bin position `level`, clamped to the plot.
pub fn level_x(rect: Rect, bins: usize, level: f32) -> f32 {
    rect.left() + ((level + 0.5) / bins.max(1) as f32).clamp(0.0, 1.0) * rect.width()
}

/// The whole bin position under `x`, clamped to the bins. A bin owns the whole slot it is centred in.
pub fn level_at(rect: Rect, bins: usize, x: f32) -> f32 {
    ((x - rect.left()) / rect.width() * bins as f32 - 0.5).round().clamp(0.0, bins.saturating_sub(1) as f32)
}

/// The whole bin slots of `range`, from the first bin's left edge to the last bin's right edge, so a one-bin range has
/// the bin's width. `None` for a degenerate plot or a range that isn't finite.
pub fn selection_band(rect: Rect, bins: usize, range: LevelRange) -> Option<Rect> {
    if !usable(rect) || bins == 0 || !range.start.is_finite() || !range.end.is_finite() {
        return None;
    }
    let slot = |level: f32| rect.left() + (level / bins as f32).clamp(0.0, 1.0) * rect.width();
    Some(Rect::from_min_max(pos2(slot(range.start), rect.top()), pos2(slot(range.end + 1.0), rect.bottom())))
}

/// The two edge bands of `band`, each `width` wide inside it and snapped to whole points (a band narrower than two
/// edges is split between them).
pub fn selection_edges(band: Rect, width: f32) -> [Rect; 2] {
    let (left, right) = (band.left().round(), band.right().round());
    let width = width.min((right - left) / 2.0);
    [
        Rect::from_min_max(pos2(left, band.top()), pos2(left + width, band.bottom())),
        Rect::from_min_max(pos2(right - width, band.top()), pos2(right, band.bottom())),
    ]
}

/// The selected range: `color` over the whole slots of the range and a band of `width` at each edge.
pub fn selection_shapes(rect: Rect, bins: usize, range: LevelRange, width: f32, color: Color32) -> Vec<Shape> {
    let Some(band) = selection_band(rect, bins, range) else { return Vec::new() };
    let mut shapes = vec![Shape::rect_filled(band, 0.0, color)];
    shapes.extend(selection_edges(band, width).into_iter().map(|edge| Shape::rect_filled(edge.intersect(rect), 0.0, color)));
    shapes
}

/// The outline of `bins` over `data`: in the tone look on the plot's scale, so a taller outline runs off the top; in
/// the RGB look on its own.
pub fn outline_shapes(rect: Rect, data: &HistogramData, bins: &[f32], channel: ChannelColor, inks: &HistogramInks) -> Vec<Shape> {
    if !usable(rect) {
        return Vec::new();
    }
    let (heights, color) = match &data.plot {
        Plot::Tone { bins: current, .. } => (
            normalized(bins, tone_top(current, data.scale)),
            if channel == ChannelColor::Neutral { inks.outline_neutral } else { inks.tone(channel) },
        ),
        Plot::Channels { .. } => (tone_heights(bins, data.scale), inks.ink(channel)),
    };
    vec![Shape::line(outline(rect, &heights), Stroke::new(inks.line, color))]
}

/// `v / scale` clamped to 1 for every bin.
fn normalized(bins: &[f32], scale: f32) -> Vec<f32> {
    bins.iter().map(|&v| (v / scale).min(1.0)).collect()
}

/// The 95th-percentile column of the interior columns (every bin but the two edges), whose 1.5 times is the height of
/// the full plot in [`HistogramScale::Typical`].
fn typical(columns: impl Iterator<Item = f32>) -> f32 {
    let mut columns: Vec<f32> = columns.collect();
    columns.sort_unstable_by(f32::total_cmp);
    columns.get(columns.len() * 95 / 100).copied().unwrap_or(0.0) * 1.5
}

fn interior(bins: &[f32]) -> &[f32] {
    bins.get(1..bins.len().saturating_sub(1)).unwrap_or(&[])
}

/// Heights in `0..=1` of one channel's bins.
pub fn tone_heights(bins: &[f32], scale: HistogramScale) -> Vec<f32> {
    normalized(bins, tone_top(bins, scale))
}

/// The count at the top of the plot for one channel's bins.
fn tone_top(bins: &[f32], scale: HistogramScale) -> f32 {
    let tallest = bins.iter().copied().fold(0.0f32, f32::max);
    let mut top = tallest.max(1.0) * 1.1;
    if scale == HistogramScale::Typical {
        let typical = typical(interior(bins).iter().copied());
        if typical > 0.0 {
            top = top.min(typical);
        }
    }
    top
}

/// The display smoothing of an RGB channel: a five-bin binomial kernel (1 4 6 4 1)/16 on the interior bins; the two
/// edge bins are pinned and kept out of their neighbours' averages.
fn smooth(bins: &[f32]) -> Vec<f32> {
    let n = bins.len();
    if n < 3 {
        return bins.to_vec();
    }
    let sample = |j: usize| bins[j.clamp(1, n - 2)];
    (0..n)
        .map(|i| {
            if i == 0 || i == n - 1 {
                bins[i]
            } else {
                (sample(i.saturating_sub(2)) + 4.0 * sample(i - 1) + 6.0 * bins[i] + 4.0 * sample(i + 1) + sample(i + 2)) / 16.0
            }
        })
        .collect()
}

/// Heights in `0..=1` of the channels, on one scale. Edge bins are clipped pixels: they don't set the scale unless the
/// interior is empty, and are capped at the top.
pub fn rgb_heights(channels: &[Vec<f32>], scale: HistogramScale) -> Vec<Vec<f32>> {
    let curves: Vec<Vec<f32>> = channels.iter().map(|c| smooth(c)).collect();
    let interior_peak = curves.iter().flat_map(|c| interior(c).iter().copied()).fold(0.0f32, f32::max);
    let peak = if interior_peak > 0.0 { interior_peak } else { curves.iter().flatten().copied().fold(0.0f32, f32::max) };
    let mut top = if peak > 0.0 { peak * 1.1 } else { 1.0 };
    if scale == HistogramScale::Typical {
        let n = curves.first().map_or(0, Vec::len);
        let columns = (1..n.saturating_sub(1)).map(|i| curves.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).fold(0.0f32, f32::max));
        let typical = typical(columns);
        if typical > 0.0 {
            top = top.min(typical);
        }
    }
    curves.iter().map(|c| normalized(c, top)).collect()
}

/// The top edge of a plot: flat from the left to the first bin's centre, straight between centres, flat from the last
/// to the right.
pub fn outline(rect: Rect, heights: &[f32]) -> Vec<Pos2> {
    let n = heights.len();
    let y = |h: f32| rect.bottom() - h * rect.height();
    let mut points = Vec::with_capacity(n + 2);
    let (Some(&first), Some(&last)) = (heights.first(), heights.last()) else { return points };
    points.push(pos2(rect.left(), y(first)));
    points.extend(heights.iter().enumerate().map(|(i, &h)| pos2(rect.left() + (i as f32 + 0.5) / n as f32 * rect.width(), y(h))));
    points.push(pos2(rect.right(), y(last)));
    points
}

/// One mesh of vertical strips under `heights`: no anti-aliasing feather, so no seams between bins.
pub fn tone_mesh(rect: Rect, heights: &[f32], color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    for pair in outline(rect, heights).windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let i = mesh.vertices.len() as u32;
        for p in [pos2(a.x, rect.bottom()), a, b, pos2(b.x, rect.bottom())] {
            mesh.colored_vertex(p, color);
        }
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    }
    mesh
}

fn quad(mesh: &mut Mesh, rect: Rect, x: [f32; 2], bottom: [f32; 2], top: [f32; 2], color: Color32) {
    let i = mesh.vertices.len() as u32;
    for (x, height) in [(x[0], bottom[0]), (x[0], top[0]), (x[1], top[1]), (x[1], bottom[1])] {
        mesh.colored_vertex(pos2(x, rect.bottom() - height * rect.height()), color);
    }
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
}

/// One span between two bin centres, split where two channels' edges cross so each piece has a fixed channel order:
/// the band under k channels is drawn once, in their overlap colour, so the result doesn't depend on drawing order and
/// has no cracks.
fn span(mesh: &mut Mesh, rect: Rect, x: [f32; 2], a: &[f32], b: &[f32], fill: &dyn Fn(u32) -> Color32) {
    let mut cuts = vec![0.0f32, 1.0];
    for (i, (ai, bi)) in a.iter().zip(b).enumerate() {
        for (aj, bj) in a.iter().zip(b).skip(i + 1) {
            let denominator = (bi - ai) - (bj - aj);
            if denominator.abs() > f32::EPSILON {
                let crossing = (aj - ai) / denominator;
                if crossing > 0.0 && crossing < 1.0 {
                    cuts.push(crossing);
                }
            }
        }
    }
    cuts.sort_unstable_by(f32::total_cmp);
    let all = if a.len() >= 32 { u32::MAX } else { (1u32 << a.len()) - 1 };
    for cut in cuts.windows(2) {
        let (start, end) = (cut[0], cut[1]);
        if end <= start {
            continue;
        }
        let at = |t: f32| -> Vec<f32> { a.iter().zip(b).map(|(a, b)| a + (b - a) * t).collect() };
        let (left, right) = (at(start), at(end));
        let mut order: Vec<usize> = (0..a.len()).collect();
        order.sort_by(|&i, &j| (left[i] + right[i]).total_cmp(&(left[j] + right[j])));
        let x = [x[0] + (x[1] - x[0]) * start, x[0] + (x[1] - x[0]) * end];
        let mut floor = [0.0; 2];
        let mut mask = all;
        for channel in order {
            let top = [left[channel], right[channel]];
            if top[0] > floor[0] || top[1] > floor[1] {
                quad(mesh, rect, x, floor, top, fill(mask));
            }
            floor = top;
            mask &= !(1 << channel);
        }
    }
}

/// The ribbons' fills.
pub fn rgb_mesh(rect: Rect, heights: &[Vec<f32>], colors: &[Color32], blend: Blend) -> Mesh {
    let mut mesh = Mesh::default();
    let n = heights.first().map_or(0, Vec::len);
    if n == 0 || heights.iter().any(|h| h.len() != n) || colors.len() != heights.len() {
        return mesh;
    }
    let fill = |mask: u32| blend.color(colors, mask);
    let at = |i: usize| heights.iter().map(|h| h[i]).collect::<Vec<f32>>();
    let mut previous_x = rect.left();
    let mut previous = at(0);
    for i in 0..n {
        let x = rect.left() + (i as f32 + 0.5) / n as f32 * rect.width();
        let next = at(i);
        span(&mut mesh, rect, [previous_x, x], &previous, &next, &fill);
        previous_x = x;
        previous = next;
    }
    span(&mut mesh, rect, [previous_x, rect.right()], &previous, &previous, &fill);
    mesh
}

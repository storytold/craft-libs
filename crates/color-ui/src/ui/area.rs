//! [`ColorArea`]: two components of a colour across and down a field, the others fixed.

use super::channels::{QUANTIZE_CELLS, channel_range, channel_step, quantized, settled, snapped, spoken};
use crate::AreaMode;
use craft_behavior::{Metrics, Mixed, Proposal, Sense};
use craft_theme::ModelValues;
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, DisabledLook, Edit as _, EditResponse, MeasureCx, PaintUi, Props as _, Role, Sizing, WordSetCx as _,
};
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::value_drag::{PointMark, PointPicker};
use egui::{Mesh, Pos2, Rect, Vec2, pos2};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// Vertices along each side of the area's colour mesh (PhotoCraft's 32).
const MESH: usize = 32;

/// The style of a colour area: the field is a point picker, which brings its own look.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ColorAreaStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The 2D field of a colour picker: two components of the colour across and down, the others fixed, as an
/// [`AreaMode`] lays them out (PhotoCraft's Color Picker field). It shows a `PointPicker` over the mode's two
/// channels, with the colours of the field as the picker's backdrop, so pointer, keys, wheel and AccessKit are the
/// picker's: a press jumps the point there, a drag follows the pointer (also outside the area, where the value stays
/// at the edge), the arrow keys step the across and down components (Up raises the down one, Shift steps ten) and a
/// step is the channel's (1 degree, 1 percent, 1 of 255, 0.001 of chroma).
///
/// The host gives the colour as the mode's model values and gets them back with only the area's two components
/// changed, so a host that keeps the values it was sent keeps the hue of a grey and the hue and saturation of black.
/// The area keeps its axes right to left: across still grows to the right. Its value text names both components
/// ("Saturation 40 %, Brightness 70 %", from [`names`](ColorArea::names)).
///
/// Its size comes from its [`shape`](ColorArea::shape), never from the room: the compact pickers' field at plot
/// proportions, or the Color Picker dialog's square. With `Props::sizing(Sizing::Fill)` it takes the room on both axes
/// instead, however small (the Color panel's field, as tall as its dock group leaves it).
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct ColorArea<'a> {
    #[common]
    common: Common,
    mode: AreaMode,
    values: Bind<'a, ModelValues>,
    /// Puts the colour the area reports on this many levels (at least two) per red, green and blue channel, whatever
    /// the mode: a web-safe field has 6. The pointer takes the nearest such colour to the point on the channels'
    /// steps, and a key or wheel step goes on by the channel's step until the colour is a different one. The field
    /// shows flat cells of the snapped colours.
    #[prop]
    quantize: Option<usize>,
    /// The names of the across and down components in the value text; the model's channel names without them.
    #[prop]
    names: Option<[Label; 2]>,
    /// The size of an area that does not fill its room; plot proportions by default.
    #[prop]
    shape: AreaShape,
}

/// The size of a [`ColorArea`] that does not fill its room.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AreaShape {
    /// `size.plot.width` wide and `size.plot.height` tall: the field of the Color panel's popover and the compact
    /// pickers, wider than tall.
    #[default]
    Plot,
    /// A square `size.plot.width` on a side: the Color Picker dialog's field, as Photoshop's.
    Square,
}

impl<'a> ColorArea<'a> {
    /// An area named `label` ("Saturation and brightness") showing `values`, values of `mode`'s model.
    pub fn new(label: impl Into<Label>, mode: AreaMode, values: Bind<'a, ModelValues>) -> Self {
        ColorArea { common: Common::new(), mode, values, quantize: None, names: None, shape: AreaShape::Plot }.label(label).ltr_axis()
    }

    /// The across and down channels of the mode's model.
    fn channels(&self) -> [usize; 2] {
        self.mode.area()
    }

    fn levels(&self) -> Option<usize> {
        self.quantize.map(|levels| levels.max(2))
    }

    /// The values the pointer at `at` picks in `rect`: the picker's own point-to-value mapping, then the settle and
    /// quantise steps. `ColorHud` drives it with the host's pointer.
    pub(crate) fn value_at(&self, rect: Rect, values: &ModelValues, at: Pos2) -> ModelValues {
        let mut mine = *values;
        if let Some(point) = self.picker(Some(values), false).value_at(rect, at) {
            self.settle_into(&mut mine, point.map(Some));
        }
        mine
    }

    /// Where `values` stands in `rect`: the picker's own value-to-point mapping.
    pub(crate) fn point_in(&self, rect: Rect, values: &ModelValues) -> Pos2 {
        let [across, down] = self.channels();
        let picker = self.picker(Some(values), false);
        picker.position([values.values[across], values.values[down]].map(f64::from), picker.plot(rect))
    }

    /// Writes the area's two components from `point` (an axis that is `None` stays) onto `mine`, settled on the
    /// channel's step and then on the quantised colour: the one rule for what the area reports.
    fn settle_into(&self, mine: &mut ModelValues, point: [Option<f64>; 2]) {
        let model = self.mode.model();
        for (channel, value) in self.channels().into_iter().zip(point) {
            if let Some(value) = value {
                mine.values[channel] = settled(model, channel, value as f32);
            }
        }
        if let Some(levels) = self.levels() {
            *mine = quantized(model, mine, levels);
        }
    }

    /// The picker over `shown`, whose backdrop paints the field of `base`.
    fn picker<'c>(&self, shown: Option<&ModelValues>, field: bool) -> PointPicker<'c> {
        let model = self.mode.model();
        let [across, down] = self.channels();
        let base = shown.copied().unwrap_or_else(|| ModelValues::new(model.id(), &[]));
        let point = match shown {
            Some(values) => Mixed::each([values.values[across], values.values[down]].map(f64::from)),
            None => [Mixed::Mixed; 2],
        };
        let (x, y) = (channel_range(model, across), channel_range(model, down));
        let y = *y.end()..=*y.start();
        let steps = [across, down].map(|channel| channel_step(model, channel));
        let (mode, levels) = (self.mode, self.levels());
        let mut picker = PointPicker::new(Bind::given(point), x, y).steps(steps).mark(PointMark::Ring);
        if let Some(levels) = levels {
            let color = move |point: [f64; 2]| {
                let mut values = base;
                values.values[across] = settled(model, across, point[0] as f32);
                values.values[down] = settled(model, down, point[1] as f32);
                quantized(model, &values, levels)
            };
            picker = picker.same(move |from, to| color(from) == color(to));
        }
        if field { picker.backdrop(move |painter: &ContentPainterCx, plot: Rect| paint_field(painter, plot, mode, &base, levels)) } else { picker }
    }
}

impl Component for ColorArea<'_> {
    const SINGLE_VALUE: bool = true;
    const ROLE: Role = Role::GROUP;
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = ColorAreaStyle;
    type Output = EditResponse<ModelValues>;

    fn style(_: &StyleCx<'_>, _: ColorAreaStyleKey) -> ColorAreaStyle {
        ColorAreaStyle { metrics: Metrics::new() }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        // An area that fills its room (the Color panel's, as tall as its group leaves it) is at least a hit target and
        // square on the width it is offered.
        if cx.resolved.sizing() == Some(Sizing::Fill) {
            return match cx.proposal {
                Proposal::Min => Vec2::splat(cx.tokens.size.hit.target),
                Proposal::Width(width) | Proposal::Size(Vec2 { x: width, .. }) => Vec2::splat(width),
                _ => Vec2::splat(cx.tokens.size.plot.width),
            };
        }
        let plot = &cx.tokens.size.plot;
        match self.shape {
            AreaShape::Plot => Vec2::new(plot.width, plot.height),
            AreaShape::Square => Vec2::splat(plot.width),
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, ModelValues>> {
        Some(self.values.reborrow())
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let Some(values) = cx.value() else { return };
        let model = self.mode.model();
        let channels = self.channels();
        let names = match &self.names {
            Some(names) => names.clone().map(|name| cx.env.resolve(&name).as_str().to_owned()),
            None => channels.map(|channel| model.channels()[channel].name.to_owned()),
        };
        let text = |at: usize| spoken(&names[at], model, channels[at], values.values[channels[at]]);
        access.value(format!("{}, {}", text(0), text(1)));
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let picker = self.picker(cx.value(), true).label(cx.label());
        let edit = ui.inner("picker", cx.rect, picker);
        let start = ModelValues::new(self.mode.model().id(), &[]);
        cx.adopt(&edit, &start, |point, mine| self.settle_into(mine, [0, 1].map(|axis| point[axis].same().copied())));
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<ModelValues> {
        cx.edit_response()
    }
}

/// Paints the colours of the field in `plot`: a mesh blending between 32 by 32 vertices, or with `levels` flat cells
/// of the snapped colours.
fn paint_field(painter: &ContentPainterCx, plot: Rect, mode: AreaMode, base: &ModelValues, levels: Option<usize>) {
    let model = mode.model();
    let [across, down] = mode.area();
    let (x, y) = (channel_range(model, across), channel_range(model, down));
    let at = |fx: f32, fy: f32| {
        let mut values = *base;
        values.values[across] = egui::lerp(*x.start() as f32..=*x.end() as f32, fx);
        values.values[down] = egui::lerp(*y.end() as f32..=*y.start() as f32, fy);
        model.to_srgb(&values.values)
    };
    let mut mesh = Mesh::default();
    match levels {
        Some(levels) => flat_cells(&mut mesh, plot, &at, levels),
        None => smooth_mesh(&mut mesh, plot, MESH, &at),
    }
    painter.painter.add(mesh);
}

/// The colours as a mesh of `steps` by `steps` vertices over `plot`, blending between them; `at` gives the colour at a
/// fraction across and down.
pub fn smooth_mesh(mesh: &mut Mesh, plot: Rect, steps: usize, at: &dyn Fn(f32, f32) -> craft_theme::Srgba) {
    let last = (steps - 1) as f32;
    for row in 0..steps {
        for col in 0..steps {
            let (fx, fy) = (col as f32 / last, row as f32 / last);
            mesh.colored_vertex(pos2(egui::lerp(plot.x_range(), fx), egui::lerp(plot.y_range(), fy)), at(fx, fy).into());
        }
    }
    for row in 0..steps - 1 {
        for col in 0..steps - 1 {
            let a = (row * steps + col) as u32;
            let b = a + steps as u32;
            mesh.add_triangle(a, a + 1, b + 1);
            mesh.add_triangle(a, b + 1, b);
        }
    }
}

/// The colours as `QUANTIZE_CELLS` by `QUANTIZE_CELLS` flat cells, each the snapped colour at its centre.
fn flat_cells(mesh: &mut Mesh, plot: Rect, at: &dyn Fn(f32, f32) -> craft_theme::Srgba, levels: usize) {
    let n = QUANTIZE_CELLS;
    let (w, h) = (plot.width() / n as f32, plot.height() / n as f32);
    for row in 0..n {
        for col in 0..n {
            let (fx, fy) = ((col as f32 + 0.5) / n as f32, (row as f32 + 0.5) / n as f32);
            let min = pos2(plot.left() + col as f32 * w, plot.top() + row as f32 * h);
            mesh.add_colored_rect(Rect::from_min_size(min, Vec2::new(w, h)), snapped(at(fx, fy), levels).into());
        }
    }
}

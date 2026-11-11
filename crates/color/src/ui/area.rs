//! [`ColorArea`]: two components of a colour across and down a field, the others fixed.

use super::channels::{QUANTIZE_CELLS, channel_range, channel_step, quantized, settled, snapped, spoken};
use crate::AreaMode;
use craft_behavior::{Metrics, Mixed, Proposal, Sense};
use craft_theme::ModelValues;
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, MeasureCx, PaintUi, Props as _, Role, WordSetCx as _,
};
use craft_ui::primitives::ContentPainterCx;
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::value_drag::PointPicker;
use egui::{Mesh, Rect, Vec2, pos2};

#[cfg(feature = "screen-testing")]
mod stories;

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
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct ColorArea<'a> {
    #[common]
    common: Common,
    mode: AreaMode,
    values: Bind<'a, ModelValues>,
    /// Puts the colour the area reports on this many levels (at least two) per red, green and blue channel, whatever
    /// the mode: a web-safe field has 6. The pointer takes the nearest such colour and a key steps one level of the
    /// channel's range. The field shows flat cells of the snapped colours.
    #[prop]
    quantize: Option<usize>,
    /// The names of the across and down components in the value text; the model's channel names without them.
    #[prop]
    names: Option<[Label; 2]>,
}

impl<'a> ColorArea<'a> {
    /// An area named `label` ("Saturation and brightness") showing `values`, values of `mode`'s model.
    pub fn new(label: impl Into<Label>, mode: AreaMode, values: Bind<'a, ModelValues>) -> Self {
        ColorArea { common: Common::new(), mode, values, quantize: None, names: None }.label(label).ltr_axis()
    }

    /// The across and down channels of the mode's model.
    fn channels(&self) -> [usize; 2] {
        self.mode.area()
    }

    fn levels(&self) -> Option<usize> {
        self.quantize.map(|levels| levels.max(2))
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
        let steps = [across, down].map(|channel| match self.levels() {
            Some(levels) => {
                let range = channel_range(model, channel);
                (range.end() - range.start()) / (levels - 1) as f64
            }
            None => channel_step(model, channel),
        });
        let (mode, levels) = (self.mode, self.levels());
        let picker = PointPicker::new(Bind::given(point), x, y).steps(steps);
        if field { picker.backdrop(move |painter: &ContentPainterCx, plot: Rect| paint_field(painter, plot, mode, &base, levels)) } else { picker }
    }
}

impl Component for ColorArea<'_> {
    const SINGLE_VALUE: bool = true;
    const ROLE: Role = Role::GROUP;
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
        let mut picker = self.picker(cx.value(), false);
        cx.child("picker", &mut picker, Proposal::Ideal)
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
        let model = self.mode.model();
        let [across, down] = self.channels();
        let picker = self.picker(cx.value(), true).label(cx.label());
        let edit = ui.inner("picker", cx.rect, picker);
        let levels = self.levels();
        let start = ModelValues::new(model.id(), &[]);
        cx.adopt(&edit, &start, |point, mine| {
            for (channel, axis) in [(across, 0), (down, 1)] {
                if let Some(value) = point[axis].same() {
                    mine.values[channel] = settled(model, channel, *value as f32);
                }
            }
            if let Some(levels) = levels {
                *mine = quantized(model, mine, levels);
            }
        });
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
        None => smooth_mesh(&mut mesh, plot, &at),
    }
    painter.painter.add(mesh);
}

/// The colours as a mesh of vertices, blending between them.
fn smooth_mesh(mesh: &mut Mesh, plot: Rect, at: &dyn Fn(f32, f32) -> craft_theme::Srgba) {
    let last = (MESH - 1) as f32;
    for row in 0..MESH {
        for col in 0..MESH {
            let (fx, fy) = (col as f32 / last, row as f32 / last);
            mesh.colored_vertex(pos2(egui::lerp(plot.x_range(), fx), egui::lerp(plot.y_range(), fy)), at(fx, fy).into());
        }
    }
    for row in 0..MESH - 1 {
        for col in 0..MESH - 1 {
            let a = (row * MESH + col) as u32;
            let b = a + MESH as u32;
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

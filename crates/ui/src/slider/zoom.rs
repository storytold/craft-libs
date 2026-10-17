//! The status bar zoom control of the Office-style Craft apps: `−`, a logarithmic [`Slider`], `+`
//! and the percentage, laid out as one fixed-size block so nothing moves when the number gains a
//! digit, whatever direction the surrounding layout runs.

use egui::{Align2, Color32, CornerRadius, FontId, Layout, Response, Sense, Stroke, Ui, pos2, vec2};

use super::{Scale, Slider, SliderStyle};

/// Colours of a [`ZoomBar`]: its slider's plus the `−`/`+` glyphs, their hover fill and the text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomBarStyle {
    pub slider: SliderStyle,
    pub icon: Color32,
    pub hover: Color32,
    pub text: Color32,
    pub font: f32,
}

/// The accessible names (and tooltips) of the parts, in the interface language.
#[derive(Clone, Copy, Debug)]
pub struct ZoomBarLabels<'a> {
    pub zoom: &'a str,
    pub zoom_in: &'a str,
    pub zoom_out: &'a str,
}

/// What happened this frame.
#[derive(Debug)]
pub struct ZoomBarResponse {
    /// The percentage changed (slider, keyboard or a button); apply it.
    pub changed: bool,
    /// The percentage text was clicked: Office apps open their Zoom dialog.
    pub percent_clicked: bool,
    /// The whole block.
    pub response: Response,
}

/// A zoom bar; see the module documentation.
pub struct ZoomBar<'a> {
    percent: &'a mut f64,
    range: std::ops::RangeInclusive<f64>,
    labels: ZoomBarLabels<'a>,
    style: &'a ZoomBarStyle,
    stops: &'a [f64],
    slider_width: f32,
}

const BUTTON: f32 = 18.0;
const GAP: f32 = 4.0;
/// Space between `−`/`+` and the slider: none, since the slider already keeps its thumb inset
/// from its ends, so the glyphs sit right at the track.
const TIGHT: f32 = 0.0;

impl<'a> ZoomBar<'a> {
    pub fn new(percent: &'a mut f64, range: std::ops::RangeInclusive<f64>, labels: ZoomBarLabels<'a>, style: &'a ZoomBarStyle) -> Self {
        Self { percent, range, labels, style, stops: &[], slider_width: 110.0 }
    }

    /// The values `−` and `+` step through (sorted, in percent). Without stops they step to the
    /// previous or next multiple of 10.
    pub fn stops(mut self, stops: &'a [f64]) -> Self {
        self.stops = stops;
        self
    }

    pub fn slider_width(mut self, width: f32) -> Self {
        self.slider_width = width;
        self
    }

    fn step(&self, up: bool) -> f64 {
        let v = *self.percent;
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let next = if self.stops.is_empty() {
            if up { (v / 10.0).floor() * 10.0 + 10.0 } else { (v / 10.0).ceil() * 10.0 - 10.0 }
        } else if up {
            self.stops.iter().copied().find(|s| *s > v + 0.5).unwrap_or(hi)
        } else {
            self.stops.iter().rev().copied().find(|s| *s < v - 0.5).unwrap_or(lo)
        };
        next.clamp(lo, hi)
    }

    /// The block's size: both buttons, the slider and the widest percentage (`500%` for a range
    /// ending at 500), with the gaps between them.
    fn label_width(&self, ui: &Ui) -> f32 {
        let font = FontId::proportional(self.style.font);
        let widest = format!("{:.0}%", self.range.end().abs().max(self.range.start().abs()));
        ui.fonts_mut(|f| f.layout_no_wrap(widest, font, Color32::WHITE).size().x) + 1.0
    }

    pub fn show(self, ui: &mut Ui) -> ZoomBarResponse {
        let label_w = self.label_width(ui);
        let size = vec2(BUTTON + TIGHT + self.slider_width + TIGHT + BUTTON + 2.0 * GAP + label_w, BUTTON.max(20.0));
        let (down_to, up_to) = (self.step(false), self.step(true));
        let mut changed = false;
        let mut percent_clicked = false;
        let style = self.style;
        let labels = self.labels;
        let range = self.range.clone();
        let slider_width = self.slider_width;
        let percent = self.percent;
        let inner = ui.allocate_ui_with_layout(size, Layout::left_to_right(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = TIGHT;
            if glyph_button(ui, false, labels.zoom_out, style) {
                *percent = down_to;
                changed = true;
            }
            let r =
                Slider::new(percent, range, labels.zoom, &style.slider).width(slider_width).scale(Scale::Logarithmic).step(1.0).mark(100.0).show(ui);
            changed |= r.changed();
            if glyph_button(ui, true, labels.zoom_in, style) {
                *percent = up_to;
                changed = true;
            }
            ui.add_space(2.0 * GAP);
            let (rect, resp) = ui.allocate_exact_size(vec2(label_w, size.y), Sense::click());
            ui.painter().text(rect.right_center(), Align2::RIGHT_CENTER, format!("{:.0}%", *percent), FontId::proportional(style.font), style.text);
            let value = *percent;
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{}: {value:.0}%", labels.zoom)));
            percent_clicked = resp.on_hover_text(labels.zoom).clicked();
        });
        ZoomBarResponse { changed, percent_clicked, response: inner.response }
    }
}

/// A thin `−` or `+` button with a hover fill; returns whether it was clicked.
fn glyph_button(ui: &mut Ui, plus: bool, label: &str, style: &ZoomBarStyle) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::click());
    if resp.hovered() || resp.has_focus() {
        ui.painter().rect_filled(rect, CornerRadius::same(3), style.hover);
    }
    let c = rect.center();
    let stroke = Stroke::new(1.3, if ui.is_enabled() { style.icon } else { style.icon.gamma_multiply(0.45) });
    let half = 5.0;
    ui.painter().line_segment([pos2(c.x - half, c.y), pos2(c.x + half, c.y)], stroke);
    if plus {
        ui.painter().line_segment([pos2(c.x, c.y - half), pos2(c.x, c.y + half)], stroke);
    }
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    resp.on_hover_text(label).clicked()
}

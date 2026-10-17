//! A slider with a visible, filled track and an optional editable value field.
//!
//! egui's built-in slider draws a thin rail in `widgets.inactive.bg_fill`, which several Craft
//! themes make nearly invisible, so only a lone handle shows and users don't recognise it as a
//! slider. This one paints a track that is always visible, fills it with the accent up to the
//! thumb, marks the default value with a tick (double-click the track to return to it), and keeps
//! an optional value field at a fixed width so nothing next to it moves when the number gains a
//! digit. Applications supply the colors through [`SliderStyle`]; nothing global is changed.
//!
//! Interaction: drag or click the track; the thumb follows the pointer smoothly even when the
//! value snaps to a [`Slider::step`]. With keyboard focus, Left/Down and Right/Up move one step
//! (or 1% of the track), Page Up/Down ten, Home/End to the ends. Assistive technology sees a slider
//! with the given label and value, and its increment/decrement actions work.
//!
//! [`ZoomBar`] combines a logarithmic slider with `−`/`+` buttons and a fixed-width percentage, the
//! status bar zoom control of the Office-style apps.

use std::ops::RangeInclusive;

use egui::{Color32, CornerRadius, EventFilter, Key, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, pos2, vec2};

mod zoom;

pub use zoom::{ZoomBar, ZoomBarLabels, ZoomBarResponse, ZoomBarStyle};

/// A number typed into the value field, with or without its suffix. The suffix itself is taken
/// off first, so one with a digit in it (`24fps`) is not cut short; then whatever is left of a
/// half-typed one. A decimal comma counts as a point.
fn parse_typed(text: &str, suffix: &str) -> Option<f64> {
    let t = text.trim();
    let t = if suffix.is_empty() { t } else { t.strip_suffix(suffix).unwrap_or(t) };
    t.trim_end_matches(|c: char| !c.is_ascii_digit()).trim().replace(',', ".").parse().ok()
}

#[cfg(test)]
mod tests;

/// Colors for a [`Slider`], built from the application's own tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderStyle {
    /// The unfilled part of the track.
    pub track: Color32,
    /// The track from its start to the thumb, usually the accent.
    pub fill: Color32,
    /// The thumb's face and border.
    pub thumb: Color32,
    pub thumb_stroke: Stroke,
    /// The dot in the thumb's centre, usually the accent; it grows while hovered or dragged.
    pub dot: Color32,
    /// The tick at the default value ([`Slider::mark`]).
    pub mark: Color32,
    /// An outline around the thumb while the slider has keyboard focus; `Stroke::NONE` opts out.
    pub focus_stroke: Stroke,
}

impl SliderStyle {
    /// A style derived from egui visuals: the accent from `selection.bg_fill`, the track from the
    /// noninteractive border, the thumb from the window fill. Applications with design tokens
    /// should construct the style from those instead.
    pub fn from_visuals(v: &egui::Visuals) -> Self {
        Self {
            track: v.widgets.noninteractive.bg_stroke.color,
            fill: v.selection.bg_fill,
            thumb: v.window_fill,
            thumb_stroke: Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color),
            dot: v.selection.bg_fill,
            mark: v.weak_text_color(),
            focus_stroke: v.selection.stroke,
        }
    }
}

/// How positions on the track map to values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scale {
    #[default]
    Linear,
    /// Equal distances are equal ratios (zoom: 50% and 200% sit equally far from 100%). The range
    /// must be positive; otherwise the slider falls back to linear.
    Logarithmic,
}

/// A slider; see the module documentation. Build it, then call [`Slider::show`].
pub struct Slider<'a> {
    value: &'a mut f64,
    range: RangeInclusive<f64>,
    label: &'a str,
    style: &'a SliderStyle,
    width: f32,
    scale: Scale,
    step: Option<f64>,
    mark: Option<f64>,
    field: Option<(usize, &'a str)>,
}

const HEIGHT: f32 = 20.0;
const THUMB_RADIUS: f32 = 7.0;

impl<'a> Slider<'a> {
    /// `label` names the slider for assistive technology and becomes its tooltip.
    pub fn new(value: &'a mut f64, range: RangeInclusive<f64>, label: &'a str, style: &'a SliderStyle) -> Self {
        Self { value, range, label, style, width: 110.0, scale: Scale::Linear, step: None, mark: None, field: None }
    }

    /// The track's width in points (default 110).
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn scale(mut self, scale: Scale) -> Self {
        self.scale = scale;
        self
    }

    /// Snap values to multiples of `step` (zoom: whole percent). Without it the value is continuous.
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step).filter(|s| s.is_finite() && *s > 0.0);
        self
    }

    /// The default value: a tick on the track, and double-clicking the track returns to it.
    pub fn mark(mut self, value: f64) -> Self {
        self.mark = Some(value).filter(|v| v.is_finite());
        self
    }

    /// Show an editable value field after the track, with `decimals` decimals and a `suffix`
    /// (`%`, `×`, ` pt`). The field accepts a comma or a point and ignores the suffix when typed.
    pub fn editable(mut self, decimals: usize, suffix: &'a str) -> Self {
        self.field = Some((decimals.min(6), suffix));
        self
    }

    fn log(&self) -> bool {
        self.scale == Scale::Logarithmic && *self.range.start() > 0.0 && *self.range.end() > 0.0
    }

    /// Track fraction (0–1) of `v`.
    fn fraction(&self, v: f64) -> f64 {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        if hi <= lo {
            return 0.0;
        }
        let f = if self.log() { (v.max(lo).ln() - lo.ln()) / (hi.ln() - lo.ln()) } else { (v - lo) / (hi - lo) };
        if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 }
    }

    /// The value at track fraction `f`, snapped to the step and clamped to the range.
    fn value_at(&self, f: f64) -> f64 {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let f = f.clamp(0.0, 1.0);
        let v = if self.log() { (lo.ln() + f * (hi.ln() - lo.ln())).exp() } else { lo + f * (hi - lo) };
        self.snap(v)
    }

    fn snap(&self, v: f64) -> f64 {
        let (lo, hi) = (*self.range.start(), *self.range.end());
        let v = match self.step {
            Some(s) => (v / s).round() * s,
            None => v,
        };
        if hi >= lo { v.clamp(lo, hi) } else { lo }
    }

    fn text(&self, v: f64, decimals: usize, suffix: &str) -> String {
        format!("{v:.decimals$}{suffix}")
    }

    /// Paint and handle the slider; the response reports `changed()` when the value changed
    /// this frame (by pointer, keyboard, accessibility action or the field) and
    /// `drag_stopped()` when a drag on the track ended.
    pub fn show(self, ui: &mut Ui) -> Response {
        // The track and its value field stay on one row, whatever the surrounding layout.
        if self.field.is_some() && !ui.layout().is_horizontal() {
            return ui.horizontal(|ui| self.show_row(ui)).inner;
        }
        self.show_row(ui)
    }

    fn show_row(self, ui: &mut Ui) -> Response {
        if !self.value.is_finite() {
            *self.value = *self.range.start();
        }
        let enabled = ui.is_enabled();
        let (rect, mut response) = ui.allocate_exact_size(vec2(self.width.max(2.0 * THUMB_RADIUS + 4.0), HEIGHT), Sense::click_and_drag());
        let (x0, x1) = (rect.left() + THUMB_RADIUS, rect.right() - THUMB_RADIUS);
        let drag_id = response.id.with("drag-fraction");
        let mut thumb_fraction = self.fraction(*self.value);

        if enabled {
            if response.double_clicked()
                && let Some(mark) = self.mark
            {
                let v = self.snap(mark);
                if v != *self.value {
                    *self.value = v;
                    response.mark_changed();
                }
                thumb_fraction = self.fraction(v);
            } else if let Some(p) = response.interact_pointer_pos()
                && (response.dragged() || response.clicked() || response.drag_started())
            {
                let f = f64::from(((p.x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0));
                ui.data_mut(|d| d.insert_temp(drag_id, f));
                thumb_fraction = f;
                let v = self.value_at(f);
                if v != *self.value {
                    *self.value = v;
                    response.mark_changed();
                }
            }
            // The release frame has no drag position of its own: report where the drag ended, so a
            // caller that applies on release (and re-seeds `value` from its model every frame) gets
            // the dragged-to value, not the one it started from.
            if response.drag_stopped()
                && let Some(f) = ui.data(|d| d.get_temp::<f64>(drag_id))
            {
                let v = self.value_at(f);
                if v != *self.value {
                    *self.value = v;
                    response.mark_changed();
                }
            }
            if response.drag_stopped() || !response.dragged() {
                ui.data_mut(|d| d.remove::<f64>(drag_id));
            }

            let (mut down, mut up) = (0i32, 0i32);
            // Home and End go straight to an end of the range instead of counting steps.
            let mut jump = None;
            if response.has_focus() {
                ui.memory_mut(|m| {
                    m.set_focus_lock_filter(response.id, EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
                });
                ui.input(|i| {
                    down += i.num_presses(Key::ArrowLeft) as i32 + i.num_presses(Key::ArrowDown) as i32 + 10 * i.num_presses(Key::PageDown) as i32;
                    up += i.num_presses(Key::ArrowRight) as i32 + i.num_presses(Key::ArrowUp) as i32 + 10 * i.num_presses(Key::PageUp) as i32;
                    if i.key_pressed(Key::Home) {
                        jump = Some(*self.range.start());
                    }
                    if i.key_pressed(Key::End) {
                        jump = Some(*self.range.end());
                    }
                });
            }
            ui.input(|i| {
                use egui::accesskit::Action;
                down += i.num_accesskit_action_requests(response.id, Action::Decrement) as i32;
                up += i.num_accesskit_action_requests(response.id, Action::Increment) as i32;
            });
            let steps = up - down;
            if steps != 0 || jump.is_some() {
                let v = match (jump, self.step) {
                    (Some(end), _) => self.snap(end),
                    (None, Some(s)) => self.snap(*self.value + f64::from(steps) * s),
                    (None, None) => self.value_at(self.fraction(*self.value) + f64::from(steps) * 0.01),
                };
                if v != *self.value {
                    *self.value = v;
                    response.mark_changed();
                }
                thumb_fraction = self.fraction(v);
            }
        }

        // Paint. While dragging, the thumb sits under the pointer even if the value snapped.
        if response.dragged()
            && let Some(f) = ui.data(|d| d.get_temp::<f64>(drag_id))
        {
            thumb_fraction = f;
        }
        let s = self.style;
        let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
        let y = rect.center().y;
        let x = x0 + thumb_fraction as f32 * (x1 - x0);
        let track = Rect::from_min_max(pos2(x0, y - 2.0), pos2(x1, y + 2.0));
        let painter = ui.painter();
        painter.rect_filled(track, CornerRadius::same(2), fade(s.track));
        painter.rect_filled(Rect::from_min_max(track.min, pos2(x, track.max.y)), CornerRadius::same(2), fade(s.fill));
        if let Some(mark) = self.mark {
            let mx = x0 + self.fraction(mark) as f32 * (x1 - x0);
            painter.line_segment([pos2(mx, y - 6.0), pos2(mx, y + 6.0)], Stroke::new(1.0, fade(s.mark)));
        }
        let hot = enabled && (response.hovered() || response.dragged());
        let center = pos2(x, y);
        painter.circle(center, THUMB_RADIUS, fade(s.thumb), Stroke::new(s.thumb_stroke.width, fade(s.thumb_stroke.color)));
        painter.circle_filled(center, if hot { 4.5 } else { 3.5 }, fade(s.dot));
        if response.has_focus() && s.focus_stroke != Stroke::NONE {
            let r = Rect::from_center_size(center, Vec2::splat(2.0 * THUMB_RADIUS + 4.0));
            painter.rect_stroke(r, CornerRadius::same(THUMB_RADIUS as u8 + 2), s.focus_stroke, StrokeKind::Outside);
        }

        let value = *self.value;
        let label = self.label.to_string();
        response.widget_info(|| egui::WidgetInfo::slider(enabled, value, &label));
        let mut response = response.on_hover_text(self.label);

        if let Some((decimals, suffix)) = self.field {
            // A fixed width (the wider end of the range), so the field never resizes.
            let font = egui::TextStyle::Body.resolve(ui.style());
            let widest = [*self.range.start(), *self.range.end()]
                .iter()
                .map(|v| ui.fonts_mut(|f| f.layout_no_wrap(self.text(*v, decimals, suffix), font.clone(), Color32::WHITE).size().x))
                .fold(0.0f32, f32::max);
            let suffix_owned = suffix.to_string();
            let typed_suffix = suffix.trim().to_string();
            let range = self.range.clone();
            let mut v = *self.value;
            let field = ui.add_sized(
                vec2(widest + 2.0 * ui.spacing().button_padding.x + 4.0, HEIGHT),
                egui::DragValue::new(&mut v)
                    .range(range)
                    .speed(0.0)
                    .custom_formatter(move |v, _| format!("{v:.decimals$}{suffix_owned}"))
                    .custom_parser(move |t| parse_typed(t, &typed_suffix)),
            );
            if field.changed() {
                let v = self.snap(v);
                if v != *self.value {
                    *self.value = v;
                    response.mark_changed();
                }
            }
            response = response.union(field);
        }
        response
    }
}

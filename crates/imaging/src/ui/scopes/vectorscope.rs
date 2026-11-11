//! [`VectorscopePlot`]: graticule, targets and the hover target ([`scope_point`]).

use super::words::ScopeWords;
use craft_behavior::{Axis, Metrics, Sense};
use craft_ui::contract::{Access, Common, Component, Cx, MeasureCx, PaintUi, Props as _, Role, WordSetCx as _};
use craft_ui::fonts::FontRef;
use craft_ui::primitives::{DocRun, DocText, HAlign, PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx, SurfaceKind};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Vec2, vec2};

/// The rings' alpha on `text.faint`.
const RING_ALPHA: f32 = 0.45;
/// The spokes' alpha on `text.faint`.
const SPOKE_ALPHA: f32 = 0.4;
/// The share of its full opacity a cloud dot has at the least density, so a sparse bin stays visible.
const DENSITY_FLOOR: f32 = 0.2;
/// The smallest half-side of a cloud dot.
const DOT_MIN: f32 = 0.7;
/// The largest half-side of a cloud dot.
const DOT_MAX: f32 = 3.0;
/// The margin between the outer ring and the plot's edge.
const MARGIN: f32 = 14.0;
/// The radius of the dot at the end of a spoke.
const SPOKE_DOT: f32 = 2.0;
/// Where the letters sit, as a multiple of the outer ring's radius.
const LETTER_RADIUS: f32 = 1.11;
/// The skin-tone line's angle, in degrees of hue.
const SKIN_TONE_DEGREES: f32 = 23.0;
/// The hover target's circle radius and the arm of its cross.
const HOVER_RADIUS: f32 = 5.0;
const ARM_SHARE: f32 = 0.55;
/// The plot's smallest and largest default height, in control heights.
const MIN_ROWS: f32 = 3.6;
const MAX_ROWS: f32 = 7.8;

/// The vectorscope's counts for one pass: a hue-major grid of `hues × saturations` cells
/// ([`Vectorscope::counts`](crate::scopes::Vectorscope::counts)) and the sample under the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct VectorscopeFrame<'a> {
    /// `hues × saturations` counts, hue-major.
    pub counts: &'a [u32],
    /// Hue cells ([`HUES`](crate::scopes::HUES)).
    pub hues: usize,
    /// Saturation cells ([`SATURATIONS`](crate::scopes::SATURATIONS)).
    pub saturations: usize,
    /// The hue (turns) and saturation (0..=1) of the pixel under the pointer: the hover target.
    pub hover: Option<[f32; 2]>,
}

impl<'a> VectorscopeFrame<'a> {
    /// The counts of `hues x saturations` cells, hue-major, and the hover target if any.
    pub const fn new(counts: &'a [u32], hues: usize, saturations: usize, hover: Option<[f32; 2]>) -> Self {
        VectorscopeFrame { counts, hues, saturations, hover }
    }
}

impl VectorscopeFrame<'_> {
    /// The AccessKit value of the plot, [`ScopeWords::vectorscope_value`] filled from the
    /// counts, the numbers in `format`'s digits ([`Vectorscope::summary`](crate::scopes::Vectorscope::summary)'s rules); `None`
    /// when nothing was counted.
    pub fn summary(&self, words: &ScopeWords, format: &craft_i18n::Format) -> Option<String> {
        if self.saturations < 2 || self.hues == 0 {
            return None;
        }
        let mut peak_saturation: Option<f32> = None;
        let (mut sin, mut cos) = (0.0f64, 0.0f64);
        for (index, count) in self.counts.iter().enumerate().filter(|(_, count)| **count > 0) {
            let hue = (index / self.saturations) as f32 / self.hues as f32;
            let saturation = (index % self.saturations) as f32 / (self.saturations - 1) as f32;
            peak_saturation = Some(peak_saturation.map_or(saturation, |peak| peak.max(saturation)));
            let weight = f64::from(*count) * f64::from(saturation);
            let angle = f64::from(hue) * std::f64::consts::TAU;
            sin += weight * angle.sin();
            cos += weight * angle.cos();
        }
        let peak_saturation = peak_saturation?;
        let mean_hue = if sin == 0.0 && cos == 0.0 { 0.0 } else { sin.atan2(cos).to_degrees().rem_euclid(360.0) };
        let saturation = craft_i18n::ltr(&format.number(f64::from(peak_saturation) * 100.0, 0));
        let hue = craft_i18n::ltr(&format.number(mean_hue, 0));
        Some(words.vectorscope_value.fill(&[("saturation", &saturation), ("hue", &hue)]))
    }
}

/// The radius of the rings' outer edge in a vectorscope drawn in `rect`: half the shorter side
/// less `margin` (the vector margin), at least 1.
fn ring_radius(rect: Rect, margin: f32) -> f32 {
    (rect.width().min(rect.height()) * 0.5 - margin).max(1.0)
}

/// Where hue `hue` (turns) at `saturation` (0..=1) sits in a vectorscope drawn in `rect`: the
/// radius is half the shorter side less `margin` (the vector margin), red at the 3 o'clock position when `red_right`, else
/// at −100°, hue turning counter-clockwise. The cloud, the targets and the hover target use it.
pub fn scope_point(rect: Rect, margin: f32, red_right: bool, hue: f32, saturation: f32) -> Pos2 {
    // pc: crates/ui-egui/src/dialogs/camera_raw_scope_ui.rs:443 `scope_point`.
    let radius = ring_radius(rect, margin);
    let offset = if red_right { 0.0 } else { -100.0f32.to_radians() };
    let angle = offset - hue * std::f32::consts::TAU;
    rect.center() + vec2(angle.cos(), angle.sin()) * saturation * radius
}

/// The colour of hue `hue` (turns) at `saturation`, at the brightness of the dark data colours
/// (`level`, 0..=1): PhotoCraft's `scope_hue`.
fn scope_hue(hue: f32, saturation: f32, level: f32) -> Color32 {
    // pc: crates/ui-egui/src/theme.rs:806 `scope_hue`.
    let channel = |offset: f32| {
        let k = (offset + hue * 6.0).rem_euclid(6.0);
        (level * (1.0 - saturation * k.min(4.0 - k).clamp(0.0, 1.0)) * 255.0) as u8
    };
    Color32::from_rgb(channel(5.0), channel(3.0), channel(1.0))
}

/// The style of the vectorscope: dark in every theme, so the plot sits in a dark theme scope.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct VectorscopeStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The plot's well.
    well: StyleColor,
    radius: CornerRadius,
    ring: StyleColor,
    spoke: StyleColor,
    skin_tone: StyleColor,
    /// The hover target's inner and outer strokes.
    target_inner: StyleColor,
    target_outer: StyleColor,
    /// The arm of the hover target's cross.
    target_arm: f32,
    /// The brightness of the hue colours, from `data.red`.
    level: f32,
    letter_size: f32,
    /// The default height range.
    height: [f32; 2],
}

/// The vectorscope plot, usable alone or under the [`ScopePanel`](super::ScopePanel).
///
/// **What the user sees.** On `data.scope`, rounded like a control well: rings at 0.25, 0.5, 0.75 and 1 of the radius
/// in `text.faint`; spokes to R, Y, G, C, B, M, each ending in a dot and its letter in the hue's colour; the skin-tone
/// line at 23 degrees in `text.muted` (unless off); the cloud as square dots of half-side `radius / (saturations - 1)`
/// clamped, coloured by hue and saturation at an alpha that grows with the count; the hover target as a circle and a
/// cross. It never mirrors.
///
/// **AccessKit.** An image named by the label with the value text of [`VectorscopeFrame::summary`].
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(content_painter)]
pub struct VectorscopePlot<'a> {
    #[common]
    common: Common,
    frame: &'a VectorscopeFrame<'a>,
    /// Show Red at 3 o'clock (else red sits at -100 degrees).
    #[prop]
    red_right: bool,
    /// Show Skin Tone Indicator.
    #[prop]
    skin_tone: bool,
}

impl<'a> VectorscopePlot<'a> {
    /// A plot of `frame` named `label`; red at -100 degrees and the skin-tone line on by default.
    pub fn new(label: impl Into<Label>, frame: &'a VectorscopeFrame<'a>) -> Self {
        VectorscopePlot { common: Common::new(), frame, red_right: false, skin_tone: true }.label(label)
    }
}

impl Component for VectorscopePlot<'_> {
    const ROLE: Role = Role::IMAGE;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    type Style = VectorscopeStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: VectorscopePlotStyleKey) -> VectorscopeStyle {
        let colors = &cx.tokens.color;
        let rows = cx.control().height;
        VectorscopeStyle {
            metrics: Metrics::new(),
            well: cx.color(colors.data.scope),
            radius: cx.surface(SurfaceKind::Well).radius,
            ring: cx.color(colors.text.faint.faded(RING_ALPHA)),
            spoke: cx.color(colors.text.faint.faded(SPOKE_ALPHA)),
            skin_tone: cx.color(colors.text.muted),
            target_inner: cx.color(colors.text.on_strong),
            target_outer: cx.color(colors.data.scope),
            target_arm: cx.control().icon * ARM_SHARE,
            level: f32::from(colors.data.red.0[0]) / 255.0,
            letter_size: cx.tokens.typography.caption.size,
            height: [rows * MIN_ROWS, rows * MAX_ROWS],
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(cx.style.height[0], cx.style.height[1])
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<ScopeWords>();
        if let Some(value) = self.frame.summary(&words, cx.format()) {
            access.value(value);
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (rect, style, frame) = (cx.rect, cx.style, self.frame);
        let red_right = self.red_right;
        let point = |hue: f32, saturation: f32| scope_point(rect, MARGIN, red_right, hue, saturation);
        let (center, radius) = (rect.center(), ring_radius(rect, MARGIN));
        let draw = ui.draw();
        draw.fill(&PaintShape::Rect(rect, style.radius), style.well);
        for ring in [0.25, 0.5, 0.75, 1.0] {
            draw.stroke(&PaintShape::Circle(center, radius * ring), StrokeWidth::Hairline, style.ring);
        }
        let painter = ui.content_painter(cx);
        let peak = frame.counts.iter().copied().max().unwrap_or(1).max(1) as f32;
        if frame.saturations > 1 && frame.hues > 0 {
            let dot = (radius / (frame.saturations - 1) as f32).clamp(DOT_MIN, DOT_MAX);
            for (index, count) in frame.counts.iter().enumerate().filter(|(_, count)| **count > 0) {
                let hue = (index / frame.saturations) as f32 / frame.hues as f32;
                let saturation = (index % frame.saturations) as f32 / (frame.saturations - 1) as f32;
                let color = scope_hue(hue, saturation, style.level);
                let square = Rect::from_center_size(point(hue, saturation), vec2(dot * 2.0, dot * 2.0));
                draw.faded(density_share(*count as f32 / peak), |dot| {
                    dot.data_fill(&PaintShape::Rect(square, CornerRadius::ZERO), DataColor::new(color))
                });
            }
        }
        for (index, letter) in ["R", "Y", "G", "C", "B", "M"].into_iter().enumerate() {
            let hue = index as f32 / 6.0;
            let color = scope_hue(hue, 1.0, style.level);
            draw.stroke(&PaintShape::Path(vec![center, point(hue, 1.0)]), StrokeWidth::Hairline, style.spoke);
            draw.data_fill(&PaintShape::Circle(point(hue, 1.0), SPOKE_DOT), DataColor::new(color));
            let runs = [DocRun::new(letter, FontRef::proportional(), style.letter_size, DataColor::new(color))];
            let mut document = DocText::new(&runs);
            document.align = HAlign::Center;
            let text = painter.layout(&document);
            let at = point(hue, LETTER_RADIUS) - text.size() / 2.0;
            painter.painter.galley(at, text.galley.clone(), color);
        }
        if self.skin_tone {
            draw.stroke(&PaintShape::Path(vec![center, point(SKIN_TONE_DEGREES / 360.0, 1.0)]), StrokeWidth::Thin, style.skin_tone);
        }
        if let Some([hue, saturation]) = frame.hover {
            let at = point(hue, saturation);
            for (width, color) in [(StrokeWidth::Medium, style.target_outer), (StrokeWidth::Thin, style.target_inner)] {
                draw.stroke(&PaintShape::Circle(at, HOVER_RADIUS), width, color);
                draw.stroke(&PaintShape::Path(vec![at - vec2(style.target_arm, 0.0), at + vec2(style.target_arm, 0.0)]), width, color);
                draw.stroke(&PaintShape::Path(vec![at - vec2(0.0, style.target_arm), at + vec2(0.0, style.target_arm)]), width, color);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// How opaque a dot is for a bin holding `density` (0 to 1) of the densest bin's count: from [`DENSITY_FLOOR`] up to
/// full, rising with the square root so mid-density bins read clearly.
fn density_share(density: f32) -> f32 {
    DENSITY_FLOOR + (1.0 - DENSITY_FLOOR) * density.sqrt()
}

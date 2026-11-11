//! `Progress`: a horizontal bar, filled by a fraction or moving a segment while the extent is unknown.

use crate::contract::{Access, Animate as _, Common, Component, Cx, MeasureCx, Numeric, PaintUi, Role, Styled as _, WordSetCx as _};
use crate::primitives::{PaintShape, StrokeWidth};
use crate::style::{ComponentStyle, Flags, StyleColor, StyleCx, TrackLook};
use craft_behavior::{Flow, Metrics, Sense};
use craft_theme::variant::{Emphasis, Tone};
use craft_theme::{Stateful, Variant};
use egui::{CornerRadius, Rect, Response, Vec2};

#[cfg(feature = "testing")]
mod stories;

/// Seconds for the unknown-extent segment to cross the track.
const SWEEP: f64 = 1.25;
/// The segment's length as a fraction of the track.
const SEGMENT: f32 = 0.3;
/// Where the still segment starts, as a fraction of the track.
const STILL_START: f32 = 0.3;
/// Dash and gap lengths of the still segment, in track heights.
const DASH: f32 = 2.0;
const DASH_GAP: f32 = 1.0;

/// The style of a progress bar.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[style(variants = [Variant::PRIMARY, Variant::look(Emphasis::Solid, Tone::Good), Variant::look(Emphasis::Solid, Tone::Warn), Variant::DANGER])]
#[non_exhaustive]
pub struct ProgressStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The groove and its thickness, as every value track has them.
    track: TrackLook,
    /// The filled part: the solid fill of the variant's tone.
    fill: Stateful<StyleColor>,
    radius: CornerRadius,
    length: f32,
}

/// What a bar shows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Amount {
    Fraction(f32),
    Unknown,
}

/// A progress bar. The variant picks the fill's tone: accent by default, or a good, warning or danger fill. The fill grows from the reading start, so it mirrors under right-to-left, and is never shorter
/// than the bar is thick unless it is empty. While the extent is unknown a segment moves along the track; where
/// motion is off the segment stands still and is drawn as dashes, so it cannot be mistaken for a fill. AccessKit
/// reports a progress indicator with the fraction as its value, none while the extent is unknown.
#[derive(Clone, crate::contract::Component)]
pub struct Progress {
    #[common]
    common: Common,
    amount: Amount,
}

impl Progress {
    /// A bar `fraction` full, clamped to 0..=1; a fraction that is not a number counts as empty.
    pub fn new(fraction: f32) -> Self {
        Progress { common: Common::new(), amount: Amount::Fraction(fraction) }
    }

    /// A bar of unknown extent.
    pub fn indeterminate() -> Self {
        Progress { common: Common::new(), amount: Amount::Unknown }
    }

    /// `fraction` as drawn and reported: clamped to 0..=1, and 0 when it is not a number.
    pub fn clamped(fraction: f32) -> f32 {
        if fraction.is_nan() { 0.0 } else { fraction.clamp(0.0, 1.0) }
    }

    fn fraction(&self) -> Option<f32> {
        match self.amount {
            Amount::Fraction(fraction) => Some(Progress::clamped(fraction)),
            Amount::Unknown => None,
        }
    }
}

/// The still segment of an unknown extent, as dashes over a track `width` wide and `thickness` thick.
fn dashes(width: f32, thickness: f32) -> Vec<(f32, f32)> {
    let (dash, gap) = (DASH * thickness / width.max(1.0), DASH_GAP * thickness / width.max(1.0));
    let end = STILL_START + SEGMENT;
    let mut spans = Vec::new();
    let mut at = STILL_START;
    while at < end {
        spans.push((at, (at + dash).min(end)));
        at += dash + gap;
    }
    spans
}

/// The track of a bar: a sunken, edged groove `size.meter` high.
fn bar_track(cx: &StyleCx<'_>) -> TrackLook {
    let colors = &cx.tokens.color;
    let mut track = cx.track();
    let sunken = cx.color(colors.surface.sunken);
    track.unfilled = Stateful::new(sunken);
    track.thickness = cx.tokens.size.meter;
    track
}

impl Component for Progress {
    const ROLE: Role = Role::PROGRESS_INDICATOR;
    const PARTS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = ProgressStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: ProgressStyleKey) -> ProgressStyle {
        ProgressStyle {
            metrics: Metrics::new(),
            track: bar_track(cx),
            fill: cx.variant_look().fill,
            radius: crate::style::UiTokens::corner(cx.tokens.shape.radius.pill),
            length: cx.tokens.size.length.track,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(cx.style.length, cx.style.track.thickness)
    }

    fn sense(&self) -> Sense {
        super::display_sense(&self.common, true)
    }

    fn flags(&self) -> Flags {
        if self.fraction().is_none() { Flags::BUSY } else { Flags::empty() }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        match self.fraction() {
            Some(fraction) => {
                let text = cx.format().percent(f64::from(fraction), 0);
                access.numeric(Numeric::new(f64::from(fraction), 0.0..=1.0).text(text));
            }
            None => {
                access.numeric(Numeric::empty(0.0..=1.0));
                access.busy(true);
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let thickness = cx.style.track.thickness;
        let track = Rect::from_center_size(cx.content_rect.center(), Vec2::new(cx.content_rect.width(), thickness));
        let width = track.width();
        let spans = match self.fraction() {
            Some(fraction) if fraction > 0.0 => vec![(0.0, fraction.max(thickness / width.max(1.0)).min(1.0))],
            Some(_) => Vec::new(),
            None => match cx.phase(SWEEP, None) {
                Some(phase) => {
                    let start = -SEGMENT + phase * (1.0 + SEGMENT);
                    vec![(start.max(0.0), (start + SEGMENT).min(1.0))]
                }
                None => dashes(width, thickness),
            },
        };
        let flow = Flow::new(track, cx.flow.is_rtl());
        let color = cx.resolve(&cx.style.fill);
        let draw = ui.draw();
        let outline = PaintShape::Rect(track, cx.style.radius);
        draw.fill(&outline, cx.resolve(&cx.style.track.unfilled));
        draw.stroke(&outline, StrokeWidth::Thin, cx.resolve(&cx.style.track.edge));
        for (from, to) in spans {
            let span = flow.span(from * width, (to - from) * width);
            if span.width() > 0.0 {
                draw.fill(&PaintShape::Rect(span, cx.style.radius), color);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

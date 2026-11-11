//! The channel row: a label that scrubs a slider whose track shows the channel's colours, and a number field after
//! it. Shared with the panels that show channels.

use std::ops::RangeInclusive;

use craft_behavior::{Axis, Sense};
use craft_theme::{Channel, ColorModel, Srgba};
use craft_ui::contract::{Bind, Common, Component, Cx, EditResponse, MeasureCx, Role, Show as _};
use craft_ui::form::SliderField;
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::{Label, UnitKind};
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use craft_ui::value_drag::TrackFace;
use egui::{Color32, CornerRadius, Ui, Vec2};

/// How a channel's value steps and settles: a channel's or the alpha's.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChannelRule {
    pub range: RangeInclusive<f64>,
    pub step: f64,
    pub decimals: u8,
}

impl ChannelRule {
    /// The alpha row's rule: 0 to 100 %, whole percents.
    pub(crate) const ALPHA: ChannelRule = ChannelRule { range: RangeInclusive::new(0.0, 100.0), step: 1.0, decimals: 0 };

    pub(crate) fn of(channel: &Channel) -> ChannelRule {
        ChannelRule { range: f64::from(channel.min)..=f64::from(channel.max), step: f64::from(channel.step), decimals: channel.decimals }
    }
}

/// What a channel's number counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChannelUnit {
    None,
    Percent,
    Degrees,
    /// Any other unit text a model declares.
    Text(&'static str),
}

impl ChannelUnit {
    pub(crate) fn of(unit: &'static str) -> ChannelUnit {
        match unit {
            "" => ChannelUnit::None,
            "%" => ChannelUnit::Percent,
            "°" => ChannelUnit::Degrees,
            other => ChannelUnit::Text(other),
        }
    }
}

/// Channel `channel` of `model`'s accessible name: the channel's name, or its qualified name ("OKLCH hue") when
/// another of the `visible` models has a channel of that name.
pub(crate) fn channel_name(model: &dyn ColorModel, channel: usize, visible: &[&dyn ColorModel]) -> Label {
    let ch = model.channels()[channel];
    let clash = visible.iter().any(|m| m.id() != model.id() && m.channels().iter().any(|other| other.name == ch.name));
    Label::from(if clash { ch.qualified } else { ch.name })
}

/// The style of a track ramp: the line round its edge.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TrackRampStyle {
    #[style(metrics)]
    metrics: craft_behavior::Metrics,
    edge: StyleColor,
    /// The ramp's own size when nothing sets it: a value track's length and groove.
    length: f32,
    thickness: f32,
}

/// The look of a slider's track that shows colours: stops spread evenly from the start to the end, over the
/// transparency checkerboard where `checker` is set.
#[derive(Clone, craft_ui::contract::Component)]
#[component(content_painter)]
pub(crate) struct TrackRamp {
    #[common]
    common: Common,
    stops: Vec<Srgba>,
    checker: bool,
}

impl TrackRamp {
    pub(crate) fn new(stops: Vec<Srgba>, checker: bool) -> Self {
        TrackRamp { common: Common::new(), stops, checker }
    }
}

impl Component for TrackRamp {
    const ROLE: Role = Role::IMAGE;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const OVERLAYS: bool = false;
    type Style = TrackRampStyle;
    type Output = egui::Response;

    fn style(cx: &StyleCx<'_>, _: TrackRampStyleKey) -> TrackRampStyle {
        TrackRampStyle {
            metrics: craft_behavior::Metrics::new(),
            edge: cx.color(cx.tokens.color.line.control_edge),
            length: cx.tokens.size.length.track,
            thickness: cx.tokens.size.ramp.groove,
        }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(cx.style.length, cx.style.thickness)
    }

    fn paint(&mut self, ui: &mut craft_ui::contract::PaintUi<'_>, cx: &Cx<'_, Self>) {
        let rect = cx.rect;
        let painter = ui.content_painter(cx);
        if self.checker {
            painter.checker(rect);
        }
        let last = self.stops.len().saturating_sub(1).max(1) as f32;
        let stops: Vec<(f32, DataColor)> =
            self.stops.iter().enumerate().map(|(i, color)| (i as f32 / last, DataColor::new(Color32::from(*color)))).collect();
        painter.ramp(rect, &stops, Axis::X);
        ui.draw().stroke(&PaintShape::Rect(rect, CornerRadius::ZERO), StrokeWidth::Thin, cx.style.edge);
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

/// The slider and number field of a channel, over `value`, with `track` colours on the groove.
pub(crate) fn channel_slider<'a>(
    name: Label,
    value: Bind<'a, f64>,
    rule: &ChannelRule,
    unit: ChannelUnit,
    track: Option<(Vec<Srgba>, bool)>,
) -> SliderField<'a> {
    let field = SliderField::new(name, value, rule.range.clone()).step(rule.step);
    let field = match unit {
        ChannelUnit::None => field,
        ChannelUnit::Percent => field.unit_kind(UnitKind::Percent),
        ChannelUnit::Degrees => field.unit_kind(UnitKind::Degrees),
        ChannelUnit::Text(text) => field.unit(Label::from(text)),
    };
    match track {
        Some((stops, checker)) => field.track_face(TrackFace::new(TrackRamp::new(stops, checker))),
        None => field,
    }
}

/// One channel: the word label and number field on a line over the slider across the channel's colours.
#[derive(craft_ui::contract::Pattern)]
pub(crate) struct ChannelRow<'a> {
    label: Label,
    value: Bind<'a, f64>,
    rule: ChannelRule,
    track: Option<Vec<Srgba>>,
    /// Whether the track shows over the transparency checkerboard.
    checker: bool,
    unit: ChannelUnit,
}

impl<'a> ChannelRow<'a> {
    pub(crate) fn new(label: impl Into<Label>, value: Bind<'a, f64>, rule: ChannelRule) -> Self {
        ChannelRow { label: label.into(), value, rule, track: None, checker: false, unit: ChannelUnit::None }
    }

    /// The colours of the track, from the channel's start to its end.
    pub(crate) fn track(mut self, stops: Vec<Srgba>) -> Self {
        self.track = Some(stops);
        self
    }

    /// The track over the transparency checkerboard, for the colours that carry alpha.
    pub(crate) fn checker(mut self, on: bool) -> Self {
        self.checker = on;
        self
    }

    pub(crate) fn unit(mut self, unit: ChannelUnit) -> Self {
        self.unit = unit;
        self
    }

    /// Shows the row; the edit of the slider or the field, `None` when the row did not run.
    pub(crate) fn show(self, ui: &mut Ui) -> Option<EditResponse<f64>> {
        let ChannelRow { label, mut value, rule, track, checker, unit } = self;
        Some(channel_slider(label, value.reborrowed(), &rule, unit, track.map(|stops| (stops, checker))).show(ui))
    }
}

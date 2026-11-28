//! The channel row: on one line, a label that scrubs, a slider whose track shows the channel's colours, and a number
//! field after it. Shared with the panels that show channels.

use std::ops::RangeInclusive;

use craft_behavior::press::MULTI_CLICK_TIME;
use craft_behavior::{Axis, Metrics, PartKey, PressPhase, Sense};
use craft_theme::{Channel, ColorModel, Srgba};
use craft_ui::contract::{
    Bind, Common, Component, Cx, DisabledLook, EditResponse, Gap, GlyphLabel, LabelKind, MeasureCx, NoValue, Outcome, PartsCx, Pointer as _, Role,
    Show as _, ShowContainer as _, SideLabel, Sizing,
};
use craft_ui::form::{Field, SliderField, SliderFieldLayout};
use craft_ui::layout_components::Row;
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::{Label, UnitKind};
use craft_ui::runtime::pointer;
use craft_ui::style::{ComponentStyle, DataColor, StyleColor, StyleCx};
use craft_ui::value_drag::{SliderThumb, TrackFace};
use egui::{Color32, CornerRadius, Ui, Vec2};

/// How a channel's value steps and settles: a channel's or the alpha's.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ChannelRule {
    /// The values the channel takes, from its least to its greatest.
    pub range: RangeInclusive<f64>,
    /// One arrow key's step.
    pub step: f64,
    /// The decimals the number field shows.
    pub decimals: u8,
}

impl ChannelRule {
    /// The alpha row's rule: 0 to 100 %, whole percents.
    pub const ALPHA: ChannelRule = ChannelRule { range: RangeInclusive::new(0.0, 100.0), step: 1.0, decimals: 0 };

    /// The rule of `channel`.
    pub fn of(channel: &Channel) -> ChannelRule {
        ChannelRule { range: f64::from(channel.min)..=f64::from(channel.max), step: f64::from(channel.step), decimals: channel.decimals }
    }
}

/// What a channel's number counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChannelUnit {
    /// No unit.
    None,
    /// Percent.
    Percent,
    /// Degrees.
    Degrees,
    /// Any other unit text a model declares.
    Text(&'static str),
}

impl ChannelUnit {
    /// The unit a model declares as text.
    pub fn of(unit: &'static str) -> ChannelUnit {
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
pub struct TrackRamp {
    #[common]
    common: Common,
    stops: Vec<Srgba>,
    checker: bool,
}

impl TrackRamp {
    /// A ramp through `stops`, over the checkerboard where `checker` is set.
    pub fn new(stops: Vec<Srgba>, checker: bool) -> Self {
        TrackRamp { common: Common::new(), stops, checker }
    }
}

impl Component for TrackRamp {
    const DISABLED: DisabledLook = DisabledLook::Faded;
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
            thickness: cx.tokens.size.ramp.bar,
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
        painter.ramp(rect, egui::CornerRadius::ZERO, &stops, Axis::X);
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

/// One channel on one line, as Photoshop's Color panel draws it: the label at the start (the channel's short label
/// where one is given, "C"), which scrubs the value, the slider over the channel's colours with the pointer under its
/// track, and the number field with its unit at the end, as wide as its widest value.
#[derive(craft_ui::contract::Pattern)]
pub struct ChannelRow<'a> {
    label: Label,
    /// The text shown in place of the label: the channel's short label.
    short: Option<&'static str>,
    value: Bind<'a, f64>,
    rule: ChannelRule,
    track: Option<Vec<Srgba>>,
    /// Whether the track shows over the transparency checkerboard.
    checker: bool,
    unit: ChannelUnit,
    marks: &'a [f64],
}

/// What [`ChannelRow::show_with_units`] reports.
#[non_exhaustive]
pub struct ChannelRowOutput {
    /// The edit of the slider or the field, `None` when the row did not run.
    pub edit: Option<EditResponse<f64>>,
    /// The units cell was double-clicked in this pass: the host switches the units the row counts in.
    pub units_double_clicked: bool,
}

impl<'a> ChannelRow<'a> {
    /// A row named `label` over `value`, ruled by `rule`.
    pub fn new(label: impl Into<Label>, value: Bind<'a, f64>, rule: ChannelRule) -> Self {
        ChannelRow { label: label.into(), short: None, value, rule, track: None, checker: false, unit: ChannelUnit::None, marks: &[] }
    }

    /// Shows `short` at the line's start in place of the label, which stays the row's name.
    pub fn short(mut self, short: &'static str) -> Self {
        self.short = Some(short).filter(|short| !short.is_empty());
        self
    }

    /// The colours of the track, from the channel's start to its end.
    pub fn track(mut self, stops: Vec<Srgba>) -> Self {
        self.track = Some(stops);
        self
    }

    /// The track over the transparency checkerboard, for the colours that carry alpha.
    pub fn checker(mut self, on: bool) -> Self {
        self.checker = on;
        self
    }

    /// What the number counts.
    pub fn unit(mut self, unit: ChannelUnit) -> Self {
        self.unit = unit;
        self
    }

    /// The ticks under the track, passed to `SliderField::marks`; not the labelled form, whose caption line has no room
    /// in a channel row.
    #[must_use]
    pub fn marks(mut self, marks: &'a [f64]) -> Self {
        self.marks = marks;
        self
    }

    /// Shows the row; the edit of the slider or the field, `None` when the row did not run.
    pub fn show(self, ui: &mut Ui) -> Option<EditResponse<f64>> {
        self.show_row(ui, false).edit
    }

    /// Shows the row with a blank units cell after the number field, for a row whose host can switch the units the
    /// value counts in: the edit of the slider or the field, and whether the cell was double-clicked. The cell is
    /// highlighted under the pointer and takes no keyboard focus, so the host offers the same switch as a command; in
    /// a disabled container it does not report a double click. The slider gives up the cell's width, so the row keeps
    /// its width.
    pub fn show_with_units(self, ui: &mut Ui) -> ChannelRowOutput {
        self.show_row(ui, true)
    }

    fn show_row(self, ui: &mut Ui, units_cell: bool) -> ChannelRowOutput {
        let ChannelRow { label, short, mut value, rule, track, checker, unit, marks } = self;
        let side = match short {
            Some(short) => SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::text(Label::from(short)))),
            None => SideLabel::new(),
        };
        let mut field = |ui: &mut Ui| {
            Field::new(label.clone())
                .side(side.clone().scrub(true))
                .show(ui, |ui, _| {
                    channel_slider(label.clone(), value.reborrowed(), &rule, unit, track.clone().map(|stops| (stops, checker)))
                        .marks(marks)
                        .layout(SliderFieldLayout::Inline)
                        .field_sizing(Sizing::Content)
                        .thumb(SliderThumb::Triangle)
                        .show(ui)
                        .edit
                })
                .inner
        };
        if !units_cell {
            return ChannelRowOutput { edit: field(ui), units_double_clicked: false };
        }
        let shown = Row::new().fill_width().gap(Gap::Tight).show(ui, |ui, _| {
            let edit = field(ui);
            let cell = UnitsCell::new().show(ui);
            (edit, cell.double_clicked)
        });
        let (edit, units_double_clicked) = shown.inner.unwrap_or((None, false));
        ChannelRowOutput { edit, units_double_clicked }
    }
}

/// The units cell's one part, which takes the pointer.
const UNITS: PartKey = PartKey::new("units");

/// The style of a units cell: its size.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct UnitsCellStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The cell's size: `size.icon.glyph_sm` wide, a control high, so it fills the row's line.
    size: Vec2,
}

/// The blank cell after a channel row's number field ([`ChannelRow::show_with_units`]): highlighted under the pointer, it
/// reports a double click. It has no keyboard stop and no node: the host's command that switches the units is its
/// keyboard path.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(sample = UnitsCell::new)]
struct UnitsCell {
    #[common]
    common: Common,
    double_clicked: bool,
}

impl UnitsCell {
    fn new() -> Self {
        UnitsCell { common: Common::new(), double_clicked: false }
    }
}

/// What a units cell reports: its response and whether it was double-clicked in this pass.
struct UnitsCellOutput {
    response: egui::Response,
    double_clicked: bool,
}

impl Outcome for UnitsCellOutput {
    type Value = NoValue;

    fn response(&self) -> &egui::Response {
        &self.response
    }
}

impl Component for UnitsCell {
    const ROLE: Role = Role::None;
    const OVERLAYS: bool = false;
    const STOPS: bool = false;
    type Style = UnitsCellStyle;
    type Output = UnitsCellOutput;

    fn style(cx: &StyleCx<'_>, _: UnitsCellStyleKey) -> UnitsCellStyle {
        UnitsCellStyle { metrics: Metrics::new(), size: Vec2::new(cx.tokens.size.icon.glyph_sm, cx.control().height) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        cx.style.size
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: egui::Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(UNITS, rect).sense(Sense::CLICK).state_layer().pointer_only("the host's command switches the units");
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let first = cx.id.with("units-first-click");
        let ctx = &cx.response.ctx;
        // A press off the cell ends the cell's click run, even when it starts a new count that the next click on the
        // cell continues.
        if pointer::frame(ctx).press.is_some_and(|press| !cx.response.rect.contains(press.origin)) {
            ctx.data_mut(|data| data.remove::<(f64, u32)>(first));
        }
        let PressPhase::Release { start, click: true, .. } = cx.part(UNITS).press.phase else { return };
        // The click count also counts a press just beside the cell, on the number field, so a click is the second of a
        // double click only when the click just before it, by the count, was a click on this cell.
        let earlier = ctx.data(|data| data.get_temp::<(f64, u32)>(first));
        let follows = earlier.is_some_and(|(time, clicks)| start.clicks == clicks + 1 && start.time - time <= MULTI_CLICK_TIME);
        if start.clicks >= 2 && follows {
            self.double_clicked = true;
            ctx.data_mut(|data| data.remove::<(f64, u32)>(first));
        } else {
            ctx.data_mut(|data| data.insert_temp(first, (start.time, start.clicks)));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> UnitsCellOutput {
        UnitsCellOutput { response: cx.response.clone(), double_clicked: self.double_clicked }
    }
}

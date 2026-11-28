//! Tone zones ([`ToneZone`], [`ZoneScale`], [`ZoneValues`], [`ToneZones`]): Camera Raw's and Lightroom's draggable
//! ranges over the histogram.

use super::words::ScopeWords;
use craft_behavior::Mixed;
use craft_behavior::{AccessAction, CursorIcon, Metrics, NavMove, PartKey, PressPhase, Sense};
use craft_ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Focus as _, Input, Name, Numeric, PartsCx, Pointer as _, Props as _,
    Remember as _, Role, Tooltip, TooltipSpec, WordSetCx as _,
};
use craft_ui::contract::{MeasureCx, PaintUi};
use craft_ui::primitives::PaintShape;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{ActivateKeys, KeyUse};
use craft_ui::style::{ComponentStyle, FocusLook, StyleColor, StyleCx};
use egui::{CornerRadius, Rect, Vec2};
use std::ops::RangeInclusive;

/// One zone's slider node and part; the index names the zone.
pub(super) const ZONE: PartKey = PartKey::new("zone");

/// Shift drags a tenth as far (LightCraft's fine factor).
const FINE: f32 = 0.1;

/// The steps one arrow press with Shift held takes.
const LARGE_STEP: i32 = 10;

/// The alpha of the hovered zone's band over `text.normal`.
const BAND_ALPHA: f32 = 0.055;

/// One tone zone over the plot: a range of the plot's width (0..=1) that edits one of the host's controls. Zones are
/// data: PhotoCraft's Camera Raw and LightCraft's Basic panel disagree, so each app passes its own set.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ToneZone {
    /// A stable key the host maps to its control ("blacks", "shadows", "exposure", "highlights", "whites").
    pub key: &'static str,
    /// Where the zone starts, as a share of the plot width.
    pub from: f32,
    /// Where it ends.
    pub to: f32,
}

impl ToneZone {
    /// Camera Raw's five zones: Blacks 0-0.10, Shadows 0.10-0.25, Exposure 0.25-0.75, Highlights 0.75-0.90, Whites
    /// 0.90-1.
    pub const CAMERA_RAW: [ToneZone; 5] = [
        ToneZone { key: "blacks", from: 0.0, to: 0.10 },
        ToneZone { key: "shadows", from: 0.10, to: 0.25 },
        ToneZone { key: "exposure", from: 0.25, to: 0.75 },
        ToneZone { key: "highlights", from: 0.75, to: 0.90 },
        ToneZone { key: "whites", from: 0.90, to: 1.0 },
    ];

    /// Lightroom's five zones: Blacks 0-0.15, Shadows 0.15-0.35, Exposure 0.35-0.65, Highlights 0.65-0.85, Whites
    /// 0.85-1.
    pub const LIGHTROOM: [ToneZone; 5] = [
        ToneZone { key: "blacks", from: 0.0, to: 0.15 },
        ToneZone { key: "shadows", from: 0.15, to: 0.35 },
        ToneZone { key: "exposure", from: 0.35, to: 0.65 },
        ToneZone { key: "highlights", from: 0.65, to: 0.85 },
        ToneZone { key: "whites", from: 0.85, to: 1.0 },
    ];

    /// The zone's name: the [`ScopeWords`] word for the five standard keys, else the key.
    pub fn label<'l>(&self, words: &'l ScopeWords) -> &'l str {
        match self.key {
            "blacks" => words.blacks.as_str(),
            "shadows" => words.shadows.as_str(),
            "exposure" => words.exposure.as_str(),
            "highlights" => words.highlights.as_str(),
            "whites" => words.whites.as_str(),
            other => other,
        }
    }

    /// The zone at `x`, a share of the plot width (clamped to 0..=1): the first zone whose `to` lies past `x`, the
    /// last one at 1.
    pub fn at(zones: &[ToneZone], x: f32) -> Option<usize> {
        let x = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
        zones.iter().position(|zone| zone.to > x).or(zones.len().checked_sub(1))
    }
}

/// How the control one zone edits moves, as the host's slider row for it has it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ZoneScale {
    /// The lowest value.
    pub min: f32,
    /// The highest value.
    pub max: f32,
    /// The change a drag across the plot's full width makes (PhotoCraft: twice the range's end, so 10 stops for
    /// Exposure and 200 for the others). Shift drags a tenth as far.
    pub drag: f32,
    /// The change of one arrow key press or screen reader Increment.
    pub step: f32,
    /// The value a double click on the zone restores.
    pub reset: f32,
}

impl ZoneScale {
    /// A control over `range` that a drag across the plot's width moves by `drag`, an arrow key by `step`, and a
    /// double click resets to `reset`.
    pub fn new(range: RangeInclusive<f32>, drag: f32, step: f32, reset: f32) -> Self {
        ZoneScale { min: *range.start(), max: *range.end(), drag, step, reset }
    }

    /// `value` held to the range.
    fn hold(&self, value: f32) -> f32 {
        value.max(self.min).min(self.max)
    }
}

/// The values of the controls the tone zones edit, one per zone in the zones' order, each in its control's units.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ZoneValues(pub Vec<f32>);

impl ZoneValues {
    /// The values of the zones in order, one per zone.
    pub fn new(values: Vec<f32>) -> Self {
        Self(values)
    }

    /// These values with zone `at` set to what `to` makes of its value and scale, held to the zone's range; `None`
    /// when the zone has no value or no scale.
    fn with(&self, scales: &[ZoneScale], at: usize, to: impl FnOnce(f32, &ZoneScale) -> f32) -> Option<ZoneValues> {
        let scale = scales.get(at)?;
        let mut next = self.clone();
        let value = next.0.get_mut(at)?;
        *value = scale.hold(to(*value, scale));
        Some(next)
    }
}

/// The style of the tone zones: the hovered zone's band and the strip along the plot's top that belongs to the clipping
/// warnings.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ToneZonesStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// The band over the zone the pointer or the focus is on.
    band: StyleColor,
    /// The height of the strip the clipping warnings keep.
    strip: f32,
    radius: CornerRadius,
}

/// The zone the pointer pressed or the keys last moved to.
#[derive(Clone, Default)]
struct Held {
    focused: Option<usize>,
    pressed: Option<usize>,
}

/// The tone zones over a plot, shown at the plot's rect with `show_at`: one Tab stop for the plot, a part per zone. A
/// press locks the zone under it for the whole drag; the hovered or pressed zone shows its band; Page Up and Page Down
/// move the focused zone, the arrows step it and a double click resets it.
///
/// The value is the [`ZoneValues`] of the controls the zones edit, each moved as its [`ZoneScale`] says: a drag sets
/// the pressed zone's value to its value at the press plus the travel's share of the plot width times
/// [`ZoneScale::drag`], one gesture; each arrow press, Increment, Decrement, SetValue and double
/// click is a one-shot change. An app maps the values to its controls. AccessKit: a group named
/// [`ScopeWords::tone_zones`] holding one slider per zone, named by its label, with the zone's value and range.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct ToneZones<'a> {
    #[common]
    common: Common,
    /// The values of the controls the zones edit.
    #[prop]
    value: Bind<'a, ZoneValues>,
    zones: &'a [ToneZone],
    scales: &'a [ZoneScale],
    texts: &'a [Label],
}

impl<'a> ToneZones<'a> {
    /// The zones over the plot, with each zone's scale and value text in the same order (the text is the tooltip's
    /// and the slider's value).
    pub fn new(label: impl Into<Label>, zones: &'a [ToneZone], scales: &'a [ZoneScale], texts: &'a [Label]) -> Self {
        ToneZones { common: Common::new(), value: Bind::kept(), zones, scales, texts }.label(label).ltr_axis()
    }

    fn value_text(&self, zone: usize) -> &str {
        self.texts.get(zone).map_or("", Label::as_str)
    }

    /// The rect of a zone below the strip.
    fn zone_rect(&self, rect: Rect, strip: f32, zone: &ToneZone) -> Rect {
        let width = rect.width();
        Rect::from_min_max(
            egui::pos2(rect.left() + zone.from * width, (rect.top() + strip).min(rect.bottom())),
            egui::pos2(rect.left() + zone.to * width, rect.bottom()),
        )
    }
}

impl Component for ToneZones<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_moves().with_value(true).with_activate(ActivateKeys::Neither);
    type Style = ToneZonesStyle;
    type Output = EditResponse<ZoneValues>;

    fn style(cx: &StyleCx<'_>, _: ToneZonesStyleKey) -> ToneZonesStyle {
        ToneZonesStyle {
            metrics: Metrics::new(),
            focus: cx.focus_look(),
            band: cx.color(cx.tokens.color.text.normal.faded(BAND_ALPHA)),
            strip: cx.control().icon,
            radius: CornerRadius::ZERO,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let words = cx.words::<ScopeWords>();
        let strip = cx.style.strip;
        for (at, zone) in self.zones.iter().enumerate() {
            let tip = words.zone_tooltip.fill(&[("zone", zone.label(&words)), ("value", self.value_text(at))]);
            cx.part(ZONE.at(at as u64), self.zone_rect(rect, strip, zone))
                .sense(Sense::DRAG | Sense::CLICK)
                .cursor(CursorIcon::ResizeHorizontal)
                .tooltip(Tooltip::from(TooltipSpec::new(Label::from(tip))))
                .pointer_only("the arrow keys step the focused zone and Page Up and Page Down move between zones");
        }
    }

    fn bind(&mut self) -> Option<BindMut<'_, ZoneValues>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let zones = self.zones;
        if zones.is_empty() {
            return;
        }
        let scales = self.scales;
        let width = cx.rect.width().max(1.0);
        // A mixed value steps and drags from the zones' reset values, as a slider steps from its middle.
        let resets = ZoneValues(scales.iter().map(|scale| scale.reset).collect());
        // The value now with zone `at` set by `to`: arrows, Increment, Decrement, SetValue and a reset.
        let shown = |cx: &Cx<'_, Self>, at: usize, to: &dyn Fn(f32, &ZoneScale) -> f32| cx.value().unwrap_or(&resets).with(scales, at, to);
        for at in 0..zones.len() {
            let press = cx.part(ZONE.at(at as u64)).press;
            let fine = if press.mods.shift { FINE } else { 1.0 };
            // The gesture's start with the zone moved by the travel since the press.
            let dragged = |cx: &Cx<'_, Self>, origin: egui::Pos2, pos: egui::Pos2| {
                let moved = |value: f32, scale: &ZoneScale| value + (pos.x - origin.x) / width * fine * scale.drag;
                match cx.gesture_before()? {
                    Mixed::Same(start) => start.with(scales, at, moved),
                    Mixed::Mixed => resets.with(scales, at, moved),
                }
            };
            match press.phase {
                PressPhase::Down { .. } => {
                    cx.focus_own();
                    cx.memory::<Held>().focused = Some(at);
                }
                PressPhase::DragStart { start, pos, .. } => {
                    cx.focus_own();
                    let held = cx.memory::<Held>();
                    held.focused = Some(at);
                    held.pressed = Some(at);
                    cx.begin();
                    if let Some(values) = dragged(cx, start.origin, pos) {
                        cx.propose(values);
                    }
                }
                PressPhase::Drag { start, pos, .. } => {
                    if let Some(values) = dragged(cx, start.origin, pos) {
                        cx.propose(values);
                    }
                }
                PressPhase::Release { start, pos, click, .. } => {
                    cx.memory::<Held>().pressed = None;
                    if cx.gesture().is_some() {
                        if let Some(values) = dragged(cx, start.origin, pos) {
                            cx.propose(values);
                        }
                        cx.commit();
                    } else if click
                        && start.clicks >= 2
                        && let Some(values) = shown(cx, at, &|_: f32, scale: &ZoneScale| scale.reset)
                    {
                        cx.set(values);
                    }
                }
                _ => {}
            }
        }
        let running = cx.gesture().is_some();
        let rtl = cx.flow.is_rtl();
        while let Some((movement, mods)) = cx.intents().take_move() {
            if running {
                continue;
            }
            let focused = cx.memory::<Held>().focused.unwrap_or_else(|| ToneZone::at(zones, 0.5).unwrap_or(0)).min(zones.len() - 1);
            let times = if mods.extend { LARGE_STEP } else { 1 };
            let steps = match movement {
                NavMove::PageUp | NavMove::PageDown => {
                    let next = if movement == NavMove::PageUp { focused + 1 } else { focused.saturating_sub(1) };
                    cx.memory::<Held>().focused = Some(next.min(zones.len() - 1));
                    continue;
                }
                NavMove::Up => times,
                NavMove::Down => -times,
                // The plot never mirrors: right raises the value in either direction.
                NavMove::Forward => {
                    if rtl {
                        -times
                    } else {
                        times
                    }
                }
                NavMove::Back => {
                    if rtl {
                        times
                    } else {
                        -times
                    }
                }
                _ => continue,
            };
            cx.memory::<Held>().focused = Some(focused);
            if let Some(values) = shown(cx, focused, &|value: f32, scale: &ZoneScale| value + steps as f32 * scale.step) {
                cx.set(values);
            }
        }
        for at in 0..zones.len() {
            let key = ZONE.at(at as u64);
            if let Some(to) = cx.intents().take_set_value_at(key).filter(|value| value.is_finite() && !running)
                && let Some(values) = shown(cx, at, &|_: f32, _: &ZoneScale| to as f32)
            {
                cx.set(values);
            }
        }
        while let Some(request) = cx.intents().take_access() {
            let Some(at) = (0..zones.len()).find(|at| request.target == Some(ZONE.at(*at as u64))) else { continue };
            let steps = match request.action {
                AccessAction::Increment => 1,
                AccessAction::Decrement => -1,
                _ => continue,
            };
            if !running && let Some(values) = shown(cx, at, &|value: f32, scale: &ZoneScale| value + steps as f32 * scale.step) {
                cx.set(values);
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<ScopeWords>();
        for (at, zone) in self.zones.iter().enumerate() {
            let node = access.node(ZONE.at(at as u64), Role::SLIDER);
            node.name = Name::Text(zone.label(&words).to_owned());
            let Some(scale) = self.scales.get(at) else { continue };
            let range = f64::from(scale.min)..=f64::from(scale.max);
            let numeric = match cx.value().and_then(|values| values.0.get(at)) {
                Some(value) => Numeric::new(f64::from(*value), range),
                None => Numeric::empty(range),
            };
            node.numeric = Some(numeric.step(f64::from(scale.step)).text(self.value_text(at).to_owned()));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let held = cx.recall::<Held>();
        let over = |at: &usize| {
            let phase = cx.part(ZONE.at(*at as u64)).press.phase;
            matches!(phase, PressPhase::Hover) || phase.button_down()
        };
        let hovered = (0..self.zones.len()).find(over);
        let shown = held.pressed.or(hovered).or(cx.is_focused().then_some(held.focused).flatten());
        let Some(zone) = shown.and_then(|at| self.zones.get(at)) else { return };
        let band = PaintShape::Rect(self.zone_rect(cx.rect, 0.0, zone), cx.style.radius);
        ui.draw().fill(&band, cx.style.band);
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<ZoneValues> {
        cx.edit_response()
    }
}

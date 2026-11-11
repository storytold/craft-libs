//! Tone zones ([`ToneZone`], [`ScopeZoneEdit`], [`ToneZones`]): Camera Raw's and Lightroom's draggable ranges over the
//! histogram.

use super::words::ScopeWords;
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

/// One zone's slider node and part; the index names the zone.
pub(super) const ZONE: PartKey = PartKey::new("zone");

/// Shift drags a tenth as far (LightCraft's fine factor).
const FINE: f32 = 0.1;

/// The slider nodes accept any value: the zone's range belongs to the host's control.
const UNBOUNDED: f64 = 1.0e9;

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

/// What the user did to one zone.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ZoneChange {
    /// A drag over the plot, one gesture: the signed share of the plot width moved since the press (Shift drags a
    /// tenth as far). The host maps the share to its control (PhotoCraft: `share x limit x 2`, limit 5 for Exposure,
    /// else 100) and makes one undo step for the drag.
    Drag(f32),
    /// Arrow keys with the plot focused, or a screen reader's Increment and Decrement: this many of the control's own
    /// steps (Right and Up +1, Left and Down -1, Shift x10). One undo step each.
    Step(i32),
    /// A double click on the zone: the host resets the control to its default. One undo step.
    Reset,
    /// A screen reader's SetValue, in the control's own units.
    SetValue(f64),
}

/// A tone-zone edit.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScopeZoneEdit {
    /// The zone's index in the slice given to [`ToneZones::new`].
    pub zone: usize,
    /// The zone's key.
    pub key: &'static str,
    /// What happened.
    pub change: ZoneChange,
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
/// The value is the last edit; an app reads the edit events and maps them to its controls. AccessKit: a group named
/// [`ScopeWords::tone_zones`] holding one slider per zone, named by its label.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct ToneZones<'a> {
    #[common]
    common: Common,
    /// The last edit, bound so the host can show it as mixed or refuse it.
    #[prop]
    value: Bind<'a, ScopeZoneEdit>,
    zones: &'a [ToneZone],
    values: &'a [Label],
}

impl<'a> ToneZones<'a> {
    /// The zones over the plot, each zone's value text in the same order (the tooltip and the slider's value).
    pub fn new(label: impl Into<Label>, zones: &'a [ToneZone], values: &'a [Label]) -> Self {
        ToneZones { common: Common::new(), value: Bind::kept(), zones, values }.label(label).ltr_axis()
    }

    fn value_text(&self, zone: usize) -> &str {
        self.values.get(zone).map_or("", Label::as_str)
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
    type Output = EditResponse<ScopeZoneEdit>;

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

    fn bind(&mut self) -> Option<BindMut<'_, ScopeZoneEdit>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let zones = self.zones;
        if zones.is_empty() {
            return;
        }
        let width = cx.rect.width().max(1.0);
        let edit = |zone: usize, change: ZoneChange| ScopeZoneEdit { zone, key: zones[zone].key, change };
        for at in 0..zones.len() {
            let press = cx.part(ZONE.at(at as u64)).press;
            let fine = if press.mods.shift { FINE } else { 1.0 };
            let share = |origin: egui::Pos2, pos: egui::Pos2| (pos.x - origin.x) / width * fine;
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
                    cx.propose(edit(at, ZoneChange::Drag(share(start.origin, pos))));
                }
                PressPhase::Drag { start, pos, .. } => cx.propose(edit(at, ZoneChange::Drag(share(start.origin, pos)))),
                PressPhase::Release { start, pos, click, .. } => {
                    cx.memory::<Held>().pressed = None;
                    if cx.gesture().is_some() {
                        cx.propose(edit(at, ZoneChange::Drag(share(start.origin, pos))));
                        cx.commit();
                    } else if click && start.clicks >= 2 {
                        cx.set(edit(at, ZoneChange::Reset));
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
            cx.set(edit(focused, ZoneChange::Step(steps)));
        }
        for at in 0..zones.len() {
            let key = ZONE.at(at as u64);
            if let Some(value) = cx.intents().take_set_value_at(key).filter(|value| value.is_finite() && !running) {
                cx.set(edit(at, ZoneChange::SetValue(value)));
            }
        }
        while let Some(request) = cx.intents().take_access() {
            let Some(at) = (0..zones.len()).find(|at| request.target == Some(ZONE.at(*at as u64))) else { continue };
            let steps = match request.action {
                AccessAction::Increment => 1,
                AccessAction::Decrement => -1,
                _ => continue,
            };
            if !running {
                cx.set(edit(at, ZoneChange::Step(steps)));
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<ScopeWords>();
        for (at, zone) in self.zones.iter().enumerate() {
            let node = access.node(ZONE.at(at as u64), Role::SLIDER);
            node.name = Name::Text(zone.label(&words).to_owned());
            node.numeric = Some(Numeric::new(0.0, -UNBOUNDED..=UNBOUNDED).step(1.0).text(self.value_text(at).to_owned()));
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

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<ScopeZoneEdit> {
        cx.edit_response()
    }
}

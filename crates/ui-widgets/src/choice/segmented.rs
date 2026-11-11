//! `Segmented`: one of a few options as a strip of segments. A leaf with a part per segment; each segment wears a
//! face that is flat and muted, with a hover fill, and takes the selection fill and edge while it is picked. The row is one Tab stop over its segments: arrows, Home and End move focus between them, Space
//! or a click picks the focused one.

use crate::buttons::face::{self, FaceLook, FaceSpec};
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Entry, Input, MeasureCx, Name, PaintUi, PartsCx, Props as _, Role,
    SetPosition,
};
use crate::primitives::IconSpec;
use crate::runtime::env::Label;
use crate::runtime::focus::{ActivateKeys, KeyUse};
use crate::style::{ComponentStyle, Flags, FocusLook, IconLook, StyleCx, SurfaceLook, TextLook, TypeRole, UiTokens};
use craft_behavior::{Flow, Metrics, NavShape, PartKey, PartRef, Roving, Sense, Value};
use craft_theme::{Rgba, Stateful, States};
use egui::{Rect, Vec2};

const SEGMENT: PartKey = PartKey::new("segment");

/// The style of a segmented row.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SegmentedStyle {
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    /// Between neighbouring segments.
    gap: f32,
    height: f32,
}

/// A row of segments over one value. A `Mixed` binding picks none.
#[derive(crate::contract::Component)]
#[component(probe(Segmented<'static, u8>))]
pub struct Segmented<'a, T: Value + 'static> {
    #[common]
    common: Common,
    value: Bind<'a, T>,
    entries: &'a [Entry<T>],
}

impl<'a, T: Value + 'static> Segmented<'a, T> {
    /// A row over `value` named `label`, one segment per entry. An entry's label is its text, or its tooltip and name
    /// when it has an icon; separators are not segments.
    pub fn new(label: impl Into<Label>, value: Bind<'a, T>, entries: &'a [Entry<T>]) -> Self {
        debug_assert!(entries.iter().all(|entry| !entry.is_separator()), "a `Segmented` has no separators");
        Segmented { common: Common::new(), value, entries }.label(label)
    }

    fn part(at: usize) -> PartRef {
        SEGMENT.at(at as u64)
    }

    fn position_of(&self, value: Option<&T>) -> Option<usize> {
        let value = value?;
        self.entries.iter().position(|entry| entry.value() == Some(value))
    }

    /// The face segment `entry` wears: icon only when the entry has an icon.
    fn face(entry: &Entry<T>) -> Segment {
        Segment::new(entry.label().clone(), entry.icon_spec().cloned()).enabled(entry.is_enabled())
    }

    /// Each segment's width: its face's own measure.
    fn widths(&self, cx: &MeasureCx<'_, Self>) -> Vec<f32> {
        self.entries.iter().map(|entry| cx.face(Self::face(entry)).x).collect()
    }
}

impl<T: Value + 'static> Component for Segmented<'_, T> {
    const ROLE: Role = Role::RADIO_GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_activate(ActivateKeys::Space);
    type Style = SegmentedStyle;
    type Output = EditResponse<T>;

    fn style(cx: &StyleCx<'_>, _: SegmentedStyleKey<T>) -> SegmentedStyle {
        let control = cx.control();
        SegmentedStyle {
            metrics: Metrics::new().min_size(Vec2::new(0.0, control.height)),
            focus: cx.focus_look(),
            gap: cx.tokens.space.gap.item,
            height: control.height,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        let widths = self.widths(cx);
        let gaps = widths.len().saturating_sub(1) as f32;
        Vec2::new(widths.iter().sum::<f32>() + cx.style.gap * gaps, cx.style.height)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let widths = self.widths(cx);
        let gap = cx.style.gap;
        let current = self.position_of(cx.value());
        let mut flow = Flow::new(rect, cx.flow.is_rtl());
        for (at, (entry, width)) in self.entries.iter().zip(widths).enumerate() {
            let (cell, rest) = flow.take_start(width);
            flow = rest.take_start(gap).1;
            let mut part = cx.part(Self::part(at), cell);
            if entry.icon_spec().is_some() {
                part = part.tooltip(entry.label().clone());
            }
            if !entry.is_enabled() {
                continue;
            }
            let part = part.sense(Sense::CLICK | Sense::FOCUS).focusable();
            if current == Some(at) {
                part.flags(Flags::CHECKED).selected();
            }
        }
        cx.roving(Roving::new(NavShape::Flat));
    }

    fn bind(&mut self) -> Option<BindMut<'_, T>> {
        Some(self.value.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let current = self.position_of(cx.value());
        for (at, entry) in self.entries.iter().enumerate() {
            let hit = cx.part_activated(Self::part(at)).is_some();
            if let Some(value) = entry.value().filter(|_| entry.is_enabled() && hit && current != Some(at)) {
                cx.set(value.clone());
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        for (at, entry) in self.entries.iter().enumerate() {
            let node = access.node(Self::part(at), Role::RADIO_BUTTON);
            node.name = Name::Text(cx.env.resolve(entry.label()).as_str().to_owned());
            node.position = Some(SetPosition::new(at).of(self.entries.len()));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, _: &Cx<'_, Self>) {
        for (at, entry) in self.entries.iter().enumerate() {
            ui.face(Self::part(at), Self::face(entry));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<T> {
        cx.edit_response()
    }
}

/// The style of one segment: no fill or edge until hovered or picked, then the theme's selection fill and edge.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct SegmentStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    #[style(text)]
    text: TextLook,
    #[style(icon)]
    icon: IconLook,
    face: FaceLook,
}

/// What a segment of a `Segmented` shows: a label, or an icon with the label as its tooltip and name.
#[derive(crate::contract::Component)]
#[component(sample = Segment::sample)]
struct Segment {
    #[common]
    common: Common,
    icon: Option<IconSpec<'static>>,
}

impl Segment {
    #[cfg(feature = "testing")]
    fn sample() -> Self {
        Segment::new(Label::from("Segment"), None)
    }

    fn new(label: Label, icon: Option<IconSpec<'static>>) -> Self {
        Segment { common: Common::new(), icon }.label(label)
    }

    fn spec<'s>(&'s self, label: &'s str) -> FaceSpec<'s> {
        FaceSpec { label: if self.icon.is_some() { "" } else { label }, role: None, lead: self.icon.as_ref(), trail: None, loading: false }
    }
}

impl Component for Segment {
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    type Style = SegmentStyle;
    type Output = egui::Response;

    fn shows_label(&self) -> bool {
        self.icon.is_none()
    }

    fn style(cx: &StyleCx<'_>, _: SegmentStyleKey) -> SegmentStyle {
        let colors = &cx.tokens.color;
        let c = |rgba| cx.color(rgba);
        let (clear, hover, selected) = (c(Rgba::TRANSPARENT), c(colors.state.hover), c(colors.state.selected));
        let disabled = c(colors.state.disabled_fill);
        let fill = Stateful::new(clear).off(States::new(clear, hover, hover, clear)).on(States::new(selected, selected, selected, disabled));
        let edge = Stateful::new(c(colors.line.control_edge)).off(States::new(clear, clear, clear, clear)).on(States::new(
            c(colors.line.control_edge),
            c(colors.line.control_edge),
            c(colors.line.control_edge),
            c(colors.line.divider),
        ));
        let (muted, on_selected, off) = (c(colors.text.muted), c(colors.text.on_selected), c(colors.text.disabled));
        let ink = Stateful::new(muted).off(States::new(muted, muted, muted, off)).on(States::new(on_selected, on_selected, on_selected, off));
        let control = cx.segment_control();
        SegmentStyle {
            surface: SurfaceLook::new(fill).radius(UiTokens::corner(cx.tokens.shape.radius.control)).border(edge, cx.tokens.stroke.thin),
            metrics: control.metrics(),
            focus: face::focus(cx),
            text: TextLook::new(TypeRole::Label, ink.clone()),
            icon: IconLook::new(control.icon, ink),
            face: FaceLook::of(cx),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        face::measure(cx, &self.spec(cx.resolved.label))
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        face::paint(ui, cx, (&cx.style.text, &cx.style.icon), &cx.style.face, &self.spec(cx.resolved.label), cx.content_rect);
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

#[cfg(feature = "testing")]
mod stories;

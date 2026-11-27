//! `Segmented`: one of a few options as a strip of segments. A leaf with a part per segment; each segment wears a
//! face that is flat and muted, with a hover fill, and takes the selection fill and edge while it is picked. The row is one Tab stop over its segments: arrows, Home and End move focus between them, Space,
//! Enter or a click picks the focused one. In a column narrower than the strip, the segments that do not fit move to
//! the next line; in a row or bar the strip keeps one line.

use crate::buttons::ActiveLook;
use crate::buttons::face::{self, FaceLook, FaceSpec};
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Cx, Edit as _, EditResponse, Entry, MeasureCx, Name, PaintUi, PartsCx, Props as _, Role, SetPosition,
};
use crate::primitives::IconSpec;
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, Flags, FocusLook, IconLook, StyleCx, SurfaceKind, SurfaceLook, TextLook, TypeRole, UiTokens};
use craft_behavior::{Flow, Metrics, NavShape, PartKey, PartRef, Proposal, Roving, Sense, Value};
use craft_theme::{Rgba, Stateful, States};
use egui::{Rect, Vec2, pos2, vec2};

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

    /// The segments measured and flowed into lines `room` wide. At the least proposal the room is the widest segment,
    /// so each segment keeps its whole face. In a row or bar the strip keeps one line, so its least is the whole line
    /// and the row's shrink step never wraps it.
    fn arrange(&self, cx: &MeasureCx<'_, Self>, room: f32) -> Placement {
        let wants = self.widths(cx);
        let room = if cx.in_row() {
            f32::INFINITY
        } else if cx.proposal == Proposal::Min {
            wants.iter().copied().fold(0.0, f32::max).min(room)
        } else {
            room
        };
        place(&wants, room, cx.style.gap)
    }
}

/// Where the segments sit: each segment's line and its start offset in the line, in reading order.
struct Placement {
    /// The width each segment wants: its face's measure.
    wants: Vec<f32>,
    /// Each segment's width, cut to the line.
    widths: Vec<f32>,
    cells: Vec<(usize, f32)>,
    lines: usize,
    /// The extent of the longest line.
    extent: f32,
}

/// Flows segments `wants` wide into lines `room` wide, `gap` apart: a segment that does not fit what is left of a
/// line starts the next, and a segment wider than a line is cut to the line. With widths 36, 36 and 36 in a room of
/// 90 and a gap of 8 the first two share line 0 (at 0 and 44, ending at 80) and the third starts line 1 at 0.
fn place(wants: &[f32], room: f32, gap: f32) -> Placement {
    let mut cells = Vec::with_capacity(wants.len());
    let mut widths = Vec::with_capacity(wants.len());
    let (mut line, mut at, mut extent) = (0, 0.0_f32, 0.0_f32);
    for &want in wants {
        let width = want.min(room);
        if at > 0.0 && at + width > room {
            line += 1;
            at = 0.0;
        }
        cells.push((line, at));
        widths.push(width);
        extent = extent.max(at + width);
        at += width + gap;
    }
    let lines = if wants.is_empty() { 0 } else { line + 1 };
    Placement { wants: wants.to_vec(), widths, cells, lines, extent }
}

impl Placement {
    /// The strip's size: its longest line, and its lines `height` tall and `gap` apart.
    fn size(&self, height: f32, gap: f32) -> Vec2 {
        let lines = self.lines.max(1) as f32;
        Vec2::new(self.extent, lines * height + (lines - 1.0) * gap)
    }

    /// The rect of segment `at` in `rect`, its lines sharing the height `rect` leaves after the gaps between them, and
    /// each line's start edge the reading start. A segment that reaches past `rect` is cut at its edges.
    fn rect(&self, at: usize, rect: Rect, gap: f32, rtl: bool) -> Rect {
        let (line, start) = self.cells[at];
        let lines = self.lines.max(1) as f32;
        let height = ((rect.height() - (lines - 1.0) * gap) / lines).max(0.0);
        let top = rect.min.y + line as f32 * (height + gap);
        let line = Rect::from_min_size(pos2(rect.min.x, top), vec2(rect.width(), height));
        let cell = Flow::new(line, rtl).span(start, self.widths[at]);
        let clamp = |x: f32| x.clamp(line.min.x, line.max.x);
        Rect::from_x_y_ranges(clamp(cell.min.x)..=clamp(cell.max.x), cell.y_range())
    }
}

impl<T: Value + 'static> Component for Segmented<'_, T> {
    const ROLE: Role = Role::RADIO_GROUP;
    const OVERLAYS: bool = false;
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

    /// The lines the segments take in the column's room: one line when the strip fits it, and always in a row.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        self.arrange(cx, cx.column_room().unwrap_or(f32::INFINITY)).size(cx.style.height, cx.style.gap)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    /// A part per segment, flowed into the lines of `rect`. A `rect` with room for one line only keeps the segments on
    /// that line. A segment with an icon, or one cut to its line, shows its label as its tooltip.
    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let (gap, rtl) = (cx.style.gap, cx.flow.is_rtl());
        let one_line = rect.height() < 2.0 * cx.style.height + gap - 0.5;
        let placement = if one_line { place(&self.widths(cx), f32::INFINITY, gap) } else { self.arrange(cx, rect.width()) };
        let current = self.position_of(cx.value());
        for (at, entry) in self.entries.iter().enumerate() {
            let cell = placement.rect(at, rect, gap, rtl);
            let mut part = cx.part(Self::part(at), cell);
            if entry.icon_spec().is_some() || cell.width() < placement.wants[at] - 0.5 {
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
    fn sample() -> Self {
        Segment::new(Label::from("Segment"), None)
    }

    fn new(label: Label, icon: Option<IconSpec<'static>>) -> Self {
        Segment { common: Common::new(), icon }.label(label)
    }

    fn spec<'s>(&'s self, label: &'s str) -> FaceSpec<'s> {
        FaceSpec {
            arrangement: crate::buttons::Arrangement::Inline,
            label: if self.icon.is_some() { "" } else { label },
            role: None,
            lead: self.icon.as_ref(),
            picture: None,
            trail: None,
            loading: false,
            mnemonic: None,
            align: crate::primitives::HAlign::Center,
            chord: None,
            detail: None,
        }
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
        // The picked segment wears the look of an icon button that is on (`face::filled_parts` under `Pressed` and `Control`).
        let (selected, selected_hover, on_ink) = match cx.env::<ActiveLook>() {
            ActiveLook::Accent => (c(colors.state.selected), c(colors.state.selected_hover), c(colors.text.on_selected)),
            ActiveLook::Pressed => (c(colors.state.pressed), c(colors.state.pressed), c(colors.text.normal)),
            ActiveLook::Control => (c(colors.surface.control), c(colors.surface.control), c(colors.text.normal)),
        };
        let (clear, disabled) = (c(Rgba::TRANSPARENT), c(colors.state.disabled_fill));
        let fill = cx.state_layer().on(States::new(selected, selected_hover, selected_hover, disabled));
        let edge = Stateful::new(c(colors.line.control_edge)).off(States::new(clear, clear, clear, clear)).on(States::new(
            c(colors.line.control_edge),
            c(colors.line.control_edge),
            c(colors.line.control_edge),
            c(colors.line.divider),
        ));
        let (muted, off) = (c(colors.text.muted), c(colors.text.disabled));
        let ink = Stateful::new(muted).off(States::new(muted, muted, muted, off)).on(States::new(on_ink, on_ink, on_ink, off));
        let control = cx.segment_control();
        SegmentStyle {
            // The selected segment rises: a bevelled theme draws the raised bevel, a flat one the edge alone.
            surface: SurfaceLook::new(fill)
                .kind_on(SurfaceKind::Raised)
                .radius(UiTokens::corner(cx.tokens.shape.radius.control))
                .border(edge, cx.tokens.stroke.thin)
                .border_replaces_edge(),
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

#[cfg(feature = "suite")]
pub(crate) mod stories;

//! The annotations of a node graph: backdrops, notes and markers, which are document data the app stores.

use super::kind::NodeId;
use super::{NodeGraph, NodeGraphStyle};
use crate::contract::{Bind, Cx, PaintUi, PartsCx, Pointer as _, Role, Styled as _};
use crate::fonts::FontRef;
use crate::primitives::{DocRun, DocText, Draw, IconSpec, PaintShape, StrokeWidth, TextRun};
use crate::style::{DataColor, Flags, StyleColor, UiTokens};
use craft_behavior::{
    CancelReason, CanvasTransform, EditEvent, GestureId, HitLevel, HitPriority, IconId, Mixed, PartKey, PointerButton, PressCancel, PressPhase,
    PressStart, Selection, Sense,
};
use egui::{CornerRadius, Pos2, Rect, Vec2};

/// The body of a backdrop: a press selects it, a drag moves it.
pub(super) const BACKDROP: PartKey = PartKey::new("backdrop");
/// The resize grip at a backdrop's bottom-right corner.
pub(super) const BACKDROP_GRIP: PartKey = PartKey::new("backdrop_grip");
/// A note.
pub(super) const NOTE: PartKey = PartKey::new("note");
/// A marker: its pin and its label.
pub(super) const MARKER: PartKey = PartKey::new("marker");

/// What every annotation part senses: a press selects, a drag moves or resizes, a secondary click asks for the menu.
const SENSE: Sense = Sense::PRESS.union(Sense::CONTEXT_CLICK);

/// Which annotation: the kind and the app's id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AnnotationKey {
    /// The backdrop with this id.
    Backdrop(u64),
    /// The note with this id.
    Note(u64),
    /// The marker with this id.
    Marker(u64),
}

/// A titled, tinted rectangle under the nodes, with a grip at its end-bottom corner to resize it.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Backdrop<'a> {
    /// The app's id.
    pub id: u64,
    /// The backdrop's rect, in document units.
    pub rect: Rect,
    /// The title above the rect.
    pub title: Option<&'a str>,
    /// The description inside the rect.
    pub description: &'a str,
    /// The fill, document data.
    pub fill: DataColor,
}

impl<'a> Backdrop<'a> {
    /// Backdrop `id` over `rect` filled with `fill`, with no title and no description.
    pub const fn new(id: u64, rect: Rect, fill: DataColor) -> Self {
        Backdrop { id, rect, title: None, description: "", fill }
    }

    /// The title above the rect.
    #[must_use]
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    /// The description inside the rect.
    #[must_use]
    pub fn description(mut self, description: &'a str) -> Self {
        self.description = description;
        self
    }
}

/// A note over the nodes: text that wraps to its rect's width.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Note<'a> {
    /// The app's id.
    pub id: u64,
    /// The note's rect, in document units.
    pub rect: Rect,
    /// The note's text.
    pub text: &'a str,
}

impl<'a> Note<'a> {
    /// Note `id` over `rect` saying `text`.
    pub const fn new(id: u64, rect: Rect, text: &'a str) -> Self {
        Note { id, rect, text }
    }
}

/// A marker over the notes: the pin icon with its label.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Marker<'a> {
    /// The app's id.
    pub id: u64,
    /// Where the pin points, in document units.
    pub at: Pos2,
    /// The label beside the pin.
    pub label: &'a str,
}

impl<'a> Marker<'a> {
    /// Marker `id` at `at` labelled `label`.
    pub const fn new(id: u64, at: Pos2, label: &'a str) -> Self {
        Marker { id, at, label }
    }
}

/// What the annotations report.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum AnnotationEvent {
    /// The selected annotation changed.
    Selected(Option<AnnotationKey>),
    /// An annotation was moved; the offset is in document units.
    #[non_exhaustive]
    Move {
        /// Which annotation.
        key: AnnotationKey,
        /// The edit of its offset.
        edit: EditEvent<Vec2>,
    },
    /// A backdrop was resized; its rect is in document units.
    #[non_exhaustive]
    Resize {
        /// Which annotation.
        key: AnnotationKey,
        /// The edit of its rect.
        edit: EditEvent<Rect>,
    },
    /// The context menu was asked for on an annotation, at a screen point.
    #[non_exhaustive]
    ContextMenu {
        /// Which annotation.
        key: AnnotationKey,
        /// Where the menu opens.
        at: Pos2,
    },
}

/// The sizes the layers draw at this pass: the type roles' sizes through the zoom, as the nodes' own text scales, and
/// the pin at the theme's icon size, which a zoom leaves alone.
struct Sizes {
    title: f32,
    text: f32,
    icon: f32,
}

impl Sizes {
    fn of(tokens: &UiTokens, style: &NodeGraphStyle, zoom: CanvasTransform) -> Self {
        let size = |role| tokens.typography.get(role).size * zoom.scale;
        Sizes { title: size(style.title_role), text: size(style.text_role), icon: tokens.size.icon.md }
    }
}

/// `rect`, in document units, on screen: through the zoom from the graph's top left at `origin`, as the wires' ends.
fn on_screen(zoom: CanvasTransform, origin: Pos2, rect: Rect) -> Rect {
    Rect::from_min_max(origin + zoom.to_layer(rect.min).to_vec2(), origin + zoom.to_layer(rect.max).to_vec2())
}

/// The grip of a backdrop shown at `frame`: a square of `side` in its bottom-right corner.
fn grip_rect(frame: Rect, side: f32) -> Rect {
    Rect::from_min_max(frame.max - Vec2::splat(side), frame.max)
}

/// One run of annotation text at `size`, in `ink`.
fn run(text: &str, size: f32, ink: StyleColor) -> [DocRun<'_>; 1] {
    [DocRun::new(text, FontRef::proportional(), size, DataColor::new(ink.egui()))]
}

/// Where a note shown at `frame` covers: its rect, taller where its text wrapped to the rect's width runs on.
fn note_rect(frame: Rect, text_height: f32) -> Rect {
    Rect::from_min_size(frame.min, Vec2::new(frame.width(), frame.height().max(text_height)))
}

/// The pin of a marker pointing at `at` on screen: a square of `side` whose bottom middle is the pin's tip.
fn pin_rect(at: Pos2, side: f32) -> Rect {
    Rect::from_min_size(at - Vec2::new(side / 2.0, side), Vec2::splat(side))
}

/// The label of a marker whose pin is at `pin`: after the pin, centred on its height.
fn label_rect(pin: Rect, label: Vec2) -> Rect {
    Rect::from_min_size(Pos2::new(pin.right(), pin.center().y - label.y / 2.0), label)
}

/// Paints the backdrop `backdrop` shown at `frame`: its fill, its edge (wider when selected), the description wrapped
/// and clipped inside, and the grip. The title above is the caller's, which lays it out to place it.
fn paint_backdrop(draw: &Draw<'_>, frame: Rect, backdrop: &Backdrop<'_>, sizes: &Sizes, style: &NodeGraphStyle, look: (StyleColor, StyleColor, bool)) {
    let (edge, ink, selected) = look;
    let shape = PaintShape::Rect(frame, CornerRadius::ZERO);
    draw.data_fill(&shape, backdrop.fill);
    draw.stroke(&shape, if selected { StrokeWidth::Medium } else { StrokeWidth::Thin }, edge);
    if !backdrop.description.is_empty() {
        let inner = frame.shrink(style.grip_side);
        let runs = run(backdrop.description, sizes.text, ink);
        let text = DocText { wrap: Some(inner.width()), ..DocText::new(&runs) };
        draw.clipped(inner, |draw| {
            draw.doc(inner, &text);
        });
    }
    let grip = grip_rect(frame, style.grip_side);
    draw.stroke(&PaintShape::Path(vec![grip.left_bottom(), grip.right_top()]), StrokeWidth::Thin, style.grip);
}

/// Paints the note `note` shown at `frame`: its text wrapped to the rect's width, in `ink`.
fn paint_note(draw: &Draw<'_>, frame: Rect, note: &Note<'_>, sizes: &Sizes, ink: StyleColor) {
    let runs = run(note.text, sizes.text, ink);
    draw.doc(frame, &DocText { wrap: Some(frame.width()), ..DocText::new(&runs) });
}

/// Paints the marker `marker` with its pin at `pin`: the pin icon, the label after it, and an outline around both in
/// `edge` when selected.
fn paint_marker(draw: &Draw<'_>, pin: Rect, marker: &Marker<'_>, style: &NodeGraphStyle, selected: Option<StyleColor>) {
    draw.icon(pin, &IconSpec::svg(IconId::new("pin")).color(style.marker));
    let label = TextRun::new(marker.label).role(style.text_role);
    let size = draw.layout(&label, f32::INFINITY).size();
    let bounds = pin.union(label_rect(pin, size));
    draw.text(label_rect(pin, size), &label);
    if let Some(edge) = selected {
        draw.stroke(&PaintShape::Rect(bounds, CornerRadius::ZERO), StrokeWidth::Thin, edge);
    }
}

/// Why a captured press ended, as the reason its gesture ends without a commit.
fn cancel_reason(reason: PressCancel) -> CancelReason {
    match reason {
        PressCancel::Escape => CancelReason::Escape,
        PressCancel::FocusLost => CancelReason::Blur,
        PressCancel::Removed | PressCancel::Disabled => CancelReason::Removed,
        PressCancel::Host => CancelReason::HostChanged,
        _ => CancelReason::Dismissed,
    }
}

/// The gesture a press runs: its id is the press's own, so a pass reads it from the phase and nothing is kept between
/// passes, and a new press, even at the same point, is a new gesture.
fn gesture_of(start: PressStart) -> GestureId {
    GestureId::new(start.time.to_bits())
}

/// The seam with `NodeGraph`: the annotation layers it draws and lets the pointer reach.
pub(super) struct Layers<'a> {
    pub(super) backdrops: &'a [Backdrop<'a>],
    pub(super) notes: &'a [Note<'a>],
    pub(super) markers: &'a [Marker<'a>],
    // PLAN: the parts' selected flags and the paint's accent need the selected annotation, which ge.C29 does not
    // carry; graph-w4 builds the layers with `Layers::of(self)`.
    pub(super) selected: Option<AnnotationKey>,
}

impl<'a> Layers<'a> {
    /// The layers of `graph`: its annotation slices and the selected annotation its binding holds.
    pub(super) fn of<E: 'static>(graph: &NodeGraph<'a, E>) -> Self {
        let selected = graph.annotation_selection.as_ref().and_then(|bind| bind.value().copied().flatten());
        Layers { backdrops: graph.backdrops, notes: graph.notes, markers: graph.markers, selected }
    }

    fn flags(&self, key: AnnotationKey) -> Flags {
        if self.selected == Some(key) { Flags::SELECTED } else { Flags::empty() }
    }

    /// The parts of the layers: `BACKDROP`, `BACKDROP_GRIP`, `NOTE` and `MARKER`, each at the annotation's id.
    ///
    /// The backdrops are the graph's own parts, under its nodes, so a press on a node inside a backdrop takes the
    /// node; a later backdrop stacks over an earlier one, so the smaller frame the app sorts after a larger one takes
    /// a press inside both. Notes and markers are `over` parts, above the nodes, and a marker ranks one sub-level over
    /// a note where they overlap.
    pub(super) fn parts<E: 'static>(&self, zoom: CanvasTransform, origin: Pos2, cx: &mut PartsCx<'_, NodeGraph<'_, E>>) {
        let sizes = Sizes::of(cx.tokens, cx.style, zoom);
        let (grip_side, text_role) = (cx.style.grip_side, cx.style.text_role);
        for (order, backdrop) in self.backdrops.iter().enumerate() {
            let frame = on_screen(zoom, origin, backdrop.rect);
            let key = AnnotationKey::Backdrop(backdrop.id);
            let stacking = u32::try_from(order).unwrap_or(u32::MAX);
            let body = cx.part(BACKDROP.at(backdrop.id), frame).sense(SENSE).stacking(stacking).flags(self.flags(key));
            if let Some(title) = backdrop.title {
                body.node(Role::GROUP, title);
            }
            cx.part(BACKDROP_GRIP.at(backdrop.id), grip_rect(frame, grip_side))
                .sense(SENSE)
                .priority(HitPriority::new(HitLevel::Body, 1))
                .stacking(stacking)
                .exact_hit()
                .flags(self.flags(key))
                .pointer_only("a backdrop's size is set by the app's annotation commands");
        }
        for note in self.notes {
            let frame = on_screen(zoom, origin, note.rect);
            let runs = run(note.text, sizes.text, StyleColor(egui::Color32::PLACEHOLDER));
            let height = cx.layout_doc(&DocText { wrap: Some(frame.width()), ..DocText::new(&runs) }).size().y;
            let key = AnnotationKey::Note(note.id);
            cx.part(NOTE.at(note.id), note_rect(frame, height))
                .sense(SENSE)
                .priority(HitPriority::new(HitLevel::Body, 0))
                .over()
                .flags(self.flags(key))
                .node(Role::PARAGRAPH, note.text);
        }
        for marker in self.markers {
            let pin = pin_rect(origin + zoom.to_layer(marker.at).to_vec2(), sizes.icon);
            let label = cx.layout(&TextRun::new(marker.label).role(text_role)).size();
            let key = AnnotationKey::Marker(marker.id);
            cx.part(MARKER.at(marker.id), pin.union(label_rect(pin, label)))
                .sense(SENSE)
                .priority(HitPriority::new(HitLevel::Body, 1))
                .over()
                .flags(self.flags(key))
                .node(Role::BUTTON, marker.label);
        }
    }

    /// Paints the backdrops, under the items, in slice order.
    pub(super) fn paint_under<E: 'static>(&self, zoom: CanvasTransform, origin: Pos2, ui: &mut PaintUi<'_>, cx: &Cx<'_, NodeGraph<'_, E>>) {
        let style = cx.style;
        let sizes = ui.with_style(cx.resolved.variant, |scx| Sizes::of(scx.tokens, style, zoom));
        let ink = cx.resolve(&style.note_text);
        let draw = ui.draw();
        for backdrop in self.backdrops {
            let frame = on_screen(zoom, origin, backdrop.rect);
            let part = BACKDROP.at(backdrop.id);
            let selected = self.selected == Some(AnnotationKey::Backdrop(backdrop.id));
            paint_backdrop(&draw, frame, backdrop, &sizes, style, (cx.resolve_part(part, &style.backdrop_edge), ink, selected));
            if let Some(title) = backdrop.title {
                let runs = run(title, sizes.title, ink);
                let text = DocText { wrap: Some(frame.width()), ..DocText::new(&runs) }.elide();
                let height = cx.layout_doc(&text).size().y;
                draw.doc(Rect::from_min_size(Pos2::new(frame.left(), frame.top() - height), Vec2::new(frame.width(), height)), &text);
            }
        }
    }

    /// Paints the notes and the markers, over the parts: every note, then every marker, so a marker shows over a note
    /// as it takes the press there.
    pub(super) fn paint_over<E: 'static>(&self, zoom: CanvasTransform, origin: Pos2, ui: &mut PaintUi<'_>, cx: &Cx<'_, NodeGraph<'_, E>>) {
        let style = cx.style;
        let sizes = ui.with_style(cx.resolved.variant, |scx| Sizes::of(scx.tokens, style, zoom));
        let draw = ui.draw();
        for note in self.notes {
            paint_note(&draw, on_screen(zoom, origin, note.rect), note, &sizes, cx.resolve_part(NOTE.at(note.id), &style.note_text));
        }
        for marker in self.markers {
            let pin = pin_rect(origin + zoom.to_layer(marker.at).to_vec2(), sizes.icon);
            let selected = (self.selected == Some(AnnotationKey::Marker(marker.id))).then(|| cx.resolve_part(MARKER.at(marker.id), &style.backdrop_edge));
            paint_marker(&draw, pin, marker, style, selected);
        }
    }

    /// Reads the pointer on the layers and returns what it did. A press on an annotation writes `annotations` and
    /// empties `nodes`; a press on a node or on empty space writes `None`.
    ///
    /// Each part's phase is this pass's own: a rerun pass of a frame carries no press (`runtime::pointer::PointerFrame`)
    /// and a drag that did not move in it proposes nothing, so a second pass adds no event. Offsets are measured from
    /// the press's start, never summed, so a pass reports where the drag stands whatever passes came before.
    pub(super) fn update<E: 'static>(
        &self,
        zoom: CanvasTransform,
        // The parts' phases are already in the graph's layer space, and an offset is a difference of two of them.
        _origin: Pos2,
        cx: &mut Cx<'_, NodeGraph<'_, E>>,
        annotations: Option<&mut Bind<'_, Option<AnnotationKey>>>,
        nodes: Option<&mut Bind<'_, Selection<NodeId>>>,
    ) -> Vec<AnnotationEvent> {
        let mut events = Vec::new();
        let mut pressed = None;
        let min = cx.style.backdrop_min;
        let parts = self
            .backdrops
            .iter()
            .flat_map(|backdrop| {
                let key = AnnotationKey::Backdrop(backdrop.id);
                [(BACKDROP.at(backdrop.id), key, None), (BACKDROP_GRIP.at(backdrop.id), key, Some(backdrop.rect))]
            })
            .chain(self.notes.iter().map(|note| (NOTE.at(note.id), AnnotationKey::Note(note.id), None)))
            .chain(self.markers.iter().map(|marker| (MARKER.at(marker.id), AnnotationKey::Marker(marker.id), None)));
        let parts: Vec<_> = parts.collect();
        for (part, key, resize) in parts {
            let Some(result) = cx.part_if_declared(part) else { continue };
            let phase = result.press.phase;
            if let PressPhase::Down { start, .. } = phase
                && start.button == PointerButton::Primary
            {
                pressed = Some(key);
            }
            self.gesture(phase, key, resize, (zoom, min), cx, &mut events);
        }
        let current = annotations.as_ref().map_or(self.selected, |bind| bind.value().copied().flatten());
        let press = crate::runtime::pointer::frame(&cx.core.ctx).press;
        let elsewhere = pressed.is_none() && press.is_some_and(|press| cx.rect.contains(press.origin));
        let label = cx.core.label_arc();
        let target = match pressed {
            Some(key) => Some(Some(key)),
            None if elsewhere && current.is_some() => Some(None),
            None => None,
        };
        if let Some(target) = target {
            if target != current {
                if let Some(bind) = annotations {
                    bind.reborrow().write(&mut *cx, &EditEvent::set(Mixed::Same(current), target, label.clone(), None));
                }
                events.push(AnnotationEvent::Selected(target));
            }
            if target.is_some()
                && let Some(bind) = nodes
                && bind.value().is_none_or(|held| !held.items.is_empty())
            {
                let before = bind.value().cloned().map_or(Mixed::Mixed, Mixed::Same);
                bind.reborrow().write(&mut *cx, &EditEvent::set(before, Selection::default(), label, None));
            }
        }
        events
    }

    /// What `phase` on the part of `key` adds to `events`: a drag runs a `Move`, or a `Resize` of the backdrop at
    /// `resize`, as Begin, Update, then Commit or Cancel, in document units; a secondary click asks for the menu at the
    /// release point, in the graph's layer space, which is its parent's. A press that comes up where it went down
    /// changes nothing.
    fn gesture<E: 'static>(
        &self,
        phase: PressPhase,
        key: AnnotationKey,
        resize: Option<Rect>,
        (zoom, min): (CanvasTransform, Vec2),
        cx: &Cx<'_, NodeGraph<'_, E>>,
        events: &mut Vec<AnnotationEvent>,
    ) {
        let scale = zoom.scale.max(f32::EPSILON);
        let offset = |start: PressStart, pos: Pos2| (pos - start.origin) / scale;
        let report = |edit: EditEvent<Vec2>| match resize {
            None => AnnotationEvent::Move { key, edit },
            // No movement leaves the rect as the app gave it, under the least size or not.
            Some(rect) => AnnotationEvent::Resize {
                key,
                edit: edit.map(|by| if by == Vec2::ZERO { rect } else { Rect::from_min_size(rect.min, (rect.size() + by).max(min)) }),
            },
        };
        let before = || Mixed::Same(Vec2::ZERO);
        match phase {
            PressPhase::DragStart { start, pos, .. } if start.button == PointerButton::Primary => {
                let id = gesture_of(start);
                events.push(report(EditEvent::begin(id, before(), cx.core.label_arc(), None)));
                events.push(report(EditEvent::update(id, offset(start, pos))));
            }
            PressPhase::Drag { start, pos, delta, .. } if start.button == PointerButton::Primary && delta != Vec2::ZERO => {
                events.push(report(EditEvent::update(gesture_of(start), offset(start, pos))));
            }
            PressPhase::Release { start, pos, click: false, .. } if start.button == PointerButton::Primary => {
                let (id, by) = (gesture_of(start), offset(start, pos));
                let edit = if by == Vec2::ZERO {
                    EditEvent::cancel(id, CancelReason::Unchanged, Some(before()))
                } else {
                    EditEvent::commit(id, before(), by)
                };
                events.push(report(edit));
            }
            PressPhase::Release { start, pos, click: true, .. } if start.button == PointerButton::Secondary => {
                events.push(AnnotationEvent::ContextMenu { key, at: pos });
            }
            PressPhase::Cancel { start, reason, .. } if start.button == PointerButton::Primary => {
                // Only a press that moved past the slop began a gesture; the pointer still lies where the drag reached.
                let pointer = crate::runtime::pointer::frame(&cx.core.ctx);
                if pointer.pos.is_some_and(|pos| pos.distance(start.origin) > pointer.slop) {
                    events.push(report(EditEvent::cancel(gesture_of(start), cancel_reason(reason), Some(before()))));
                }
            }
            _ => {}
        }
    }
}

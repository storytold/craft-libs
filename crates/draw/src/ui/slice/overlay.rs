//! [`SliceOverlay`]: the slices of a canvas, a canvas control (20.6). Every slice is painted; with
//! [`pick`](SliceOverlay::pick) each pickable slice is a focusable part in the canvas's arena and the selected user
//! slice has eight handle parts, so a press on a slice stays with the overlay and one elsewhere reaches the tool.

use super::{Run, SliceEvent, SliceItem, SliceOrigin, SliceRules, SliceToolWords, cancel_reason, handle_point, snapped_move, whole};
use crate::geometry::slice::{HANDLES, resized_rect};
use craft_behavior::kurbo::{Point, Rect as DocRect, Vec2 as DocVec};
use craft_behavior::{
    Axis, CursorIcon, GestureId, Granularity, HitLevel, HitPriority, HitShape, Intent, Metrics, NavShape, PartKey, PointerButton, PressPhase, Roving,
    Sense, resize_cursor,
};
use craft_keymap::Mods;
use craft_theme::{Opacity, Srgba, TypeRole};
use craft_ui::canvas::{CanvasSpace, NUDGE, NUDGE_FAR, Snap, canvas_snap, canvas_space};
use craft_ui::contract::{
    Access, Common, Component, Cx, Focus as _, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Remember as _, Role,
    Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{Dashes, PaintShape, StrokeWidth, TextRun, dashed};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, DataColor, Flags, HandleLook, StyleColor, StyleCx};
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Vec2};

/// The part of one slice: `SLICE.at(number)`.
const SLICE: PartKey = PartKey::new("slice");

/// The part of one handle of the selected user slice: `HANDLE.at(i)` with `i` an index into [`HANDLES`].
const HANDLE: PartKey = PartKey::new("handle");

/// A layer slice's mark after its number, as a share of the number's height.
const LAYER_MARK: f32 = 0.45;

/// The style of the slice overlay.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct SliceOverlayStyle {
    #[style(metrics)]
    metrics: Metrics,
    handle: HandleLook,
    /// The slice colour, unless the app's preference overrides it.
    line: StyleColor,
    /// The selected slice.
    selected: StyleColor,
    /// An auto slice's badge.
    auto_fill: StyleColor,
    /// The badge text.
    text: StyleColor,
    /// A badge's padding around the number and its offset from the slice's corner.
    gap: f32,
    /// A badge's corner radius.
    radius: f32,
    /// The dash and the gap of an auto slice's line.
    dash: f32,
    dash_gap: f32,
}

/// What the control reports: the events of the pass in order.
#[derive(Debug)]
#[non_exhaustive]
pub struct SliceOverlayOutput {
    /// The overlay's response.
    pub response: Response,
    /// What the pointer and the keys did: selections, moves, resizes, commands and refusals.
    pub events: Vec<SliceEvent>,
}

impl Outcome for SliceOverlayOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What a press took, kept from its press to its release.
#[derive(Clone, Debug, Default)]
enum Held {
    /// A press on a slice that does not move (an auto or layer slice, or a click on a handle).
    #[default]
    Hold,
    Move {
        number: usize,
        rect: DocRect,
        run: Run<DocRect>,
        start: Point,
    },
    Resize {
        number: usize,
        rect: DocRect,
        handle: [i8; 2],
        run: Run<DocRect>,
    },
}

/// The slices drawn over the document, and in Select mode taken by the pointer and the keyboard.
///
/// - Every slice is painted: a line inside the edge in the slice colour, the selected slice in the selected colour,
///   auto slices dashed at a share of the slice colour (shown unless [`hide_auto`](Self::hide_auto)). A badge with
///   the slice's two-digit number sits at the top left corner of every slice that holds it on screen; a layer slice's
///   badge carries a small square after the number.
/// - With [`pick`](Self::pick) each pickable slice is a part: stored slices over auto slices, the latest on top,
///   hidden auto slices none. The slices are one Tab stop. A click selects the slice (Shift adds or removes it under
///   [`SliceRules::multi_select`]); a drag on a user slice moves the whole slice, also when the press hit a covered
///   piece of it, snapped by the canvas's snapping; auto and layer slices only select. A double click opens the
///   slice's options. The selected user slice has eight handles that resize it; a click on a handle resizes nothing.
/// - Keys on the focused slice: the arrows move a user slice one pixel (Shift ten), Delete and Backspace delete a
///   stored slice (the key is taken, so the app's Delete Layer never sees it; on an auto slice nothing), Enter opens
///   its options. Escape cancels a drag.
/// - [`locked`](Self::locked) slices take no part: the press reaches the tool, which clears the selection and refuses.
/// - Document geometry never mirrors right to left.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct SliceOverlay<'a> {
    #[common]
    common: Common,
    items: &'a [SliceItem],
    /// The selected slice's number.
    #[prop(name = "selected_slice")]
    selected: Option<usize>,
    /// Select mode: the slices are parts.
    #[prop]
    pick: bool,
    /// The slice colour preference (Guides, Grid & Slices); the theme's when not set.
    #[prop]
    color: Option<Srgba>,
    /// Show Slice Numbers.
    #[prop]
    numbers: bool,
    /// Hide Auto Slices.
    #[prop]
    hide_auto: bool,
    /// View › Lock Slices.
    #[prop]
    locked: bool,
    /// Which product's rules the control follows.
    #[prop]
    rules: SliceRules,
    events: Vec<SliceEvent>,
}

impl<'a> SliceOverlay<'a> {
    /// The overlay named `label` over `items`, with numbers shown and auto slices shown.
    pub fn new(label: impl Into<Label>, items: &'a [SliceItem]) -> Self {
        SliceOverlay {
            common: Common::new(),
            items,
            selected: None,
            pick: false,
            color: None,
            numbers: true,
            hide_auto: false,
            locked: false,
            rules: SliceRules::PHOTOSHOP,
            events: Vec::new(),
        }
        .label(label)
    }

    fn shown(&self) -> impl Iterator<Item = &SliceItem> {
        self.items.iter().filter(|item| !(self.hide_auto && item.origin == SliceOrigin::Auto))
    }

    fn selected_user(&self) -> Option<&SliceItem> {
        let number = self.selected?;
        self.items.iter().find(|item| item.number == number && item.origin == SliceOrigin::User)
    }

    fn item(&self, number: usize) -> Option<&SliceItem> {
        self.items.iter().find(|item| item.number == number)
    }

    /// The pointer on the slices and the handles.
    fn run_pointer(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, snap: Option<&Snap>) {
        let words = cx.words::<SliceToolWords>();
        let mut held: Option<Held> = cx.press_slot::<Held>().clone();
        let snapping = |mods: Mods| move |along: Axis, position: f64| snap.map_or(position, |snap| snap.apply(along, position, mods));
        let numbers: Vec<usize> = self.shown().map(|item| item.number).collect();
        for number in numbers {
            let Some(result) = cx.part_if_declared(SLICE.at(number as u64)) else { continue };
            let press = result.press;
            let Some(item) = self.item(number).cloned() else { continue };
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    let add = self.rules.multi_select && press.mods.shift;
                    self.events.push(SliceEvent::Select { number: Some(number), add, mods: press.mods });
                    if press.double_pressed() {
                        self.events.push(SliceEvent::Command(super::SliceCommand::Options));
                    }
                    let at = space.view.to_doc(start.origin);
                    held = Some(match item.origin {
                        SliceOrigin::User => {
                            Held::Move { number, rect: item.rect, run: Run::new(next_id(cx), words.move_slice.as_str().into(), item.rect), start: at }
                        }
                        _ => Held::Hold,
                    });
                }
                PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                    if let Some(Held::Move { number, rect, run, start }) = held.as_mut() {
                        let raw = space.view.to_doc(pos) - *start;
                        let by = whole(snapped_move(*rect, raw, &snapping(press.mods)));
                        if by != DocVec::ZERO || run.began() {
                            let events = run.follow(*rect + by);
                            self.events.extend(events.into_iter().map(|event| SliceEvent::Move { number: *number, event }));
                        }
                    }
                }
                PressPhase::Release { pos, .. } => match held.take() {
                    Some(Held::Move { number, rect, run, start }) => {
                        let raw = space.view.to_doc(pos) - start;
                        let by = whole(snapped_move(rect, raw, &snapping(press.mods)));
                        if by != DocVec::ZERO || run.began() {
                            self.events.extend(run.commit(rect + by).into_iter().map(|event| SliceEvent::Move { number, event }));
                        }
                    }
                    Some(Held::Hold) | None => {}
                    other => held = other,
                },
                PressPhase::Cancel { reason, .. } => match held.take() {
                    Some(Held::Move { number, run, .. }) => {
                        if let Some(event) = run.cancel(cancel_reason(reason)) {
                            self.events.push(SliceEvent::Move { number, event });
                        }
                    }
                    other => held = other,
                },
                _ => {}
            }
        }
        if let Some(user) = self.selected_user().cloned() {
            for (index, handle) in HANDLES.into_iter().enumerate() {
                let Some(result) = cx.part_if_declared(HANDLE.at(index as u64)) else { continue };
                let press = result.press;
                match press.phase {
                    PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                        held = Some(Held::Resize {
                            number: user.number,
                            rect: user.rect,
                            handle,
                            run: Run::new(next_id(cx), words.resize_slice.as_str().into(), user.rect),
                        });
                    }
                    PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                        if let Some(Held::Resize { number, rect, handle, run }) = held.as_mut() {
                            let now = resized_to(&space.view, *rect, *handle, pos, &snapping(press.mods));
                            if now != *rect || run.began() {
                                let events = run.follow(now);
                                self.events.extend(events.into_iter().map(|event| SliceEvent::Resize { number: *number, event }));
                            }
                        }
                    }
                    PressPhase::Release { pos, .. } => match held.take() {
                        Some(Held::Resize { number, rect, handle, run }) => {
                            let now = resized_to(&space.view, rect, handle, pos, &snapping(press.mods));
                            if now != rect || run.began() {
                                self.events.extend(run.commit(now).into_iter().map(|event| SliceEvent::Resize { number, event }));
                            }
                        }
                        Some(Held::Hold) | None => {}
                        other => held = other,
                    },
                    PressPhase::Cancel { reason, .. } => match held.take() {
                        Some(Held::Resize { number, run, .. }) => {
                            if let Some(event) = run.cancel(cancel_reason(reason)) {
                                self.events.push(SliceEvent::Resize { number, event });
                            }
                        }
                        other => held = other,
                    },
                    _ => {}
                }
            }
        }
        *cx.press_slot::<Held>() = held;
    }

    /// The keys of the focused slice: selection follows the focus, the arrows move a user slice, Delete removes a
    /// stored one and Enter opens the options.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace) {
        let words = cx.words::<SliceToolWords>();
        let focused = cx.active_part().filter(|part| part.key == SLICE).and_then(|part| part.index);
        // Focus arriving on a slice by a key or a request selects it; a press selects by its own rule.
        let arrived = cx.focus_arrived_by_key().is_some_and(|part| part.key == SLICE && part.index.is_some());
        let Some(item) = focused.and_then(|number| self.item(number as usize)).cloned() else { return };
        if arrived && self.events.is_empty() {
            self.events.push(SliceEvent::Select { number: Some(item.number), add: false, mods: Mods::NONE });
        }
        if let Some((direction, mods)) = cx.intents().take_nudge()
            && item.origin == SliceOrigin::User
        {
            let step = if mods.shift { NUDGE_FAR } else { NUDGE };
            let doc = space.view.unmap_vec(direction);
            let rect = item.rect + whole(doc * (step / doc.hypot().max(f64::EPSILON)));
            let event = Run::set(words.move_slice.as_str().into(), item.rect, rect);
            self.events.push(SliceEvent::Move { number: item.number, event });
        }
        if item.origin.stored() {
            let forward = cx.intents().take_exact(&Intent::delete(true, Granularity::Unit));
            let backward = cx.intents().take_exact(&Intent::delete(false, Granularity::Unit));
            if forward || backward {
                self.events.push(SliceEvent::Command(super::SliceCommand::Delete));
            }
        }
        if cx.intents().take(Intent::Submit) {
            self.events.push(SliceEvent::Command(super::SliceCommand::Options));
        }
    }
}

/// A gesture id no gesture of this context has had.
fn next_id<C: Component>(cx: &Cx<'_, C>) -> GestureId {
    craft_ui::runtime::edit::next_gesture_id(&cx.response.ctx)
}

/// `rect` with `handle` dragged to the pointer at screen position `pos`: the travel is measured from the handle's own
/// point, so a snapped pointer is where the edge ends up.
fn resized_to(view: &craft_behavior::CanvasView, rect: DocRect, handle: [i8; 2], pos: Pos2, snap: &dyn Fn(Axis, f64) -> f64) -> DocRect {
    let doc = view.to_doc(pos);
    let to = Point::new(snap(Axis::Y, doc.x), snap(Axis::X, doc.y));
    resized_rect(rect, handle, to - handle_point(rect, handle))
}

impl Component for SliceOverlay<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves().with_deletes().with_chrome_key(false);
    type Style = SliceOverlayStyle;
    type Output = SliceOverlayOutput;

    fn style(cx: &StyleCx<'_>, _: SliceOverlayStyleKey) -> SliceOverlayStyle {
        let colors = &cx.tokens.color;
        SliceOverlayStyle {
            metrics: Metrics::new(),
            handle: cx.handle(),
            line: cx.color(colors.overlay.line),
            selected: cx.color(colors.status.warn.solid),
            auto_fill: cx.color(colors.text.muted),
            text: cx.color(colors.text.on_status),
            gap: cx.tokens.space.gap.tight,
            radius: cx.tokens.shape.radius.sm,
            dash: cx.tokens.stroke.dash,
            dash_gap: cx.tokens.stroke.dash_gap,
        }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// The pickable slices (auto below stored, in the app's order, the latest on top) and the selected user slice's
    /// handles.
    fn parts(&mut self, _rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        if !self.pick || self.locked {
            return;
        }
        let view = space.view;
        let mut any = false;
        for auto in [true, false] {
            for item in self.shown().filter(|item| (item.origin == SliceOrigin::Auto) == auto) {
                let level = HitPriority::new(HitLevel::Body, u8::from(!auto));
                let part = cx
                    .part(SLICE.at(item.number as u64), HitShape::Polygon(view.quad(item.rect).to_vec()))
                    .sense(Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS)
                    .priority(level)
                    .cursor(if item.origin == SliceOrigin::User { CursorIcon::Move } else { CursorIcon::Default })
                    .focusable();
                if self.selected == Some(item.number) {
                    part.selected().flags(Flags::SELECTED);
                }
                any = true;
            }
        }
        if any {
            cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
        }
        if let Some(user) = self.selected_user() {
            let radius = cx.style.handle.side / 2.0;
            for (index, handle) in HANDLES.into_iter().enumerate() {
                let at = view.to_screen(handle_point(user.rect, handle));
                cx.part(HANDLE.at(index as u64), HitShape::circle(at, radius))
                    .sense(Sense::DRAG | Sense::CLICK)
                    .priority(HitPriority::new(HitLevel::Handle, 0))
                    .cursor(resize_cursor(&view, DocVec::new(f64::from(handle[0]), f64::from(handle[1]))))
                    .pointer_only("the arrow keys on the focused slice move it and Slice Options names it");
            }
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        if !self.pick || self.locked {
            return;
        }
        let snap = canvas_snap(&cx.env);
        self.run_pointer(cx, &space, snap.as_ref());
        self.run_keys(cx, &space);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let view = space.view;
        let style = cx.style;
        let draw = ui.draw();
        let slice_color = self.color.map_or(style.line.egui(), Color32::from);
        let line = DataColor::new(slice_color);
        let format = cx.format();
        // Auto slices first, dashed and underneath, then the stored ones.
        for auto in [true, false] {
            for item in self.shown().filter(|item| (item.origin == SliceOrigin::Auto) == auto) {
                let selected = self.selected == Some(item.number);
                let quad: Vec<Pos2> = view.quad(item.rect).to_vec();
                let screen = Rect::from_points(&quad);
                if auto {
                    let mut path = quad.clone();
                    path.push(quad[0]);
                    // The dash geometry is the kit's; the colour is the slice's data colour, faded as a derived mark.
                    let dashes = dashed(&path, Dashes::new(style.dash, style.dash_gap), 1.0, slice_color);
                    draw.faded(Opacity::DERIVED, |faded| {
                        for dash in dashes {
                            match dash {
                                egui::Shape::LineSegment { points, .. } => faded.stroke(&PaintShape::Path(points.to_vec()), StrokeWidth::Thin, line),
                                egui::Shape::Path(path) => faded.stroke(&PaintShape::Path(path.points), StrokeWidth::Thin, line),
                                _ => {}
                            }
                        }
                    });
                } else {
                    draw.stroke(&PaintShape::Polygon(quad.clone()), StrokeWidth::Thin, line);
                }
                if selected {
                    draw.stroke(&PaintShape::Polygon(quad.clone()), StrokeWidth::Medium, style.selected);
                }
                if !self.numbers {
                    continue;
                }
                let text = number_text(format, item.number);
                let run = TextRun::new(text).role(TypeRole::Caption).color(style.text);
                let layout = draw.layout(&run, f32::INFINITY);
                let layer = item.origin == SliceOrigin::Layer;
                let mark = if layer { layout.size().y * LAYER_MARK } else { 0.0 };
                let gap = mark;
                let size = Vec2::new(layout.size().x + gap + mark, layout.size().y) + Vec2::splat(style.gap) * 2.0;
                if screen.width() <= size.x || screen.height() <= size.y {
                    continue;
                }
                let badge = Rect::from_min_size(screen.left_top() + Vec2::splat(style.gap), size);
                let fill = if auto {
                    style.auto_fill.egui()
                } else if selected {
                    style.selected.egui()
                } else {
                    slice_color
                };
                draw.data_fill(&PaintShape::Rect(badge, CornerRadius::same(style.radius as u8)), DataColor::new(fill));
                let origin = badge.min + Vec2::splat(style.gap);
                draw.galley(origin, &layout);
                if layer {
                    let centre = Pos2::new(origin.x + layout.size().x + gap + mark / 2.0, origin.y + layout.size().y / 2.0);
                    draw.fill(&PaintShape::Rect(Rect::from_center_size(centre, Vec2::splat(mark)), CornerRadius::ZERO), style.text);
                }
            }
        }
        if let Some(user) = self.selected_user().filter(|_| self.pick && !self.locked) {
            let look = &style.handle;
            for (index, handle) in HANDLES.into_iter().enumerate() {
                let square = PaintShape::Rect(
                    Rect::from_center_size(view.to_screen(handle_point(user.rect, handle)), Vec2::splat(look.side)),
                    CornerRadius::ZERO,
                );
                draw.fill(&square, cx.resolve_part(HANDLE.at(index as u64), &look.fill));
                draw.stroke(&square, StrokeWidth::Thin, look.stroke);
            }
        }
    }

    /// One node per pickable slice, named by its number, origin and place.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if !self.pick || self.locked {
            return;
        }
        let words = cx.words::<SliceToolWords>();
        let format = cx.format();
        let count = |value: f64| format.number_ungrouped(value, 0);
        for item in self.shown() {
            let origin = match item.origin {
                SliceOrigin::User => &words.user,
                SliceOrigin::Auto => &words.auto,
                SliceOrigin::Layer => &words.layer,
            };
            let name = words.slice.fill(&[
                ("number", &number_text(format, item.number)),
                ("origin", origin.as_str()),
                ("width", &count(item.rect.width())),
                ("height", &count(item.rect.height())),
                ("x", &count(item.rect.x0)),
                ("y", &count(item.rect.y0)),
            ]);
            access.node(SLICE.at(item.number as u64), Role::BUTTON).name = Name::Text(name);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> SliceOverlayOutput {
        SliceOverlayOutput { response: cx.response.clone(), events: self.events }
    }
}

/// A slice's number as two digits in the locale's digits ("03").
fn number_text(format: &craft_i18n::Format, number: usize) -> String {
    let text = format.number_ungrouped(number as f64, 0);
    if number < 10 { format!("{}{text}", format.number_ungrouped(0.0, 0)) } else { text }
}

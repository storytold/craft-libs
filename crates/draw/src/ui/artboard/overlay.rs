//! [`ArtboardOverlay`]: the boards' bodies and the active board's handles, a canvas control (20.6). The boards' frames and
//! names stay the `Pasteboard`'s.

use super::{ArtboardEvent, ArtboardItem, ArtboardRules, ArtboardToolWords, Boards, KeyAct, key_events, label, resized};
use crate::geometry::artboard::drawn_rect;
use crate::geometry::slice::HANDLES;
use crate::ui::slice::{Run, cancel_reason, handle_point, snapped_move, whole};
use craft_behavior::kurbo::{Point, Rect as DocRect, Vec2 as DocVec};
use craft_behavior::{
    Axis, CursorIcon, GestureId, HitLevel, HitPriority, HitShape, Intent, Metrics, NavShape, PartKey, PointerButton, PressPhase, Roving, Sense,
    resize_cursor,
};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasSpace, Snap, canvas_snap, canvas_space};
use craft_ui::contract::{
    Access, Common, Component, Cx, Focus as _, MeasureCx, Name, NoValue, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Remember as _, Role,
    Styled as _, WordSetCx as _,
};
use craft_ui::primitives::{PaintShape, StrokeWidth};
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::KeyUse;
use craft_ui::style::{ComponentStyle, Flags, HandleLook, StyleCx};
use egui::{CornerRadius, Pos2, Rect, Response, Vec2};

/// The part of one board: `BOARD.at(index)`.
const BOARD: PartKey = PartKey::new("board");

/// The part of one handle of the active board: `HANDLE.at(i)` with `i` an index into [`HANDLES`].
const HANDLE: PartKey = PartKey::new("handle");

/// The style of the artboard overlay.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ArtboardOverlayStyle {
    #[style(metrics)]
    metrics: Metrics,
    handle: HandleLook,
}

/// What the control reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct ArtboardOverlayOutput {
    /// The overlay's response.
    pub response: Response,
    /// What the pointer and the keys did, in order.
    pub events: Vec<ArtboardEvent>,
    /// A drag runs: the app hides the "+" buttons meanwhile.
    pub dragging: bool,
}

impl Outcome for ArtboardOverlayOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What a press took, kept from its press to its release.
#[derive(Clone, Debug, Default)]
enum Held {
    /// A press on a board that does not move (a locked board, a double click).
    #[default]
    Hold,
    /// The boards `indices` move; a click that stays a click narrows a larger selection to `narrow`.
    Move { indices: Vec<usize>, copy: bool, narrow: Option<usize>, rect: DocRect, run: Run<DocVec>, start: Point },
    /// Board `index`, whose rect was `rect` at the press, resized by `handle`.
    Resize { index: usize, rect: DocRect, handle: [i8; 2], run: Run<DocRect> },
    /// Shift-press inside board `toggle`: a new board when it drags, else the board toggles in the selection.
    New { toggle: usize, run: Run<DocRect>, start: Point },
}

impl Held {
    fn began(&self) -> bool {
        match self {
            Held::Move { run, .. } => run.began(),
            Held::Resize { run, .. } | Held::New { run, .. } => run.began(),
            Held::Hold => false,
        }
    }
}

/// The boards of a canvas, taken by the pointer and the keyboard.
///
/// - A press in a board selects it; a drag moves it with its contents in whole pixels (the canvas's snapping applies
///   to the boards' edges); a locked board selects and never moves. The active board's eight handles resize it (Shift
///   on a corner keeps proportions, also when Shift goes down mid-drag; Alt about the centre).
/// - Under [`ArtboardRules`]: Shift-click toggles a board, Shift-drag inside a board draws a new one, Alt-drag moves a
///   copy, a double click reports `OpenOptions`.
/// - The boards, then the active board's handles, are one Tab stop. On a focused part the arrows move the selected
///   boards by the rules' step (Shift ten times), Alt and the arrows resize from the end edges, Delete reports `Delete`,
///   Enter `OpenOptions`.
/// - Cursors: the move arrow over a board, [`resize_cursor`] over the handles.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(canvas_control)]
pub struct ArtboardOverlay<'a> {
    #[common]
    common: Common,
    items: &'a [ArtboardItem],
    selected: &'a [usize],
    /// Which product's rules the control follows.
    #[prop]
    rules: ArtboardRules,
    events: Vec<ArtboardEvent>,
    dragging: bool,
}

impl<'a> ArtboardOverlay<'a> {
    /// The overlay named `label` over `items`, with the boards `selected` (the active board first).
    pub fn new(label: impl Into<Label>, items: &'a [ArtboardItem], selected: &'a [usize]) -> Self {
        ArtboardOverlay { common: Common::new(), items, selected, rules: ArtboardRules::PHOTOSHOP, events: Vec::new(), dragging: false }.label(label)
    }

    fn boards(&self) -> Boards<'a> {
        Boards { items: self.items, selected: self.selected }
    }

    fn handles_shown(&self) -> Option<usize> {
        self.boards().active().filter(|&i| !self.items[i].locked)
    }

    fn pointer(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, snap: Option<&Snap>) {
        let words = cx.words::<ArtboardToolWords>();
        let boards = self.boards();
        let mut held: Held = cx.press_slot::<Held>().clone().unwrap_or_default();
        let mut live = cx.press_slot::<Held>().is_some();
        let snapping = |mods: Mods| move |along: Axis, position: f64| snap.map_or(position, |snap| snap.apply(along, position, mods));
        for index in 0..self.items.len() {
            let Some(result) = cx.part_if_declared(BOARD.at(index as u64)) else { continue };
            let press = result.press;
            let item = &self.items[index];
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                    let at = space.view.to_doc(start.origin);
                    let mods = press.mods;
                    live = true;
                    held = Held::Hold;
                    if self.rules.multi_select && mods.shift {
                        let run = Run::new(mint(cx), label(&words.new_board), DocRect::from_origin_size(at, (0.0, 0.0)));
                        held = Held::New { toggle: index, run, start: at };
                    } else if self.rules.open_on_double_click && press.double_pressed() {
                        self.events.push(ArtboardEvent::OpenOptions { index });
                    } else if item.locked {
                        if self.selected != [index] {
                            self.events.push(ArtboardEvent::Select { indices: vec![index], mods });
                        }
                    } else {
                        let mut indices = vec![index];
                        let mut narrow = None;
                        if self.selected.contains(&index) {
                            if self.selected.len() > 1 {
                                indices = boards.movable();
                                narrow = Some(index);
                            }
                        } else {
                            self.events.push(ArtboardEvent::Select { indices: vec![index], mods });
                        }
                        let rect = union(self.items, &indices);
                        let run = Run::new(mint(cx), label(&words.move_board), DocVec::ZERO);
                        held = Held::Move { indices, copy: self.rules.alt_copies && mods.alt, narrow, rect, run, start: at };
                    }
                }
                PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                    let to = space.view.to_doc(pos);
                    match &mut held {
                        Held::Move { indices, copy, rect, run, start, .. } => {
                            let by = whole(snapped_move(*rect, to - *start, &snapping(press.mods)));
                            self.events.extend(run.follow(by).into_iter().map(|event| ArtboardEvent::Move {
                                indices: indices.clone(),
                                event,
                                copy: *copy,
                            }));
                        }
                        Held::New { run, start, .. } => {
                            let rect = drawn_rect(*start, to);
                            if rect.width() >= 1.0 && rect.height() >= 1.0 {
                                self.events.extend(run.follow(rect).into_iter().map(ArtboardEvent::Draw));
                            }
                        }
                        _ => {}
                    }
                }
                PressPhase::Release { pos, .. } => {
                    let to = space.view.to_doc(pos);
                    let mods = press.mods;
                    match std::mem::take(&mut held) {
                        Held::Move { indices, copy, narrow, rect, run, start } => {
                            if run.began() {
                                let by = whole(snapped_move(rect, to - start, &snapping(mods)));
                                self.events.extend(run.commit(by).into_iter().map(|event| ArtboardEvent::Move {
                                    indices: indices.clone(),
                                    event,
                                    copy,
                                }));
                            } else if let Some(only) = narrow {
                                self.events.push(ArtboardEvent::Select { indices: vec![only], mods });
                            }
                        }
                        Held::New { toggle, run, start } => {
                            let rect = drawn_rect(start, to);
                            if run.began() && rect.width() >= 1.0 && rect.height() >= 1.0 {
                                self.events.extend(run.commit(rect).into_iter().map(ArtboardEvent::Draw));
                            } else if let Some(event) = run.cancel(craft_behavior::CancelReason::Dismissed) {
                                self.events.push(ArtboardEvent::Draw(event));
                            } else {
                                let mut indices = self.selected.to_vec();
                                match indices.iter().position(|&k| k == toggle) {
                                    Some(at) => drop(indices.remove(at)),
                                    None => indices.insert(0, toggle),
                                }
                                self.events.push(ArtboardEvent::Select { indices, mods });
                            }
                        }
                        _ => {}
                    }
                    live = false;
                }
                PressPhase::Cancel { reason, .. } => {
                    match std::mem::take(&mut held) {
                        Held::Move { indices, copy, run, .. } => {
                            if let Some(event) = run.cancel(cancel_reason(reason)) {
                                self.events.push(ArtboardEvent::Move { indices, event, copy });
                            }
                        }
                        Held::New { run, .. } => self.events.extend(run.cancel(cancel_reason(reason)).map(ArtboardEvent::Draw)),
                        _ => {}
                    }
                    live = false;
                }
                _ => {}
            }
        }
        if let Some(active) = self.handles_shown() {
            let rect = self.items[active].rect;
            for (h, handle) in HANDLES.into_iter().enumerate() {
                let Some(result) = cx.part_if_declared(HANDLE.at(h as u64)) else { continue };
                let press = result.press;
                match press.phase {
                    PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                        let run = Run::new(mint(cx), label(&words.resize_board), rect);
                        held = Held::Resize { index: active, rect, handle, run };
                        live = true;
                    }
                    PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                        if let Held::Resize { index, rect, handle, run } = &mut held {
                            let now = resize_to(&space.view, *rect, *handle, pos, press.mods, &snapping(press.mods));
                            self.events.extend(run.follow(now).into_iter().map(|event| ArtboardEvent::Resize { index: *index, event }));
                        }
                    }
                    PressPhase::Release { pos, .. } => {
                        if let Held::Resize { index, rect, handle, run } = std::mem::take(&mut held)
                            && run.began()
                        {
                            let now = resize_to(&space.view, rect, handle, pos, press.mods, &snapping(press.mods));
                            self.events.extend(run.commit(now).into_iter().map(|event| ArtboardEvent::Resize { index, event }));
                        }
                        live = false;
                    }
                    PressPhase::Cancel { reason, .. } => {
                        if let Held::Resize { index, run, .. } = std::mem::take(&mut held) {
                            self.events.extend(run.cancel(cancel_reason(reason)).map(|event| ArtboardEvent::Resize { index, event }));
                        }
                        live = false;
                    }
                    _ => {}
                }
            }
        }
        self.dragging = live && held.began();
        *cx.press_slot::<Held>() = live.then_some(held);
    }

    fn keys(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace) {
        let words = cx.words::<ArtboardToolWords>();
        let focused = cx.active_part().filter(|part| part.key == BOARD || part.key == HANDLE);
        // Focus arriving on a board by a key or a request selects it; a press selects by its own rule.
        let arrived = cx.focus_arrived_by_key().is_some_and(|part| part.key == BOARD);
        let Some(part) = focused else { return };
        if arrived
            && part.key == BOARD
            && let Some(index) = part.index.map(|i| i as usize).filter(|&i| i < self.items.len())
            && self.selected != [index]
            && self.events.is_empty()
        {
            self.events.push(ArtboardEvent::Select { indices: vec![index], mods: Mods::NONE });
        }
        let boards = self.boards();
        let mut acts = Vec::new();
        if let Some((direction, mods)) = cx.intents().take_nudge() {
            let doc = space.view.unmap_vec(direction);
            let unit = doc / doc.hypot().max(f64::EPSILON);
            acts.push(KeyAct::Nudge { direction: DocVec::new(unit.x.round(), unit.y.round()), mods });
        }
        if cx.intents().take_exact(&Intent::delete(true, craft_behavior::Granularity::Unit))
            || cx.intents().take_exact(&Intent::delete(false, craft_behavior::Granularity::Unit))
        {
            acts.push(KeyAct::Delete);
        }
        if cx.intents().take(Intent::Submit) {
            acts.push(KeyAct::Submit);
        }
        for act in acts {
            self.events.extend(key_events(&boards, &self.rules, act, &words));
        }
    }
}

/// A gesture id no gesture of this context has had.
fn mint<C: Component>(cx: &Cx<'_, C>) -> GestureId {
    craft_ui::runtime::edit::next_gesture_id(&cx.response.ctx)
}

/// The bounding rect of the boards `indices`.
fn union(items: &[ArtboardItem], indices: &[usize]) -> DocRect {
    indices.iter().filter_map(|&i| items.get(i)).map(|item| item.rect.abs()).reduce(|a, b| a.union(b)).unwrap_or(DocRect::ZERO)
}

/// `rect` with `handle` dragged to the pointer at screen position `pos`, measured from the handle's own point.
fn resize_to(view: &craft_behavior::CanvasView, rect: DocRect, handle: [i8; 2], pos: Pos2, mods: Mods, snap: &dyn Fn(Axis, f64) -> f64) -> DocRect {
    let doc = view.to_doc(pos);
    let to = Point::new(snap(Axis::Y, doc.x), snap(Axis::X, doc.y));
    resized(rect, handle, to - handle_point(rect, handle), mods)
}

impl Component for ArtboardOverlay<'_> {
    const ROLE: Role = Role::GROUP;
    const OVERLAYS: bool = false;
    const KEYS: KeyUse = KeyUse::CANVAS.with_moves().with_deletes().with_chrome_key(false);
    type Style = ArtboardOverlayStyle;
    type Output = ArtboardOverlayOutput;

    fn style(cx: &StyleCx<'_>, _: ArtboardOverlayStyleKey) -> ArtboardOverlayStyle {
        ArtboardOverlayStyle { metrics: Metrics::new(), handle: cx.handle() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    /// The boards, then the active board's handles, in one roving group.
    fn parts(&mut self, _rect: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let view = space.view;
        for (index, item) in self.items.iter().enumerate() {
            let part = cx
                .part(BOARD.at(index as u64), HitShape::Polygon(view.quad(item.rect.abs()).to_vec()))
                .sense(Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS)
                .priority(HitPriority::new(HitLevel::Body, 0))
                .cursor(if item.locked { CursorIcon::Default } else { CursorIcon::Move })
                .focusable();
            if self.selected.first() == Some(&index) {
                part.selected().flags(Flags::SELECTED);
            }
        }
        if let Some(active) = self.handles_shown() {
            let radius = cx.style.handle.side / 2.0;
            for (h, handle) in HANDLES.into_iter().enumerate() {
                let at = view.to_screen(handle_point(self.items[active].rect.abs(), handle));
                cx.part(HANDLE.at(h as u64), HitShape::circle(at, radius))
                    .sense(Sense::DRAG | Sense::TAKE_FOCUS)
                    .priority(HitPriority::new(HitLevel::Handle, 0))
                    .cursor(resize_cursor(&view, DocVec::new(f64::from(handle[0]), f64::from(handle[1]))))
                    .focusable();
            }
        }
        if !self.items.is_empty() {
            cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
        }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let Some(space) = canvas_space(&cx.env) else { return };
        let snap = canvas_snap(&cx.env);
        self.pointer(cx, &space, snap.as_ref());
        self.keys(cx, &space);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let (Some(space), Some(active)) = (canvas_space(&cx.env), self.handles_shown()) else { return };
        let draw = ui.draw();
        let look = &cx.style.handle;
        for (h, handle) in HANDLES.into_iter().enumerate() {
            let at = space.view.to_screen(handle_point(self.items[active].rect.abs(), handle));
            let square = PaintShape::Rect(Rect::from_center_size(at, Vec2::splat(look.side)), CornerRadius::ZERO);
            draw.fill(&square, cx.resolve_part(HANDLE.at(h as u64), &look.fill));
            draw.stroke(&square, StrokeWidth::Thin, look.stroke);
        }
    }

    /// One node per board and per handle of the active board.
    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<ArtboardToolWords>();
        let format = cx.format();
        let count = |value: f64| format.number_ungrouped(value, 0);
        for (index, item) in self.items.iter().enumerate() {
            let r = item.rect.abs();
            let name = words.board.fill(&[
                ("index", &count((index + 1) as f64)),
                ("width", &count(r.width())),
                ("height", &count(r.height())),
                ("x", &count(r.x0)),
                ("y", &count(r.y0)),
            ]);
            access.node(BOARD.at(index as u64), Role::BUTTON).name = Name::Text(name);
        }
        if let Some(active) = self.handles_shown() {
            for (h, handle) in HANDLES.into_iter().enumerate() {
                let at = handle_point(self.items[active].rect.abs(), handle);
                let name = match handle {
                    [-1, -1] => &words.handle_top_left,
                    [1, -1] => &words.handle_top_right,
                    [-1, 1] => &words.handle_bottom_left,
                    [1, 1] => &words.handle_bottom_right,
                    [0, -1] => &words.handle_top,
                    [0, 1] => &words.handle_bottom,
                    [-1, 0] => &words.handle_left,
                    _ => &words.handle_right,
                };
                let node = access.node(HANDLE.at(h as u64), Role::BUTTON);
                node.name = Name::Text(name.as_str().to_owned());
                node.description = Some(words.handle_at.fill(&[("x", &count(at.x)), ("y", &count(at.y))]));
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> ArtboardOverlayOutput {
        ArtboardOverlayOutput { response: cx.response.clone(), events: self.events, dragging: self.dragging }
    }
}

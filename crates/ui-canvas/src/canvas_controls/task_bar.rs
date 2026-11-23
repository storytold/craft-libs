//! `TaskBar`: the contextual task bar over a canvas: hung below its anchor until the user moves it, over a selection,
//! fixed beside its anchor, or fixed at the canvas view's top edge.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::CanvasControlWords;
use crate::bars::{Bar, BarKind};
use crate::buttons::IconButton;
use crate::canvas::canvas_space;
use crate::contract::overlay::{Anchor, Dismiss, Outside, Place, PlacementChange, PlacementKind};
use crate::contract::{
    Bind, BindMut, Common, Component, Container, ContainerProps, Cx, DragBand, HeaderRegion, Inner, Mount, NoValue, Outcome, Overlay, Pad, PopupKind,
    Props as _, Role, Show as _, ShowContainer as _, Sizing, Trigger,
};
use crate::display::Icon;
use crate::menu::{Menu, MenuItem};
use crate::primitives::IconSpec;
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::kurbo::Point;
use craft_behavior::overlay::{Align, Side};
use craft_behavior::{EditEvent, GesturePhase, IconId, Metrics, Mixed, Sense};
use egui::{Pos2, Rect, Response, Ui, Vec2};
use std::sync::Arc;

/// Where the task bar sits: pinned or following its anchor, and where the user moved it. The one owner of the bar's
/// place; the serde shape is PhotoCraft's, so saved preferences load unchanged, and a non-finite `at` counts as unset.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct TaskBarPlace {
    /// The bar keeps its place instead of following the anchor.
    #[serde(default)]
    pub pinned: bool,
    /// The bar's top left corner relative to the area's, once moved; `None` while it follows the anchor.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
}

impl TaskBarPlace {
    /// A place that is pinned or follows the anchor, with the top left corner `at` once moved.
    pub const fn new(pinned: bool, at: Option<[f32; 2]>) -> Self {
        Self { pinned, at }
    }

    /// The bar's top left in `area` while it has a place; `None` while it follows its anchor. A non-finite `at`
    /// counts as unset.
    fn placed_in(&self, area: Rect) -> Option<Pos2> {
        let [x, y] = self.at.filter(|at| at.iter().all(|v| v.is_finite()))?;
        Some(Pos2::new(area.min.x + x, area.min.y + y))
    }
}

/// The contextual task bar: a movable modeless layer hung below the screen bounds of `anchor` while its place is
/// unset, with a body of members and a More menu (Hide, Reset, Pin). One place, pin and Hide setting for Crop and
/// Frame.
///
/// - Place: centred under the anchor's bounds on screen, a section gap below them, following the anchor each pass.
///   Dragged by its grip (any free point of the bar), it stays where it is dropped and keeps its left edge while the
///   anchor moves or the bar grows; a press with no drag changes nothing. Pinned, it stays where it shows.
/// - Area: the bar shows inside `area` (the canvas rect): a move stays inside it, and a stored place outside it comes
///   back inside, its left and top edges winning where the bar does not fit.
/// - Menu: Hide bar reports `hide`; Reset bar position clears the place, so the bar follows its anchor again; Pin bar
///   position keeps it where it shows.
/// - `place` is the caller's value (`Bind::bound`): the bar reads it before it shows and writes the pass's change
///   after.
///
/// A selection bar ([`TaskBar::selection`]) hangs above a selection, flipped and kept inside `area`. A fixed bar
/// ([`TaskBar::fixed`]) hangs centred a section gap below its anchor's bounds, above them when there is no room below,
/// and follows the anchor. An edge bar ([`TaskBar::at_edge`]) hangs at the top edge of `area`, the canvas view's rect,
/// and takes no anchor. None of them has a grip, a move or a More menu, and none reports `hide`.
///
/// Nothing shows outside a canvas, while `area` is not finite or has no size, or, for an anchored bar, while the
/// anchor's bounds are not finite.
#[must_use = "show the task bar with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct TaskBar<'a> {
    label: Label,
    hang: Hang<'a>,
    place: Bind<'a, TaskBarPlace>,
    /// Which bar this is: the task bar, a bar over a selection, or a fixed bar.
    #[prop]
    kind: TaskBarKind,
    /// The side of the anchor the bar hangs on; `None` gives the kind's side: above for `Selection`, below for `Task`.
    #[prop]
    side: Option<Side>,
    /// What an outside press does to the bar.
    #[prop]
    dismiss: TaskBarDismiss,
    /// The bar is compact: its members at the small control size in a tight frame with a popup's smaller corners, as
    /// PdfCraft's text box bar shows Done and Discard (production's 24-point buttons in a 2-point frame). Off by
    /// default.
    #[prop]
    compact: bool,
}

/// What a bar hangs from: the screen bounds of document points, or the canvas view's top edge, aligned in reading
/// direction.
#[derive(Clone, Copy)]
enum Hang<'a> {
    Points(&'a [Point]),
    Edge(Align),
}

/// Which bar a [`TaskBar`] is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskBarKind {
    /// The contextual task bar, hung below its anchor.
    #[default]
    Task,
    /// A bar anchored above a selection.
    Selection,
    /// A bar that hangs where it is placed, below its anchor or at the view's top edge: no grip, no move band and no
    /// More menu.
    Fixed,
}

/// What an outside press does to a [`TaskBar`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskBarDismiss {
    /// An outside press closes the bar.
    #[default]
    Close,
    /// The bar survives outside presses while its graphic is selected.
    Keep,
}

/// What the task bar reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct TaskBarOutput {
    /// The bar's response.
    pub response: Response,
    /// Hide bar was picked from the More menu.
    pub hide: bool,
    /// The bar's rect on screen.
    pub rect: Rect,
    /// The bar closed: an outside press, Escape or its anchor going. The caller stops showing it.
    pub closed: bool,
}

impl Outcome for TaskBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The context the task bar's body gets.
pub struct TaskBarCx<'a> {
    menu: &'a mut MoreMenu,
}

impl TaskBarCx<'_> {
    /// Places the More menu button here, before the members that follow it (the Crop bar puts Cancel and Done after
    /// it); without the call the button comes after the body. A selection or fixed bar has no More menu: the call places
    /// nothing.
    pub fn more_menu(&mut self, ui: &mut Ui) {
        self.menu.place(ui);
    }
}

/// A row of the More menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuPick {
    Hide,
    Reset,
    Pin,
}

/// The More button and its menu, placed once per run of the bar's body.
struct MoreMenu {
    words: Arc<CanvasControlWords>,
    /// Whether the bar has a More menu: only the task bar has one.
    offered: bool,
    placed: bool,
    pick: Option<MenuPick>,
}

impl MoreMenu {
    /// Shows the button and its menu here unless this run of the body showed them already.
    fn place(&mut self, ui: &mut Ui) {
        if !self.offered || std::mem::replace(&mut self.placed, true) {
            return;
        }
        let words = Arc::clone(&self.words);
        let more = IconButton::new(IconId::new("more")).popup(PopupKind::Menu).label(words.task_bar_more.clone()).show(ui);
        let mut pick = None;
        let rows = [
            (MenuPick::Hide, "eye_off", &words.task_bar_hide),
            (MenuPick::Reset, "reset", &words.task_bar_reset),
            (MenuPick::Pin, "pin", &words.task_bar_pin),
        ];
        let _ = Menu::new(words.task_bar_more.clone()).opened_by(&more, Trigger::Click).show(ui, |ui, _| {
            for (row, icon, name) in rows {
                if MenuItem::new(name.clone()).icon(IconSpec::svg(IconId::new(icon))).show(ui).ran {
                    pick = Some(row);
                }
            }
        });
        self.pick = pick.or(self.pick);
    }
}

impl<'a> TaskBar<'a> {
    /// A bar named `label` hung below the screen bounds of `anchor` (document points) until `place` moves it.
    pub fn new(label: impl Into<Label>, anchor: &'a [Point], place: Bind<'a, TaskBarPlace>) -> Self {
        TaskBar {
            label: label.into(),
            hang: Hang::Points(anchor),
            place,
            kind: TaskBarKind::Task,
            side: None,
            dismiss: TaskBarDismiss::Close,
            compact: false,
        }
    }

    /// A fixed bar named `label` hung below the screen bounds of `anchor`, above them when there is no room below.
    pub fn fixed(label: impl Into<Label>, anchor: &'a [Point]) -> Self {
        TaskBar::anchored(label, Hang::Points(anchor), TaskBarKind::Fixed)
    }

    /// A fixed bar at the canvas view's top edge, aligned `edge` in reading direction and inset by the regular pad.
    pub fn at_edge(label: impl Into<Label>, edge: Align) -> Self {
        TaskBar::anchored(label, Hang::Edge(edge), TaskBarKind::Fixed)
    }

    /// A bar of `kind` that never moves, so it keeps no place.
    fn anchored(label: impl Into<Label>, hang: Hang<'a>, kind: TaskBarKind) -> Self {
        TaskBar {
            label: label.into(),
            hang,
            place: Bind::given(Mixed::Same(TaskBarPlace::default())),
            kind,
            side: None,
            dismiss: TaskBarDismiss::Close,
            compact: false,
        }
    }

    /// A bar named `label` anchored above or at `side` of a selection whose bounds are `anchor` (document points),
    /// flipped and clamped to fit.
    pub fn selection(label: impl Into<Label>, anchor: &'a [Point]) -> Self {
        TaskBar::anchored(label, Hang::Points(anchor), TaskBarKind::Selection)
    }

    /// Shows the bar inside `area`; `body` fills its members.
    pub fn show<R>(self, ui: &mut Ui, area: Rect, mut body: impl FnMut(&mut Ui, &mut TaskBarCx<'_>) -> R) -> Inner<TaskBarOutput, R> {
        let TaskBar { label, hang, mut place, kind, side, dismiss, compact } = self;
        let side = side.unwrap_or(match kind {
            TaskBarKind::Task | TaskBarKind::Fixed => Side::Below,
            TaskBarKind::Selection => Side::Above,
        });
        let env = Env::of(ui);
        let words = env.words::<CanvasControlWords>();
        let movable = kind == TaskBarKind::Task;
        let room = area.is_finite() && area.is_positive();
        let hung = match hang {
            Hang::Points(anchor) => {
                // The anchor maps through this pass's view, so the bar follows a scroll or zoom in the same pass.
                let bounds = canvas_space(&env.view()).map_or(Rect::NOTHING, |space| {
                    anchor.iter().fold(Rect::NOTHING, |mut bounds, point| {
                        bounds.extend_with(space.view.to_screen(*point));
                        bounds
                    })
                });
                let gap = env.tokens.space.gap.section;
                // A fixed bar keeps the gap on both sides of its anchor: it opens above when there is no room below.
                let rect = if kind == TaskBarKind::Fixed { bounds.expand2(Vec2::new(0.0, gap)) } else { hang_rect(bounds, gap, side) };
                (room && bounds.is_finite()).then_some((rect, Align::Center))
            }
            Hang::Edge(align) => room.then(|| (edge_rect(area, env.tokens.space.pad.regular), align)),
        };
        let shows = hung.is_some();
        let held = match place.reborrow().get() {
            Some(Mixed::Same(held)) => held,
            _ => TaskBarPlace::default(),
        };
        let overlay = match kind {
            TaskBarKind::Selection => selection_overlay(hung.map(|(rect, _)| rect), area, dismiss, side),
            TaskBarKind::Task | TaskBarKind::Fixed => layer_overlay(&held, hung, area, side, movable),
        };
        let layer = TaskBarLayer::new(label, overlay, shows, kind);
        let mut menu = MoreMenu { words: Arc::clone(&words), offered: movable, placed: false, pick: None };
        // Only the task bar moves, so only it starts with the grip.
        let grip = movable.then(|| words.task_bar_move.clone());
        let mut shown = layer.show(ui, |ui, _| {
            // The bar is the one surface: the grip is its first member, on the members' line inside its padding.
            let bar = Bar::new(Label::Empty, BarKind::Tool).surface(SurfaceKind::Floating);
            // A compact bar takes a popup's smaller corners and lighter shadow, as production's small frame.
            let bar = if compact { bar.compact().surface(SurfaceKind::Popup) } else { bar };
            bar.show(ui, |ui, _| {
                if let Some(grip) = &grip {
                    let _ = Icon::new(IconSpec::svg(IconId::new("grip"))).label(grip.clone()).show(ui);
                }
                menu.placed = false;
                let inner = body(ui, &mut TaskBarCx { menu: &mut menu });
                menu.place(ui);
                inner
            })
        });
        let rect = shown.output.response.rect;
        let pick = menu.pick;
        if shows && movable {
            // Only a drag's drop reports a move (`placement_changed` keeps the Commit): a press on the grip with no drag
            // reports none, so no place changes. The layer shows at the drop already, so the move is the bar's rect.
            let next = settled(held, rect, area, shown.output.moved, pick);
            if next != held {
                let word = match pick {
                    Some(MenuPick::Reset) => &words.task_bar_reset,
                    Some(MenuPick::Pin) => &words.task_bar_pin,
                    _ => &words.task_bar_move,
                };
                let event = EditEvent::set(Mixed::Same(held), next, Arc::from(word.as_str()), None);
                let _ = shown.settle(|cx, _| place.reborrow().write(cx, &event));
            }
        }
        let output =
            TaskBarOutput { response: shown.output.response, hide: pick == Some(MenuPick::Hide), rect, closed: shown.output.closed.is_some() };
        match shown.inner {
            Some(bar) => Inner { output, inner: bar.inner, settle: bar.settle },
            None => Inner { output, inner: None, settle: None },
        }
    }
}

/// The rect the bar hangs from: the anchor's screen `bounds` grown by `gap` on the bar's `side`. A side along the
/// reading direction grows both ends, so the gap is kept in either direction. The overlay host keeps the layer inside
/// `Overlay::bounds`.
fn hang_rect(bounds: Rect, gap: f32, side: Side) -> Rect {
    match side {
        Side::Above => Rect::from_min_max(Pos2::new(bounds.min.x, bounds.min.y - gap), bounds.max),
        Side::Below => Rect::from_min_max(bounds.min, Pos2::new(bounds.max.x, bounds.max.y + gap)),
        _ => bounds.expand2(egui::vec2(gap, 0.0)),
    }
}

/// The rect an edge bar hangs below: the top edge of `area`, inset by `pad` on its sides, with no height.
fn edge_rect(area: Rect, pad: f32) -> Rect {
    Rect::from_min_max(Pos2::new(area.left() + pad, area.top()), Pos2::new(area.right() - pad, area.top()))
}

/// The layer of a selection bar: hung on `side` of `hang`, flipped and kept inside `area`, never moved by the user.
/// `dismiss` says what an outside press does to it.
fn selection_overlay(hang: Option<Rect>, area: Rect, dismiss: TaskBarDismiss, side: Side) -> Overlay {
    let outside = match dismiss {
        TaskBarDismiss::Close => Outside::Close,
        TaskBarDismiss::Keep => Outside::Keep,
    };
    let overlay = Overlay::floating().modeless().body_scroll(false).dismiss(Dismiss::new().outside(outside));
    match hang {
        Some(hang) => overlay.bounds(area).place(Place::anchored(Anchor::Rect(hang), side, Align::Center)),
        None => overlay,
    }
}

/// The layer the bar shows in: hung on `side` of `hang`, aligned as it says, while `held` has no place, at its place
/// once it has one. Only a `movable` bar moves, and a move stays inside `area`. A bar that does not show (`hang` is
/// `None`) gets a closed layer with no place.
fn layer_overlay(held: &TaskBarPlace, hang: Option<(Rect, Align)>, area: Rect, side: Side, movable: bool) -> Overlay {
    let overlay = Overlay::floating().modeless().movable(movable).body_scroll(false);
    let Some((hang, align)) = hang else { return overlay };
    let overlay = overlay.bounds(area);
    match held.placed_in(area).filter(|_| movable) {
        Some(at) => overlay.placed_at(at),
        None => overlay.place(Place::anchored(Anchor::Rect(hang), side, align)),
    }
}

/// The place after a pass that showed the bar at `shown`. A move that ended (`moved`, where it was dropped) keeps the
/// bar there; Reset clears the place; Pin, and a pinned bar with no place, keep the bar at `shown`. A
/// non-finite place is cleared. With no move and no pick, the place stays unchanged; the overlay host keeps the layer
/// inside the area.
fn settled(held: TaskBarPlace, shown: Rect, area: Rect, moved: Option<Rect>, pick: Option<MenuPick>) -> TaskBarPlace {
    let offset = |rect: Rect| -> [f32; 2] { (rect.min - area.min).into() };
    let placed = held.placed_in(area).is_some();
    let mut next = TaskBarPlace { at: held.at.filter(|_| placed), ..held };
    if let Some(moved) = moved {
        next.at = Some(offset(moved));
    }
    match pick {
        Some(MenuPick::Reset) => next = TaskBarPlace::default(),
        Some(MenuPick::Pin) => next = TaskBarPlace { pinned: true, at: Some(offset(shown)) },
        Some(MenuPick::Hide) | None => {}
    }
    if next.pinned && next.at.is_none() {
        next.at = Some(offset(shown));
    }
    next
}

/// The style of the task bar's layer: the surface comes from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TaskBarLayerStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What the task bar's layer reports: its response, the move that ended in this pass and why it closed.
#[derive(Debug)]
struct TaskBarLayerOutput {
    response: Response,
    moved: Option<Rect>,
    closed: Option<CloseReason>,
}

impl Outcome for TaskBarLayerOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The task bar's layer: it holds the bar, which paints the one floating surface and starts with the grip. The layer
/// paints nothing of its own, so the bar reads as one rounded shape. Its inline header is empty: it is there for its
/// move band, which lies under the whole line, so the grip and every free point between the members move the layer; a
/// selection or fixed bar's layer has no header.
#[derive(crate::contract::Component)]
#[component(container, unregistered)]
struct TaskBarLayer {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// Open while the bar shows; the caller shows the bar again each pass it wants it.
    open: Bind<'static, bool>,
    /// Where a move by the grip ended in this pass. The layer host shows the bar under the pointer while the move
    /// runs, so the place is written once, on the drop.
    moved: Option<Rect>,
    /// Why the layer closed, once it has.
    closed: Option<CloseReason>,
}

impl TaskBarLayer {
    /// A layer for a bar of `kind`: only the task bar has the empty inline header whose move band moves it.
    fn new(label: Label, overlay: Overlay, open: bool, kind: TaskBarKind) -> Self {
        let mut props = ContainerProps::new().mount(Mount::Layer(overlay)).pad(Pad::None).sizing(Sizing::Content);
        let salt = match kind {
            TaskBarKind::Task => {
                props = props.header(HeaderRegion::new().band(DragBand::Move).inline(true));
                "task-bar"
            }
            TaskBarKind::Selection => "selection-bar",
            TaskBarKind::Fixed => "task-bar",
        };
        TaskBarLayer { common: Common::new(), props, open: Bind::given(open), moved: None, closed: None }.label(label).id_salt(salt)
    }
}

impl Component for TaskBarLayer {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = TaskBarLayerStyle;
    type Output = TaskBarLayerOutput;

    fn style(_: &StyleCx<'_>, _: TaskBarLayerStyleKey) -> TaskBarLayerStyle {
        TaskBarLayerStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn finish(self, cx: Cx<'_, Self>) -> TaskBarLayerOutput {
        TaskBarLayerOutput { response: cx.response.clone(), moved: self.moved, closed: self.closed }
    }
}

impl Container for TaskBarLayer {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }

    fn closed(&mut self, why: CloseReason) {
        self.closed.get_or_insert(why);
    }

    fn placement_changed(&mut self, change: &PlacementChange) {
        if change.kind == PlacementKind::Move && change.phase == GesturePhase::Commit {
            self.moved = Some(change.rect);
        }
    }
}

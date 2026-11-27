//! `TaskBar`: the contextual task bar over a canvas: hung beside its anchor until the user moves it, over a selection,
//! fixed beside its anchor, or fixed at the canvas view's top edge.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::CanvasControlWords;
use crate::bars::{Bar, BarKind};
use crate::buttons::IconButton;
use crate::canvas::canvas_space;
use crate::contract::overlay::{Anchor, Dismiss, Outside, Place, PlacementChange, PlacementKind};
use crate::contract::{
    Bind, BindMut, Common, Component, Container, ContainerProps, Cx, DragBand, Gap, HeaderRegion, Inner, MeasureCx, Mount, NoValue, Outcome, Overlay,
    Pad, PaintUi, PopupKind, Props as _, Role, Show as _, ShowContainer as _, Sizing, Styled as _, Trigger,
};
use crate::display::{Divider, Icon};
use crate::menu::{Menu, MenuItem, PanelMenu, PanelRow};
use crate::primitives::{IconSpec, PaintShape};
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::style::{ComponentStyle, StyleColor, StyleCx, SurfaceKind, UiTokens};
use craft_behavior::kurbo::Point;
use craft_behavior::overlay::{Align, PlaceRequest, Side};
use craft_behavior::{EditEvent, GesturePhase, IconId, Metrics, Mixed, Sense};
use craft_theme::{Stateful, States};
use egui::{Pos2, Rect, Response, Ui, Vec2};
use std::sync::Arc;

/// Where the task bar sits: pinned or following its anchor, and where the user moved it. The one owner of the bar's
/// place; the serde shape is PhotoCraft's, so saved preferences load unchanged, and a non-finite `at` or `offset`
/// counts as unset.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct TaskBarPlace {
    /// The bar keeps its place instead of following the anchor.
    #[serde(default)]
    pub pinned: bool,
    /// The bar's top left corner relative to the area's while it is pinned; an unpinned place that holds it turns it
    /// into `offset` at once.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    /// Where an unpinned bar sits relative to its anchor, once moved; the bar follows the anchor with this offset.
    #[serde(default)]
    pub offset: Option<[f32; 2]>,
}

impl TaskBarPlace {
    /// A place that is pinned or follows the anchor, with the top left corner `at` once moved.
    pub const fn new(pinned: bool, at: Option<[f32; 2]>) -> Self {
        Self { pinned, at, offset: None }
    }

    /// The bar's top left in `area` while it has a place; `None` while it follows its anchor. A non-finite `at`
    /// counts as unset.
    fn placed_in(&self, area: Rect) -> Option<Pos2> {
        let [x, y] = self.at.filter(|at| at.iter().all(|v| v.is_finite()))?;
        Some(Pos2::new(area.min.x + x, area.min.y + y))
    }

    /// How far the bar sits from its unmoved place; `None` while it sits there. A non-finite `offset` counts as unset.
    fn offset_in(&self) -> Option<Vec2> {
        let [x, y] = self.offset.filter(|offset| offset.iter().all(|v| v.is_finite()))?;
        Some(Vec2::new(x, y))
    }
}

/// The contextual task bar: a movable modeless layer hung on a side of the screen bounds of `anchor`, with a body of
/// members and a More menu (Hide, Pin, Reset, then the host's rows). One place, pin and Hide setting for Crop and
/// Frame.
///
/// - Place: on `side` of the anchor's bounds on screen (below, centred, by default), a section gap away (or as far as
///   [`hang_gap`](Self::hang_gap) says), following the anchor each pass; it flips to the other side when there is no
///   room. Dragged by its grip (any free point of the bar), it keeps its offset from that place while the anchor moves
///   or the bar grows; a press with no drag changes nothing. A moved bar whose offset place would cover the `keep_clear` point shows at its unmoved place and keeps
///   its offset. Pinned, it stays where it shows.
/// - Area: the bar shows inside `area` (the canvas rect): a move stays inside it, and a stored place outside it comes
///   back inside, its left and top edges winning where the bar does not fit. An anchor that leaves no room on either
///   side (a crop frame larger than the view) has the bar over it, on its side's edge of the area, with all its
///   members. Its members continue on a new line when the area is too narrow for them.
/// - Menu: Hide bar reports `hide`; Reset bar position clears the place, so the bar follows its anchor again; Pin bar
///   position keeps it where it shows and, picked again, lets it follow its anchor from where it shows.
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
    rows: Vec<PanelRow>,
    /// Which bar this is: the task bar, a bar over a selection, or a fixed bar.
    #[prop]
    kind: TaskBarKind,
    /// The side of the anchor the bar hangs on; `None` gives the kind's side: above for `Selection`, below for `Task`.
    side: Option<Side>,
    align: Align,
    /// A moved bar whose offset place would cover this point (a document point, like the anchor) shows at its unmoved
    /// place and keeps its offset.
    #[prop]
    keep_clear: Option<Point>,
    /// What an outside press does to the bar.
    #[prop]
    dismiss: TaskBarDismiss,
    /// The bar is compact: its members at the small control size in a tight frame with a popup's smaller corners, as
    /// PdfCraft's text box bar shows Done and Discard (production's 24-point buttons in a 2-point frame). Off by
    /// default.
    #[prop]
    compact: bool,
    /// How the task bar's grip looks: the `grip` icon by default.
    #[prop]
    grip: TaskBarGrip,
    /// The space between the bar's members, in place of the tool bar's tight gap: `Gap::Item` sets them a control gap
    /// apart.
    #[prop(into)]
    member_gap: Option<Gap>,
    /// How far from its anchor's bounds a bar hung from document points hangs: a section gap by default.
    #[prop]
    hang_gap: TaskBarHang,
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

/// How the grip of a [`TaskBar`] looks. Either way the grip takes no press of its own: the bar's move band under it
/// moves the bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskBarGrip {
    /// The `grip` icon.
    #[default]
    Icon,
    /// One rounded upright bar, a value track's groove wide and the members' line less a row gap at each end tall, in
    /// the control edge's line colour; muted text under the pointer and while the bar moves (VectorCraft's).
    Bar,
}

/// How far from its anchor's bounds a [`TaskBar`] hangs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TaskBarHang {
    /// A section gap away.
    #[default]
    Gap,
    /// The handle ring and a section gap away: clear of a selection's handles on that side and their hit area.
    ClearOfHandles,
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
    /// Whether the bar is pinned: the Pin row shows a check mark.
    pinned: bool,
    /// The host's rows, after the bar's own.
    rows: Vec<PanelRow>,
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
            (MenuPick::Hide, "eye_off", &words.task_bar_hide, false),
            (MenuPick::Pin, "pin", &words.task_bar_pin, self.pinned),
            (MenuPick::Reset, "reset", &words.task_bar_reset, false),
        ];
        let host = &self.rows;
        let _ = Menu::new(words.task_bar_more.clone()).opened_by(&more, Trigger::Click).show(ui, |ui, _| {
            for (row, icon, name, checked) in rows {
                if MenuItem::new(name.clone()).icon(IconSpec::svg(IconId::new(icon))).check(checked).show(ui).ran {
                    pick = Some(row);
                }
            }
            if !host.is_empty() {
                Divider::horizontal().show(ui);
                let _ = PanelMenu::new(host.clone(), Vec::new()).show(ui);
            }
        });
        self.pick = pick.or(self.pick);
    }
}

/// Whether the More menu offers `row`: a dock row (`PanelRow::Item`), which only a dock reads, does not belong in it.
fn offered(row: &PanelRow) -> bool {
    match row {
        PanelRow::Item(_) => false,
        PanelRow::Submenu(_, rows) => rows.iter().all(offered),
        _ => true,
    }
}

impl<'a> TaskBar<'a> {
    /// A bar named `label` hung below the screen bounds of `anchor` (document points) until `place` moves it.
    pub fn new(label: impl Into<Label>, anchor: &'a [Point], place: Bind<'a, TaskBarPlace>) -> Self {
        TaskBar {
            label: label.into(),
            hang: Hang::Points(anchor),
            place,
            rows: Vec::new(),
            kind: TaskBarKind::Task,
            side: None,
            align: Align::Center,
            keep_clear: None,
            dismiss: TaskBarDismiss::Close,
            compact: false,
            grip: TaskBarGrip::Icon,
            member_gap: None,
            hang_gap: TaskBarHang::Gap,
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
            rows: Vec::new(),
            kind,
            side: None,
            align: Align::Center,
            keep_clear: None,
            dismiss: TaskBarDismiss::Close,
            compact: false,
            grip: TaskBarGrip::Icon,
            member_gap: None,
            hang_gap: TaskBarHang::Gap,
        }
    }

    /// A bar named `label` anchored above or at `side` of a selection whose bounds are `anchor` (document points),
    /// flipped and clamped to fit.
    pub fn selection(label: impl Into<Label>, anchor: &'a [Point]) -> Self {
        TaskBar::anchored(label, Hang::Points(anchor), TaskBarKind::Selection)
    }

    /// Adds `rows` to the More menu after Hide, Pin and Reset and a divider: command rows take their label, chord, check
    /// and enabled state from the command, and submenus show as in panel menus. A dock row (`PanelRow::Item`) is not
    /// offered.
    pub fn more_rows(mut self, rows: Vec<PanelRow>) -> Self {
        debug_assert!(rows.iter().all(offered), "the task bar's More menu offers no dock rows");
        self.rows = rows;
        self
    }

    /// Hangs the bar on `side` of the anchor's screen bounds, aligned by `align`, in place of the kind's side (below
    /// for the task bar, above for a selection bar) and centred.
    pub fn side(mut self, side: Side, align: Align) -> Self {
        self.side = Some(side);
        self.align = align;
        self
    }

    /// Shows the bar inside `area`; `body` fills its members.
    pub fn show<R>(self, ui: &mut Ui, area: Rect, mut body: impl FnMut(&mut Ui, &mut TaskBarCx<'_>) -> R) -> Inner<TaskBarOutput, R> {
        let TaskBar { label, hang, mut place, rows, kind, side, align, keep_clear, dismiss, compact, grip: grip_look, member_gap, hang_gap } = self;
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
                let space = canvas_space(&env.view());
                let to_screen = |point: Point| space.as_ref().map(|space| space.view.to_screen(point));
                let bounds = anchor.iter().filter_map(|point| to_screen(*point)).fold(Rect::NOTHING, |mut bounds, at| {
                    bounds.extend_with(at);
                    bounds
                });
                let clear = if hang_gap == TaskBarHang::ClearOfHandles { env.tokens.size.handle.ring } else { 0.0 };
                let gap = env.tokens.space.gap.section + clear;
                // A fixed bar keeps the gap on both sides of its anchor: it opens above when there is no room below.
                let rect = if kind == TaskBarKind::Fixed { bounds.expand2(Vec2::new(0.0, gap)) } else { hang_rect(bounds, gap, side) };
                (room && bounds.is_finite()).then(|| Hung {
                    rect,
                    side,
                    align,
                    clear: keep_clear.and_then(to_screen).map(|at| Rect::from_min_max(at, at)),
                })
            }
            Hang::Edge(edge_align) => {
                room.then(|| Hung { rect: edge_rect(area, env.tokens.space.pad.regular), side, align: edge_align, clear: None })
            }
        };
        let shows = hung.is_some();
        let held = match place.reborrow().get() {
            Some(Mixed::Same(held)) => held,
            _ => TaskBarPlace::default(),
        };
        let rtl = env.layer_direction().is_rtl();
        let overlay = match kind {
            TaskBarKind::Selection => selection_overlay(hung.as_ref(), area, dismiss),
            TaskBarKind::Task | TaskBarKind::Fixed => layer_overlay(&held, hung.as_ref(), area, movable),
        };
        let layer = TaskBarLayer::new(label, overlay, shows, kind);
        let mut menu = MoreMenu { words: Arc::clone(&words), offered: movable, pinned: held.pinned, rows, placed: false, pick: None };
        // Only the task bar moves, so only it starts with the grip.
        let grip = movable.then(|| words.task_bar_move.clone());
        let mut shown = layer.show(ui, |ui, layer| {
            let moving = layer.moving();
            // The bar is the one surface: the grip is its first member, on the members' line inside its padding.
            let bar = Bar::new(Label::Empty, BarKind::Tool).surface(SurfaceKind::Floating).wrap();
            // A compact bar takes a popup's smaller corners and lighter shadow, as production's small frame.
            let bar = if compact { bar.compact().surface(SurfaceKind::Popup) } else { bar };
            let bar = match member_gap {
                Some(gap) => bar.gap(gap),
                None => bar,
            };
            bar.show(ui, |ui, _| {
                match (&grip, grip_look) {
                    (Some(grip), TaskBarGrip::Bar) => {
                        let _ = GripBar { common: Common::new(), moving }.label(grip.clone()).show(ui);
                    }
                    (Some(grip), _) => {
                        let _ = Icon::new(IconSpec::svg(IconId::new("grip"))).label(grip.clone()).show(ui);
                    }
                    (None, _) => {}
                }
                menu.placed = false;
                let inner = body(ui, &mut TaskBarCx { menu: &mut menu });
                menu.place(ui);
                inner
            })
        });
        let rect = shown.output.response.rect;
        let pick = menu.pick;
        if let Some(hung) = hung.as_ref().filter(|_| movable) {
            // Only a drag's drop reports a move (`placement_changed` keeps the Commit): a press on the grip with no drag
            // reports none, so no place changes. The layer shows at the drop already, so the move is the bar's rect.
            let next = settled(held, rect, area, shown.output.moved, pick, hung.unmoved(rect.size(), area, rtl));
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

/// What the bar hangs from, in screen space.
struct Hung {
    /// The anchor's bounds grown by the section gap on the hanging side ([`hang_rect`]).
    rect: Rect,
    side: Side,
    align: Align,
    /// A rect the bar keeps clear of while it is offset.
    clear: Option<Rect>,
}

impl Hung {
    /// Where a bar of `size` hangs inside `area` before any offset: the place the overlay host computes for the
    /// anchored layer, taken from the one placement rule.
    fn unmoved(&self, size: Vec2, area: Rect, rtl: bool) -> Rect {
        let request = PlaceRequest::new(self.rect, size, area).side(self.side).align(self.align).rtl(rtl).flips_across(true).covers_anchor(true);
        craft_behavior::overlay::place(&request).rect
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

/// The layer of a selection bar: hung from `hang`, flipped and kept inside `area`, never moved by the user. `dismiss`
/// says what an outside press does to it.
fn selection_overlay(hang: Option<&Hung>, area: Rect, dismiss: TaskBarDismiss) -> Overlay {
    let outside = match dismiss {
        TaskBarDismiss::Close => Outside::Close,
        TaskBarDismiss::Keep => Outside::Keep,
    };
    let overlay = Overlay::floating().modeless().body_scroll(false).dismiss(Dismiss::new().outside(outside));
    match hang {
        Some(hang) => overlay.bounds(area).place(Place::anchored(Anchor::Rect(hang.rect), hang.side, hang.align).covering_anchor()),
        None => overlay,
    }
}

/// The layer the bar shows in: at its place while `held` has one, else hung from `hang` with its offset, keeping clear
/// of `hang.clear`. Only a `movable` bar moves, and a move stays inside `area`. A bar that does not show (`hang` is
/// `None`) gets a closed layer with no place.
fn layer_overlay(held: &TaskBarPlace, hang: Option<&Hung>, area: Rect, movable: bool) -> Overlay {
    let overlay = Overlay::floating().modeless().movable(movable).body_scroll(false);
    let Some(hang) = hang else { return overlay };
    let overlay = overlay.bounds(area);
    if let Some(at) = held.placed_in(area).filter(|_| movable) {
        return overlay.placed_at(at);
    }
    // An anchor that fills the area (a crop frame larger than the view) leaves no room on either side: the bar keeps
    // its members and slides over the anchor, inside the area, instead of showing an empty surface.
    let mut hung = Place::anchored(Anchor::Rect(hang.rect), hang.side, hang.align).flipping_across().covering_anchor();
    if movable {
        hung = hung.offset(held.offset_in().unwrap_or(Vec2::ZERO));
    }
    overlay.place(match hang.clear {
        Some(clear) => hung.clear_of(clear),
        None => hung,
    })
}

/// The place after a pass that showed the bar at `shown`, whose unmoved place is `unmoved`. A move that ended (`moved`,
/// where it was dropped) sets the offset of an unpinned bar from `unmoved` to there, and `at` of a pinned bar; Reset
/// gives the default place; Pin pins the bar where it shows, and picked while pinned unpins it, the bar keeping where
/// it shows. An unpinned place that holds `at` turns it into the offset that keeps the bar where it shows. With no
/// move and no pick the place stays unchanged; the overlay host keeps the layer inside the area.
fn settled(held: TaskBarPlace, shown: Rect, area: Rect, moved: Option<Rect>, pick: Option<MenuPick>, unmoved: Rect) -> TaskBarPlace {
    if pick == Some(MenuPick::Reset) {
        return TaskBarPlace::default();
    }
    let at = |rect: Rect| -> [f32; 2] { (rect.min - area.min).into() };
    let placed = held.placed_in(area).is_some();
    let mut next = TaskBarPlace { pinned: held.pinned, at: held.at.filter(|_| placed), offset: held.offset_in().map(<[f32; 2]>::from) };
    if pick == Some(MenuPick::Pin) {
        next = TaskBarPlace { pinned: !held.pinned, at: Some(at(shown)), offset: None };
    }
    if next.pinned && next.at.is_none() {
        next.at = Some(at(shown));
    }
    if !next.pinned && next.at.is_some() {
        next.at = None;
        next.offset = Some((shown.min - unmoved.min).into());
    }
    if let Some(moved) = moved {
        if next.pinned {
            next.at = Some(at(moved));
        } else {
            next.offset = Some((moved.min - unmoved.min).into());
        }
    }
    next
}

/// The style of the [`TaskBarGrip::Bar`] grip.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GripBarStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The bar's width and height.
    size: Vec2,
    /// The bar's ink: the line colour at rest, muted text under the pointer.
    ink: Stateful<StyleColor>,
    /// The ink while the bar moves.
    moving: StyleColor,
}

/// The [`TaskBarGrip::Bar`] grip: one rounded upright bar centred on the members' line. It senses only hover, so a
/// press on it reaches the layer's move band under it.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(unregistered)]
struct GripBar {
    #[common]
    common: Common,
    /// The layer's move band is moving the bar this pass.
    moving: bool,
}

impl Component for GripBar {
    const ROLE: Role = Role::IMAGE;
    const PARTS: bool = false;
    const INPUT: craft_behavior::Input = craft_behavior::Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = GripBarStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: GripBarStyleKey) -> GripBarStyle {
        let colors = &cx.tokens.color;
        let (rest, lit) = (cx.color(colors.line.control_edge), cx.color(colors.text.muted));
        GripBarStyle {
            metrics: Metrics::new(),
            size: Vec2::new(cx.tokens.size.groove, cx.control().height - 2.0 * cx.tokens.space.gap.row),
            ink: Stateful::new(rest).off(States::new(rest, lit, lit, rest)),
            moving: lit,
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        cx.style.size
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let ink = if self.moving { cx.style.moving } else { cx.resolve(&cx.style.ink) };
        let bar = Rect::from_center_size(cx.rect.center(), cx.style.size);
        ui.draw().fill(&PaintShape::Rect(bar, UiTokens::corner(cx.style.size.x / 2.0)), ink);
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
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

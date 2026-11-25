//! `FloatingGroup`: the window a floating dock group shows in.
//!
//! The group is a movable, resizable container on a layer (or in an OS window) whose rect the caller owns: the
//! caller places it with the rect the dock model keeps and takes the rect the output reports after each step of
//! a gesture. The title row carries the Close action; a context request on its band opens the `menu` rows, as the
//! Menu action does where `actions` asks for one. The body step supplies the move band, the edge grips and the
//! keyboard Move and Size modes.
//! A dock window's title row is also the bar of the column the window holds, with the column's double arrow.

use super::dock_column::{CHEVRON, ColumnArrow, describe_chevron, paint_chevron};
use super::drop::start_window_drag;
use super::{DockDropTarget, DockDropped, DockSizes, DockWords, GroupDrag, PanelDrag, PanelKey, native};
use crate::contract::BodyCx;
use crate::contract::SizeLimits;
use crate::contract::overlay::{PlacementChange, PlacementKind, Popup, ResizeEdges};
use crate::contract::{
    Access, Bind, BindMut, Common, Component, Container, ContainerProps, Cx, DragBand, HeaderEdge, HeaderParts, HeaderRegion, Inner, MeasureCx,
    Mount, NoValue, OsFiles, Outcome, Overlay, Pad, PaintUi, PartsCx, Pointer as _, Props as _, Role, Show as _, ShowContainer as _, Sizing,
    WordSetCx as _,
};
use crate::display::Icon;
use crate::primitives::IconSpec;
use crate::runtime::drag::OsFile;
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::windows::{ResizeEdge, WindowSpec};
use crate::sections::{HeaderAction, HeaderStrength, MenuRows, TitleRow};
use crate::style::{ComponentStyle, StyleCx, SurfaceKind, UiTokens};
use craft_behavior::dock::DockEdge;
use craft_behavior::{GesturePhase, IconId, Metrics, Sense};
use craft_ui_widgets::popover::WholeWindow;
use egui::{Id, Pos2, Rect, Response, Ui, Vec2};
use std::cell::RefCell;
use std::hash::Hash;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The height of a floating group's title row: a `TitleRow` of the `Bar` strength, one text row high.
pub(crate) fn title_row(tokens: &UiTokens) -> f32 {
    tokens.size.row.text
}

/// The actions a floating group's title row shows unless `actions` says otherwise: Close alone, as a floating panel's
/// title bar has beside its double arrow; the menu opens from the band.
const ACTIONS: [HeaderAction; 1] = [HeaderAction::Close];

/// The style of a floating group: the surface and padding come from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct FloatingGroupStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The column a dock's floating window holds. The window's title row is the column's bar: it shows the column's
/// double arrow at the end on `edge`, and a collapsed column shows against that edge of the window's rect.
///
/// A column whose content fixes its height ([`held`](Self::held)) shows at that height whatever the rect's, and the
/// rect keeps its own height for when the content no longer fixes it: a column collapsed to icons, and one whose
/// every group is minimized.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WindowColumn {
    /// The edge of the float area, in reading order, that the window keeps to: the dock model's `Floating::edge`,
    /// else the one nearer the window.
    pub edge: DockEdge,
    /// The height of the column under the title row when its content fixes it: a collapsed column's icons, or the
    /// tab strips of a column whose every group is minimized. `None` while the rect gives the height.
    pub held: Option<f32>,
    /// The width of the collapsed column; `None` while the column is expanded, and the rect gives the width.
    pub collapsed: Option<f32>,
    /// The least and most width the sides of a collapsed column's window resize it to
    /// ([`DockSizes::iconic_bounds`](super::DockSizes::iconic_bounds)); unbounded while the column is expanded.
    pub widths: (f32, f32),
    /// The size of an expanded column whose content sets it (a column with steps) under the title row; `None` for a
    /// column the rect sizes.
    pub fitted: Option<Vec2>,
    /// What the double arrow does: collapse or expand the column unless [`arrow`](Self::arrow) says otherwise.
    pub arrow: ColumnArrow,
}

impl WindowColumn {
    /// An expanded column that keeps to `edge`.
    pub fn expanded(edge: DockEdge) -> Self {
        WindowColumn { edge, held: None, collapsed: None, widths: UNBOUNDED, fitted: None, arrow: ColumnArrow::Collapse }
    }

    /// A column collapsed to icons, `size` under the title row, that keeps to `edge`. The window's sides resize its
    /// width within `widths`, as a docked collapsed column's inner edge does; its height stays `size`'s.
    pub fn collapsed(edge: DockEdge, size: Vec2, widths: (f32, f32)) -> Self {
        WindowColumn { edge, held: Some(size.y), collapsed: Some(size.x), widths, fitted: None, arrow: ColumnArrow::Collapse }
    }

    /// An expanded column whose every group is minimized, their strips `height` under the title row, that keeps to
    /// `edge`. The window shows at its rect's place and width and that height; its sides resize the width, and the
    /// rect keeps its height, which the window takes back once a group is restored.
    pub fn minimized(edge: DockEdge, height: f32) -> Self {
        WindowColumn { edge, held: Some(height), collapsed: None, widths: UNBOUNDED, fitted: None, arrow: ColumnArrow::Collapse }
    }

    /// An expanded column whose content is `size` under the title row, that keeps to `edge`. The window shows at
    /// that size at its rect's place; a resize the user makes is reported for the host to turn into the column's
    /// steps, and a move keeps the rect's size.
    pub fn fitted(edge: DockEdge, size: Vec2) -> Self {
        WindowColumn { edge, held: None, collapsed: None, widths: UNBOUNDED, fitted: Some(size), arrow: ColumnArrow::Collapse }
    }

    /// The column with the double arrow `arrow` ([`ColumnArrow::of`]).
    pub fn arrow(mut self, arrow: ColumnArrow) -> Self {
        self.arrow = arrow;
        self
    }

    /// Where a window that keeps `rect` shows while its content fixes its height ([`Self::held`]): the column under
    /// the window's chrome, `chrome` tall (`DockSizes::window_chrome`), at the top of `rect`, as wide as a collapsed
    /// column or else as `rect`, and against the edge the window keeps to. `None` while the rect gives the height.
    pub fn shown(self, rect: Rect, chrome: f32, rtl: bool) -> Option<Rect> {
        let size = Vec2::new(self.collapsed.unwrap_or(rect.width()), self.held? + chrome);
        let left = if at_left(self.edge, rtl) { rect.left() } else { rect.right() - size.x };
        Some(Rect::from_min_size(Pos2::new(left, rect.top()), size))
    }

    /// The rect a window that keeps `rect` keeps when it shows at `shown`, so that it keeps its height while its
    /// content fixes the shown one ([`Self::shown`]). While collapsed, `rect` moved with the collapsed column so that
    /// the column's side on the edge the window keeps to stays on that side of `rect`, whatever width the column shows
    /// at; `rect` keeps its expanded size. While every group is minimized, `shown`'s place and width with `rect`'s
    /// height. Otherwise `shown`.
    pub fn kept(self, shown: Rect, rect: Rect, chrome: f32, rtl: bool) -> Rect {
        let Some(at) = self.shown(rect, chrome, rtl) else { return shown };
        if self.collapsed.is_none() {
            return Rect::from_min_size(shown.min, Vec2::new(shown.width(), rect.height()));
        }
        let dx = if at_left(self.edge, rtl) { shown.left() - at.left() } else { shown.right() - at.right() };
        rect.translate(Vec2::new(dx, shown.top() - at.top()))
    }

    /// The edge of `bounds`, in reading order, nearer the window at `rect`: the start edge when the window's centre is
    /// in the half of the bounds there or `bounds` is empty.
    pub fn nearer_edge(rect: Rect, bounds: Rect, rtl: bool) -> DockEdge {
        let at_left = !bounds.is_positive() || rect.center().x - bounds.left() <= bounds.right() - rect.center().x;
        if at_left != rtl { DockEdge::Start } else { DockEdge::End }
    }

    /// Where a window whose move ends at `rect` keeps to, and the rect it keeps: for a collapsed window, the edge of
    /// `bounds` nearer its strip as it was dropped, shown against [`Self::edge`], and the rect that shows the strip
    /// at the same place against that edge ([`Self::around_strip`]); an expanded window keeps `rect`, and the edge
    /// nearer it (`None`).
    pub fn dropped(self, rect: Rect, bounds: Rect, rtl: bool) -> (Rect, Option<DockEdge>) {
        let (Some(width), Some(height)) = (self.collapsed, self.held) else { return (rect, None) };
        let size = Vec2::new(width, height);
        let left = if at_left(self.edge, rtl) { rect.left() } else { rect.right() - size.x };
        let (rect, edge) = Self::around_strip(Rect::from_min_size(Pos2::new(left, rect.top()), size), rect.size(), bounds, rtl);
        (rect, Some(edge))
    }

    /// The edge of `bounds` nearer the strip of a collapsed column at `strip`, whose top is its window's, and the
    /// window rect of `size` that shows the strip at `strip` against that edge.
    pub fn around_strip(strip: Rect, size: Vec2, bounds: Rect, rtl: bool) -> (Rect, DockEdge) {
        let edge = Self::nearer_edge(strip, bounds, rtl);
        let left = if at_left(edge, rtl) { strip.left() } else { strip.right() - size.x };
        (Rect::from_min_size(Pos2::new(left, strip.top()), size), edge)
    }
}

/// The widths of a column that is not collapsed: its window's limits are the panels'.
const UNBOUNDED: (f32, f32) = (0.0, f32::INFINITY);

/// Whether `edge`, in reading order, is the left one of the screen.
fn at_left(edge: DockEdge, rtl: bool) -> bool {
    (edge == DockEdge::Start) != rtl
}

/// What a floating group reports in a pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct FloatingGroupOutput {
    /// The group's response.
    pub response: Response,
    /// The last step of a move or resize this pass, with the rect the caller keeps: while the group is minimized
    /// the rect has the height the group has when restored.
    pub placed: Option<PlacementChange>,
    /// The title row's action part taken this pass, or Close when the OS asked the group's window to close. Float and
    /// Close are the host's: the dock model docks or closes the group. Menu opens the `menu` rows.
    pub action: Option<HeaderAction>,
    /// Whether a double click on the title band asks to minimize the group, or restore it when it is minimized. A
    /// group with a [`WindowColumn`] reports that click in `iconic` instead.
    pub minimized_toggled: bool,
    /// The user asked to collapse the window's column to icons (`true`) or expand it (`false`): a click on the double
    /// arrow, or a double click on the title band. Only a group with a [`WindowColumn`] reports it.
    pub iconic: Option<bool>,
    /// The width the user gave the collapsed column of the window this pass, by a side of the window or the keyboard
    /// Size mode: the host keeps it as the column's iconic width. The rect in `placed` stays the expanded one.
    pub collapsed_width: Option<f32>,
    /// The files dropped on the group's window that no target there took.
    pub files: Vec<OsFile>,
    /// The band the group is moved by this pass, with the id of its accessibility node; `None` when it has none.
    pub grip: Option<(Rect, Id)>,
    /// The grips the group is resized by this pass, each with the edge or corner it moves and the id of its
    /// accessibility node; none while it does not resize (minimized).
    pub resize: Vec<(ResizeEdge, Rect, Id)>,
    /// Where `files` were dropped, in the group's layer space, when the drop session knew the pointer.
    pub files_at: Option<Pos2>,
}

impl Outcome for FloatingGroupOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A floating dock group: a titled window the user moves by its title, resizes by its edges and corners, and
/// minimizes by a double click on the title.
///
/// - Placement: the group shows at the rect given to `new`; the user's moves and resizes come back in
///   `FloatingGroupOutput::placed` and the caller shows the next pass at that rect. A move or resize stays inside
///   `bounds` and the panel `limits` (the limits are the body's; the title row adds its height).
/// - Sizing: with `Sizing::Content` (the default) the rect fixes the position and the width, and the height follows
///   the body; with another sizing the rect is the size. The user's resizes are reported either way.
/// - Minimized: only the title row shows, at the width of the rect, and the edges do not resize. The rect given to
///   `new` keeps the height the group has when restored.
/// - Column: a dock window holds one column ([`column`](FloatingGroup::column)); its title row is the column's bar,
///   with the double arrow at the end the window keeps to. While the column is collapsed the group shows at the
///   collapsed size against that edge of the rect, its title row shows no actions (the band and the double arrow;
///   a context request on the band opens the menu), its sides resize its width within the column's widths (reported
///   in [`FloatingGroupOutput::collapsed_width`]) while its height stays the icons', and the rect keeps its size. A
///   column its content sizes ([`WindowColumn::fitted`]) shows at that size at the rect's place; its resizes are
///   reported as the user makes them, and in an OS window the app runs them, so the window takes only the size the
///   host gives it. A column whose every group is minimized ([`WindowColumn::minimized`]) shows at the rect's place
///   and width and its strips' height; its sides resize the width, and the rect keeps its height.
/// - Docking: with [`window_of`](FloatingGroup::window_of) set, a move by the title starts a window-unit drag
///   (`GroupDrag`) for the dock's `DockDropTarget`, which shows the drop mark and docks the window on release.
/// - Window: with `native` set the group opens in an OS window of that kind, with the layer as the fallback where
///   the platform has no windows. The window comes to the front when the user begins to move or resize it, and in
///   the pass [`raise`](FloatingGroup::raise) is set.
/// - Menu: the Menu action and a context request on the title row open the `menu` rows, under the action's part.
/// - Show it with [`show_group`](FloatingGroup::show_group): the closure fills the body, and is not called while
///   the group is minimized.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct FloatingGroup<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    rect: Rect,
    /// The rect a move or resize keeps the group inside.
    #[prop]
    bounds: Option<Rect>,
    /// The body's size limits.
    #[prop]
    limits: SizeLimits,
    /// The group's opacity, from 0 (invisible) to 1.
    #[prop(quantity = "opacity")]
    opacity: f32,
    /// Whether only the title row shows.
    #[prop]
    minimized: bool,
    /// The column the window holds, whose bar the title row is.
    #[prop]
    column: Option<WindowColumn>,
    /// Whether the height follows the body (`Sizing::Content`) or the rect gives it.
    #[prop]
    sized: Sizing,
    /// Opens the group in an OS window of this kind.
    #[prop]
    native: Option<WindowSpec>,
    /// The rows of the menu a context request on the title row, or the Menu action, opens.
    #[prop(func = "FnMut(&mut Ui, &mut BodyCx<'_>)")]
    menu: Option<MenuRows<'a>>,
    actions: Vec<HeaderAction>,
    /// The title row is a bare grip (a small handle that moves the window), with no menu, float or close action, no
    /// column arrow and no rule under it: a quick-action strip's look.
    #[prop]
    grip: bool,
    /// The surfaces the body paints keep to the window's rounded lower corners (`frame_surfaces`), and the window's
    /// edge is drawn again over the body, which covers it along the sides: the window reads as one rounded piece.
    #[prop]
    rounded_body: bool,
    /// Always open: the caller shows the group while its window exists, and a close comes out as the Close action.
    open: Bind<'a, bool>,
    taken: Option<HeaderAction>,
    menu_opener: Option<Id>,
    toggled: bool,
    /// The double arrow was declared this pass, and whether it was clicked.
    chevron_declared: bool,
    chevron: bool,
    /// The title row's height, the height above the body (`DockSizes::window_chrome`) and the layout direction, read
    /// when the group shows.
    header: f32,
    chrome: f32,
    rtl: bool,
    placed: Option<PlacementChange>,
    /// The width a resize gave the collapsed column this pass.
    collapsed_width: Option<f32>,
    /// The display scale of the OS window the group shows in (`native::pixels_per_point`), when it differs from the
    /// running window's: its chrome, the rule under its title row included, is sized at that scale.
    #[prop(quantity = "display scale, pixels per point")]
    scale: Option<f32>,
    /// Starts the window-unit drag; set by `window_of`.
    start_drag: Option<StartDrag<'a>>,
    /// Takes the drops released on the group, on the group's own layer.
    take_drop: Option<TakeDrop<'a>>,
    /// A move of the window began this pass.
    moving: bool,
    /// That move continues a press another component handed over (`continue_move`), not a press on the title row.
    continued: bool,
    /// The window takes over the held primary press in this pass: its own move gesture begins from that press with the
    /// pointer at this point inside the window, so the window follows the pointer and, with
    /// [`window_of`](FloatingGroup::window_of), docks where it would dock on release. A request of the pass, for the
    /// pass after a rail's `torn_off`; without a held press it does nothing. Only a window on a layer takes the press:
    /// the OS keeps a held button's moves with the window it went down in, so a group shown in an OS window
    /// ([`native`](FloatingGroup::native)) moves only when pressed again.
    #[prop(quantity = "grab point")]
    continue_move: Option<Vec2>,
    /// Brings the window to the front of the windows it overlaps in this pass: its layer above the other floating
    /// layers, or its OS window raised and focused. A request of the pass, for the pass the host reveals a panel of the
    /// window in.
    #[prop(flag)]
    raise: bool,
    /// The files dropped on the group's OS window that no target there took.
    files: Vec<OsFile>,
    /// Where they were dropped.
    files_at: Option<Pos2>,
}

/// Starts the window-unit drag: a move by the title row when the flag is set, else a drag the window took over.
type StartDrag<'a> = Box<dyn for<'b, 'c> FnOnce(&'b mut Cx<'c, FloatingGroup<'a>>, bool) + 'a>;

type TakeDrop<'a> = Box<dyn for<'b, 'c> FnMut(&'b mut Cx<'c, FloatingGroup<'a>>) + 'a>;

impl<'a> FloatingGroup<'a> {
    /// A floating group titled `title` at `rect`, with the Close action.
    pub fn new(title: impl Into<Label>, rect: Rect) -> Self {
        let props = ContainerProps::new()
            .surface(SurfaceKind::Floating)
            .pad(Pad::None)
            .sizing(Sizing::Content)
            .header(HeaderRegion::new().band(DragBand::Move).rule(true));
        let props = match <DockWords as craft_i18n::WordSet>::word("floatingGroup.resize") {
            Some(word) => props.resize_name(Label::from(word)),
            None => props,
        };
        FloatingGroup {
            common: Common::new(),
            props,
            rect,
            bounds: None,
            limits: SizeLimits::default(),
            opacity: 1.0,
            minimized: false,
            column: None,
            sized: Sizing::Content,
            native: None,
            menu: None,
            actions: ACTIONS.to_vec(),
            grip: false,
            rounded_body: false,
            open: Bind::given(true),
            taken: None,
            menu_opener: None,
            toggled: false,
            chevron_declared: false,
            chevron: false,
            header: 0.0,
            chrome: 0.0,
            rtl: false,
            placed: None,
            collapsed_width: None,
            scale: None,
            start_drag: None,
            take_drop: None,
            moving: false,
            continued: false,
            continue_move: None,
            raise: false,
            files: Vec::new(),
            files_at: None,
        }
        .label(title)
    }

    /// The group is the floating window of the dock model that holds `anchor`: a move by its title starts a
    /// window-unit drag, so the dock marks where the window would dock while it moves and docks it on release.
    #[must_use]
    pub fn window_of<P: Clone + Hash + Send + Sync + 'static>(mut self, anchor: P) -> Self {
        let active = PanelKey::of(&anchor);
        let size = self.rect.size();
        self.start_drag = Some(Box::new(move |cx, title_row| start_window_drag(cx, anchor, active, size, title_row)));
        self
    }

    /// The group takes the drops released over it through `target`, the dock's target of the spots its groups
    /// register, on the group's own layer: a drop over a floating window lands on it, not on what lies under it.
    /// The drop taken in a pass lands in `dropped`.
    #[must_use]
    #[doc(hidden)]
    pub fn takes_drops<P>(mut self, target: DockDropTarget<'a, P>, dropped: &'a RefCell<Option<DockDropped<P>>>) -> Self
    where
        P: Clone + Eq + Hash + Send + Sync + 'static,
    {
        self.take_drop = Some(Box::new(move |cx| {
            let taken = cx.drop_target::<GroupDrag<P>, _>(&target).or_else(|| cx.drop_target::<PanelDrag, _>(&target));
            if taken.is_some() {
                *dropped.borrow_mut() = taken;
            }
        }));
        self
    }

    /// The action parts at the end of the title row, in reading order.
    #[must_use]
    pub fn actions(mut self, actions: impl IntoIterator<Item = HeaderAction>) -> Self {
        self.actions = actions.into_iter().collect();
        self
    }

    /// Shows the group; `body` fills the area under the title row unless the group is minimized.
    pub fn show_group<R>(mut self, ui: &mut Ui, mut body: impl FnMut(&mut Ui, &mut BodyCx<'_>) -> R) -> Inner<FloatingGroupOutput, R> {
        let env = Env::of(ui);
        // A window in the app window keeps the band over its title row that a layer the user moves keeps for its top
        // grip; an OS window's grips lie over its title row. The rule under the title row takes a band of its own.
        let sizes = DockSizes::style(&StyleCx::of_env(ui.ctx(), &env));
        let sizes = self.scale.map_or(sizes, |scale| sizes.at_scale(scale));
        // A grip title row has no rule under it, so the chrome keeps no band for one.
        (self.header, self.chrome, self.rtl) =
            (sizes.title_row, sizes.window_chrome_ruled(self.native.is_none(), !self.grip), env.direction.is_rtl());
        if self.grip {
            self.props = self.props.header(HeaderRegion::new().band(DragBand::Move).rule(false));
        }
        let mount = self.mount();
        self.props = self.props.mount(mount);
        let minimized = self.minimized;
        // The window's lower corners: its body ends at the window's foot and sides.
        let window = UiTokens::corner(env.tokens.shape.radius.window);
        let lower = egui::CornerRadius { nw: 0, ne: 0, sw: window.sw, se: window.se };
        let rounded = self.rounded_body;
        let shown = self.show(ui, |ui, cx| {
            if rounded {
                crate::primitives::frame_surfaces(ui, cx.rect, lower);
            }
            (!minimized).then(|| body(ui, cx))
        });
        Inner { output: shown.output, inner: shown.inner.flatten(), settle: shown.settle }
    }

    /// The layer or window the group shows in.
    ///
    /// A minimized group, and one with `Sizing::Content`, is placed by its position and width; its height is its
    /// content's, which is the chrome alone while minimized.
    fn mount(&self) -> Mount {
        // The frame's limits are the body's with the chrome on top.
        let frame = self.limits.around(Vec2::new(0.0, self.chrome));
        // A collapsed column's width stays within its own bounds, a minimized one's within the panels' limits.
        let widths = self.column.map(|column| match column.collapsed {
            Some(_) => column.widths,
            None => (frame.min.x, frame.max.map_or(f32::INFINITY, |max| max.x)),
        });
        let held = self.held_rect().zip(widths);
        let mut overlay = Overlay::floating().movable(true).resizable(!self.minimized).opacity(self.opacity).body_scroll(false);
        overlay = if let Some((shown, (least, most))) = held {
            // The sides resize the column's width; its content fixes its height.
            let height = shown.height();
            let edges = if self.minimized { ResizeEdges::NONE } else { ResizeEdges::LEFT.union(ResizeEdges::RIGHT) };
            overlay.placed(shown).resize_edges(edges).size_limits(SizeLimits::new(Vec2::new(least, height), Vec2::new(most.max(least), height)))
        } else if let Some(size) = self.column.and_then(|column| column.fitted) {
            // The host turns a resize into the column's steps, and the window takes the size they make.
            overlay.placed(Rect::from_min_size(self.rect.min, size + Vec2::new(0.0, self.chrome))).sized_by_caller()
        } else if self.minimized {
            let width = self.rect.width();
            overlay.placed_at(self.rect.min).size_limits(SizeLimits::new(Vec2::new(width, 0.0), Vec2::new(width, f32::INFINITY)))
        } else if self.sized == Sizing::Content {
            let width = self.rect.width().max(frame.min.x).min(frame.max.map_or(f32::INFINITY, |max| max.x));
            overlay.placed_at(self.rect.min).size_limits(SizeLimits { min: Vec2::new(width, frame.min.y), max: frame.max })
        } else {
            overlay.placed(self.rect).size_limits(frame)
        };
        if let Some(bounds) = self.bounds {
            overlay = overlay.bounds(bounds);
        }
        // A dock window's move by its title row docks it, so the app runs that move in an OS window too.
        if self.start_drag.is_some() {
            overlay = overlay.moved_by_app();
        }
        if let Some(grab) = self.continue_move {
            overlay = overlay.continue_move(grab);
        }
        match &self.native {
            Some(spec) => Mount::viewport_with(overlay, spec.clone()),
            None => Mount::Layer(overlay),
        }
    }

    /// Where the group shows while its column's content fixes its height ([`WindowColumn::shown`]). `None` while the
    /// rect gives the height.
    fn held_rect(&self) -> Option<Rect> {
        self.column?.shown(self.rect, self.chrome, self.rtl)
    }

    /// The double arrow's square at the end of the title row the window keeps to.
    fn chevron_edge(&self) -> Option<HeaderEdge> {
        let column = self.column.filter(|column| column.arrow.shown() && !self.grip)?;
        Some(if column.edge == DockEdge::End { HeaderEdge::End } else { HeaderEdge::Start })
    }
}

impl Component for FloatingGroup<'_> {
    const ROLE: Role = Role::GROUP;
    type Style = FloatingGroupStyle;
    type Output = FloatingGroupOutput;

    fn style(_: &StyleCx<'_>, _: FloatingGroupStyleKey) -> FloatingGroupStyle {
        FloatingGroupStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// The column's double arrow, in its square of the title row.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let (Some(edge), Some(column)) = (self.chevron_edge(), self.column) else { return };
        let Some(slot) = cx.header_slot(edge) else { return };
        let name = column.arrow.name(&cx.words::<DockWords>(), column.collapsed.is_none());
        let _ = cx.header_part(edge, CHEVRON, slot).sense(Sense::CLICK | Sense::FOCUS).focusable().tooltip(name);
        self.chevron_declared = true;
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.chevron = self.chevron_declared && cx.part_activated(CHEVRON).is_some();
        if let Some(mut take) = self.take_drop.take() {
            take(cx);
            self.take_drop = Some(take);
        }
        // A layer float lies in the app window, whose own targets take those drops.
        if self.native.is_some() {
            let target = WholeWindow::new(cx.rect);
            if let Some(dropped) = cx.window_drop_target::<OsFiles, _>(&target) {
                self.files = dropped.files;
                // The drop session's pointer, which a drop the OS sent before any position waits for.
                self.files_at = cx.dragging::<OsFiles>().map(|(_, at)| at);
            }
        }
    }

    /// The double arrow, over the title row's surface.
    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if self.rounded_body {
            ui.draw().surface_edge(cx.rect, SurfaceKind::Floating);
        }
        let Some(column) = self.column.filter(|_| self.chevron_declared) else { return };
        paint_chevron(ui, column.edge, column.arrow, column.collapsed.is_none(), cx.flow.is_rtl(), &cx.words::<DockWords>());
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        if let Some(column) = self.column.filter(|_| self.chevron_declared) {
            describe_chevron(access, &column.arrow.name(&cx.words::<DockWords>(), column.collapsed.is_none()));
        }
    }

    fn finish(mut self, mut cx: Cx<'_, Self>) -> FloatingGroupOutput {
        if self.moving
            && let Some(start) = self.start_drag.take()
        {
            start(&mut cx, !self.continued);
        }
        if let Some(opener) = self.menu_opener {
            Popup::toggle(opener, &mut cx);
        }
        // A dock window's band double click collapses or expands its column, as a dock column's bar does.
        let collapsed = self.column.map(|column| column.collapsed.is_some());
        let iconic = collapsed.filter(|_| self.chevron || self.toggled).map(|collapsed| !collapsed);
        let output = FloatingGroupOutput {
            response: cx.response.clone(),
            placed: self.placed,
            action: self.taken,
            minimized_toggled: self.toggled && self.column.is_none(),
            iconic,
            collapsed_width: self.collapsed_width,
            files: self.files,
            grip: cx.move_band(),
            resize: cx.resize_grips(),
            files_at: self.files_at,
        };
        native::raise(&cx.core.ctx, &output, self.raise);
        native::fade(&cx.core.ctx, &output);
        output
    }
}

impl Container for FloatingGroup<'_> {
    /// The double arrow's square at the end of the title row the window keeps to.
    fn header_parts(&self, _: &MeasureCx<'_, Self>) -> HeaderParts {
        match self.chevron_edge() {
            Some(HeaderEdge::End) => HeaderParts::NONE.end(self.header),
            Some(_) => HeaderParts::NONE.start(self.header),
            None => HeaderParts::NONE,
        }
    }

    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }

    fn closed(&mut self, why: CloseReason) {
        if why == CloseReason::Requested {
            self.taken = Some(HeaderAction::Close);
        }
    }

    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        // A drag bar with no title: the tabs below name the panels. A collapsed column's title row is its bar alone,
        // the band and the double arrow; the menu, which a context request on the band opens, keeps its rows.
        let collapsed = self.column.is_some_and(|column| column.collapsed.is_some());
        if self.grip {
            ui.vertical_centered(|ui| {
                let _ = Icon::new(IconSpec::svg(IconId::new("grip"))).label(cx.words::<DockWords>().move_window.clone()).show(ui);
            });
            self.toggled |= cx.band_double_pressed();
            return;
        }
        let actions = if collapsed { &[][..] } else { &self.actions[..] };
        let shown = TitleRow::new("").strength(HeaderStrength::Bar).actions(actions.iter().copied()).show(ui);
        self.taken = shown.action.or(self.taken);
        if let Some(rows) = self.menu.as_mut() {
            self.menu_opener = rows.hang(ui, &shown, cx.words::<DockWords>().panel_menu.clone());
        }
        self.toggled |= cx.band_double_pressed();
    }

    fn placement_changed(&mut self, change: &PlacementChange) {
        let mut change = *change;
        let fitted = self.column.is_some_and(|column| column.fitted.is_some());
        if let Some(column) = self.column.filter(|column| column.held.is_some()) {
            // A resize of the collapsed window sets the column's width; the rect keeps its expanded size. A minimized
            // window's rect takes the width and keeps its height.
            let resized = match change.kind {
                PlacementKind::Size(_) => true,
                PlacementKind::Keyboard => self.held_rect().is_some_and(|shown| change.rect.width() != shown.width()),
                _ => false,
            };
            if resized && column.collapsed.is_some() {
                self.collapsed_width = Some(change.rect.width());
            }
            change.rect = column.kept(change.rect, self.rect, self.chrome, self.rtl);
        } else if (fitted || self.sized == Sizing::Content) && change.kind == PlacementKind::Move {
            // A window its content sizes moves (a cancel too, which reports the rect from before the gesture); the
            // rect keeps the size the caller gave, not the one measured from the body.
            change.rect = Rect::from_min_size(change.rect.min, self.rect.size());
        } else if self.minimized {
            change.rect = Rect::from_min_size(change.rect.min, Vec2::new(change.rect.width(), self.rect.height()));
        }
        if change.kind == PlacementKind::Move && change.phase == GesturePhase::Begin {
            self.moving = true;
            self.continued = change.continued;
        }
        self.placed = Some(change);
    }
}

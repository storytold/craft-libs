use std::{collections::HashMap, fmt::Debug, hash::Hash};

use egui::{Color32, Context, FontId, Id, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use super::{Action, DockError, DropTarget, Layout, Monitor, Permissions, render::Renderer, resize::AnchoredResize};

/// Horizontal room around a tab's label, both when measuring its natural width and when cutting it.
pub(super) const TAB_PADDING: f32 = 24.0;

/// Application-supplied chrome. Body renderers retain their own UI style and contents.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DockStyle {
    pub tab_height: f32,
    pub gap: f32,
    pub min_pane: f32,
    pub background: Color32,
    pub tab_background: Color32,
    pub active_background: Color32,
    pub text: Color32,
    pub inactive_text: Color32,
    pub border: Stroke,
    pub accent: Color32,
    pub font: FontId,
    /// Localize these action labels in the application adapter.
    pub float_label: String,
    pub close_label: String,
    /// Tab menu item that moves one panel into another group (submenu of target panels).
    pub move_panel_to_label: String,
    pub panels_label: String,
    pub resize_label: String,
    pub resize_window_label: String,
    /// Accessibility label of a group's drag bar and top border.
    pub move_group_label: String,
    pub close_group_label: String,
    pub minimize_label: String,
    pub expand_label: String,
    pub float_group_label: String,
    /// Group menu item that moves the whole group into another group (submenu of targets).
    pub move_group_to_label: String,
    /// Minimum width of the drag bar left of the group header. Tabs shrink, then overflow,
    /// before the bar narrows; a strip narrower than the header plus this width narrows it.
    pub drag_bar_width: f32,
    /// Narrowest a tab gets before the tabs that do not fit go to the overflow menu: about
    /// three characters and the "…" at [`Self::font`]'s size, plus the tab's 24 pt padding
    /// (54 pt at the default 12 pt font). Tabs whose names are shorter keep their natural
    /// width. Set it again after changing the font.
    pub min_tab_width: f32,
    /// Painted under every floating window, so it stays opaque when the panel colors are not.
    pub floating_background: Color32,
    pub floating_stroke: Stroke,
    /// Painted over a native floating window while [`DockArea::input_blocked`] is on.
    pub blocked_dim: Color32,
    /// Height of a floating window's title bar.
    pub title_height: f32,
    /// Width of the band along the layout's outer edge that splits at the root (28 pt). It is
    /// at most a quarter of the layout's width and height. Not used with
    /// [`Self::side_edge_zones`], whose band is 8 pt.
    pub edge_band: f32,
    /// The split-tree profile. The outer edge band also runs along the left and right edges,
    /// where a drop adds a full-height column. A group's side zones are trapezoids
    /// [`crate::docking::SIDE_EDGE_ZONE_DEPTH`] deep (see [`crate::docking::drop_zone`]), a drag
    /// outlines them, and a split previews the half of the group it takes. Off: only the top and
    /// bottom edges add a full-width row, a group's zones are [`Self::side_zone_fraction`] of its
    /// body, and every preview is a line or a tab outline.
    pub side_edge_zones: bool,
    /// Which sides of the layout's outer edge, and of a group that takes no tabs (a canvas)
    /// without [`Self::new_dock_zones`], take a drop (all four). With [`Self::new_dock_zones`]
    /// a canvas body takes no drop, and without [`Self::side_edge_zones`] the outer left and
    /// right edges are new docks, so only top and bottom apply there. A dock-column app whose
    /// palettes dock only at the left and right sets [`crate::docking::Sides::NONE`] with
    /// [`Self::new_dock_zones`] on. Not used with [`Self::side_edge_zones`]. Groups that take
    /// tabs keep all four sides.
    pub drop_edges: super::Sides,
    /// The share of a group's body, below the tab strip, that means "beside" on each side: of
    /// its width from the left and right edges, of its height from the top and bottom edges
    /// (one third, at most one half). The middle joins the group as a tab. Not used with
    /// [`Self::side_edge_zones`].
    pub side_zone_fraction: f32,
    /// Width of the line that a drop on a band along the layout's outer edge (a new dock, a new
    /// outermost column or the outer edge band), a drop between the icon groups of an iconic
    /// column, and every dock drop with [`Self::side_edge_zones`] preview with, and of the line
    /// [`crate::docking::ZoneRule::Fraction`] gives hosts as a side's preview (4 pt, more than
    /// zero). Other drops beside a group, column or dock preview the rect the moved panels
    /// take. Border bands reach 4 pt past this line into a tab strip.
    pub drop_line_width: f32,
    /// [D] The docking profile. The docked area's left and right edges, where no dock is yet,
    /// take a drop as a new dock (see [`Self::new_dock_band`]). Without
    /// [`Self::side_edge_zones`], the body of a frame or floating group that takes no tabs (a
    /// canvas) takes no drop: a release there floats as over no target.
    pub new_dock_zones: bool,
    /// [D] Height of the bar above each dock column. It drags the column and holds its »/« button.
    pub column_bar_height: f32,
    /// [D] Height of the band centred on the seam between two stacked groups in a dock column
    /// (28 pt).
    pub seam_band: f32,
    /// [D] Width of the band centred on each edge of a dock column, where a drop makes a new
    /// column (28 pt).
    pub column_edge_band: f32,
    /// [D] Width of the band along the docked area's left and right edges, and inside an empty
    /// dock, that makes a new dock (28 pt).
    pub new_dock_band: f32,
    /// [D] Side of a panel icon in an iconic column; each icon row is 10 pt taller.
    pub icon_size: f32,
    /// [D] Width of an iconic column without a saved width, and the narrowest it gets.
    pub iconic_width: f32,
    /// [D] Widest an iconic column gets by dragging its inner edge.
    pub iconic_max_width: f32,
    /// [D] An iconic column at least this wide shows panel names beside the icons.
    pub iconic_label_width: f32,
    /// [D] Column menu item that toggles Auto-Collapse Iconic Panels.
    pub auto_collapse_label: String,
    /// [D] Lay docks out right to left: `Side::Start` is the right edge. egui has no app-wide
    /// layout direction, so the host sets this from its locale. Splits keep their sides.
    pub right_to_left: bool,
    /// [D] Accessibility label of a column bar.
    pub move_column_label: String,
    /// [D] Column menu and group menu items.
    pub collapse_to_icons_label: String,
    pub expand_panels_label: String,
    /// [D] Accessibility labels of the column bar's »/« button, distinct from the menu items.
    pub collapse_column_label: String,
    pub expand_column_label: String,
    pub move_window_label: String,
    pub close_window_label: String,
    /// Group menu items Close Other Panels in Group, and Maximize Frame / Restore Frame Size
    /// for groups of the split tree, each shown when its flag is on.
    pub close_others_label: String,
    pub close_others_item: bool,
    pub maximize_label: String,
    pub restore_frame_label: String,
    pub maximize_item: bool,
    /// Shown under the pointer while a drag offers a docking target, when [`Self::float_hint`]
    /// is on. `{key}` stands for the modifier that floats the drop instead: ⌘ on macOS, Ctrl
    /// elsewhere.
    pub float_hint_label: String,
    pub float_hint: bool,
    /// Ctrl/Cmd held when a tab drag of a split-tree group starts makes its drop float, even
    /// when the key is let go during the drag. Off: only the key held at the drop counts.
    pub command_press_floats: bool,
    /// A drag released over no drop target inside the window floats what it carries, such as
    /// a panel dropped on a protected canvas. Off: that release cancels the drag; only
    /// Ctrl/Cmd or a release outside the window floats.
    pub float_on_empty_drop: bool,
    /// Dragging a floating window's title bar onto a drop target docks the whole window there.
    /// Off: the title bar only moves the window; tabs and drag bars dock.
    pub title_bar_docks: bool,
    /// Show the built-in Close items in tab and group menus (Close Panel, Close Tab Group,
    /// Close Other Panels in Group) for closable panels. A host that draws its own Close items
    /// turns this off and keeps [`Permissions::closable`], so its close actions still apply.
    pub close_menu_items: bool,
    /// Show × on the title bar of a floating window whose panels are all closable.
    pub close_button: bool,
    /// The mouse wheel over a tab strip selects the next or previous tab, one per notch.
    pub wheel_selects_tabs: bool,
    /// A middle click on a tab closes its panel when the panel is closable.
    pub middle_click_closes_tabs: bool,
}

impl Default for DockStyle {
    /// [`Self::from_visuals`] with egui's default visuals.
    fn default() -> Self {
        Self::from_visuals(&egui::Visuals::default())
    }
}

impl DockStyle {
    /// Colors from the `ui`'s visuals; adjust fields afterwards.
    pub fn from_ui(ui: &Ui) -> Self {
        Self::from_visuals(ui.visuals())
    }

    pub fn from_visuals(visuals: &egui::Visuals) -> Self {
        let font = FontId::proportional(12.0);
        Self {
            tab_height: 26.0,
            gap: 4.0,
            min_pane: 40.0,
            background: visuals.panel_fill,
            tab_background: visuals.extreme_bg_color,
            active_background: visuals.widgets.active.bg_fill,
            text: visuals.text_color(),
            inactive_text: visuals.weak_text_color(),
            border: visuals.widgets.noninteractive.bg_stroke,
            accent: visuals.selection.stroke.color,
            min_tab_width: font.size * 2.5 + TAB_PADDING,
            font,
            float_label: "Float Panel".into(),
            close_label: "Close Panel".into(),
            move_panel_to_label: "Move Panel to Group".into(),
            panels_label: "Panels".into(),
            resize_label: "Resize panels".into(),
            resize_window_label: "Resize panel window".into(),
            move_group_label: "Move tab group".into(),
            close_group_label: "Close Tab Group".into(),
            minimize_label: "Minimize".into(),
            expand_label: "Expand".into(),
            float_group_label: "Float Tab Group".into(),
            move_group_to_label: "Move Group To".into(),
            drag_bar_width: 40.0,
            floating_background: visuals.window_fill,
            floating_stroke: visuals.window_stroke,
            blocked_dim: visuals.panel_fill.gamma_multiply(0.6),
            title_height: super::chrome::TITLE_HEIGHT,
            edge_band: 28.0,
            side_edge_zones: false,
            drop_edges: super::Sides::ALL,
            side_zone_fraction: 1.0 / 3.0,
            drop_line_width: 4.0,
            new_dock_zones: true,
            column_bar_height: 14.0,
            seam_band: 28.0,
            column_edge_band: 28.0,
            new_dock_band: 28.0,
            icon_size: 20.0,
            iconic_width: 36.0,
            iconic_max_width: 240.0,
            iconic_label_width: 96.0,
            auto_collapse_label: "Auto-Collapse Iconic Panels".into(),
            right_to_left: false,
            move_column_label: "Move panel column".into(),
            collapse_to_icons_label: "Collapse to Icons".into(),
            expand_panels_label: "Expand Panels".into(),
            collapse_column_label: "Collapse column to icons".into(),
            expand_column_label: "Expand column".into(),
            move_window_label: "Move panel window".into(),
            close_window_label: "Close panel window".into(),
            close_others_label: "Close Other Panels in Group".into(),
            close_others_item: false,
            maximize_label: "Maximize Frame".into(),
            restore_frame_label: "Restore Frame Size".into(),
            maximize_item: false,
            float_hint_label: "Hold {key} to undock".into(),
            float_hint: false,
            command_press_floats: false,
            float_on_empty_drop: true,
            title_bar_docks: true,
            close_menu_items: true,
            close_button: true,
            wheel_selects_tabs: false,
            middle_click_closes_tabs: false,
        }
    }

    /// The sides [`Self::drop_edges`] gives in this profile: all four with
    /// [`Self::side_edge_zones`].
    pub(super) fn drop_sides(&self) -> super::Sides {
        if self.side_edge_zones { super::Sides::ALL } else { self.drop_edges }
    }

    /// The sides of a frame or floating group that takes no tabs (a canvas) whose body takes a
    /// drop: none in the docking profile ([`Self::new_dock_zones`] without
    /// [`Self::side_edge_zones`]), where only the bands along borders dock beside it, else
    /// [`Self::drop_sides`].
    pub(super) fn canvas_sides(&self) -> super::Sides {
        if self.side_edge_zones || !self.new_dock_zones { self.drop_sides() } else { super::Sides::NONE }
    }

    /// The outer edges that take a drop: all four with [`Self::side_edge_zones`], else the top
    /// and bottom of [`Self::drop_edges`].
    pub(super) fn outer_edges(&self) -> super::Sides {
        let sides = self.drop_sides();
        if self.side_edge_zones { sides } else { super::Sides { left: false, right: false, ..sides } }
    }

    /// How a group's body divides into drop zones: [`crate::docking::SIDE_EDGE_ZONE_DEPTH`] deep
    /// with [`Self::side_edge_zones`], else [`Self::side_zone_fraction`] of it, with
    /// [`Self::drop_line_width`] previews. Pass it to [`crate::docking::resolve_drop_by`].
    pub fn zone_rule(&self) -> super::ZoneRule {
        if self.side_edge_zones {
            super::ZoneRule::Depth(super::SIDE_EDGE_ZONE_DEPTH)
        } else {
            super::ZoneRule::Fraction { fraction: self.side_zone_fraction, line: self.drop_line_width }
        }
    }

    pub(super) fn valid(&self) -> bool {
        [
            self.tab_height,
            self.gap,
            self.min_pane,
            self.border.width,
            self.font.size,
            self.drag_bar_width,
            self.min_tab_width,
            self.floating_stroke.width,
            self.title_height,
            self.edge_band,
            self.side_zone_fraction,
            self.drop_line_width,
            self.column_bar_height,
            self.seam_band,
            self.column_edge_band,
            self.new_dock_band,
            self.icon_size,
            self.iconic_width,
            self.iconic_max_width,
            self.iconic_label_width,
        ]
        .iter()
        .all(|v| v.is_finite() && (0.0..=4096.0).contains(v))
            && self.iconic_width <= self.iconic_max_width
            && self.tab_height > 0.0
            && self.font.size > 0.0
            && self.drop_line_width > 0.0
            && self.side_zone_fraction <= 0.5
    }
}

#[derive(Clone, Debug)]
pub struct DockOutput<P> {
    pub actions: Vec<Action<P>>,
    pub error: Option<DockError>,
    /// The panel the user focused this frame: a clicked tab or menu entry, a drop, or a press
    /// of any pointer button in a panel body (stripless panels included) or in a group header,
    /// which focuses the group's shown panel. Only user input sets
    /// it; [`DockArea::raise_panel`] and host commands do not.
    pub focused: Option<P>,
    /// Whole-floating-group gesture identity; host may display its own group drop preview.
    pub group_drag: Option<P>,
    /// Whole-group release for host-owned atomic docking policy.
    pub group_drop: Option<(P, Pos2)>,
    /// Requests that are not layout actions.
    pub events: Vec<DockEvent<P>>,
    /// Key, text and clipboard events (`Key`, `Text`, `Copy`, `Cut`, `Paste`) that reached a
    /// floating window in its own OS window and that none of its widgets used (feature
    /// `native-windows`). egui hands each OS window only its own input, so an application
    /// passes these to the shortcut handling it runs on the main window's input. A window
    /// where a widget has keyboard focus forwards nothing.
    pub forwarded_events: Vec<egui::Event>,
    /// The modifiers held in the native window that forwarded `forwarded_events`, where it
    /// forwarded any. egui-winit turns Cmd/Ctrl+C, X and V into `Copy`, `Cut` and `Paste`
    /// without the key, so these tell them from the keyboard's own Copy, Cut and Paste keys.
    pub forwarded_modifiers: Option<egui::Modifiers>,
    /// Files dragged over or dropped on floating windows in their own OS windows this frame,
    /// one entry per window (feature `native-windows`). egui hands each OS window only its own
    /// input, so the main window's `RawInput` never holds them.
    pub window_files: Vec<WindowFiles<P>>,
}

/// Files dragged over or dropped on a native floating window in one frame.
#[derive(Clone, Debug)]
pub struct WindowFiles<P> {
    /// [`super::Floating::id`] of the window.
    pub window: u64,
    /// The window's egui viewport.
    pub viewport: egui::ViewportId,
    /// The panel whose body is under the pointer, else the window's shown panel: the window
    /// system may send no pointer moves while files are dragged in.
    pub panel: P,
    /// The pointer's last position in the window's coordinates, where egui has one.
    pub pos: Option<Pos2>,
    /// The window's `RawInput::hovered_files`.
    pub hovered: Vec<egui::HoveredFile>,
    /// The window's `RawInput::dropped_files`.
    pub dropped: Vec<egui::DroppedFileHandle>,
}

impl<P> Default for DockOutput<P> {
    fn default() -> Self {
        Self {
            actions: Vec::new(),
            error: None,
            focused: None,
            group_drag: None,
            group_drop: None,
            events: Vec::new(),
            forwarded_events: Vec::new(),
            forwarded_modifiers: None,
            window_files: Vec::new(),
        }
    }
}

/// Room between the dock area's top-left corner and a unit a command floats.
const FLOAT_INSET: f32 = 24.0;

/// Size a command floats a panel at when its [`FloatSpace`] never drew it.
#[cfg_attr(not(feature = "serde"), allow(dead_code))]
pub(super) const DEFAULT_FLOAT: Vec2 = Vec2::new(260.0, 320.0);

/// Where a docking command floats a panel, group or column when it gives no rect, and how the
/// area draws its outer edge (see `DockCommand::to_actions`, feature `serde`). [`DockArea`]
/// records one each frame; read it with [`float_space`].
#[derive(Clone, Debug, PartialEq)]
pub struct FloatSpace<P> {
    /// The rect the area drew in. A floated unit goes inside it, 24 points from its top-left
    /// corner, so it stays clear of the app's menu bar.
    pub area: Rect,
    /// What a drag floats from each drawn panel: the panel alone, its group and its dock column.
    pub(super) panels: Vec<(P, Vec2)>,
    pub(super) groups: Vec<(P, Vec2)>,
    pub(super) columns: Vec<(P, Vec2)>,
    /// The height each panel's content used when it was last drawn, which fits a floated group
    /// to its content; `None` for content that filled its body.
    pub(super) contents: Vec<(P, Option<f32>)>,
    /// The area draws with [`DockStyle::side_edge_zones`]: an outer edge drop wraps the part
    /// between the docks, never only the canvas column.
    pub(super) side_edge_zones: bool,
}

impl<P: PartialEq> FloatSpace<P> {
    /// An area with no recorded sizes: commands float a panel at 260x320, and a group or
    /// column at its dock column's width (else 260) by 320.
    pub fn new(area: Rect) -> Self {
        Self { area, panels: Vec::new(), groups: Vec::new(), columns: Vec::new(), contents: Vec::new(), side_edge_zones: false }
    }

    /// For an area drawn with [`DockStyle::side_edge_zones`] when the area has not drawn yet
    /// (a headless run): an outer edge command then wraps what that profile's edge band wraps.
    pub fn with_side_edge_zones(mut self, on: bool) -> Self {
        self.side_edge_zones = on;
        self
    }

    /// Adds the size a command floats `panel` at alone when the area has not drawn it, such as
    /// a hidden panel or a headless run. A size the area recorded for the panel takes precedence.
    pub fn with_panel_size(mut self, panel: P, size: Vec2) -> Self {
        self.panels.push((panel, size));
        self
    }
}

impl<P: PartialEq> FloatSpace<P> {
    /// Record the height `panel`'s content used in a body `available` tall. Content that
    /// reaches the body's bottom filled it and has no height of its own, unless the body was
    /// sized to the height recorded before (a fitted window). `at` is the panel's entry in
    /// `contents` if it has one; returns where the entry is.
    pub(super) fn record_content(&mut self, at: Option<usize>, panel: P, used: f32, available: f32) -> usize {
        const TOLERANCE: f32 = 1.0;
        let used = if used.is_finite() { used.max(0.0) } else { 0.0 };
        let recorded = at.and_then(|i| self.contents.get_mut(i)).filter(|(p, _)| *p == panel);
        let fitted = recorded.as_ref().and_then(|(_, height)| *height).is_some_and(|height| height >= available - TOLERANCE);
        let height = (used < available - TOLERANCE || fitted).then_some(used);
        match (recorded, at) {
            (Some((_, recorded)), Some(at)) => {
                *recorded = height;
                at
            }
            _ => {
                self.contents.push((panel, height));
                self.contents.len() - 1
            }
        }
    }
}

/// Lookups for `DockCommand`, which needs feature `serde`.
#[cfg_attr(not(feature = "serde"), allow(dead_code))]
impl<P: PartialEq> FloatSpace<P> {
    pub(super) fn panel(&self, panel: &P) -> Option<Vec2> {
        Self::find(&self.panels, panel)
    }

    pub(super) fn group(&self, panel: &P) -> Option<Vec2> {
        Self::find(&self.groups, panel)
    }

    pub(super) fn column(&self, panel: &P) -> Option<Vec2> {
        Self::find(&self.columns, panel)
    }

    /// The first record wins: an iconic panel is recorded at its icon before its flyout.
    fn find(sizes: &[(P, Vec2)], panel: &P) -> Option<Vec2> {
        sizes.iter().find(|(p, _)| p == panel).map(|(_, size)| *size)
    }

    /// `[x, y, width, height]` of `size` inside the area, inset from its top-left corner and
    /// shrunk to fit.
    pub(super) fn place(&self, size: Vec2) -> [f32; 4] {
        let size = size.min(self.area.size());
        let min = (self.area.min + Vec2::splat(FLOAT_INSET)).min(self.area.max - size).max(self.area.min);
        [min.x, min.y, size.x, size.y]
    }
}

/// The [`FloatSpace`] the area recorded on its last frame.
pub fn float_space<P: Clone + Send + Sync + 'static>(ctx: &Context, area_id: Id) -> Option<FloatSpace<P>> {
    ctx.data(|d| d.get_temp(float_space_key(area_id)))
}

fn float_space_key(area: Id) -> Id {
    area.with("float-space")
}

/// The panel whose tab held keyboard focus at the end of the last frame, where it was, and
/// whether that frame's own actions may have moved it.
type TabFocus<P> = (P, super::dock_model::Place, bool);

fn tab_focus_key(area: Id) -> Id {
    area.with("tab-focus")
}

/// The layout an area drew while one of its drags was in progress, and the actions it returned.
#[derive(Clone)]
struct DragFrame<P> {
    layout: Layout<P>,
    actions: Vec<Action<P>>,
}

fn drag_frame_key(area: Id) -> Id {
    area.with("drag-frame")
}

/// The controls [`cancel_drag`] ended drags of, until the next frame's native windows, each
/// with its own egui drag, have drawn.
#[cfg(feature = "native-windows")]
pub(in crate::docking) fn native_cancel_key(area: Id) -> Id {
    area.with("cancel-native")
}

/// A drag ends when its layout changes other than by the area's own actions: a workspace
/// switch, Reset or an app command. The app applies all of them or none; an action that
/// fails on its own is skipped, as it is for an app that applies them one at a time. Sizes
/// and rectangles are not compared, so an app that clamps a moved window or adjusts a resized
/// split keeps the drag going.
fn cancel_drag_if_replaced<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(ctx: &Context, area: Id, layout: &Layout<P>) {
    let Some(last) = ctx.data_mut(|d| {
        let last = d.get_temp::<DragFrame<P>>(drag_frame_key(area));
        d.remove::<DragFrame<P>>(drag_frame_key(area));
        last
    }) else {
        return;
    };
    let now = layout.shape();
    if now == last.layout.shape() {
        return;
    }
    let mut applied = last.layout;
    for action in last.actions {
        let _ = applied.apply(action);
    }
    if now != applied.shape() {
        cancel_drag::<P>(ctx, area);
    }
}

/// Keep what [`cancel_drag_if_replaced`] compares the next frame against, while a drag of this
/// area goes on.
fn record_drag_frame<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(
    ctx: &Context,
    area: Id,
    layout: &Layout<P>,
    owned: &[Id],
    native_held: bool,
    actions: &[Action<P>],
) {
    let held = native_held || ctx.dragged_id().is_some_and(|id| owned.contains(&id));
    #[cfg(feature = "native-windows")]
    let held = held || ctx.data(|d| d.get_temp::<super::render::WindowDrag>(area.with("window-drag")).is_some());
    if held || is_dragging::<P>(ctx, area) {
        let frame = DragFrame { layout: layout.clone(), actions: actions.to_vec() };
        ctx.data_mut(|d| d.insert_temp(drag_frame_key(area), frame));
    }
}

/// Keyboard focus stays on a tab only while its panel stays where the user put it: a panel
/// hidden, closed or moved by the host (a command or a workspace reset) gives focus up.
fn release_moved_tab_focus<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(ctx: &Context, area: Id, layout: &Layout<P>) {
    let Some((panel, place, moved_by_area)) = ctx.data(|d| d.get_temp::<TabFocus<P>>(tab_focus_key(area))) else { return };
    let tab = area.with(("tab", &panel));
    let place_now = layout.place(&panel);
    if ctx.memory(|m| m.has_focus(tab)) && (place_now.is_none() || (place_now != Some(place) && !moved_by_area)) {
        ctx.memory_mut(|m| m.surrender_focus(tab));
    }
}

fn record_tab_focus<P: Clone + Eq + Hash + Debug + Send + Sync + 'static>(ctx: &Context, area: Id, layout: &Layout<P>, acted: bool) {
    let focused = ctx.memory(|m| m.focused());
    let tab = focused.and_then(|id| layout.panels().into_iter().find(|panel| area.with(("tab", *panel)) == id));
    let record = tab.and_then(|panel| Some((panel.clone(), layout.place(panel)?, acted)));
    ctx.data_mut(|d| match record {
        Some(record) => {
            d.insert_temp(tab_focus_key(area), record);
        }
        None => d.remove::<TabFocus<P>>(tab_focus_key(area)),
    });
}

/// A request for host state outside the layout.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DockEvent<P> {
    /// Maximize Frame: maximize the frame holding `panel`, or restore it when it is the one
    /// passed to [`DockArea::maximized`].
    ToggleMaximize { panel: P },
    /// [D] Auto-Collapse Iconic Panels was chosen in a column menu. The host flips its
    /// preference and passes it to [`DockArea::auto_collapse_iconic`].
    ToggleAutoCollapse,
    /// [D] The area's flyout opened, switched to another panel or closed (`None`), whatever
    /// changed it: an icon, Escape, a press outside, a raised panel, a group that is no longer
    /// iconic, or [`super::open_flyout`] and [`super::close_flyout`]. It comes once per change,
    /// in the frame that draws the change.
    FlyoutChanged { panel: Option<P> },
}

/// Which panels [`DockArea::hide_panels`] hides. Protected panels always stay.
pub enum HidePanels<'a, P> {
    None,
    All,
    /// Every panel except those for which the function returns true.
    Except(&'a dyn Fn(&P) -> bool),
}

impl<P> Clone for HidePanels<'_, P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P> Copy for HidePanels<'_, P> {}

/// Preferred body bounds. Tiny viewports relax minima proportionally rather than producing
/// inverted rectangles. Maximums constrain a pane when its sibling can occupy the remainder. A
/// tab group takes the largest minimum and the largest maximum of its panels, so a panel without
/// a maximum lifts the maximum of the panels it joins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelLimits {
    pub min: Vec2,
    pub max: Vec2,
}

impl Default for PanelLimits {
    fn default() -> Self {
        Self { min: Vec2::ZERO, max: Vec2::splat(1_000_000.0) }
    }
}

impl PanelLimits {
    pub(super) fn valid(self) -> bool {
        self.min.is_finite()
            && self.max.is_finite()
            && self.min.x >= 0.0
            && self.min.y >= 0.0
            && self.max.x >= self.min.x
            && self.max.y >= self.min.y
            && self.max.x <= 1_000_000.0
            && self.max.y <= 1_000_000.0
    }
}

/// Body heights a panel asks for in a dock column while the user has not resized its group
/// (`Group::height` is `None`). Like [`PanelLimits`] they exclude the tab strip. A group takes
/// the largest of each among its panels.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct DockHeights {
    /// The height the group gets when the column has room.
    pub default: f32,
    /// The height it gives way down to, so the group that fills the column keeps its `fill`.
    pub compact: f32,
    /// The height it asks for when it fills the column: groups at their default shrink towards
    /// `compact` before it gets less.
    pub fill: f32,
}

impl DockHeights {
    /// A `default` height that does not give way (`compact` is `default`) and, filling the
    /// column, asks for no more than its minimum (`fill` is 0).
    pub fn new(default: f32) -> Self {
        Self { default, compact: default, fill: 0.0 }
    }

    pub fn compact(mut self, compact: f32) -> Self {
        self.compact = compact;
        self
    }

    pub fn fill(mut self, fill: f32) -> Self {
        self.fill = fill;
        self
    }

    pub(super) fn valid(self) -> bool {
        [self.default, self.compact, self.fill].iter().all(|h| h.is_finite() && *h >= 0.0 && *h <= 1_000_000.0)
    }
}

/// How a floating window in its own OS window behaves (feature `native-windows`), see
/// [`DockContent::window_kind`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WindowKind {
    /// A tool palette: it stays above other windows and hides while none of the app's windows
    /// has focus.
    #[default]
    Palette,
    /// A window of its own, such as a viewer on a second display: normal window level, and it
    /// stays shown while the app is in the background. When the app is activated, these
    /// windows and the main window come forward, and the main window is left at the normal
    /// level.
    Normal,
}

#[derive(Clone)]
pub(super) struct Drag<P> {
    pub(super) panel: P,
    pub(super) origin: Pos2,
    pub(super) source: Rect,
    pub(super) grab: Vec2,
    /// Ctrl/Cmd held at the press, with [`DockStyle::command_press_floats`]: the drop floats.
    pub(super) undock: bool,
}

/// What a group gesture moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GroupGesture {
    /// A floating window's grip: the window follows the pointer.
    Window,
    /// A drag bar or top border: a ghost follows the pointer, the release picks the target.
    Group,
    /// [D] A column bar: like `Group`, for the whole dock column.
    Column,
}

#[derive(Clone)]
pub(super) struct GroupDrag<P> {
    /// The first member; identifies the gesture.
    pub(super) panel: P,
    /// The floating window rect at the press, restored when a window move is canceled.
    pub(super) original: [f32; 4],
    pub(super) gesture: GroupGesture,
    pub(super) members: Vec<P>,
    pub(super) origin: Pos2,
    /// The group (or window) rect at the press. A floated group keeps this size.
    pub(super) source: Rect,
}

/// Start a docking drag from an application's existing tab/header. Use the same area ID when
/// switching from a legacy layout to [`DockArea`]. The model is untouched until a valid release.
pub fn begin_drag<P: Clone + Send + Sync + 'static>(ctx: &Context, area_id: Id, panel: P, source_rect: Rect) -> bool {
    let Some(origin) = ctx.pointer_interact_pos().filter(|p| p.is_finite()) else { return false };
    if !super::targets::usable(source_rect) {
        return false;
    }
    let grab = (origin - source_rect.min).clamp(Vec2::ZERO, source_rect.size());
    ctx.data_mut(|d| d.insert_temp(area_id.with("drag"), Drag { panel, origin, source: source_rect, grab, undock: false }));
    true
}

/// Whether this area's tab or group gesture is active.
pub fn is_dragging<P: Clone + Send + Sync + 'static>(ctx: &Context, area_id: Id) -> bool {
    ctx.data(|data| data.get_temp::<Drag<P>>(area_id.with("drag")).is_some() || data.get_temp::<GroupDrag<P>>(area_id.with("group-drag")).is_some())
}

/// Discard pending tab/group gestures, a native window's bar drag and an actively dragged
/// renderer-owned control, in the main window now and in a native window when it next draws.
/// Body/custom-header/menu gestures and other areas are untouched, except that an app drag in a
/// panel body no longer reaches other OS windows (feature `native-windows`).
/// Ownership is recorded as controls render; before an initial render or drag threshold there
/// may be no active egui drag to cancel. Keyboard focus is unchanged. Nothing is restored from
/// before the drag: floating and native windows go where the next layout drawn has them.
///
/// An area cancels its drags itself when the layout it is given is not the one it drew last
/// frame with its own actions applied (see [`DockArea::show`]), so replacing the workspace
/// needs no call.
pub fn cancel_drag<P: Clone + Send + Sync + 'static>(ctx: &Context, area_id: Id) {
    let dragged = ctx.dragged_id();
    let owned = ctx.data_mut(|d| {
        d.remove::<Drag<P>>(area_id.with("drag"));
        d.remove::<GroupDrag<P>>(area_id.with("group-drag"));
        #[cfg(feature = "native-windows")]
        d.remove::<super::render::WindowDrag>(area_id.with("window-drag"));
        let ids = d.remove_temp::<Vec<Id>>(area_id.with("owned-interactions"));
        let resize = d.get_temp::<Id>(area_id.with("pointer-resize"));
        d.remove::<Id>(area_id.with("pointer-resize"));
        for id in resize.iter().chain(ids.iter().flatten()) {
            d.remove::<AnchoredResize>(id.with("pointer-anchor"));
        }
        #[cfg(feature = "native-windows")]
        d.get_temp_mut_or_default::<Vec<Id>>(native_cancel_key(area_id)).extend(ids.iter().flatten());
        dragged.is_some_and(|id| resize == Some(id) || ids.is_some_and(|ids| ids.contains(&id)))
    });
    if owned {
        ctx.stop_dragging();
    }
    #[cfg(feature = "native-windows")]
    super::render::cancel_body_route(ctx, area_id);
}

/// Mark the app's drag in a panel body of the area `area_id` as one that carries something, so
/// it reaches the window under the pointer (feature `native-windows`; elsewhere it does
/// nothing). A drag with an egui drag-and-drop payload needs no call. Call it each frame while
/// the drag lasts, from the panel body or before the area shows; a call starts the routing
/// for this frame and the next, and a routing once started lasts until the drag ends. Drags
/// that are never marked (sliders, scrollbars, text selection, canvas pans) stay in their window.
pub fn route_app_drag(ctx: &Context, area_id: Id) {
    #[cfg(feature = "native-windows")]
    super::render::mark_app_drag(ctx, area_id);
    #[cfg(not(feature = "native-windows"))]
    let _ = (ctx, area_id);
}

/// The native window that has the keyboard and a focused text field in its last pass (feature
/// `native-windows`; elsewhere `None`). Not a window drawn under [`DockArea::input_blocked`].
///
/// On macOS the menu bar takes key equivalents such as ⌘C and ⌘V before the key window sees
/// them, and an app's menu glue reports them in the main window's input. While this is `Some`,
/// the dock area moves the keys a text field acts on (⌘C, ⌘X, ⌘V, ⌘A, ⌘Z and ⇧⌘Z, ⌘ with the
/// arrows, ⌘Backspace and ⌘Delete, and `Copy`, `Cut` and `Paste` events) from the main window's
/// input to that window's, as [`send_keys`] does; the app's other command keys stay with the
/// main window. A menu that runs commands itself instead of reporting keys can send the text
/// keys to this window with [`send_keys`]. An app whose own `raw_input_hook` turns the main
/// window's ⌘V into `ViewportCommand::RequestPaste` for its text fields skips that while this
/// is `Some`, so the field's window pastes instead.
pub fn typing_window(ctx: &Context) -> Option<egui::ViewportId> {
    #[cfg(feature = "native-windows")]
    return super::render::typing_window(ctx);
    #[cfg(not(feature = "native-windows"))]
    {
        let _ = ctx;
        None
    }
}

/// Give `events` to the native window `viewport` at the start of its next input, and repaint
/// it (feature `native-windows`; elsewhere it does nothing). A text field copies and pastes on
/// egui's `Copy`, `Cut` and `Paste` events, not on key presses, so ⌘C and ⌘X presses arrive as
/// `Copy` and `Cut`, and ⌘V asks the window for the clipboard text
/// (`ViewportCommand::RequestPaste`), which arrives as `Paste`. Other events arrive as they are.
/// See [`typing_window`].
pub fn send_keys(ctx: &Context, viewport: egui::ViewportId, events: Vec<egui::Event>) {
    #[cfg(feature = "native-windows")]
    super::render::send_keys(ctx, viewport, events);
    #[cfg(not(feature = "native-windows"))]
    let _ = (ctx, viewport, events);
}

/// One mutable application adapter owns bodies, menus, shaped text and per-panel policy,
/// avoiding competing mutable closures. Logical labels are used for tab accessibility.
/// [`DockArea::show_content`] reads every policy from here; the closures passed to the older
/// `show*` functions override `logical_label`, `permissions` and `limits`.
pub trait DockContent<P> {
    fn body(&mut self, ui: &mut Ui, panel: &P);
    /// Plain text name for accessibility, menus and the default visual label.
    fn logical_label(&self, panel: &P) -> String
    where
        P: Debug,
    {
        format!("{panel:?}")
    }
    fn permissions(&self, _panel: &P) -> Permissions {
        Permissions::default()
    }
    fn limits(&self, _panel: &P) -> PanelLimits {
        PanelLimits::default()
    }
    /// How the panel's group sizes itself in a dock column until the user resizes it. Groups
    /// with a [`DockHeights::fill`] above 0 share the column's free height. `None` (the
    /// default) shares it with the other groups that have none when no group asks for a fill,
    /// and takes the group's minimum when one does.
    /// Groups at their [`DockHeights::default`] give way down to their `compact` height, the one
    /// nearest the filler first, so the group that fills the column gets its `fill` height; when
    /// the column is still too short,
    /// every group gives way down to its minimum. A height the user set is kept.
    fn dock_heights(&self, _panel: &P) -> Option<DockHeights> {
        None
    }
    /// Preferred floating size, independent of the current dock geometry: the width of a
    /// floated panel and the most height it takes. A floated panel or group, and a flyout, fit
    /// their height to the content the active panel last drew (at least 120 points), up to this
    /// height, or up to 320 out of a dock when this is `None`. Content that filled its body
    /// takes this height, else 320. A rect a command gives and a window the user resized keep
    /// their size.
    fn floating_size(&self, _panel: &P) -> Option<Vec2> {
        None
    }
    /// An image for the panel's icon, tinted with the icon color. [`Self::paint_icon`] goes first.
    fn icon(&self, _panel: &P) -> Option<egui::ImageSource<'static>> {
        None
    }
    /// Paint the panel's icon in code into `rect` with `color`, and return true. False draws
    /// [`Self::icon`], or the label's initial when that is `None`. `painter` is clipped to the
    /// icon's cell.
    fn paint_icon(&mut self, _painter: &egui::Painter, _rect: Rect, _panel: &P, _color: Color32) -> bool {
        false
    }
    /// Whether the panel's floating window may be its own OS window (feature
    /// `native-windows`). A panel that returns false keeps its window in-window.
    fn allow_native(&self, _panel: &P) -> bool {
        true
    }
    /// How the panel's native floating window behaves (feature `native-windows`). A window
    /// holding any [`WindowKind::Normal`] panel is a normal window; the rest are palettes.
    fn window_kind(&self, _panel: &P) -> WindowKind {
        WindowKind::Palette
    }
    /// A native floating window drew its panel bodies (feature `native-windows`): called once
    /// per pass of the window, after its last body, with the window's `ui`. An app that
    /// publishes its own IME descriptor, outside [`crate::ime::Composer`], publishes it here for
    /// the window. Windows drawn in the main window's pass get no call.
    fn window_drawn(&mut self, _ui: &mut Ui) {}
    /// Host-owned drop resolution for targets the renderer cannot know about. Returning
    /// feedback consumes the gesture, including no-op targets; the renderer then emits no move.
    fn resolve_drag(&mut self, _ctx: &Context, _style: &DockStyle, _request: &DropRequest<'_, P>) -> Option<DropFeedback> {
        None
    }
    fn label(&mut self, _ui: &Ui, _panel: &P, logical: &str, _font: &FontId, _color: Color32) -> egui::WidgetText {
        logical.into()
    }
    /// Shape stock Float/Move/Close action text independently of panel identity.
    /// The renderer preserves `logical` for accessibility, even when the visual text differs.
    fn action_label(&mut self, _ui: &Ui, logical: &str, _font: &FontId, _color: Color32) -> egui::WidgetText {
        logical.into()
    }
    fn panel_menu(&mut self, _ui: &mut Ui, _panel: &P) {}
    /// Width of the group header at the strip's end (the host's ≡ and its own tabs).
    /// `strip_width` is the whole tab strip; the header gets at most that, and the drag bar and
    /// the group's tabs share what is left.
    fn group_header_width(&self, _panels: &[P], _active: &P, _floating: bool, _strip_width: f32) -> f32 {
        0.0
    }
    /// Draw the group header into `ui.max_rect()`, sized by [`Self::group_header_width`].
    /// The header's widgets take their own input, so a drag on them moves nothing. For a widget
    /// that should drag (a ≡ button, say), call [`begin_drag`] with the id given to
    /// [`DockArea::new`], `active` and the widget's rect when its response reports
    /// `drag_started()`; the panel then moves as its tab would.
    fn group_header(&mut self, _ui: &mut Ui, _panels: &[P], _active: &P, _floating: bool) {}
    /// Width the group's extra tabs want: the host's own tabs, drawn right after the group's
    /// panel tabs (and their "…" button), before the drag bar. They fit with the panel tabs:
    /// both shrink widest first to [`DockStyle::min_tab_width`] (the extra tabs as one tab); in a
    /// row narrower still, the panel tabs overflow and the extra tabs keep that width. Zero draws
    /// none.
    fn group_extra_tabs_width(&self, _panels: &[P], _active: &P, _floating: bool) -> f32 {
        0.0
    }
    /// Draw the extra tabs into `ui.max_rect()`, which can be narrower than
    /// [`Self::group_extra_tabs_width`] asked for; the host fits its tabs into it, with an
    /// overflow of its own. Their widgets take their own input as the header's do, and a press
    /// among them focuses the shown panel. A drop on them adds the panel after the last panel tab.
    fn group_extra_tabs(&mut self, _ui: &mut Ui, _panels: &[P], _active: &P, _floating: bool) {}
}

/// Host-owned drop resolution request, see [`DockContent::resolve_drag`].
pub struct DropRequest<'a, P> {
    pub panel: &'a P,
    pub pointer: Pos2,
    pub targets: &'a [DropTarget<P>],
    pub floating_rect: Option<Rect>,
    pub whole_group: bool,
    pub released: bool,
    pub workspace: Rect,
    pub screen: Rect,
}

#[derive(Clone, Debug, Default)]
pub struct DropFeedback {
    pub preview: Option<Rect>,
    pub insertion: Option<Rect>,
}

struct BodyContent<F>(F);
impl<P, F: FnMut(&mut Ui, &P)> DockContent<P> for BodyContent<F> {
    fn body(&mut self, ui: &mut Ui, panel: &P) {
        (self.0)(ui, panel);
    }
}

/// The older `show*` closures in front of an application's content.
struct Overrides<'a, P, C: ?Sized> {
    label: &'a dyn Fn(&P) -> String,
    permissions: &'a dyn Fn(&P) -> Permissions,
    limits: &'a dyn Fn(&P) -> PanelLimits,
    inner: &'a mut C,
}

impl<P, C: DockContent<P> + ?Sized> DockContent<P> for Overrides<'_, P, C> {
    fn body(&mut self, ui: &mut Ui, panel: &P) {
        self.inner.body(ui, panel);
    }
    fn logical_label(&self, panel: &P) -> String {
        (self.label)(panel)
    }
    fn permissions(&self, panel: &P) -> Permissions {
        (self.permissions)(panel)
    }
    fn limits(&self, panel: &P) -> PanelLimits {
        (self.limits)(panel)
    }
    fn dock_heights(&self, panel: &P) -> Option<DockHeights> {
        self.inner.dock_heights(panel)
    }
    fn floating_size(&self, panel: &P) -> Option<Vec2> {
        self.inner.floating_size(panel)
    }
    fn icon(&self, panel: &P) -> Option<egui::ImageSource<'static>> {
        self.inner.icon(panel)
    }
    fn paint_icon(&mut self, painter: &egui::Painter, rect: Rect, panel: &P, color: Color32) -> bool {
        self.inner.paint_icon(painter, rect, panel, color)
    }
    fn allow_native(&self, panel: &P) -> bool {
        self.inner.allow_native(panel)
    }
    fn window_kind(&self, panel: &P) -> WindowKind {
        self.inner.window_kind(panel)
    }
    fn window_drawn(&mut self, ui: &mut Ui) {
        self.inner.window_drawn(ui);
    }
    fn resolve_drag(&mut self, ctx: &Context, style: &DockStyle, request: &DropRequest<'_, P>) -> Option<DropFeedback> {
        self.inner.resolve_drag(ctx, style, request)
    }
    fn label(&mut self, ui: &Ui, panel: &P, logical: &str, font: &FontId, color: Color32) -> egui::WidgetText {
        self.inner.label(ui, panel, logical, font, color)
    }
    fn action_label(&mut self, ui: &Ui, logical: &str, font: &FontId, color: Color32) -> egui::WidgetText {
        self.inner.action_label(ui, logical, font, color)
    }
    fn panel_menu(&mut self, ui: &mut Ui, panel: &P) {
        self.inner.panel_menu(ui, panel);
    }
    fn group_header_width(&self, panels: &[P], active: &P, floating: bool, strip_width: f32) -> f32 {
        self.inner.group_header_width(panels, active, floating, strip_width)
    }
    fn group_header(&mut self, ui: &mut Ui, panels: &[P], active: &P, floating: bool) {
        self.inner.group_header(ui, panels, active, floating);
    }
    fn group_extra_tabs_width(&self, panels: &[P], active: &P, floating: bool) -> f32 {
        self.inner.group_extra_tabs_width(panels, active, floating)
    }
    fn group_extra_tabs(&mut self, ui: &mut Ui, panels: &[P], active: &P, floating: bool) {
        self.inner.group_extra_tabs(ui, panels, active, floating);
    }
}

/// Optional renderer for applications without an existing docking surface. Apps with custom
/// chrome can instead use the model, target resolver and resize/tab primitives independently.
pub struct DockArea<'a, P = ()> {
    id: Id,
    activation: Option<P>,
    blockers: Vec<Rect>,
    locked: bool,
    maximized: Option<P>,
    hide: HidePanels<'a, P>,
    auto_collapse: bool,
    float_bounds: Option<Rect>,
    monitors: Vec<Monitor>,
    input_blocked: bool,
    native_windows: Option<bool>,
    #[cfg(feature = "native-windows")]
    input_hook: Option<super::render::InputHook>,
    #[cfg(feature = "native-windows")]
    window_system: super::render::WindowSystem,
}

impl<'a, P: Clone + Eq + Hash + Debug + Send + Sync + 'static> DockArea<'a, P> {
    pub fn new(id: Id) -> Self {
        Self {
            id,
            activation: None,
            blockers: Vec::new(),
            locked: false,
            maximized: None,
            hide: HidePanels::None,
            auto_collapse: false,
            float_bounds: None,
            monitors: Vec::new(),
            input_blocked: false,
            native_windows: None,
            #[cfg(feature = "native-windows")]
            input_hook: None,
            #[cfg(feature = "native-windows")]
            window_system: Default::default(),
        }
    }

    /// The connected monitors (feature `native-windows`). A native floating window stores the
    /// one it is on, and a restored layout puts each window back on its monitor, or on the
    /// main window's when that monitor is gone. egui 0.36 reports no monitors; without a list
    /// windows store the monitor size egui reports and restore onto the main window's monitor.
    /// eframe 0.36 keeps winit's event loop, and with it `available_monitors`, to itself, so an
    /// eframe application builds the list from the platform's own display API.
    pub fn monitors(mut self, monitors: Vec<Monitor>) -> Self {
        self.monitors = monitors;
        self
    }

    /// Native floating windows ignore input while `blocked` (feature `native-windows`): pass
    /// true while the application shows a modal dialog. egui's `Modal` blocks only the
    /// viewport it is shown in, so the OS windows of floating panels would otherwise still take
    /// clicks and keys. A blocked window is drawn under [`DockStyle::blocked_dim`]; its bar,
    /// tabs, body and resize edges take no pointer, keys, text, scrolling or files, forward only
    /// key releases, and offer no drop targets, and its OS close request is cancelled. Turning
    /// it on cancels any drag in progress. Floating windows drawn in the main window need
    /// nothing from this: the modal covers them.
    pub fn input_blocked(mut self, blocked: bool) -> Self {
        self.input_blocked = blocked;
        self
    }

    /// Whether floating windows open in their own OS windows (feature `native-windows`), where
    /// the backend has viewports. Off, they stay in the app window, and their native rects are
    /// kept for a later run with it on. By default it is on, except on Wayland: the compositor
    /// reports no window positions, so a native window could not dock again by drag or open
    /// where it was.
    pub fn native_windows(mut self, on: bool) -> Self {
        self.native_windows = Some(on);
        self
    }

    /// The egui viewport of the native window of the floating window with [`super::Floating::id`]
    /// `window`, in the dock area `id` (feature `native-windows`).
    #[cfg(feature = "native-windows")]
    pub fn window_viewport(id: Id, window: u64) -> egui::ViewportId {
        super::render::native_viewport(id, window)
    }

    /// Rewrite the input of each native window of this area before egui reads it (feature
    /// `native-windows`), as `eframe::App::raw_input_hook` does for the main window only. Pass
    /// it every frame; a frame without it runs none. The hook runs for every pass of each
    /// window's frame, before any widget sees the input, so rewritten pointer events count as
    /// sent. `RawInput::viewport_id` names the window (see [`Self::window_viewport`]). State
    /// shared with the app's own `raw_input_hook`, which takes `&mut self`, goes in an
    /// `Arc<Mutex<…>>`.
    #[cfg(feature = "native-windows")]
    pub fn native_input_hook(mut self, hook: impl FnMut(&Context, &mut egui::RawInput) + Send + 'static) -> Self {
        self.input_hook = Some(Box::new(hook));
        self
    }

    /// A stand-in for the window system that moves native windows and tells the cursor.
    #[cfg(all(feature = "native-windows", any(test, feature = "testing")))]
    pub(in crate::docking) fn window_system(mut self, window_system: super::render::WindowSystem) -> Self {
        self.window_system = window_system;
        self
    }

    /// [D] Auto-Collapse Iconic Panels (default off): a flyout closes on Escape or a press
    /// outside it. Off, it stays open until its icon or another icon is clicked.
    pub fn auto_collapse_iconic(mut self, on: bool) -> Self {
        self.auto_collapse = on;
        self
    }

    /// Maximize Frame: the docked frame holding `panel` fills the area; other frames and
    /// floating windows do not render or take input. A floating or absent panel is ignored.
    pub fn maximized(mut self, panel: Option<P>) -> Self {
        self.maximized = panel;
        self
    }

    /// Hide panels (Tab, Shift+Tab); their space goes to the panels that stay. Protected
    /// panels always stay. The layout is unchanged.
    pub fn hide_panels(mut self, hide: HidePanels<'a, P>) -> Self {
        self.hide = hide;
        self
    }

    /// Whether keyboard focus belongs to a rendered docking control in this layout.
    /// Panel bodies and application-provided header/menu widgets are excluded. Hosts may
    /// route nonconflicting global shortcuts through chrome while retaining widget ownership
    /// of activation, traversal, resize and cancellation keys. Invalid layouts return false.
    pub fn chrome_has_focus(&self, ctx: &egui::Context, layout: &Layout<P>) -> bool {
        let Some(focused) = ctx.memory(|memory| memory.focused()) else { return false };
        if layout.validate().is_err() || ctx.read_response(focused).is_none() {
            return false;
        }
        super::hits::named(self.id, layout).contains_key(&focused)
    }

    /// Raise a panel's floating window (or open its flyout) on this frame. Apply Activate
    /// through the application's command path as well to select its tab. A raise is not user
    /// input: it moves no keyboard focus and is not reported as [`DockOutput::focused`].
    pub fn raise_panel(mut self, panel: P) -> Self {
        self.activation = Some(panel);
        self
    }

    /// Where floating windows, flyouts and drag ghosts may go (default: the whole viewport).
    /// An app whose own title bar sits under a transparent OS title bar (macOS full-size
    /// content) passes the viewport below that strip: a window drawn there would take presses
    /// that the OS or the app turns into moving the whole app window. Windows outside the
    /// bounds, such as ones restored from a saved layout, are drawn moved inside.
    pub fn float_bounds(mut self, bounds: Rect) -> Self {
        self.float_bounds = Some(bounds);
        self
    }

    /// Draw a locked workspace: pass [`super::Workspace::locked`]. Every panel gets
    /// [`Permissions::locked`], so no tab, bar or native window starts a move, float or close
    /// that the workspace would refuse.
    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }

    /// Host overlays that must reject docking gestures, including floating fallback.
    pub fn blocked_rects(mut self, rects: &[Rect]) -> Self {
        self.blockers = rects.to_vec();
        self
    }

    /// Render a validated snapshot and return requested actions for the app's command handler.
    /// No action is applied here. Panel contents get a global stable egui ID across moves.
    /// A drag in progress (tab, group, column, window bar, divider) ends when `layout` is not
    /// the one drawn last frame, with or without the returned actions applied, apart from sizes
    /// and rectangles: a workspace switch, Reset or a command changed it.
    /// Floating windows are egui areas inside this viewport; with feature `native-windows`, each
    /// gets its own OS window where the backend has viewports (see docs/ui-docking.md).
    pub fn show(
        self,
        ui: &mut Ui,
        layout: &Layout<P>,
        style: &DockStyle,
        label: impl Fn(&P) -> String,
        permissions: impl Fn(&P) -> Permissions,
        body: impl FnMut(&mut Ui, &P),
    ) -> DockOutput<P> {
        self.show_with_limits(ui, layout, style, label, permissions, |_| PanelLimits::default(), body)
    }

    /// Like [`Self::show`], with per-panel size policies supplied by the application.
    #[allow(clippy::too_many_arguments)]
    pub fn show_with_limits(
        self,
        ui: &mut Ui,
        layout: &Layout<P>,
        style: &DockStyle,
        label: impl Fn(&P) -> String,
        permissions: impl Fn(&P) -> Permissions,
        limits: impl Fn(&P) -> PanelLimits,
        body: impl FnMut(&mut Ui, &P),
    ) -> DockOutput<P> {
        self.show_customized(ui, layout, style, label, permissions, limits, &mut BodyContent(body))
    }

    /// Render with application-owned chrome and shaped labels. Header callbacks queue app
    /// commands; structural changes are applied after rendering, like DockOutput actions.
    #[allow(clippy::too_many_arguments)]
    pub fn show_customized(
        self,
        ui: &mut Ui,
        layout: &Layout<P>,
        style: &DockStyle,
        label: impl Fn(&P) -> String,
        permissions: impl Fn(&P) -> Permissions,
        limits: impl Fn(&P) -> PanelLimits,
        content: &mut impl DockContent<P>,
    ) -> DockOutput<P> {
        let mut content = Overrides { label: &label, permissions: &permissions, limits: &limits, inner: content };
        self.show_content(ui, layout, style, &mut content)
    }

    /// Render with every label, policy and size from `content`.
    pub fn show_content(self, ui: &mut Ui, layout: &Layout<P>, style: &DockStyle, content: &mut dyn DockContent<P>) -> DockOutput<P> {
        #[cfg(feature = "native-windows")]
        super::render::set_input_hook(ui.ctx(), self.id, self.input_hook);
        let rect = ui.available_rect_before_wrap();
        if let Err(error) = layout.validate() {
            cancel_drag::<P>(ui.ctx(), self.id);
            return DockOutput { error: Some(error), ..Default::default() };
        }
        if !style.valid() || !super::targets::usable(rect) || layout.panels().into_iter().any(|p| !content.limits(p).valid()) {
            cancel_drag::<P>(ui.ctx(), self.id);
            return DockOutput { error: Some(DockError::InvalidGeometry), ..Default::default() };
        }
        // Blocking starts while the pointer may be down: a drag that went on would act on its
        // release, under the modal.
        let blocked_key = self.id.with("input-blocked");
        let starts = self.input_blocked && !ui.ctx().data(|d| d.get_temp::<bool>(blocked_key).unwrap_or(false));
        if starts {
            cancel_drag::<P>(ui.ctx(), self.id);
        }
        #[cfg(feature = "native-windows")]
        super::render::focus_main_from_native(ui.ctx(), self.id, layout, self.input_blocked, starts);
        ui.ctx().data_mut(|d| d.insert_temp(blocked_key, self.input_blocked));
        cancel_drag_if_replaced(ui.ctx(), self.id, layout);
        release_moved_tab_focus(ui.ctx(), self.id, layout);
        let (view, frame) = super::render::view(layout, self.maximized.as_ref(), self.hide, &*content, !style.side_edge_zones);
        let places = layout.places();
        // A panel keeps the content height it last drew while hidden or minimized.
        let contents: Vec<(P, Option<f32>)> = float_space::<P>(ui.ctx(), self.id)
            .map(|space| space.contents.into_iter().filter(|(panel, _)| places.contains_key(panel)).collect())
            .unwrap_or_default();
        let content_index = contents.iter().enumerate().map(|(i, (panel, _))| (panel.clone(), i)).collect();
        let mut renderer = Renderer {
            id: self.id,
            style,
            content,
            activation: self.activation,
            blockers: self.blockers,
            locked: self.locked,
            layout,
            workspace: rect,
            float_bounds: self.float_bounds.map_or(ui.ctx().content_rect(), |bounds| bounds.intersect(ui.ctx().content_rect())),
            targets: Vec::new(),
            dock_targets: Vec::new(),
            dock_groups: Vec::new(),
            column_scroll: HashMap::new(),
            owned_interactions: Vec::new(),
            output: DockOutput::default(),
            frame,
            iconic: super::render::Iconic::new(self.auto_collapse),
            float_space: FloatSpace { contents, side_edge_zones: style.side_edge_zones, ..FloatSpace::new(rect) },
            places,
            contents: content_index,
            dividers: Vec::new(),
            tree: None,
            monitors: self.monitors,
            input_blocked: self.input_blocked,
            native_windows: self.native_windows,
            #[cfg(feature = "native-windows")]
            cross: Default::default(),
            #[cfg(feature = "native-windows")]
            window_system: self.window_system,
        };
        if let Some(root) = &view.root {
            renderer.node(ui, root, rect, &mut Vec::new(), (root, rect));
            ui.allocate_rect(rect, Sense::hover());
        }
        renderer.flyout(ui.ctx());
        renderer.floating(ui.ctx(), &view.floating);
        #[cfg(feature = "native-windows")]
        renderer.finish_window_drag(ui);
        renderer.finish_group_drag(ui);
        renderer.finish_drag(ui.ctx());
        #[cfg(feature = "native-windows")]
        renderer.end_cross(ui.ctx());
        renderer.report_flyout(ui.ctx());
        super::hits::record(ui.ctx(), self.id, layout, &renderer.owned_interactions);
        record_tab_focus(ui.ctx(), self.id, layout, !renderer.output.actions.is_empty());
        #[cfg(feature = "native-windows")]
        let native_held = renderer.cross.held;
        #[cfg(not(feature = "native-windows"))]
        let native_held = false;
        record_drag_frame(ui.ctx(), self.id, layout, &renderer.owned_interactions, native_held, &renderer.output.actions);
        #[cfg(feature = "native-windows")]
        ui.ctx().data_mut(|d| d.remove::<Vec<Id>>(native_cancel_key(self.id)));
        ui.ctx().data_mut(|d| d.insert_temp(float_space_key(self.id), renderer.float_space));
        ui.ctx().data_mut(|d| d.insert_temp(self.id.with("owned-interactions"), renderer.owned_interactions));
        // egui runs a discarded pass again with the same hovered files (dropped ones are
        // taken), so only the next pass reports them.
        if ui.ctx().will_discard() {
            renderer.output.window_files.retain_mut(|files| {
                files.hovered.clear();
                !files.dropped.is_empty()
            });
        }
        renderer.output
    }
}

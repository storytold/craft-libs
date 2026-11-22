//! `DockGroup`: a group of panels as a container: a header of the panels' tab strip, the drag band and the panel menu
//! button, over the body of the active panel. The band and the button are the group's own parts.

use super::commands::DockCommands;
use super::drop::{self, DragUnit, start_group_drag};
use super::targets::{DockSizes, DockSpot, DockTargets, ZoneRule};
use super::words::DockWords;
use super::{PanelDrag, PanelKey, PanelTitle, Titles};
use crate::buttons::IconButton;
use crate::contract::overlay::Anchor;
use crate::contract::{
    Access, Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, ContextMenuRequest, Cx, HeaderEdge, HeaderParts, HeaderRegion,
    ListDrag, MeasureCx, MemoProps, MemoView, Name, Outbox, Outcome, OverlayUi, Overlays as _, PaintUi, PartsCx, Pointer as _, PopupKind, Props as _,
    Role, ShowCollection as _, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::menu::{GroupItem, Menu, PanelMenu, PanelRow};
use crate::runtime::commands::{CommandSet as _, CommandState, Commands as _};
use crate::runtime::env::Label;
use crate::runtime::focus::{self, FocusTarget};
use crate::runtime::memory::sub;
use crate::style::{ComponentStyle, Corners, RuleLook, RuleWidth, StyleColor, StyleCx, SurfaceKind};
use crate::tabs::{Tab, TabKind, TabStrip, TornOff};
use craft_behavior::{DockSide, IconId, Metrics, PartKey, PressPhase, Sense, keyed};
use craft_theme::{ControlSize, Rgba, Tokens};
use egui::{Id, Pos2, Rect, Response, Ui, Vec2};
use std::borrow::Cow;
use std::fmt::Debug;
use std::hash::Hash;

/// The style of a group grip: its size comes from the tokens.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GroupGripStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The marks a grip with [`ticks`](GroupGrip::ticks) shows.
    marks: GripMarks,
}

/// How many ticks a grip's row of marks has, at every display scale.
const GRIP_TICKS: u16 = 10;
/// How tall a grip's tick is, in points.
const GRIP_TICK_LENGTH: f32 = 4.0;
/// The share of the panel's lightness a grip's ticks keep: the references' marks are a little darker than the panel
/// they sit on in every interface colour (`#454545` on `#535353`, `#272727` on `#323232`, `#9e9e9e` on `#b8b8b8`).
const GRIP_INK_SHARE: f32 = 0.83;

/// The row of marks a dock grip shows, as the references draw it: ten upright ticks, a thin stroke wide, four points
/// tall and a thin stroke apart, centred on the grip, a little darker than the panel. They keep their ink under the
/// pointer, as the references' do. A toolbar's grip under its column bar and the grip above each icon group of a
/// collapsed column paint the same marks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GripMarks {
    ink: StyleColor,
    /// The width of a tick and the gap between two of them.
    side: f32,
    /// The height of a tick.
    length: f32,
}

impl GripMarks {
    /// The marks in the tokens of `cx`.
    pub(super) fn style(cx: &StyleCx<'_>) -> Self {
        GripMarks { ink: cx.color(grip_ink(cx.tokens.color.surface.panel)), side: cx.tokens.stroke.thin, length: GRIP_TICK_LENGTH }
    }

    /// Paints the row centred on `centre`.
    pub(super) fn paint(&self, ui: &PaintUi<'_>, centre: Pos2) {
        let look = RuleLook { color: self.ink.egui(), width: RuleWidth::Points(self.side) };
        ui.draw().grip_ticks(&look, centre, GRIP_TICKS, self.side, self.length);
    }
}

/// The ink of a grip's ticks on a `panel` surface: the panel's channels at [`GRIP_INK_SHARE`].
fn grip_ink(panel: Rgba) -> Rgba {
    let [r, g, b, a] = panel.0;
    let darker = |channel: u8| (f32::from(channel) * GRIP_INK_SHARE).round().clamp(0.0, 255.0) as u8;
    Rgba([darker(r), darker(g), darker(b), a])
}

/// What a group grip reports: its response, the second press of a double click and whether it started a group
/// drag this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct GroupGripOutput {
    /// The grip's response.
    pub response: Response,
    /// The second press of a double click went down on the grip this pass.
    pub double_pressed: bool,
    /// A drag of the whole group started on the grip this pass.
    pub drag_started: bool,
}

impl Outcome for GroupGripOutput {
    type Value = crate::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The handle a group is dragged by: an unmarked band over the empty room of the group's header. A drag starts through [`start_group_drag`] with the grip's unit; a double
/// click is reported for the group to minimize. The drag starts with no extent: a caller whose unit has one sets it on
/// the drag (`GroupDrag::source`, or `strip` for a collapsed column's icons) with `drag::amend` in the pass the grip
/// reports `drag_started`. A grip in a memoized view so holds no size in its props, which a measuring run sees as zero.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(probe(GroupGrip<usize>))]
pub struct GroupGrip<P: Clone + Send + Sync + 'static> {
    #[common]
    common: Common,
    unit: DragUnit<P>,
    active: PanelKey,
    /// The name the unit's drop region shows.
    #[prop]
    ghost: Option<Label>,
    /// A drag on the grip drags the unit.
    #[prop]
    movable: bool,
    /// The grip shows the dock's row of grip marks in its middle; else it is an unmarked band.
    #[prop]
    ticks: bool,
    double: bool,
    started: bool,
}

impl<P: Clone + Send + Sync + 'static> GroupGrip<P> {
    /// The grip of `unit`, whose shown panel is `active`.
    pub fn new(unit: DragUnit<P>, active: PanelKey) -> Self {
        GroupGrip { common: Common::new(), unit, active, ghost: None, movable: true, ticks: false, double: false, started: false }
    }
}

impl<P: Clone + Send + Sync + 'static> Component for GroupGrip<P> {
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = GroupGripStyle;
    type Output = GroupGripOutput;

    fn style(cx: &StyleCx<'_>, _: GroupGripStyleKey<P>) -> GroupGripStyle {
        GroupGripStyle { metrics: Metrics::new(), marks: GripMarks::style(cx) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(grip_width(cx.tokens), cx.tokens.size.panel_header)
    }

    fn sense(&self) -> Sense {
        if self.movable { Sense::CLICK | Sense::DRAG | Sense::FOCUS } else { Sense::CLICK | Sense::FOCUS }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.double = cx.press().double_pressed();
        if self.movable && matches!(cx.press().phase, PressPhase::DragStart { .. }) {
            start_group_drag(cx, self.unit.clone(), self.active, None, self.ghost.clone());
            self.started = true;
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if self.ticks {
            cx.style.marks.paint(ui, cx.rect.center());
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> GroupGripOutput {
        GroupGripOutput { response: cx.response.clone(), double_pressed: self.double, drag_started: self.started }
    }
}

/// The band at the header's end the group is dragged by.
const GRIP: PartKey = PartKey::new("grip");

/// The width of the band a group is dragged by: an icon's room with the panel padding on each side. The strip keeps it
/// free at its end, so long tab names shrink before the band narrows, as in the old dock headers.
fn grip_width(tokens: &Tokens) -> f32 {
    tokens.size.icon.lg + 2.0 * tokens.space.pad.regular
}
/// The panel menu button at the header's end edge.
pub(super) const MENU: PartKey = PartKey::new("panel_menu");
/// Where the group menu hangs while a context request on a tab holds it open: the request's point.
const MENU_AT: &str = "menu_at";
/// The point of a context request on a tab in the last pass, which opens the group menu.
const MENU_ASKED: &str = "menu_asked";

#[derive(Clone, Copy, Default)]
struct MenuAt(Option<Pos2>);

#[derive(Clone, Copy, Default)]
struct MenuAsked(Option<Pos2>);

type HeaderHookFn<'a> = Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a>;
type MenuRowsFn<'a> = Box<dyn FnMut() -> Vec<PanelRow> + 'a>;

/// The rows of a group's menu, built while the menu is open: the panel menu button and a context request on a tab open
/// it.
pub struct MenuRows<'a>(MenuRowsFn<'a>);

impl<'a> MenuRows<'a> {
    /// Rows that `rows` builds each pass the menu shows.
    pub fn new(rows: impl FnMut() -> Vec<PanelRow> + 'a) -> Self {
        MenuRows(Box::new(rows))
    }

    fn rows(&mut self) -> Vec<PanelRow> {
        (self.0)()
    }
}

/// A closure that adds the host's items to a group's header, before the tab strip.
pub struct HeaderHook<'a>(HeaderHookFn<'a>);

impl<'a> HeaderHook<'a> {
    /// A hook that runs `add` in the header's row, with the group's `BodyCx`.
    pub fn new(add: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        HeaderHook(Box::new(add))
    }

    fn run(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        (self.0)(ui, cx);
    }
}

/// What a dock group reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct DockGroupOutput {
    /// The group's response.
    pub response: Response,
    /// The panel whose tab was picked this pass (a click, or the overflow menu); the app makes it the group's active
    /// panel.
    pub picked: Option<PanelKey>,
    /// The panels whose close button or middle click closed their tab this pass.
    pub closed: Vec<PanelKey>,
    /// A context request on a tab (right click, the Menu key, Shift+F10): the group menu opened at the pointer.
    pub menu: Option<ContextMenuRequest<PanelKey>>,
    /// The dock row of the group menu that ran this pass: a Move Panel To target. The menu's command rows fire their
    /// commands, which come back in `commands`.
    pub item: Option<GroupItem>,
    /// The panel commands that reached the group's scope this pass, from a menu row, a key or the palette.
    pub commands: Vec<DockCommands>,
    /// The collapse arrow, or a double click on the band or the strip's empty room, asked to minimize or expand the
    /// group.
    pub toggle_minimized: bool,
    /// A tab was dragged out of the dock's windows.
    pub torn_off: Option<TornOff<PanelKey>>,
    /// A drag started on the group this pass: of the whole group on its band or its strip's empty room, or of a tab.
    /// The host may add the size
    /// the dragged unit floats at with `drag::amend` in this pass.
    pub drag_started: bool,
}

impl Outcome for DockGroupOutput {
    type Value = crate::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The style of a dock group: the header height and the sizes of the spots it registers.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct DockGroupStyle {
    #[style(metrics)]
    metrics: Metrics,
    header: f32,
    sizes: DockSizes,
}

#[derive(Default)]
struct Reported {
    picked: Option<PanelKey>,
    closed: Vec<PanelKey>,
    menu: Option<ContextMenuRequest<PanelKey>>,
    item: Option<GroupItem>,
    commands: Vec<DockCommands>,
    toggle_minimized: bool,
    torn_off: Option<TornOff<PanelKey>>,
    drag_started: bool,
    /// A drag started on the strip's empty room, which `finish` starts as a drag of the group.
    empty_drag: bool,
}

/// A group of panels: a header of tab strip and panel menu button, dragged by its empty part, over the active panel's
/// body. Show it with `show_at(group_rect)` in the body of a `SplitArea`, with an `IdSource::Id` made from the
/// group's anchor so its state survives a re-dock. Under a `dock` id it registers the spots a drop lands on: the
/// strip, and (while expanded) the sides and the middle of the body; in a floating window
/// ([`in_window`](DockGroup::in_window)) the top, the bottom and the middle only. The group is an instance of the
/// `dockGroup` command scope: it publishes the panel command states it is given and reports the commands that reach it.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, overlay, probe(DockGroup<'static, usize>))]
pub struct DockGroup<'a, P: Clone + Hash + Send + Sync + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    anchor: P,
    panels: Titles,
    active: PanelKey,
    /// Only the header shows; the body and its drop spots are left out.
    #[prop(flag)]
    minimized: bool,
    /// The open state the body step reads: closed while minimized, so no body runs.
    open: Bind<'a, bool>,
    /// The grip drags the group.
    #[prop]
    movable: bool,
    /// Tabs of panels that are closable show a close button and close on a middle click.
    #[prop(flag)]
    closable: bool,
    /// The wheel over the tabs shows the next or previous panel, one per wheel notch.
    #[prop(flag)]
    wheel_selects_tabs: bool,
    /// The dock whose drop registry the group's spots go to.
    #[prop]
    dock: Option<Id>,
    /// How the body's side zones are sized.
    #[prop]
    zones: ZoneRule,
    /// The group is in a floating window, which is one column with nothing beside it: the body's start and end
    /// sides take no drop, and its top, bottom and middle zones reach across them.
    #[prop(flag)]
    in_window: bool,
    /// The rows of the group menu, read while it is open: the panel menu button and a context request on a tab open
    /// it.
    #[prop(func = "FnMut() -> Vec<PanelRow>")]
    menu: Option<MenuRows<'a>>,
    /// Whether the panel menu button shows beside the tabs; a context request on a tab opens the menu either way.
    #[prop]
    panel_menu_shown: bool,
    /// The state of each panel command for the shown panel, published in the group's scope every pass.
    #[prop]
    command_states: Vec<(DockCommands, CommandState)>,
    /// The host's items before the tab strip.
    #[prop(func = "FnMut(&mut egui::Ui, &mut BodyCx<'_>)")]
    extra: Option<HeaderHook<'a>>,
    /// The size of the group this pass, where its drag starts.
    size: Vec2,
    reported: Reported,
}

impl<'a, P: Clone + Hash + Send + Sync + PartialEq + Debug + 'static> DockGroup<'a, P> {
    /// The group named by `anchor` (one of its panels) with its `panels`, `active` showing. The group is named by the
    /// active panel's title.
    pub fn new(anchor: P, panels: Titles, active: PanelKey) -> Self {
        let props =
            ContainerProps::new().sizing(Sizing::Fill).header(HeaderRegion::new().fill(SurfaceKind::Background)).command_scope(DockCommands::SCOPE);
        let title = panels.as_slice().iter().find(|panel| panel.key == active).map(|panel| panel.title.clone()).unwrap_or_default();
        DockGroup {
            common: Common::new(),
            props,
            anchor,
            panels,
            active,
            minimized: false,
            open: Bind::given(true),
            movable: true,
            closable: false,
            wheel_selects_tabs: false,
            dock: None,
            zones: ZoneRule::Fraction,
            in_window: false,
            menu: None,
            panel_menu_shown: true,
            command_states: Vec::new(),
            extra: None,
            size: Vec2::ZERO,
            reported: Reported::default(),
        }
        .label(title)
    }

    /// Whether the panel menu button shows: it is asked for and the group has a menu.
    fn shows_menu_button(&self) -> bool {
        self.panel_menu_shown && self.menu.is_some()
    }

    /// The panel menu button's face.
    fn menu_face(name: Label) -> IconButton<'static> {
        IconButton::new(IconId::new("more")).size(ControlSize::Sm).popup(PopupKind::Menu).label(name)
    }

    /// Starts a drag of the whole group: its drop region shows the group's name.
    fn start_drag(&mut self, cx: &mut Cx<'_, Self>) {
        start_group_drag(cx, DragUnit::Group(self.anchor.clone()), self.active, Some(self.size), Some(cx.label()));
        self.reported.drag_started = true;
    }
}

impl<P: Clone + PartialEq + Debug + Hash + Send + Sync + 'static> Component for DockGroup<'_, P> {
    const ROLE: Role = Role::TAB_PANEL;
    type Style = DockGroupStyle;
    type Output = DockGroupOutput;

    fn style(cx: &StyleCx<'_>, _: DockGroupStyleKey<P>) -> DockGroupStyle {
        let sizes = DockSizes::style(cx);
        DockGroupStyle { metrics: Metrics::new(), header: sizes.strip, sizes }
    }

    fn sense(&self) -> Sense {
        Sense::SURFACE
    }

    /// The band the group is dragged by and the panel menu button, at the end of the header: the band first. The
    /// button sits at the strip's top, level with the tab labels' caps, as the "…" of the old tab strips did.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(slot) = cx.header_slot(HeaderEdge::End) else { return };
        let words = cx.words::<DockWords>();
        let flow = craft_behavior::Flow::new(slot, cx.flow.is_rtl());
        let (band, rest) = flow.take_start(grip_width(cx.tokens));
        let sense = if self.movable { Sense::CLICK | Sense::DRAG | Sense::FOCUS } else { Sense::CLICK | Sense::FOCUS };
        let _ = cx.header_part(HeaderEdge::End, GRIP, band).sense(sense).focusable();
        if self.shows_menu_button() {
            // An icon button is square: its side is the slot's room past the band and the gap.
            let side = (slot.width() - grip_width(cx.tokens) - cx.tokens.space.gap.item).max(0.0);
            let end = rest.take_end(side).0;
            let button = Rect::from_min_size(egui::Pos2::new(end.left(), slot.top()), Vec2::new(end.width(), side.min(slot.height())));
            let _ = cx
                .header_part(HeaderEdge::End, MENU, button)
                .sense(Sense::CLICK | Sense::FOCUS)
                .focusable()
                .toggles("panel_menu")
                .tooltip(words.panel_menu.clone());
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        // Painted here, not as the container's surface, which lies over what `paint` draws. The header's band paints
        // the strip over it.
        ui.draw().surface(cx.rect, SurfaceKind::Panel, Corners::All);
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let words = cx.words::<DockWords>();
        if cx.part_if_declared(GRIP).is_some() {
            access.node(GRIP, Role::BUTTON).name = Name::Text(words.move_group.fill(&[]));
        }
        if cx.part_if_declared(MENU).is_some() {
            access.node(MENU, Role::BUTTON).name = Name::Text(words.panel_menu.as_str().to_owned());
        }
    }

    /// The group menu, opened by the panel menu button, under it, or by a context request on a tab, at the request's
    /// point. Its rows are built only while it is open.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let Some(menu) = self.menu.as_mut() else { return };
        let popup = cx.popup("panel_menu");
        let at_id = sub(cx.core.id, MENU_AT);
        if !popup.is_open(cx) {
            cx.core.memory.shown::<MenuAt>(at_id).0 = None;
            return;
        }
        let at = cx.core.memory.shown::<MenuAt>(at_id).0;
        let anchor = match at {
            Some(at) => Anchor::Point(at),
            None if cx.part_if_declared(MENU).is_some() => ui.anchor_part(MENU),
            None => Anchor::Pointer,
        };
        let name = cx.words::<DockWords>().panel_menu.clone();
        let rows = menu.rows();
        let layer = crate::menu::layer(false).anchor(anchor);
        if let Some(item) = ui.show(popup, layer, |ui, _| {
            Menu::inline(name.clone()).show(ui, |ui, _| PanelMenu::new(Vec::new(), rows.clone()).show(ui).picked).inner.flatten()
        }) {
            self.reported.item = item.or(self.reported.item);
        }
    }

    /// The panel menu button, over the header's band, and the group's frame, over the tabs and the body so the strip's
    /// tabs meet it.
    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if cx.part_if_declared(MENU).is_some() {
            ui.face(MENU, Self::menu_face(cx.words::<DockWords>().panel_menu.clone()));
        }
        ui.draw().surface(cx.rect, SurfaceKind::Frame, Corners::None);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.size = cx.rect.size();
        // A context request on a tab in the last pass opens the group menu at its point.
        let asked = std::mem::take(&mut cx.core.memory.shown::<MenuAsked>(sub(cx.core.id, MENU_ASKED)).0);
        if self.menu.is_some() {
            let popup = cx.popup("panel_menu");
            if let Some(at) = asked {
                cx.core.memory.shown::<MenuAt>(sub(cx.core.id, MENU_AT)).0 = Some(at);
                popup.open(cx);
            }
        }
        if let Some(grip) = cx.part_if_declared(GRIP).map(|grip| grip.press) {
            self.reported.toggle_minimized |= grip.double_pressed();
            if self.movable && matches!(grip.phase, PressPhase::DragStart { .. }) {
                self.start_drag(cx);
            }
        }
        let Some(dock) = self.dock else { return };
        let (rect, rtl, header) = (cx.rect, cx.flow.is_rtl(), cx.style.header);
        let strip = Rect::from_min_size(rect.min, Vec2::new(rect.width(), header));
        DockTargets::<P>::register(&cx.core.ctx, dock, DockSpot::tab(self.anchor.clone(), self.panels.as_slice().len()), strip);
        if self.minimized {
            return;
        }
        let body = Rect::from_min_max(Pos2::new(rect.left(), strip.bottom()), rect.max);
        for (side, zone) in self.zones.zones(body, &cx.style.sizes, rtl) {
            let zone = match side {
                DockSide::Start | DockSide::End if self.in_window => continue,
                _ if self.in_window => Rect::from_x_y_ranges(body.x_range(), zone.y_range()),
                _ => zone,
            };
            DockTargets::<P>::register_in(&cx.core.ctx, dock, DockSpot::group(self.anchor.clone(), side), zone, rect);
        }
    }

    /// A drag that started on the strip's empty room in the header starts here, and a context request on a tab is kept
    /// for the next pass's `update` to open the group menu: the header runs after `update`.
    fn finish(mut self, mut cx: Cx<'_, Self>) -> DockGroupOutput {
        if self.reported.empty_drag && self.movable {
            self.start_drag(&mut cx);
        }
        if let Some(request) = &self.reported.menu {
            cx.core.memory.shown::<MenuAsked>(sub(cx.core.id, MENU_ASKED)).0 = Some(request.at);
        }
        let Reported { picked, closed, menu, item, commands, toggle_minimized, torn_off, drag_started, .. } = self.reported;
        DockGroupOutput { response: cx.response.clone(), picked, closed, menu, item, commands, toggle_minimized, torn_off, drag_started }
    }
}

impl<P: Clone + Hash + Send + Sync + PartialEq + Debug + 'static> Container for DockGroup<'_, P> {
    /// Closed while minimized: the body step runs the header alone, and the body leaves no node, shape or stop.
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        self.open = Bind::given(!self.minimized);
        Some(self.open.reborrow())
    }

    /// The drag band, then the panel menu button when it shows, at the header's end edge.
    fn header_parts(&self, cx: &MeasureCx<'_, Self>) -> HeaderParts {
        let band = grip_width(cx.tokens);
        let button = if self.shows_menu_button() {
            cx.tokens.space.gap.item + cx.face(Self::menu_face(cx.words::<DockWords>().panel_menu.clone())).x
        } else {
            0.0
        };
        HeaderParts::NONE.end(band + button)
    }

    /// The host's items, then the tab row, on one line. The tab row is a memoized view (`tab_row`): it runs when its
    /// tabs, its room or the input on it change, else it replays, and what the user did on it comes out of its outbox.
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        for (command, state) in self.command_states.drain(..) {
            cx.publish(command, state);
        }
        while let Some((command, _)) = cx.take_command::<DockCommands>() {
            self.reported.commands.push(command);
        }
        let started = drop::started(&cx.env);
        let mut extra = self.extra.take();
        let env = crate::runtime::env::Env::of(ui);
        let rtl = env.direction.is_rtl();
        let (line, gap) = (env.tokens.size.panel_header, env.tokens.space.gap.item);
        ui.set_min_height(line);
        let surface_pressed = cx.surface_pressed();
        // A raw egui layout: the host's items and the tab row sit side by side, and the header column's flow facts are
        // not theirs. The tab row takes the rest of the line, so the strip's empty room is its own, and nothing but the
        // memoized tab row runs in it, so a settled header replays.
        let layout = if rtl { egui::Layout::right_to_left(egui::Align::Min) } else { egui::Layout::left_to_right(egui::Align::Min) };
        let mut sent = Vec::new();
        let strip = ui
            .with_layout(layout, |ui| {
                ui.spacing_mut().item_spacing.x = gap; // design: allow[style-mutation](the header lays its title cells with an egui layout and sets that layout's gap)
                if let Some(hook) = extra.as_mut() {
                    hook.run(ui, cx);
                }
                let row = TabRow {
                    anchor: self.anchor.clone(),
                    panels: self.panels.clone(),
                    active: self.active,
                    closable: self.closable,
                    wheel_selects: self.wheel_selects_tabs,
                    dock: self.dock,
                    sent: Outbox::new(&mut sent),
                };
                // The env around the tab row holds only values its key follows (the shell, the dock's arrangement, a
                // floating window's layer), no handle or context rebuilt each pass, so it replays at rest.
                MemoView::new(("tabs", cx.id), row, tab_row::<P>).show(ui)
            })
            .inner;
        self.extra = extra;
        // A press on the group's empty body focuses its panel, as in Photoshop.
        if surface_pressed && let Some(strip) = strip {
            focus::request(ui.ctx(), FocusTarget::Id(strip));
        }
        let size = self.size;
        let reported = &mut self.reported;
        for event in sent {
            match event {
                TabEvent::Picked(key) => reported.picked = Some(key),
                TabEvent::Closed(key) => reported.closed.push(key),
                TabEvent::Menu(request) => reported.menu = Some(request),
                TabEvent::TornOff(torn) => reported.torn_off = Some(torn),
                // The strip's empty room is a handle the group is dragged by; a double click on it minimizes the group.
                TabEvent::EmptyDrag => reported.empty_drag = true,
                TabEvent::EmptyDouble => reported.toggle_minimized = true,
                // The tab's drag records the arrangement it started on and the group's size, which only the group reads:
                // the tab row's measuring runs see no size.
                TabEvent::DragStarted => {
                    reported.drag_started = true;
                    crate::runtime::drag::amend::<PanelDrag>(ui.ctx(), |drag| {
                        drag.started = started;
                        drag.source = Some(size);
                    });
                }
            }
        }
    }
}

/// What the user did on a group's tab row, in the order the row reported it.
enum TabEvent {
    /// A tab was picked (a click, the wheel, the overflow menu); the last pick wins.
    Picked(PanelKey),
    /// A tab's close button or a middle click closed it.
    Closed(PanelKey),
    /// A context request on a tab: the group menu opens at its point.
    Menu(ContextMenuRequest<PanelKey>),
    /// A tab was dragged out of the dock's windows.
    TornOff(TornOff<PanelKey>),
    /// A drag started on the strip's empty room: the group's drag.
    EmptyDrag,
    /// A double click on the strip's empty room.
    EmptyDouble,
    /// A tab's drag started.
    DragStarted,
}

/// The props of a group's tab row: the tabs and how they show, the group's size where a tab's drag starts, and the
/// outbox for what the user did.
#[derive(MemoProps)]
struct TabRow<'a, P> {
    /// The group's anchor panel, which names its drop spots.
    #[memo_props(hash = "a dock's panels are plain values it keys by their hash")]
    anchor: P,
    #[memo_props(hash = "its Hash is the content key taken when the titles were made")]
    panels: Titles,
    active: PanelKey,
    closable: bool,
    wheel_selects: bool,
    dock: Option<Id>,
    sent: Outbox<'a, TabEvent>,
}

/// A group's tab strip with its tab insertion spots. Returns the strip's id.
fn tab_row<P: Clone + PartialEq + Debug + Hash + Send + Sync + 'static>(row: &mut TabRow<'_, P>, ui: &mut Ui) -> Id {
    let env = crate::runtime::env::Env::of(ui);
    let name = Label::from(env.words::<DockWords>().panels.as_str().to_owned());
    let rtl = env.direction.is_rtl();
    let (panels, active, closable) = (row.panels.as_slice(), row.active, row.closable);
    let from = PanelKey::of(&row.anchor);
    let drags = crate::runtime::drag::started(ui.ctx());
    let source = keyed(panels, |panel: &PanelTitle| panel.key).with_text(|panel| Cow::Borrowed(panel.title.as_str()));
    let closes = |panel: &PanelTitle| closable && panel.closable;
    let kind = if panels.iter().any(closes) { TabKind::Document } else { TabKind::Panel };
    let mut strip = TabStrip::new(name)
        .kind(kind)
        .shown(active)
        .tear_off()
        .natural_height(true)
        .drag_payload(move |drag: &ListDrag<PanelKey>| Some(PanelDrag { panel: drag.grabbed, from, source: None, started: None }));
    if row.wheel_selects {
        strip = strip.wheel_selects();
    }
    let strip = strip.show_items(ui, &source, &mut |key, _| {
        let panel = panels.iter().find(|panel| &panel.key == key);
        let title = panel.map_or_else(Label::default, |panel| panel.title.clone());
        Tab::new(title).closable(panel.is_some_and(closes))
    });
    for event in strip.response.events {
        if let crate::contract::CollectionEvent::Selection(craft_behavior::SelectionChange::Set(keys)) = event
            && let Some(key) = keys.last()
        {
            row.sent.send(TabEvent::Picked(*key));
        }
    }
    if let Some(dock) = row.dock {
        register_insertions(ui.ctx(), dock, &row.anchor, panels, &strip.tab_rects, rtl);
    }
    if crate::runtime::drag::started(ui.ctx()) != drags && crate::runtime::drag::active::<PanelDrag>(ui.ctx()).is_some_and(|drag| drag.from == from) {
        row.sent.send(TabEvent::DragStarted);
    }
    if let Some(key) = strip.picked {
        row.sent.send(TabEvent::Picked(key));
    }
    for key in strip.closed {
        row.sent.send(TabEvent::Closed(key));
    }
    if let Some(request) = strip.menu {
        row.sent.send(TabEvent::Menu(request));
    }
    if let Some(torn) = strip.torn_off {
        row.sent.send(TabEvent::TornOff(torn));
    }
    if strip.empty_drag {
        row.sent.send(TabEvent::EmptyDrag);
    }
    if strip.empty_double {
        row.sent.send(TabEvent::EmptyDouble);
    }
    strip.response.response.id
}

/// Registers the tab insertion spots of a group: the half of each shown tab nearer the start edge inserts before
/// it, the other half after it, and each spot's mark is the line at the boundary it inserts at.
fn register_insertions<P: Clone + PartialEq + Debug + Hash + Send + Sync + 'static>(
    ctx: &egui::Context,
    dock: Id,
    anchor: &P,
    panels: &[PanelTitle],
    tab_rects: &[(PanelKey, Rect)],
    rtl: bool,
) {
    for (key, tab) in tab_rects {
        let Some(at) = panels.iter().position(|panel| panel.key == *key) else { continue };
        let middle = tab.center().x;
        let (near, far) = if rtl {
            (Rect::from_min_max(Pos2::new(middle, tab.top()), tab.max), Rect::from_min_max(tab.min, Pos2::new(middle, tab.bottom())))
        } else {
            (Rect::from_min_max(tab.min, Pos2::new(middle, tab.bottom())), Rect::from_min_max(Pos2::new(middle, tab.top()), tab.max))
        };
        let (start, end) = if rtl { (tab.right(), tab.left()) } else { (tab.left(), tab.right()) };
        let boundary = |x: f32| Rect::from_min_max(Pos2::new(x, tab.top()), Pos2::new(x, tab.bottom()));
        DockTargets::<P>::register_in(ctx, dock, DockSpot::tab(anchor.clone(), at), near, boundary(start));
        DockTargets::<P>::register_in(ctx, dock, DockSpot::tab(anchor.clone(), at + 1), far, boundary(end));
    }
}

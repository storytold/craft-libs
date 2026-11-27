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
    Access, Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, ContextMenuRequest, Cx, FooterRegion, HeaderEdge, HeaderParts,
    HeaderRegion, ListDrag, MeasureCx, MemoProps, MemoView, Name, Outbox, Outcome, OverlayUi, Overlays as _, PaintUi, PartsCx, Pointer as _,
    PopupKind, Props as _, Role, Show as _, ShowCollection as _, ShowContainer as _, Sizing, WordSetCx as _,
};
use crate::display::{Shade, Text};
use crate::menu::{GroupItem, Menu, PanelMenu, PanelRow};
use crate::primitives::{IconSpec, PaintShape};
use crate::runtime::commands::{self, CommandSet as _, CommandState, Commands as _};
use crate::runtime::env::Label;
use crate::runtime::focus::{self, FocusRequest, FocusTarget};
use crate::runtime::layout_memory;
use crate::runtime::memory::sub;
use crate::sections::FooterBar;
use crate::style::{ComponentStyle, Corners, FocusLook, RuleLook, RuleWidth, StyleColor, StyleCx, SurfaceKind, TypeRole};
use crate::tabs::{PanelTabLook, Tab, TabKind, TabStrip, TornOff};
use craft_behavior::{CursorIcon, CursorPriority, DockSide, IconId, Metrics, PartKey, PressPhase, Sense, keyed};
use craft_theme::{ControlSize, GripMark, GripShape, Tokens};
use egui::{Id, Pos2, Rect, Response, Ui, Vec2};
use std::borrow::Cow;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;

/// The style of a group grip: its size comes from the tokens.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GroupGripStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The ring the grip shows while it has keyboard focus.
    #[style(focus)]
    focus: FocusLook,
    /// The marks a grip with [`ticks`](GroupGrip::ticks) shows.
    marks: GripMarks,
}

/// The row of marks a dock grip shows, centred on the grip, in the theme's `line.grip` (photocraft#16406: the
/// references' 0.83 of the panel was almost invisible on our darker panels). Its shape is the theme's `shape.grip`
/// (photocraft#16426): ticks, dots or one level bar, how many, how large and how far apart; by default eleven upright ticks, a thin
/// stroke wide, five points tall and a thin stroke apart. The marks keep their ink under the pointer. A toolbar's grip
/// under its column bar and the grip above each icon group of a collapsed column paint the same marks, unless the app
/// separates icon groups by a rule (`IconGroupSeparator::Rule`), where the groups have no grip strips.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct GripMarks {
    ink: StyleColor,
    shape: GripShape,
}

impl GripMarks {
    /// The marks in the tokens of `cx`.
    pub(super) fn style(cx: &StyleCx<'_>) -> Self {
        GripMarks { ink: cx.color(cx.tokens.color.line.grip), shape: cx.tokens.shape.grip }
    }

    /// How tall the row is, in points.
    pub(super) fn height(&self) -> f32 {
        marks_height(self.shape)
    }

    /// Paints the row in the grip strip `strip`: centred across it, and [`GRIP_ABOVE`] below its middle, so the strip
    /// keeps that much more room above the marks than below them.
    pub(super) fn paint_in(&self, ui: &PaintUi<'_>, strip: Rect) {
        self.paint(ui, strip.center() + Vec2::new(0.0, GRIP_ABOVE / 2.0));
    }

    /// Paints the row centred on `centre`.
    fn paint(&self, ui: &PaintUi<'_>, centre: Pos2) {
        let GripShape { mark, count, length, width, gap } = self.shape;
        if mark == GripMark::Bar {
            let bar = Rect::from_center_size(centre, Vec2::new(length, width));
            ui.draw().fill(&PaintShape::Rect(bar, egui::CornerRadius::ZERO), self.ink);
            return;
        }
        let look = RuleLook { color: self.ink.egui(), width: RuleWidth::Points(width) };
        ui.draw().grip_ticks(&look, centre, count.max(1), gap, self.height());
    }
}

/// How tall a grip's row of `shape` is, in points: a tick's length, a dot's side, or the bar's thickness.
fn marks_height(shape: GripShape) -> f32 {
    match shape.mark {
        GripMark::Ticks => shape.length,
        GripMark::Dots | GripMark::Bar => shape.width,
    }
}

/// The room a grip strip keeps above its marks beyond `space.gap.tight`, in points, in every theme: the marks stand a
/// little below the strip's middle, clear of the column bar or icon group above (photocraft#16426).
pub(super) const GRIP_ABOVE: f32 = 2.0;

/// The height of a grip strip (above an icon group, under a tool rail's column bar): at least the least hit size, and
/// tall enough that the theme's marks keep `space.gap.tight` below them and that plus [`GRIP_ABOVE`] above them, so
/// they never touch the strip's edge (photocraft#16426).
pub(crate) fn grip_strip(tokens: &Tokens) -> f32 {
    let marks = marks_height(tokens.shape.grip);
    tokens.size.hit.target.max(marks + 2.0 * tokens.space.gap.tight) + GRIP_ABOVE
}

/// What a group grip reports: its response, the second press of a double click and where the press of a group drag
/// it started this pass went down.
#[derive(Debug)]
#[non_exhaustive]
pub struct GroupGripOutput {
    /// The grip's response.
    pub response: Response,
    /// The second press of a double click went down on the grip this pass.
    pub double_pressed: bool,
    /// A drag of the whole group started on the grip this pass.
    pub drag_started: bool,
    /// Where the press that started that drag went down, in the grip's layer space; `None` without a drag start.
    pub drag_origin: Option<Pos2>,
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
    started: Option<Pos2>,
}

impl<P: Clone + Send + Sync + 'static> GroupGrip<P> {
    /// The grip of `unit`, whose shown panel is `active`.
    pub fn new(unit: DragUnit<P>, active: PanelKey) -> Self {
        GroupGrip { common: Common::new(), unit, active, ghost: None, movable: true, ticks: false, double: false, started: None }
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
        GroupGripStyle { metrics: Metrics::new(), focus: cx.focus_look(), marks: GripMarks::style(cx) }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(grip_width(cx.tokens), cx.tokens.size.panel_header)
    }

    fn sense(&self) -> Sense {
        if self.movable { Sense::CLICK | Sense::DRAG | Sense::FOCUS } else { Sense::CLICK | Sense::FOCUS }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.double = cx.press().double_pressed();
        let phase = cx.press().phase;
        if self.movable
            && let PressPhase::DragStart { start, .. } = phase
        {
            start_group_drag(cx, self.unit.clone(), self.active, None, self.ghost.clone());
            self.started = Some(start.origin);
        }
        // A marked grip is a handle: the grab cursor over it and grabbing while it is held or dragged, as the grips of
        // a collapsed column declare (photocraft#16467). A drop target's cursor during the drag still wins.
        if self.ticks && self.movable {
            if phase.button_down() {
                cx.cursor(CursorIcon::Grabbing, CursorPriority::Part);
            } else if cx.hit() {
                cx.cursor(CursorIcon::Grab, CursorPriority::Part);
            }
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        if self.ticks {
            cx.style.marks.paint_in(ui, cx.rect);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> GroupGripOutput {
        GroupGripOutput {
            response: cx.response.clone(),
            double_pressed: self.double,
            drag_started: self.started.is_some(),
            drag_origin: self.started,
        }
    }
}

/// The band at the header's end the group is dragged by.
const GRIP: PartKey = PartKey::new("grip");

/// The width of the band a group is dragged by: an icon's room with the panel padding on each side. The strip keeps it
/// free at its end, so long tab names shrink before the band narrows, as in the old dock headers.
fn grip_width(tokens: &Tokens) -> f32 {
    tokens.size.icon.lg + 2.0 * tokens.space.pad.regular
}

/// The least the drag band narrows to in a header too narrow for it and the shown tab: an icon's room.
fn grip_least(tokens: &Tokens) -> f32 {
    tokens.size.icon.lg
}

/// How many of the shown tab's first letters the header keeps room for before the drag band narrows, as many as a
/// shrunk panel tab shows.
const SHOWN_TAB_LETTERS: usize = 3;
/// Where the header hands forward the width the host's items took before the tab strip, with the gap after them
/// (`layout_memory::emit_width`), keyed under the group's body id.
fn host_items_key(body: Id) -> Id {
    sub(body, "host_items")
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
    /// The panel and document whose tab was picked this pass ([`PanelDocument`](super::PanelDocument)).
    pub picked_document: Option<(PanelKey, u64)>,
    /// The panels and documents whose close button or middle click closed their tab this pass.
    pub closed_documents: Vec<(PanelKey, u64)>,
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
    /// The tabs the strip shows this pass, each with its rect and the id of its accessibility node; none for a
    /// [`titled`](DockGroup::titled) group.
    pub tabs: Vec<(PanelKey, Rect, Id)>,
    /// The band the group is dragged by this pass, with the id of its accessibility node.
    pub grip: Option<(Rect, Id)>,
    /// The strip's overflow button, when some tabs do not fit, with the id of its accessibility node.
    pub overflow: Option<(Rect, Id)>,
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
    /// The ring around the drag band or the panel menu button while it has keyboard focus.
    #[style(focus)]
    focus: FocusLook,
    header: f32,
    sizes: DockSizes,
}

#[derive(Default)]
struct Reported {
    picked: Option<PanelKey>,
    closed: Vec<PanelKey>,
    picked_document: Option<(PanelKey, u64)>,
    closed_documents: Vec<(PanelKey, u64)>,
    menu: Option<ContextMenuRequest<PanelKey>>,
    item: Option<GroupItem>,
    commands: Vec<DockCommands>,
    toggle_minimized: bool,
    torn_off: Option<TornOff<PanelKey>>,
    drag_started: bool,
    /// A drag started on the strip's empty room, which `finish` starts as a drag of the group.
    empty_drag: bool,
    tabs: Vec<(PanelKey, Rect, Id)>,
    overflow: Option<(Rect, Id)>,
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
    /// The look of the tabs: [`tab_look`](DockGroup::tab_look).
    tab_look: PanelTabLook,
    /// The header shows the shown panel's title (with its count) and a close button in place of the tab strip, as a
    /// single titled panel does: the panels are shown from a rail, not by tabs.
    #[prop(flag)]
    titled: bool,
    /// The tab row takes key focus in this pass: [`take_focus`](DockGroup::take_focus).
    take_focus: bool,
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
    /// The shown panel's body is the app's own egui code (`PanelSpec::host_body`): a press on the body is the app's,
    /// and its widgets take egui focus themselves, so the press does not focus the group's tabs.
    #[prop(flag)]
    host_body: bool,
    /// The rows of the group menu, read while it is open: the panel menu button and a context request on a tab open
    /// it.
    #[prop(func = "FnMut() -> Vec<PanelRow>")]
    menu: Option<MenuRows<'a>>,
    /// Whether the panel menu button shows beside the tabs; a context request on a tab opens the menu either way.
    #[prop]
    panel_menu_shown: bool,
    /// The panel menu button's icon: `more` unless the dock gives another (`DockConfig::panel_menu_icon`).
    #[prop]
    panel_menu_icon: IconId,
    /// The state of each panel command for the shown panel, published in the group's scope every pass.
    #[prop]
    command_states: Vec<(DockCommands, CommandState)>,
    /// The host's items before the tab strip.
    #[prop(func = "FnMut(&mut egui::Ui, &mut BodyCx<'_>)")]
    extra: Option<HeaderHook<'a>>,
    /// The shown panel's footer, pinned under the body: [`footer`](DockGroup::footer).
    footer: Option<HeaderHookFn<'a>>,
    /// The size of the group this pass, where its drag starts.
    size: Vec2,
    reported: Reported,
}

impl<'a, P: Clone + Hash + Send + Sync + PartialEq + Debug + 'static> DockGroup<'a, P> {
    /// The group named by `anchor` (one of its panels) with its `panels`, `active` showing. The group is named by the
    /// active panel's title.
    pub fn new(anchor: P, panels: Titles, active: PanelKey) -> Self {
        let props = ContainerProps::new()
            .sizing(Sizing::Fill)
            .header(HeaderRegion::new().fill(strip_surface(PanelTabLook::Plain)))
            .command_scope(DockCommands::SCOPE);
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
            tab_look: PanelTabLook::Plain,
            titled: false,
            take_focus: false,
            dock: None,
            zones: ZoneRule::Fraction,
            in_window: false,
            host_body: false,
            menu: None,
            panel_menu_shown: true,
            panel_menu_icon: IconId::new("more"),
            command_states: Vec::new(),
            extra: None,
            footer: None,
            size: Vec2::ZERO,
            reported: Reported::default(),
        }
        .label(title)
    }

    /// The shown panel's footer: the closure fills a footer bar pinned under the body, across the group, that stays in
    /// view while the body scrolls. A minimized group shows none.
    #[must_use]
    pub fn footer(mut self, content: impl FnMut(&mut Ui, &mut BodyCx<'_>) + 'a) -> Self {
        self.props = self.props.footer(FooterRegion::new());
        self.footer = Some(Box::new(content));
        self
    }

    /// The group takes key focus in this pass, as a press on its body gives it: its tab row's stop takes it while the
    /// row runs, so the keys of the next pass route through the group; a titled header, which has no tab row, focuses
    /// its drag band. A request of the pass.
    #[must_use]
    pub fn take_focus(self) -> Self {
        DockGroup { take_focus: true, ..self }
    }

    /// The look of the tabs ([`DockConfig::tab_look`](super::DockConfig::tab_look)). [`PanelTabLook::Block`] tabs
    /// lie on a strip of `surface.canvas`, the others on the window's background.
    #[must_use]
    pub fn tab_look(mut self, look: PanelTabLook) -> Self {
        self.props = self.props.header(HeaderRegion::new().fill(strip_surface(look)));
        self.tab_look = look;
        self
    }

    /// Whether the panel menu button shows: it is asked for and the group has a menu.
    fn shows_menu_button(&self) -> bool {
        self.panel_menu_shown && self.menu.is_some()
    }

    /// The panel menu button's face.
    fn menu_face(&self, name: Label) -> IconButton<'static> {
        IconButton::new(self.panel_menu_icon.clone()).size(ControlSize::Sm).popup(PopupKind::Menu).label(name)
    }

    /// The room the panel menu button takes past the drag band, with the gap before it; none when it does not show.
    fn menu_room(&self, cx: &MeasureCx<'_, Self>) -> f32 {
        if self.shows_menu_button() { cx.tokens.space.gap.item + cx.face(self.menu_face(cx.words::<DockWords>().panel_menu.clone())).x } else { 0.0 }
    }

    /// The least room the strip takes to show the shown tab's first letters: the tab as the strip builds it (icon,
    /// count, mark and close button) with only those letters of its title, and beside other tabs the overflow
    /// button, which is as wide as the tab is high.
    fn shown_tab_least(&self, cx: &MeasureCx<'_, Self>) -> f32 {
        let panels = self.panels.as_slice();
        let tabs = strip_tabs(panels, self.closable);
        let shown = shown_tab(panels, self.active);
        let Some(tab) = tabs.iter().find(|tab| tab.key == shown) else { return 0.0 };
        let letters: String = tab.title.as_str().chars().take(SHOWN_TAB_LETTERS).collect();
        let marked = tab_marks(cx.ui.ctx(), panels).iter().any(|(key, _)| *key == shown);
        let mut face = Tab::new(letters).kind(TabKind::Panel).panel_look(self.tab_look).closable(tab.closes).marked(marked);
        if let Some(count) = tab.count {
            face = face.detail(cx.env.format().integer(u64::try_from(count).unwrap_or(u64::MAX)));
        }
        match tab.swatch {
            Some(Some(color)) => face = face.icon(IconSpec::swatch(color)),
            Some(None) => face = face.icon(IconSpec::empty_swatch()),
            None => {}
        }
        let size = cx.face(face);
        if tabs.len() > 1 { size.x + size.y } else { size.x }
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
        DockGroupStyle { metrics: Metrics::new(), focus: cx.focus_look(), header: sizes.strip, sizes }
    }

    fn sense(&self) -> Sense {
        Sense::SURFACE
    }

    /// The band the group is dragged by and the panel menu button, at the end of the header: the band first. The
    /// button sits at the strip's top, level with the tab labels' caps, as the "…" of the old tab strips did; beside
    /// [`PanelTabLook::Pill`] and [`PanelTabLook::Block`] tabs, in the strip's middle.
    fn parts(&mut self, _: Rect, cx: &mut PartsCx<'_, Self>) {
        let Some(slot) = cx.header_slot(HeaderEdge::End) else { return };
        let words = cx.words::<DockWords>();
        let flow = craft_behavior::Flow::new(slot, cx.flow.is_rtl());
        // The band takes what the slot leaves past the panel menu button, which a narrowed header takes from the band.
        let width = (slot.width() - self.menu_room(cx)).clamp(0.0, grip_width(cx.tokens));
        let (band, rest) = flow.take_start(width);
        let sense = if self.movable { Sense::CLICK | Sense::DRAG | Sense::FOCUS } else { Sense::CLICK | Sense::FOCUS };
        let _ = cx.header_part(HeaderEdge::End, GRIP, band).sense(sense).focusable();
        if self.shows_menu_button() {
            // An icon button is square: its side is the slot's room past the band and the gap.
            let side = (slot.width() - width - cx.tokens.space.gap.item).max(0.0);
            let end = rest.take_end(side).0;
            let height = side.min(slot.height());
            let top = if self.tab_look == PanelTabLook::Plain { slot.top() } else { slot.center().y - height / 2.0 };
            let button = Rect::from_min_size(egui::Pos2::new(end.left(), top), Vec2::new(end.width(), height));
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
            ui.face(MENU, self.menu_face(cx.words::<DockWords>().panel_menu.clone()));
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
        // A titled header has no tab row: its drag band is the group's stop.
        if self.take_focus && self.titled && cx.part_if_declared(GRIP).is_some() {
            focus::request(&cx.core.ctx, FocusTarget::Part(cx.core.id, GRIP.into()));
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
        let grip = cx.part_if_declared(GRIP).map(|grip| (grip.rect, crate::runtime::access::sub_node_id(cx.core.id, GRIP.into())));
        let Reported {
            picked,
            closed,
            picked_document,
            closed_documents,
            menu,
            item,
            commands,
            toggle_minimized,
            torn_off,
            drag_started,
            tabs,
            overflow,
            ..
        } = self.reported;
        DockGroupOutput {
            response: cx.response.clone(),
            picked,
            closed,
            picked_document,
            closed_documents,
            menu,
            item,
            commands,
            toggle_minimized,
            torn_off,
            drag_started,
            tabs,
            grip,
            overflow,
        }
    }
}

impl<P: Clone + Hash + Send + Sync + PartialEq + Debug + 'static> Container for DockGroup<'_, P> {
    /// Closed while minimized: the body step runs the header alone, and the body leaves no node, shape or stop.
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        self.open = Bind::given(!self.minimized);
        Some(self.open.reborrow())
    }

    /// The drag band, then the panel menu button when it shows, at the header's end edge. In a header too narrow for
    /// them, the host's items and the shown tab's least room, the band narrows to an icon's room, so the shown tab stays
    /// visible. The host's items count with the width the header last handed forward for them; before it has, they
    /// count as none.
    fn header_parts(&self, cx: &MeasureCx<'_, Self>) -> HeaderParts {
        let button = self.menu_room(cx);
        let parts = HeaderParts::NONE.end(grip_width(cx.tokens) + button);
        if self.titled {
            return parts;
        }
        let host = self.extra.as_ref().and_then(|_| layout_memory::width(cx.ui.ctx(), host_items_key(sub(cx.id, "body")))).unwrap_or(0.0);
        parts.end_narrows(grip_least(cx.tokens) + button, host + self.shown_tab_least(cx))
    }

    fn footer(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if let Some(content) = self.footer.as_mut() {
            FooterBar::body().show(ui, |ui, bar| content(ui, bar));
        }
    }

    /// The host's items, then the tab row, on one line; then the shown panel's caption. The tab row is a memoized view
    /// (`tab_row`): it runs when its tabs, its room or the input on it change, else it replays, and what the user did
    /// on it comes out of its outbox. A titled group shows the shown panel's title, count and close button in place of
    /// the tab row.
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        // A measuring run of the header comes before the real one in some passes: both read the states.
        for (command, state) in &self.command_states {
            cx.publish(*command, state.clone());
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
        let titled = self.titled;
        let cross = if titled { egui::Align::Center } else { egui::Align::Min };
        let layout = if rtl { egui::Layout::right_to_left(cross) } else { egui::Layout::left_to_right(cross) };
        let mut sent = Vec::new();
        let strip = ui
            .with_layout(layout, |ui| {
                ui.spacing_mut().item_spacing.x = gap; // design: allow[style-mutation](the header lays its title cells with an egui layout and sets that layout's gap)
                let shown = self.panels.as_slice().iter().find(|panel| panel.key == self.active);
                if titled && let Some(panel) = shown {
                    Text::new(panel.title.clone()).role(TypeRole::Title).show(ui);
                    if let Some(document) = panel.documents.iter().find(|document| document.shown) {
                        Text::new(document.label.clone()).role(TypeRole::Title).shade(Shade::Faint).show(ui);
                    }
                    if let Some(count) = panel.count {
                        let count = env.format().integer(u64::try_from(count).unwrap_or(u64::MAX));
                        Text::new(count).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
                    }
                }
                if let Some(hook) = extra.as_mut() {
                    // The cursor moves past each item and the gap after it, so the distance it moves is the room the
                    // items take before the tab row.
                    let before = ui.cursor();
                    hook.run(ui, cx);
                    let after = ui.cursor();
                    let took = if rtl { before.max.x - after.max.x } else { after.min.x - before.min.x };
                    layout_memory::emit_width(ui.ctx(), host_items_key(cx.id), took.max(0.0));
                }
                if titled {
                    let close_name = cx.words::<DockWords>().close_panel.clone();
                    if let Some(panel) = shown.filter(|panel| self.closable && panel.closable) {
                        let name = Label::from(close_name.fill(&[("name", panel.title.as_str())]));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if crate::contract::clicked(&IconButton::new(IconId::new("close")).size(ControlSize::Sm).label(name).show(ui)) {
                                sent.push(TabEvent::Closed(panel.key));
                            }
                        });
                    }
                    return None;
                }
                // The panels' toggle commands resolve here, outside the memoized row, so the marks are part of its key
                // and a mark that comes or goes repaints the row.
                let marks = tab_marks(ui.ctx(), self.panels.as_slice());
                let row = TabRow {
                    anchor: self.anchor.clone(),
                    panels: self.panels.clone(),
                    marks,
                    active: self.active,
                    closable: self.closable,
                    wheel_selects: self.wheel_selects_tabs,
                    look: self.tab_look,
                    take_focus: self.take_focus,
                    dock: self.dock,
                    sent: Outbox::new(&mut sent),
                };
                // The env around the tab row holds only values its key follows (the shell, the dock's arrangement, a
                // floating window's layer), no handle or context rebuilt each pass, so it replays at rest.
                MemoView::new(("tabs", cx.id), row, tab_row::<P>).show(ui)
            })
            .inner;
        self.extra = extra;
        // A press on the group's empty body focuses its panel, as in Photoshop; an app's own egui body keeps its presses.
        if surface_pressed
            && !self.host_body
            && let Some((strip, _, _)) = &strip
        {
            focus::request(ui.ctx(), FocusTarget::Id(*strip));
        }
        if let Some((strip, tabs, overflow)) = &strip {
            self.reported.tabs = tabs.iter().map(|(key, rect)| (*key, *rect, crate::contract::item_node(*strip, key))).collect();
            self.reported.overflow = *overflow;
        }
        let size = self.size;
        let (titles, reported) = (&self.panels, &mut self.reported);
        for event in sent {
            match event {
                TabEvent::Picked(key) => match titles.document_of(key) {
                    Some(document) => reported.picked_document = Some(document),
                    None => reported.picked = Some(key),
                },
                TabEvent::Closed(key) => match titles.document_of(key) {
                    Some(document) => reported.closed_documents.push(document),
                    None => reported.closed.push(key),
                },
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
        if let Some(caption) = self.panels.as_slice().iter().find(|panel| panel.key == self.active).and_then(|panel| panel.caption.clone()) {
            Text::new(caption).role(TypeRole::Caption).shade(Shade::Faint).show(ui);
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

/// The surface a dock's strips lie on with tabs in `look`: its groups' tab strips and its columns' bars.
pub(crate) fn strip_surface(look: PanelTabLook) -> SurfaceKind {
    if look == PanelTabLook::Block { SurfaceKind::Canvas } else { SurfaceKind::Background }
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
    /// The marked tabs, each with the hint that says why: resolved outside the row, so the row's key follows them.
    #[memo_props(hash = "panel keys and hint texts are plain values")]
    marks: Vec<(PanelKey, Option<Arc<str>>)>,
    active: PanelKey,
    closable: bool,
    wheel_selects: bool,
    look: PanelTabLook,
    /// The strip takes key focus in this pass (`DockGroup::take_focus`): in the key, so the row runs and the strip
    /// takes it while it runs.
    take_focus: bool,
    dock: Option<Id>,
    sent: Outbox<'a, TabEvent>,
}

/// The tabs of `panels` whose toggle command marks them, as it marks the panel's rail button, with the command's hint.
fn tab_marks(ctx: &egui::Context, panels: &[PanelTitle]) -> Vec<(PanelKey, Option<Arc<str>>)> {
    panels
        .iter()
        .filter_map(|panel| {
            let resolved = commands::resolve(ctx, panel.command.clone()?);
            IconButton::shows_marker(&resolved).then(|| (panel.key, resolved.hint.clone()))
        })
        .collect()
}

/// One tab of a group's strip: a panel's, or one of a panel's documents.
struct StripTab<'t> {
    key: PanelKey,
    title: &'t Label,
    swatch: Option<Option<craft_theme::Srgba>>,
    closes: bool,
    count: Option<usize>,
    /// A document's tab: it is not dragged.
    document: bool,
}

/// The tabs of a group's strip: the panels' tabs, then every panel's documents in panel order.
fn strip_tabs(panels: &[PanelTitle], closable: bool) -> Vec<StripTab<'_>> {
    let documents = panels.iter().flat_map(|panel| {
        panel.documents.iter().map(move |document| StripTab {
            key: panel.document_key(document.id),
            title: &document.label,
            swatch: document.swatch,
            closes: closable && document.closable,
            count: None,
            document: true,
        })
    });
    panels
        .iter()
        .map(|panel| StripTab {
            key: panel.key,
            title: &panel.title,
            swatch: panel.swatch,
            closes: closable && panel.closable,
            count: panel.count,
            document: false,
        })
        .chain(documents)
        .collect()
}

/// The tab the strip shows as selected: the shown document of the `active` panel, else that panel's own tab.
fn shown_tab(panels: &[PanelTitle], active: PanelKey) -> PanelKey {
    panels
        .iter()
        .find(|panel| panel.key == active)
        .and_then(|panel| panel.documents.iter().find(|document| document.shown).map(|document| panel.document_key(document.id)))
        .unwrap_or(active)
}

/// What a group's tab row shows: the strip's id, each shown tab's rect and the overflow button with its node id.
type TabRowShown = (Id, Vec<(PanelKey, Rect)>, Option<(Rect, Id)>);

/// A group's tab strip with its tab insertion spots. Returns the strip's id, the rect of each tab it shows and the
/// overflow button with its node id, which a replay returns again, moved with the row. The panels' documents are tabs of the same strip after the panel tabs: one
/// fit, one overflow menu and one keyboard order for both. Only panel tabs drag and take tab insertions.
fn tab_row<P: Clone + PartialEq + Debug + Hash + Send + Sync + 'static>(row: &mut TabRow<'_, P>, ui: &mut Ui) -> TabRowShown {
    let env = crate::runtime::env::Env::of(ui);
    let name = Label::from(env.words::<DockWords>().panels.as_str().to_owned());
    let rtl = env.direction.is_rtl();
    let (panels, active, closable) = (row.panels.as_slice(), row.active, row.closable);
    let from = PanelKey::of(&row.anchor);
    let drags = crate::runtime::drag::started(ui.ctx());
    let tabs = strip_tabs(panels, closable);
    let source = keyed(&tabs, |tab: &StripTab<'_>| tab.key).with_text(|tab| Cow::Borrowed(tab.title.as_str()));
    let tabs = &tabs;
    let documents: Vec<PanelKey> = tabs.iter().filter(|tab| tab.document).map(|tab| tab.key).collect();
    // A press on a panel tab leaves key focus where it is, as in Photoshop: the document's keys keep working.
    let mut strip = TabStrip::new(name)
        // Panel tabs whether or not they close: they shrink to their first letters, and the closable ones show a close
        // button.
        .kind(TabKind::Panel)
        .keeps_focus()
        .panel_look(row.look)
        .shown(shown_tab(panels, active))
        .tear_off()
        .natural_height(true)
        // Group, column and icon drags name what they move inside the drop region; a tab drag does the same.
        .drag_preview(crate::contract::RowPreview::Named)
        .can_close(move |key: &PanelKey| tabs.iter().any(|tab| &tab.key == key && tab.closes))
        .can_drag(move |key: &PanelKey| tabs.iter().any(|tab| &tab.key == key && !tab.document))
        .drag_payload(move |drag: &ListDrag<PanelKey>| {
            (!documents.contains(&drag.grabbed)).then_some(PanelDrag { panel: drag.grabbed, from, source: None, started: None })
        });
    if row.wheel_selects {
        strip = strip.wheel_selects();
    }
    // The strip takes the focus while it runs, so it runs focused and the keys of the next pass route through it.
    if row.take_focus {
        strip = strip.request_focus(FocusRequest::Take);
    }
    let marks = &row.marks;
    let strip = strip.show_items(ui, &source, &mut |key, _| {
        let tab = tabs.iter().find(|tab| &tab.key == key);
        let title = tab.map_or_else(Label::default, |tab| tab.title.clone());
        // The panel's toggle command marks its tab as it marks the panel's rail button, with the hint to say why.
        let mark = marks.iter().find(|(marked, _)| marked == key);
        let mut shown = Tab::new(title).closable(tab.is_some_and(|tab| tab.closes)).marked(mark.is_some());
        if let Some(count) = tab.and_then(|tab| tab.count) {
            shown = shown.detail(env.format().integer(u64::try_from(count).unwrap_or(u64::MAX)));
        }
        // A data colour before the name, in the tab's icon slot; not part of its name.
        match tab.and_then(|tab| tab.swatch) {
            Some(Some(color)) => shown = shown.icon(IconSpec::swatch(color)),
            Some(None) => shown = shown.icon(IconSpec::empty_swatch()),
            None => {}
        }
        match mark.and_then(|(_, hint)| hint.clone()) {
            Some(hint) => shown.tooltip(hint),
            None => shown,
        }
    });
    for event in strip.response.events {
        if let crate::contract::CollectionEvent::Selection { change: craft_behavior::SelectionChange::Set(keys), .. } = event
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
    (strip.response.response.id, strip.tab_rects, strip.overflow)
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

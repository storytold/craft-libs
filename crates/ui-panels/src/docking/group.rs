//! `DockGroup`: a group of panels as a container: a header of the panels' tab strip over the strip's drag band and the
//! panel menu button, over the body of the active panel.

use super::commands::DockCommands;
use super::drop::{self, DragUnit, start_group_drag};
use super::targets::{DockSizes, DockSpot, DockTargets, ZoneRule};
use super::words::DockWords;
use super::{PanelDrag, PanelKey, PanelTitle};
use crate::buttons::IconButton;
use crate::contract::{
    BodyCx, Common, Component, Container, ContainerProps, ContextMenuRequest, Cx, HeaderRegion, ListDrag, MeasureCx, Outcome, PaintUi, Pointer as _,
    PopupKind, Props as _, Role, Show as _, ShowCollection as _, ShowContainer as _, Sizing, Trigger, WordSetCx as _,
};
use crate::layout_components::Row;
use crate::menu::{GroupItem, Menu, PanelMenu, PanelRow};
use crate::runtime::commands::{CommandSet as _, CommandState, Commands as _};
use crate::runtime::env::Label;
use crate::runtime::focus::{self, FocusTarget};
use crate::style::{ComponentStyle, Corners, StyleCx, SurfaceKind};
use crate::tabs::{Tab, TabKind, TabStrip, TornOff};
use craft_behavior::{IconId, Metrics, PressPhase, Sense, keyed};
use craft_theme::ControlSize;
use egui::{Id, Pos2, Rect, Response, Ui, Vec2};
use std::borrow::Cow;
use std::hash::Hash;

/// The style of a group grip: its size comes from the tokens.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct GroupGripStyle {
    #[style(metrics)]
    metrics: Metrics,
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
/// click is reported for the group to minimize.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(probe(GroupGrip<usize>))]
pub struct GroupGrip<P: Clone + Send + Sync + 'static> {
    #[common]
    common: Common,
    unit: DragUnit<P>,
    active: PanelKey,
    /// The ghost the drag shows: its extent and its label.
    #[prop]
    ghost: Option<(Vec2, Label)>,
    /// A drag on the grip drags the unit.
    #[prop]
    movable: bool,
    double: bool,
    started: bool,
}

impl<P: Clone + Send + Sync + 'static> GroupGrip<P> {
    /// The grip of `unit`, whose shown panel is `active`.
    pub fn new(unit: DragUnit<P>, active: PanelKey) -> Self {
        GroupGrip { common: Common::new(), unit, active, ghost: None, movable: true, double: false, started: false }
    }
}

impl<P: Clone + Send + Sync + 'static> Component for GroupGrip<P> {
    const ROLE: Role = Role::BUTTON;
    const PARTS: bool = false;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = GroupGripStyle;
    type Output = GroupGripOutput;

    fn style(_: &StyleCx<'_>, _: GroupGripStyleKey<P>) -> GroupGripStyle {
        GroupGripStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(cx.tokens.size.handle.band_outer, cx.tokens.size.panel_header)
    }

    fn sense(&self) -> Sense {
        if self.movable { Sense::CLICK | Sense::DRAG | Sense::FOCUS } else { Sense::CLICK | Sense::FOCUS }
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.double = cx.press().double_pressed();
        if self.movable && matches!(cx.press().phase, PressPhase::DragStart { .. }) {
            start_group_drag(cx, self.unit.clone(), self.active, self.ghost.clone());
            self.started = true;
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> GroupGripOutput {
        GroupGripOutput { response: cx.response.clone(), double_pressed: self.double, drag_started: self.started }
    }
}

type HeaderHookFn<'a> = Box<dyn FnMut(&mut Ui, &mut BodyCx<'_>) + 'a>;

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
    /// The collapse arrow or a double click on the grip asked to minimize or expand the group.
    pub toggle_minimized: bool,
    /// A tab was dragged out of the dock's windows.
    pub torn_off: Option<TornOff<PanelKey>>,
    /// A drag of the whole group started on the grip.
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
}

/// A group of panels: a header of tab strip and panel menu button, dragged by its empty part, over the active panel's
/// body. Show it with `show_at(group_rect)` in the body of a `SplitArea`, with an `IdSource::Id` made from the
/// group's anchor so its state survives a re-dock. Under a `dock` id it registers the spots a drop lands on: the
/// strip, and (while expanded) the sides and the middle of the body. The group is an instance of the `dockGroup`
/// command scope: it publishes the panel command states it is given and reports the commands that reach it.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container, probe(DockGroup<'static, usize>))]
pub struct DockGroup<'a, P: Clone + Hash + Send + Sync + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    anchor: P,
    panels: &'a [PanelTitle],
    active: PanelKey,
    /// Only the header shows; the body and its drop spots are left out.
    #[prop(flag)]
    minimized: bool,
    /// The grip drags the group.
    #[prop]
    movable: bool,
    /// Tabs of panels that are closable show a close button and close on a middle click.
    #[prop(flag)]
    closable: bool,
    /// The dock whose drop registry the group's spots go to.
    #[prop]
    dock: Option<Id>,
    /// How the body's side zones are sized.
    #[prop]
    zones: ZoneRule,
    /// The rows of the group menu: the panel menu button and a context request on a tab open them.
    #[prop]
    menu: Vec<PanelRow>,
    /// Whether the panel menu button shows beside the tabs; a context request on a tab opens the menu either way.
    #[prop]
    panel_menu_shown: bool,
    /// The state of each panel command for the shown panel, published in the group's scope every pass.
    #[prop]
    command_states: Vec<(DockCommands, CommandState)>,
    /// The host's items before the tab strip.
    #[prop(func = "FnMut(&mut egui::Ui, &mut BodyCx<'_>)")]
    extra: Option<HeaderHook<'a>>,
    /// The size of the group this pass, for the drag ghost.
    size: Vec2,
    reported: Reported,
}

impl<'a, P: Clone + Hash + Send + Sync + 'static> DockGroup<'a, P> {
    /// The group named by `anchor` (one of its panels) with its `panels`, `active` showing. The group is named by the
    /// active panel's title.
    pub fn new(anchor: P, panels: &'a [PanelTitle], active: PanelKey) -> Self {
        let props = ContainerProps::new().sizing(Sizing::Fill).header(HeaderRegion::new().trailing(true)).command_scope(DockCommands::SCOPE);
        let title = panels.iter().find(|panel| panel.key == active).map(|panel| panel.title.clone()).unwrap_or_default();
        DockGroup {
            common: Common::new(),
            props,
            anchor,
            panels,
            active,
            minimized: false,
            movable: true,
            closable: false,
            dock: None,
            zones: ZoneRule::Fraction,
            menu: Vec::new(),
            panel_menu_shown: true,
            command_states: Vec::new(),
            extra: None,
            size: Vec2::ZERO,
            reported: Reported::default(),
        }
        .label(title)
    }
}

impl<P: Clone + Hash + Send + Sync + 'static> Component for DockGroup<'_, P> {
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

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let strip = Rect::from_min_size(cx.rect.min, Vec2::new(cx.rect.width(), cx.style.header));
        // Painted here, not as the container's surface, which lies over what `paint` draws.
        ui.draw().surface(cx.rect, SurfaceKind::Panel, Corners::All);
        ui.draw().surface(strip, SurfaceKind::Background, Corners::None);
    }

    /// The group's frame, over the tabs and the body so the strip's tabs meet it.
    fn paint_over(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        ui.draw().surface(cx.rect, SurfaceKind::Frame, Corners::None);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        self.size = cx.rect.size();
        let Some(dock) = self.dock else { return };
        let (rect, rtl, header) = (cx.rect, cx.flow.is_rtl(), cx.style.header);
        let strip = Rect::from_min_size(rect.min, Vec2::new(rect.width(), header));
        DockTargets::<P>::register(&cx.core.ctx, dock, DockSpot::tab(self.anchor.clone(), self.panels.len()), strip);
        if self.minimized {
            return;
        }
        let body = Rect::from_min_max(Pos2::new(rect.left(), strip.bottom()), rect.max);
        for (side, zone) in self.zones.zones(body, &cx.style.sizes, rtl) {
            DockTargets::<P>::register_in(&cx.core.ctx, dock, DockSpot::group(self.anchor.clone(), side), zone, rect);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> DockGroupOutput {
        let Reported { picked, closed, menu, item, commands, toggle_minimized, torn_off, drag_started } = self.reported;
        DockGroupOutput { response: cx.response.clone(), picked, closed, menu, item, commands, toggle_minimized, torn_off, drag_started }
    }
}

impl<P: Clone + Hash + Send + Sync + 'static> Container for DockGroup<'_, P> {
    fn header(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        for (command, state) in self.command_states.drain(..) {
            cx.publish(command, state);
        }
        while let Some((command, _)) = cx.take_command::<DockCommands>() {
            self.reported.commands.push(command);
        }
        let words = cx.words::<DockWords>();
        let from = PanelKey::of(&self.anchor);
        let started = drop::started(&cx.env);
        let (panels, active, closable) = (self.panels, self.active, self.closable);
        let menu_rows = self.menu.clone();
        let panel_menu = words.panel_menu.clone();
        let mut extra = self.extra.take();
        let name = Label::from(words.panels.as_str().to_owned());
        let (dock, anchor, rtl) = (self.dock, self.anchor.clone(), crate::runtime::env::Env::of(ui).direction.is_rtl());
        ui.set_min_height(crate::runtime::env::Env::of(ui).tokens.size.panel_header);
        let surface_pressed = cx.surface_pressed();
        let (size, movable, move_group) = (self.size, self.movable, words.move_group.fill(&[]));
        let reported = &mut self.reported;
        Row::new().show(ui, |ui, _| {
            if let Some(hook) = extra.as_mut() {
                hook.run(ui, cx);
            }

            let source = keyed(panels, |panel: &PanelTitle| panel.key).with_text(|panel| Cow::Borrowed(panel.title.as_str()));
            let closes = |panel: &PanelTitle| closable && panel.closable;
            let kind = if panels.iter().any(closes) { TabKind::Document } else { TabKind::Panel };
            let strip = TabStrip::new(name.clone())
                .kind(kind)
                .shown(active)
                .tear_off()
                .natural_height(true)
                .accept_from(|_| false)
                .drag_payload(move |drag: &ListDrag<PanelKey>| Some(PanelDrag { panel: drag.grabbed, from, started }))
                .show_items(ui, &source, &mut |key, _| {
                    let panel = panels.iter().find(|panel| &panel.key == key);
                    let title = panel.map_or_else(Label::default, |panel| panel.title.clone());
                    Tab::new(title).closable(panel.is_some_and(closes))
                });
            let strip_id = strip.response.response.id;
            // A press on the group's empty body focuses its panel, as in Photoshop.
            if surface_pressed {
                focus::request(ui.ctx(), FocusTarget::Id(strip_id));
            }
            for event in strip.response.events {
                if let crate::contract::CollectionEvent::Selection(craft_behavior::SelectionChange::Set(keys)) = event {
                    reported.picked = keys.last().copied().or(reported.picked);
                }
            }
            if let Some(dock) = dock {
                register_insertions(ui.ctx(), dock, &anchor, panels, &strip.tab_rects, rtl);
            }
            reported.picked = strip.picked.or(reported.picked);
            reported.closed.extend(strip.closed);
            reported.menu = strip.menu.or(reported.menu.take());
            reported.torn_off = strip.torn_off.or(reported.torn_off.take());
            // The strip's empty room is the handle the group is dragged by; a double click on it minimizes the group.
            if strip.empty.width() > 0.0 {
                let unit = DragUnit::Group(anchor.clone());
                let grip = GroupGrip::new(unit, active).ghost((size, cx.label())).movable(movable).label(move_group.clone()).show_at(ui, strip.empty);
                reported.toggle_minimized |= grip.double_pressed;
                reported.drag_started |= grip.drag_started;
            }
            if !menu_rows.is_empty() {
                let rows = menu_rows.clone();
                let shown = Menu::context(panel_menu.clone(), &strip.response.response)
                    .show(ui, |ui, _| PanelMenu::new(Vec::new(), rows.clone()).show(ui).picked);
                reported.item = shown.inner.flatten().or(reported.item);
            }
        });
        self.extra = extra;
    }

    /// The end of the header: the bar the group is dragged by, then the panel menu button at the end edge. The tabs take the room the
    /// two leave.
    fn controls(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let words = cx.words::<DockWords>();
        let unit = DragUnit::Group(self.anchor.clone());
        let grip = GroupGrip::new(unit, self.active).ghost((self.size, cx.label())).movable(self.movable).label(words.move_group.fill(&[])).show(ui);
        self.reported.toggle_minimized |= grip.double_pressed;
        self.reported.drag_started |= grip.drag_started;
        if !self.menu.is_empty() && self.panel_menu_shown {
            let name = words.panel_menu.clone();
            // The button sits at the strip's top, level with the tab labels' caps, as the "…" of the old tab strips did.
            let layout = egui::Layout::from_main_dir_and_cross_align(ui.layout().main_dir(), egui::Align::Min);
            let button = ui
                .with_layout(layout, |ui| {
                    IconButton::new(IconId::new("more")).size(ControlSize::Sm).popup(PopupKind::Menu).popup_mark(false).label(name.clone()).show(ui)
                })
                .inner;
            let rows = self.menu.clone();
            let shown = Menu::new(name).opened_by(&button, Trigger::Click).show(ui, |ui, _| PanelMenu::new(Vec::new(), rows.clone()).show(ui).picked);
            self.reported.item = shown.inner.flatten().or(self.reported.item);
        }
    }
}

/// Registers the tab insertion spots of a group: the half of each shown tab nearer the start edge inserts before
/// it, the other half after it, and each spot's mark is the line at the boundary it inserts at.
fn register_insertions<P: Clone + Hash + Send + Sync + 'static>(
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

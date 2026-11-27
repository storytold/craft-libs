//! The ribbon: a tab strip over a scrolling row of command groups.

pub(crate) mod gallery;
mod group;
mod plan;
#[cfg(feature = "suite")]
pub(crate) mod stories;

use std::borrow::Cow;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;

use craft_behavior::{CanvasTransform, EditEvent, IconId, Metrics, Mixed, SelectionChange, Sense, keyed};
use craft_keymap::CommandRef;
use craft_ui::buttons::IconButton;
use craft_ui::contract::overlay::Anchor;
use craft_ui::contract::{
    Access, Bind, BodyCx, CollectionEvent, Common, Component, Container, ContainerProps, Cx, NoValue, Outcome, Props as _, Role, Scroll, Show as _,
    ShowCollection as _, ShowContainer as _, Sizing, Trigger, clicked, scroll_step,
};
use craft_ui::layout_components::Row;
use craft_ui::menu::Menu;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::inspect;
use craft_ui::runtime::memory::with_session;
use craft_ui::runtime::motion::{Animate as _, spec};
use craft_ui::style::{ComponentStyle, StyleCx};
use craft_ui::tabs::{StripEnd, Tab, TabKind, TabRole, TabStrip, TabStripOutput};
use egui::{Id, Pos2, Rect, Response, Ui, Vec2, vec2};

pub use craft_ui::buttons::{GroupLevel, RibbonLook};
pub use gallery::{Gallery, GalleryFace, GalleryItem, GalleryMore, GalleryOutput};
pub use group::RibbonGroup;
pub use plan::{GroupWidths, plan};

type ContextMenuBody<'a> = &'a mut dyn FnMut(&mut Ui, &RibbonMenuRequest, &mut BodyCx<'_>);

craft_ui::words! {
    /// The words of the ribbon's own controls.
    struct RibbonWords {
        scroll_left: "ribbon.scrollLeft" = "Scroll left";
        scroll_right: "ribbon.scrollRight" = "Scroll right";
        context_menu: "ribbon.contextMenu" = "Ribbon menu";
    }
}

/// The ribbon: a [`TabKind::Ribbon`](craft_ui::tabs::TabKind) strip over a scrolling group row,
/// with minimise and a context menu. The strip and the row show inside one inspected region
/// (`inspect::record`), whose record ([`RibbonOutput::controls`]) gives each control's rect and
/// command, and [`RibbonOutput::tab_ids`] says which recorded element is which tab. On a tab change
/// the group row slides in from the side on which the new tab lies relative to the old one
/// (mirrored in right-to-left), through one `Tween`; with reduce motion the new content shows at
/// rest.
///
/// A context request (a secondary click, the Menu key or Shift+F10) on a tab, a control or the row's
/// background opens the ribbon's own menu, whose rows the app's [`context_menu`](Ribbon::context_menu)
/// body shows for that request. The ribbon holds the request while the menu is open, and focus goes back
/// where it was when the menu closes by key.
#[must_use = "show the ribbon with `show`"]
#[derive(craft_ui::contract::Pattern)]
#[pattern(probe(Ribbon<'static, usize>))]
pub struct Ribbon<'a, T: Clone + Eq + Hash + Send + Sync + Debug + 'static> {
    label: Label,
    tabs: &'a [RibbonTab<T>],
    shown: Bind<'a, T>,
    trailing: &'a [StripEnd<'a>],
    minimised: Option<Bind<'a, bool>>,
    /// The body of the context menu: the rows for the request that opened it.
    context_menu: Option<ContextMenuBody<'a>>,
    /// The icon sides of the groups' buttons.
    icons: RibbonLook,
    /// The icon of every group's dialog launcher.
    launcher: IconId,
}

impl<'a, T: Clone + Eq + Hash + Send + Sync + Debug + 'static> Ribbon<'a, T> {
    /// A ribbon named `label` over `tabs`, showing the tab `shown` binds.
    pub fn new(label: impl Into<Label>, tabs: &'a [RibbonTab<T>], shown: Bind<'a, T>) -> Self {
        Ribbon {
            label: label.into(),
            tabs,
            shown,
            trailing: &[],
            minimised: None,
            context_menu: None,
            icons: RibbonLook::default(),
            launcher: IconId::new("maximize_2"),
        }
    }

    /// The rows of the ribbon's context menu. The ribbon opens its menu on a context request and runs `body` inside
    /// it on every pass the menu is open, with the request that opened it: the command of the control it was on, or
    /// none on a tab or the row's background. Without a body a context request opens nothing.
    pub fn context_menu(mut self, body: &'a mut dyn FnMut(&mut Ui, &RibbonMenuRequest, &mut BodyCx<'_>)) -> Self {
        self.context_menu = Some(body);
        self
    }

    /// Items at the reading end of the tab strip, after the tabs.
    pub fn trailing(mut self, items: &'a [StripEnd<'a>]) -> Self {
        self.trailing = items;
        self
    }

    /// Whether the group row is minimised, bound to the app's state. A double click on a tab toggles it through a
    /// write-through binding.
    pub fn minimised(mut self, minimised: Bind<'a, bool>) -> Self {
        self.minimised = Some(minimised);
        self
    }

    /// The key of the tab shown: the binding's, else the first tab's.
    /// How the groups' controls look (`RibbonLook`): an app whose ribbon icons, menu arrows or small rows differ
    /// from its other controls sets them here. The framework's look by default.
    pub fn look(mut self, look: RibbonLook) -> Self {
        self.icons = look;
        self
    }

    /// The icon of every group's dialog launcher: an app's own glyph in place of the framework's.
    pub fn launcher_icon(mut self, icon: IconId) -> Self {
        self.launcher = icon;
        self
    }

    fn current(&mut self) -> Option<T> {
        let bound = self.shown.reborrow().get().and_then(|value| value.same().cloned());
        bound.or_else(|| self.tabs.first().map(|tab| tab.key.clone()))
    }

    /// Shows the strip and the group row of the shown tab; `body` fills the row for that tab.
    ///
    /// The tab a click picks comes back in [`RibbonOutput::shown`]; the app applies it to its own state. A request
    /// the pass's input made (a context request, a double click) is taken in the pass that received that input: the
    /// framework records it in that pass's real run only, so a second pass of the frame takes none. The context menu
    /// it opened stays open on later passes until it closes.
    pub fn show(mut self, ui: &mut Ui, mut body: impl FnMut(&mut RibbonRow<'_>, &T)) -> RibbonOutput<T> {
        let current = self.current();
        let minimised = self.minimised.as_mut().and_then(|bind| bind.reborrow().get()).and_then(|value| value.same().copied()).unwrap_or(false);
        let id = ui.scope_id().with(self.label.as_str());
        let tabs = self.tabs;
        let trailing = self.trailing;
        let icons = self.icons;
        let launcher = self.launcher.clone();
        let strip_label = self.label.clone();
        let label_of = |key: &T| tabs.iter().find(|tab| tab.key == *key).map_or(Label::Empty, |tab| tab.label.clone());
        let role_of = |key: &T| tabs.iter().find(|tab| tab.key == *key).map_or(TabRole::Standard, |tab| tab.role);

        let ((strip, region), controls) = inspect::record(ui, id, |ui| {
            let strip: Option<TabStripOutput<T>> = (!tabs.is_empty()).then(|| {
                let keys: Vec<T> = tabs.iter().map(|tab| tab.key.clone()).collect();
                let source = keyed(&keys, |key: &T| key.clone()).with_text(|key: &T| Cow::Owned(label_of(key).as_str().to_owned()));
                let strip: TabStrip<'_, T> = TabStrip::new(strip_label).kind(TabKind::Ribbon).roles(role_of);
                // PLAN: `TabStrip::trailing(&[StripEnd])` is rb.R3 (ribbon-w1) and is not in this tree yet; this call compiles once it lands.
                let mut strip = strip.trailing(trailing);
                if let Some(key) = current.clone() {
                    strip = strip.shown(key);
                }
                // The look reaches the strip too: its tabs' ink and underline (`RibbonLook::tabs`).
                craft_ui::runtime::env::with_env(
                    ui,
                    |overrides| {
                        overrides.values.set(icons);
                    },
                    |ui| strip.show_items(ui, &source, &mut |key: &T, _| Tab::new(label_of(key)).role(role_of(key))),
                )
            });
            let region = match (&current, minimised) {
                (Some(key), false) => Some(craft_ui::runtime::env::with_env(
                    ui,
                    |overrides| {
                        overrides.values.set(icons);
                    },
                    |ui| show_row(ui, id, tabs, key, (icons, &launcher), &mut body),
                )),
                _ => None,
            };
            (strip, region)
        });

        let mut chosen: Option<T> = None;
        let mut toggle = false;
        let mut picked: Option<T> = None;
        let mut asked: Option<(RibbonMenuRequest, Asker)> = None;
        let mut strip_response: Option<Response> = None;
        let mut tab_ids: Vec<(T, Id)> = Vec::new();
        if let Some(strip) = strip {
            for event in &strip.response.events {
                match event {
                    CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } => chosen = keys.last().cloned().or(chosen),
                    CollectionEvent::Activate(_) if strip.response.activation_clicks == 2 => toggle = true,
                    _ => {}
                }
            }
            // Each tab the strip shows reports its access node in its row; a tab in the overflow menu has no row.
            tab_ids = strip.response.shown.iter().map(|row| (row.key.clone(), row.id)).collect();
            picked = strip.picked;
            asked = strip.menu.map(|request| (RibbonMenuRequest { command: None, at: request.at }, Asker::Strip));
            strip_response = Some(strip.response.response.clone());
        }
        if toggle && let Some(bind) = self.minimised.as_mut() {
            let _ = bind.reborrow().apply(&EditEvent::set(Mixed::Same(minimised), !minimised, Arc::from("ribbon.minimise"), None));
        }
        if let Some(request) = region.as_ref().and_then(|region| row_request(region, controls.as_deref())) {
            asked = Some((request, Asker::Row));
        }
        if let Some(body) = self.context_menu.take() {
            let openers = Openers { strip: strip_response.as_ref(), row: region.as_ref().map(|region| &region.response) };
            context_menu(ui, id, asked, openers, body);
        }
        let tabs_rect = strip_response.as_ref().map(|response| response.rect);
        RibbonOutput { shown: chosen.or(current), picked, tab_ids, controls, tabs: tabs_rect }
    }
}

/// Which of the ribbon's regions asked for its context menu: the menu hangs from that region's response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Asker {
    /// The tab strip, for a tab.
    Strip,
    /// The group row, for a control or its background.
    Row,
}

/// The responses of this pass the ribbon's context menu can hang from; `None` for a region not shown.
struct Openers<'r> {
    strip: Option<&'r Response>,
    row: Option<&'r Response>,
}

/// The context menu the ribbon holds open: the request that opened it and the region that asked, and the frame the
/// ribbon last showed in. It is the menu's open-popup state, kept in the ribbon's own slot.
#[derive(Clone, Default)]
struct HeldMenu {
    menu: Option<(RibbonMenuRequest, Asker)>,
    frame: u64,
}

/// Shows the ribbon `id`'s context menu while it is open: `asked`, this pass's request, opens it (or moves it to the
/// new request), and `body` shows its rows. The menu hangs from this pass's response of the region that asked, at the
/// request's point; the held request goes when the menu closes or that region is not shown.
fn context_menu(
    ui: &mut Ui,
    id: Id,
    asked: Option<(RibbonMenuRequest, Asker)>,
    openers: Openers<'_>,
    body: &mut dyn FnMut(&mut Ui, &RibbonMenuRequest, &mut BodyCx<'_>),
) {
    let slot = id.with("context_menu");
    let frame = ui.ctx().cumulative_frame_nr();
    let held = with_session(ui.ctx(), slot, |held: &mut HeldMenu| {
        // A frame without the ribbon closed its menu with it: the request must not open the menu again when the
        // ribbon shows again.
        if held.frame.saturating_add(1) < frame {
            held.menu = None;
        }
        if asked.is_some() {
            held.menu = asked;
        }
        held.frame = frame;
        held.menu.clone()
    });
    let Some((request, asker)) = held else { return };
    let opener = match asker {
        Asker::Strip => openers.strip,
        Asker::Row => openers.row,
    };
    let mut open = opener.is_some();
    if let Some(opener) = opener {
        let words = Env::of(ui).words::<RibbonWords>();
        let shown = Menu::new(words.context_menu.clone())
            .opened_by(opener, Trigger::Manual)
            .anchor(Anchor::Point(request.at))
            .open(Bind::bound(&mut open))
            .show(ui, |ui, cx| body(ui, &request, cx));
        if shown.output.closed.is_some() {
            open = false;
        }
    }
    if !open {
        with_session(ui.ctx(), slot, |held: &mut HeldMenu| held.menu = None);
    }
}

/// Shows the group row of the tab `key` in the region that senses its context requests. The row slides in from the
/// side the tab lies on relative to the tab shown before, and a row wider than the room scrolls, with an edge chevron
/// on each side that has groups out of view.
fn show_row<T: Clone + Eq + Hash + Send + Sync + Debug + 'static>(
    ui: &mut Ui,
    base: Id,
    tabs: &[RibbonTab<T>],
    key: &T,
    (icons, launcher): (RibbonLook, &IconId),
    body: &mut impl FnMut(&mut RibbonRow<'_>, &T),
) -> RegionOutput {
    let place = tabs.iter().position(|tab| tab.key == *key).unwrap_or(0) as f32;
    Region::new()
        .show(ui, |ui, cx| {
            // The tab's place is the tween: it starts at the previous tab's place, which the motion's memory holds, and
            // the gap to the target is how far the row still has to travel, on the side the new tab lies on. Under
            // reduce motion the tween is at its target at once, so the row shows at rest.
            let gap = place - cx.anim("place", place, spec::MOVE);
            let side = if cx.env.direction().is_rtl() { -1.0 } else { 1.0 };
            let distance = cx.style::<Region>().map_or(0.0, |style| style.slide);
            let lead = gap.clamp(-1.0, 1.0) * side * distance;
            let chevron = cx.style::<Region>().map_or(0.0, |style| style.chevron);
            // Rows a look sets stand a tight gap apart: three of them are the body's least height.
            let least = match icons.row {
                0 => cx.style::<Region>().map_or(0.0, |style| style.body),
                row => 3.0 * f32::from(row) + 2.0 * craft_ui::contract::Gap::Tight.points(ui),
            };
            let title_line = cx.style::<Region>().map_or(0.0, |style| style.title);
            let start = cx.style::<Region>().map_or(0.0, |style| style.start);
            // The row moves through the slide container's own offset, which places its body on a layer of its own.
            let shown = Region::slide(lead)
                .show(ui, |ui, _| {
                    // The row's line is as high as its tallest group, at least the body's least height over a title line:
                    // every group fills the line, so the titles at the groups' feet share one line.
                    let row = Row::new().scroll(Scroll::horizontal()).fill_width().least_height(least + title_line).show(ui, |ui, _| {
                        // The groups start a regular pad in from the row's start edge, as the old ribbon's did.
                        craft_ui::layout::place_in_flow(ui, Vec2::new(start, 0.0));
                        let mut row = RibbonRow { ui, base, groups: Vec::new(), title_line, icons, launcher: launcher.clone() };
                        body(&mut row, key);
                        row.groups
                    });
                    (row.output.id, row.output.rect, row.inner.unwrap_or_default())
                })
                .inner;
            if let Some((owner, view, groups)) = shown {
                chevrons(ui, owner, view, &groups, chevron, lead);
            }
        })
        .output
}

/// The edge chevrons of a row `view` whose `groups` do not all show: one at each edge that has a group past it, each
/// stepping the scrolling body `owner` by one group toward its side (`contract::scroll_step`). `view` and `groups` are
/// in the points of the row's layer, which the slide offsets by `lead`; the chevrons sit at the edges as they show.
/// `width` is a chevron's width.
fn chevrons(ui: &mut Ui, owner: Id, view: Rect, groups: &[Rect], width: f32, lead: f32) {
    // Half a point past the edge counts as inside: less than a pixel shows at any scale.
    let left = groups.iter().any(|group| group.min.x < view.min.x - 0.5);
    let right = groups.iter().any(|group| group.max.x > view.max.x + 0.5);
    let words = Env::of(ui).words::<RibbonWords>();
    let edges = view.translate(vec2(lead, 0.0));
    let size = Vec2::new(width, view.height());
    for (on, right_side, icon, name) in
        [(left, false, "chevron_left", words.scroll_left.clone()), (right, true, "chevron_right", words.scroll_right.clone())]
    {
        if !on {
            continue;
        }
        let at =
            if right_side { Rect::from_min_size(Pos2::new(edges.max.x - width, edges.min.y), size) } else { Rect::from_min_size(edges.min, size) };
        let button = IconButton::new(IconId::new(icon)).label(name).focusable(false).show_at(ui, at);
        if clicked(&button.response) {
            scroll_step(ui.ctx(), owner, view, groups, right_side);
        }
    }
}

/// The context request of the group row, from the framework's requests of this pass: the deepest control of the
/// ribbon's record with a bound command that asked (a secondary click on it, or the Menu key or Shift+F10 while it
/// held focus), else the region itself when its background asked. A request on a tab is the strip's.
fn row_request(region: &RegionOutput, recorded: Option<&inspect::Recorded>) -> Option<RibbonMenuRequest> {
    let control = recorded.and_then(|recorded| {
        region
            .requests
            .iter()
            .filter_map(|(asked, rect)| {
                recorded
                    .elements
                    .iter()
                    .find(|element| element.id == *asked && element.part.is_none() && element.command.is_some())
                    .map(|element| (element, *rect))
            })
            .max_by_key(|(element, _)| element.depth)
    });
    match control {
        Some((element, rect)) => Some(RibbonMenuRequest { command: element.command.clone(), at: rect.left_bottom() }),
        None => region.own.map(|rect| RibbonMenuRequest { command: None, at: rect.left_bottom() }),
    }
}

/// One tab of the ribbon.
pub struct RibbonTab<T> {
    key: T,
    label: Label,
    role: TabRole,
}

impl<T> RibbonTab<T> {
    /// A plain tab named `label` that `key` identifies.
    pub fn new(key: T, label: impl Into<Label>) -> Self {
        RibbonTab { key, label: label.into(), role: TabRole::Standard }
    }

    /// Makes the tab a contextual one: it shows only while its context is on.
    pub fn contextual(mut self) -> Self {
        self.role = TabRole::Contextual;
        self
    }
}

/// The row of groups the ribbon's body fills for the shown tab.
pub struct RibbonRow<'a> {
    ui: &'a mut Ui,
    /// The ribbon's id: each group's id is made under it, so it stays the same in every sweep of the row.
    base: Id,
    /// The rect of each group this run of the row showed, in order: the units the edge chevrons step by.
    groups: Vec<Rect>,
    /// The height of every group's title line, the launcher's side: one line of the caption role.
    title_line: f32,
    /// The icon sides of the groups' buttons (`Ribbon::icons`).
    icons: RibbonLook,
    /// The icon of every group's dialog launcher (`Ribbon::launcher_icon`).
    launcher: IconId,
}

/// What the ribbon reports for a pass.
pub struct RibbonOutput<T> {
    /// The tab shown after this pass.
    pub shown: Option<T>,
    /// The tab the user picked this pass.
    pub picked: Option<T>,
    /// Each tab shown this pass with the id of its access node, which its element in
    /// [`controls`](RibbonOutput::controls) stands for (`Element::node`; a keytip's place); a tab in the overflow menu
    /// has none.
    pub tab_ids: Vec<(T, Id)>,
    /// The inspected record of the strip and the row: each control's rect and command.
    pub controls: Option<Arc<inspect::Recorded>>,
    /// The tab row's rect this pass; `None` while no strip shows.
    pub tabs: Option<Rect>,
}

/// A request for the ribbon's context menu, on a control, a tab or the row's background.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RibbonMenuRequest {
    /// The command of the control the request is for, when it has one; none on a tab or the row's background.
    pub command: Option<CommandRef>,
    /// Where the menu opens.
    pub at: Pos2,
}

impl RibbonMenuRequest {
    /// A request for the menu of the control of `command`, if any, opening at `at`.
    pub const fn new(command: Option<CommandRef>, at: Pos2) -> Self {
        RibbonMenuRequest { command, at }
    }
}

/// The style of the row's regions: the slide distance and the edge chevrons' width.
/// How opaque the raised surface lies over the chrome in a looks body fill: 60 %, the old ribbons band (249 over the
/// 240 bars of the light theme).
const BODY_LIFT: u8 = 153;

#[derive(Clone, Debug, PartialEq, ComponentStyle)]
struct RegionStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// How far the row travels as a tab change slides it in (`motion.slide`).
    slide: f32,
    /// The width of an edge chevron: a hit target's.
    chevron: f32,
    /// The least height of the group row's body, under which each group's title sits: three controls stacked with the
    /// row gap between them, the ribbon's three rows of small buttons. Taller content lowers every title with it.
    body: f32,
    /// The height of a group's title line, directly under its body: one line of the caption role
    /// (`typography.caption.line_height`), which the dialog launcher's square also takes.
    title: f32,
    /// The room before the first group (`space.pad.regular`), as the old ribbon kept it.
    start: f32,
    /// The body's own fill where the look asks for one (`RibbonLook::body`): the raised surface laid thinly
    /// over the chrome (`BODY_LIFT`), a band lighter than the bars around it.
    fill: Option<craft_ui::style::StyleColor>,
}

/// What a row region reports.
struct RegionOutput {
    response: Response,
    /// The region's own context request of this pass (a secondary click on its background), at this rect.
    own: Option<Rect>,
    /// Every context request of this pass made by the time the region finished, those of its controls among them.
    requests: Vec<(Id, Rect)>,
}

impl Outcome for RegionOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    fn unshown(response: Response) -> Option<Self> {
        Some(RegionOutput { response, own: None, requests: Vec::new() })
    }
}

/// A region of the group row: the outer one takes the ribbon's width, senses secondary presses on its surface and
/// reports the context requests of its controls (`Cx::context_requests`); the inner one (`slide`) shows the row
/// through its offset while a tab change slides it in.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
#[component(container, unregistered)]
struct Region {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// Whether the region senses context presses on its surface.
    senses: bool,
}

impl Region {
    /// The outer region.
    fn new() -> Self {
        Region { common: Common::new(), props: ContainerProps::new().sizing(Sizing::FillWidth), senses: true }
    }

    /// The inner region, its body offset by `lead` along x.
    fn slide(lead: f32) -> Self {
        let props = ContainerProps::new().sizing(Sizing::FillWidth).transform(CanvasTransform::new(1.0, vec2(lead, 0.0)));
        Region { common: Common::new(), props, senses: false }
    }
}

impl Component for Region {
    const ROLE: Role = Role::GROUP;
    const ALT_ROLES: &'static [Role] = &[Role::None];
    type Style = RegionStyle;
    type Output = RegionOutput;

    fn style(cx: &StyleCx<'_>, _: RegionStyleKey) -> RegionStyle {
        let body = 3.0 * cx.control().height + 2.0 * cx.tokens.space.gap.row;
        let title = cx.tokens.typography.caption.line_height;
        RegionStyle {
            metrics: Metrics::new(),
            slide: cx.tokens.motion.slide as f32,
            chevron: cx.tokens.size.hit.target,
            body,
            title,
            start: cx.tokens.space.pad.regular,
            fill: cx.env::<RibbonLook>().body.then(|| cx.color(cx.tokens.color.surface.raised.with_alpha(BODY_LIFT))),
        }
    }

    fn sense(&self) -> Sense {
        if self.senses { Sense::SURFACE | Sense::CONTEXT_CLICK } else { Sense::empty() }
    }

    /// The outer region paints the body's fill and its top and bottom rules where the look asks for them.
    fn paint(&mut self, ui: &mut craft_ui::contract::PaintUi<'_>, cx: &Cx<'_, Self>) {
        let Some(fill) = cx.style.fill.filter(|_| self.senses) else { return };
        let draw = ui.draw();
        draw.fill(&craft_ui::primitives::PaintShape::Rect(cx.rect, egui::CornerRadius::ZERO), fill);
        for side in [craft_ui::primitives::RectSide::Top, craft_ui::primitives::RectSide::Bottom] {
            draw.edge_rule(craft_ui::style::RuleKind::Divider, cx.rect, side);
        }
    }

    /// The region only lays out the groups: it is no node of its own.
    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        access.role(Role::None);
    }

    fn finish(self, cx: Cx<'_, Self>) -> RegionOutput {
        let requests = if self.senses { cx.context_requests() } else { Vec::new() };
        let own = requests.iter().find(|(asked, _)| *asked == cx.id).map(|(_, rect)| *rect);
        RegionOutput { response: cx.response.clone(), own, requests }
    }
}

impl Container for Region {}

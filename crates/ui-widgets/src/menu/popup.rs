//! `Menu` (11.4, 15.4): the container of menu rows, on a menu layer, inline, or as a submenu.

use super::item::{MenuItem, MenuItemOutput};
use super::level::{LevelMemory, MenuLevel, level_env};
use crate::contract::overlay::{Anchor, Dismiss, Outside, Popup};
use crate::contract::{
    Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, Entry, Focus as _, Gap, InMenu, LeastWidth, Mount, NoValue, Outcome,
    Overlay, Pad, Props as _, Remember as _, Role, Show as _, ShowContainer as _, TooltipSpec, Trigger,
};
use crate::display::Divider;
use crate::runtime::env::{Env, Label};
use crate::runtime::focus::{FocusPolicy, Initial};
use crate::runtime::frame;
use crate::runtime::overlay::CloseReason;
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::{Arrows, Metrics, NavShape, Roving, Sense, Value};
use egui::{Response, Ui};

/// The style of a menu: the surface comes from the container.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct MenuStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The padding of the popup around the rows.
    pad: f32,
}

/// The neighbouring menu a top-level menu asks its bar for when Left or Right reaches it unhandled. The directions
/// are the keys', not reading order: `Prev` is the Left key, which a right-to-left bar reads as the next title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MenuStep {
    /// The Left key.
    Prev,
    /// The Right key.
    Next,
}

/// What a menu returns: its response and how it closed in this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct MenuOutput {
    /// The menu's response.
    pub response: Response,
    /// How the menu closed in this pass; `None` while it stays open. Reported once for each close.
    pub closed: Option<CloseReason>,
    /// The Left or Right a top-level menu left unhandled this pass.
    pub step: Option<MenuStep>,
}

impl Outcome for MenuOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What [`Menu::pick`] returns: the response, how the menu closed and the value of the entry that ran.
#[derive(Debug)]
#[non_exhaustive]
pub struct MenuPick<T> {
    /// The menu's response.
    pub response: Response,
    /// How the menu closed in this pass.
    pub closed: Option<CloseReason>,
    /// The value of the entry that ran this pass.
    pub picked: Option<T>,
}

impl<T: Value> Outcome for MenuPick<T> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The layer of a menu, or of a submenu: a press outside closes it and reaches nothing below, and running a row
/// closes the chain.
#[doc(hidden)]
pub fn layer(submenu: bool) -> Overlay {
    let base = if submenu { Overlay::submenu() } else { Overlay::menu() };
    base.dismiss(Dismiss::new().outside(Outside::Absorb).on_activate(true)).focus(FocusPolicy::MENU.initial(Initial::Default))
}

/// Shows one row per entry, a rule for a separator, and returns the index of the entry that ran. In a value menu
/// ([`Menu::values`]) every row is a radio item and the checked entry carries the mark; elsewhere a checked entry
/// carries a check mark.
#[doc(hidden)]
pub fn entry_rows<T>(ui: &mut Ui, entries: &[Entry<T>]) -> Option<usize> {
    let values = Env::of(ui).get::<MenuLevel>().values();
    let mut ran = None;
    for (at, entry) in entries.iter().enumerate() {
        if entry.is_separator() {
            Divider::horizontal().show(ui);
            continue;
        }
        let mut row = MenuItem::new(entry.label().clone()).enabled(entry.is_enabled()).entry_checked(entry.is_checked());
        if let Some(icon) = entry.icon_spec() {
            row = row.icon(icon.clone());
        }
        if let Some(reason) = entry.why_disabled() {
            row = row.description(reason.as_str()).tooltip(TooltipSpec::new(reason.clone()).disabled_text(reason.clone()));
        }
        // The lead of a value menu starts on the checked entry.
        if values && entry.is_checked() {
            row = row.default_stop();
        }
        if row.show(ui).ran {
            ran = Some(at);
        }
    }
    ran
}

/// The value menu that hangs from `opener` and opens on a press of it: every row is a radio item, the checked entry
/// carries the mark and holds the lead when the menu opens.
pub(crate) fn pick_values_from<T: Value + 'static>(ui: &mut Ui, label: impl Into<Label>, opener: &Response, entries: &[Entry<T>]) -> MenuPick<T> {
    Menu::new(label).values().opened_by(opener, Trigger::Press).pick(ui, entries)
}

/// A menu: a popup of [`MenuItem`]s and [`Divider`]s the arrow keys move through, one roving group. Press, context
/// and submenu menus are this container with a different opener.
///
/// - Open state: the menu's open slot, which the opener's trigger and `Popup::open` write; an optional
///   [`open`](Menu::open) binding takes it over. Escape, a press outside, running a row and the other close rules
///   write `false` through the same state, and the output's `closed` says which one.
/// - Opener: [`opened_by`](Menu::opened_by) names the control it hangs from and the trigger that opens it; focus
///   returns to it on a close by key. A context menu hangs from the pointer ([`context`](Menu::context)), a submenu
///   from its row ([`submenu`](Menu::submenu)), whose `MenuItemOutput::submenu_open` says when it shows.
/// - Every row of the body registers with the menu's level: the level gives each its mnemonic and runs the row a
///   typed letter names, with or without a focused row.
/// - The body is one column group: the rows directly in it share the widest mark and icon columns, so their labels
///   start at one edge.
/// - A menu over value entries is a value menu ([`values`](Menu::values)) and shows with [`pick`](Menu::pick).
///
/// Menu-body helpers (`EditMenu`, `PanelMenu`) show their rows in the body and return an output.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(container)]
pub struct Menu<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// The caller's binding takes the open state over from the menu's slot.
    #[prop]
    open: Option<Bind<'a, bool>>,
    closed: Option<CloseReason>,
    submenu: bool,
    values: bool,
    /// The id of the control the menu hangs from: `Popup` closes the layer through it.
    opener: Option<egui::Id>,
    /// A row of a submenu level took Back.
    back: bool,
    /// The Left or Right key a top-level level left unhandled.
    step: Option<MenuStep>,
}

/// The padding of the popup around the rows.
const POPUP_PAD: Pad = Pad::Regular;

impl<'a> Menu<'a> {
    /// A menu named `label`. Its open state is the menu's slot until [`open`](Menu::open) binds it.
    pub fn new(label: impl Into<Label>) -> Self {
        let props = ContainerProps::new()
            .surface(SurfaceKind::Popup)
            .pad(POPUP_PAD)
            .body_gap(Gap::None)
            .least_width(LeastWidth::Popup)
            .roving(Roving::new(NavShape::Flat).arrows(Arrows::Vertical).wrap(true))
            .label_column(true)
            .mount(Mount::Layer(layer(false)));
        Menu { common: Common::new(), props, open: None, closed: None, submenu: false, values: false, opener: None, back: false, step: None }
            .label(label)
    }

    /// Marks the menu as a value menu: every row is a radio item, whether or not an entry is checked, and the checked
    /// entry holds the lead when the menu opens.
    #[must_use]
    pub fn values(mut self) -> Self {
        self.values = true;
        self
    }

    /// The rows of a menu in the flow of the layer that shows them, with the popup surface and padding of a menu but no
    /// close rules of their own: the menu of a control that owns its popup shows its entries through it.
    pub fn inline(label: impl Into<Label>) -> Self {
        let props = ContainerProps::new()
            .surface(SurfaceKind::Popup)
            .pad(POPUP_PAD)
            .body_gap(Gap::None)
            .least_width(LeastWidth::Popup)
            .roving(Roving::new(NavShape::Flat).arrows(Arrows::Vertical).wrap(true))
            .label_column(true);
        Menu { common: Common::new(), props, open: None, closed: None, submenu: false, values: false, opener: None, back: false, step: None }
            .label(label)
    }

    /// A context menu named `label`, hanging from where the pointer was when it opened, with `opener` getting the
    /// focus back when it closes by key.
    pub fn context(label: impl Into<Label>, opener: &Response) -> Self {
        let mut menu = Menu::new(label);
        menu.opener = Some(opener.id);
        menu.with_layer(|overlay| overlay.opened_by(opener, Trigger::Context))
    }

    /// The submenu of the row `row`, beside it. It closes with the menu the row sits in.
    pub fn submenu(label: impl Into<Label>, row: &Response) -> Self {
        let mut menu = Menu::new(label).with_layer(|_| layer(true).opened_by(row, Trigger::Click));
        menu.opener = Some(row.id);
        menu.submenu = true;
        menu
    }

    /// A control the menu hangs from, and the trigger on it that opens the menu. The menu closes when the opener is
    /// no longer shown. Called again, it names another opener: the menu opens from any of them, hangs from the
    /// one that opened it (a `Trigger::Context` opener a right click opened, at the pointer), and gives the focus
    /// back to the one the focus was on when it opened.
    #[must_use]
    pub fn opened_by(mut self, opener: &Response, trigger: Trigger) -> Self {
        self.opener.get_or_insert(opener.id);
        self.with_layer(|overlay| overlay.opened_by(opener, trigger))
    }

    /// Where the menu hangs when the opener named last opened it, instead of that opener's rect. A rect or point is
    /// in that opener's layer space.
    #[must_use]
    pub fn anchor(self, anchor: Anchor) -> Self {
        self.with_layer(|overlay| overlay.opener_anchor(anchor))
    }

    /// A row that opens a submenu showing the rows of `body`, for use inside a menu body. The menu level holds which
    /// submenu row is open, so resting on a second submenu row closes the first.
    pub fn submenu_row(ui: &mut Ui, label: impl Into<Label>, body: impl FnMut(&mut Ui, &mut BodyCx<'_>)) -> MenuItemOutput {
        Menu::submenu_row_enabled(ui, label, true, body)
    }

    /// [`submenu_row`](Menu::submenu_row) with `enabled`: a row with it false draws disabled and does not open (a
    /// submenu none of whose rows can run).
    pub fn submenu_row_enabled(ui: &mut Ui, label: impl Into<Label>, enabled: bool, body: impl FnMut(&mut Ui, &mut BodyCx<'_>)) -> MenuItemOutput {
        let label = label.into();
        let head = MenuItem::submenu(label.clone()).enabled(enabled).show(ui);
        let mut open = head.submenu_open;
        let mut body = body;
        let shown =
            Menu::submenu(label, &head.response).anchor(Anchor::Rect(head.frame)).open(Bind::bound(&mut open)).show(ui, |ui, cx| body(ui, cx));
        // `MenuItem::update` does not run in a measuring run, so `open` is false there and would clear the level's
        // open submenu the real run set.
        if crate::runtime::measure::effect(ui.ctx(), "menu::note_submenu") {
            head.note_submenu(open && shown.output.closed.is_none(), shown.output.response.rect);
        }
        head
    }

    fn with_layer(mut self, change: impl FnOnce(Overlay) -> Overlay) -> Self {
        let overlay = match std::mem::take(&mut self.props.mount) {
            Mount::Layer(overlay) => overlay,
            _ => layer(false),
        };
        self.props = self.props.mount(Mount::Layer(change(overlay)));
        self
    }

    /// Shows one row per entry (a rule for a separator) and reports the value of the entry that ran. In a value menu
    /// the checked entry carries the radio mark.
    pub fn pick<T: Value + 'static>(self, ui: &mut Ui, entries: &[Entry<T>]) -> MenuPick<T> {
        let shown = self.show(ui, |ui, _| entry_rows(ui, entries));
        let picked = shown.inner.flatten().and_then(|at| entries.get(at)).and_then(Entry::value).cloned();
        MenuPick { response: shown.output.response, closed: shown.output.closed, picked }
    }
}

impl Component for Menu<'_> {
    const ROLE: Role = Role::MENU;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = MenuStyle;
    type Output = MenuOutput;

    fn style(cx: &StyleCx<'_>, _: MenuStyleKey) -> MenuStyle {
        MenuStyle { metrics: Metrics::default(), pad: crate::contract::pad_of(cx.tokens, POPUP_PAD) }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// A row of a submenu level that took Back closes the menu. A typed letter runs the row it names; the container
    /// reads it, so the letter works with no row focused. A level the pointer opened with no active row has no lead:
    /// the first key enters it at an end. The active row is the lead, so the first key after the pointer lit a row
    /// moves from that row; a row lit in this pass leads from the next, so only a key in the same frame as the pointer
    /// move that lit it starts from the row lit before.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let level = MenuLevel::of(cx.memory::<LevelMemory>());
        let keyboard = crate::runtime::focus::keyboard_in_menu(&cx.core.ctx);
        self.props.roving = self.props.roving.map(|roving| roving.no_lead(!keyboard && !level.has_active()));
        if let Some(row) = level.active() {
            cx.roving_lead(row);
        }
        if level.take_close() {
            match self.opener {
                Some(opener) if self.open.is_none() => Popup::close(opener, cx),
                _ => self.back = true,
            }
        }
        if !self.submenu {
            self.step = level.take_step();
        }
        while let Some(text) = cx.intents().take_text() {
            let mut chars = text.chars();
            if let (Some(typed), None) = (chars.next(), chars.next())
                && let Some(row) = level.target(typed)
            {
                level.run(row);
                frame::request_repaint(&cx.core.ctx);
            }
        }
    }

    fn content_env(&self, cx: &Cx<'_, Self>, values: &mut crate::runtime::env::AppValues) {
        level_env(cx, values, self.submenu, self.values, cx.style.pad);
        values.set(InMenu(true));
    }

    fn finish(self, cx: Cx<'_, Self>) -> MenuOutput {
        MenuLevel::of(&cx.recall::<LevelMemory>()).end(cx.core.ctx.cumulative_pass_nr());
        MenuOutput { response: cx.response.clone(), closed: self.closed.or(self.back.then_some(CloseReason::Requested)), step: self.step }
    }
}

impl Container for Menu<'_> {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        self.open.as_mut().map(Bind::reborrow)
    }

    fn closed(&mut self, why: CloseReason) {
        self.closed.get_or_insert(why);
    }
}

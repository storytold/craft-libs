//! `Menu` (11.4, 15.4): the container of menu rows, on a menu layer, inline, or as a submenu.

use super::item::{MenuItem, MenuItemOutput};
use super::level::{LevelMemory, MenuLevel, level_env};
use super::words::MenuWords;
use crate::contract::overlay::{Anchor, Dismiss, Outside, Place, Popup};
use crate::contract::{
    Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, EndRegion, Entry, Focus as _, Gap, InBar, InMenu, LeastWidth, Mount,
    NoValue, Outcome, Overlay, POPUP_PAD, Pad, Pointer as _, Props as _, Remember as _, Role, Scroll, Show as _, ShowContainer as _, Sizing,
    TooltipSpec, Trigger,
};
use crate::display::Divider;
use crate::runtime::env::{Env, Label, with_env};
use crate::runtime::focus::{FocusPolicy, Initial};
use crate::runtime::frame;
use crate::runtime::overlay::CloseReason;
use crate::style::{ComponentStyle, StyleCx, SurfaceKind};
use craft_behavior::overlay::{Align, Side};
use craft_behavior::{Arrows, Metrics, MotionMode, NavShape, Roving, Sense, Value};
use craft_keymap::{Chord, Key};
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

    fn unshown(response: Response) -> Option<Self> {
        Some(MenuOutput { response, closed: None, step: None })
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

/// A row of a menu that picks a caller's value: an entry, or a submenu of rows.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum MenuRow<T> {
    /// An entry, shown as [`entry_rows`] shows it.
    Entry(Entry<T>),
    /// A submenu row of the label that shows `rows`, recursively; a submenu with no enabled row is disabled.
    Submenu(Label, Vec<MenuRow<T>>),
}

impl<T> From<Entry<T>> for MenuRow<T> {
    fn from(entry: Entry<T>) -> Self {
        MenuRow::Entry(entry)
    }
}

/// Shows `rows` as `entry_rows` shows entries, a submenu as `Menu::submenu_row_enabled`, and returns the value picked at
/// any depth.
pub fn menu_rows<T: Clone>(ui: &mut Ui, rows: &[MenuRow<T>]) -> Option<T> {
    let mut picked = None;
    for row in rows {
        match row {
            MenuRow::Entry(entry) => {
                if entry_rows(ui, std::slice::from_ref(entry)).is_some() {
                    picked = entry.value().cloned();
                }
            }
            MenuRow::Submenu(label, inner) => {
                let mut chosen = None;
                Menu::submenu_row_enabled(ui, label.clone(), has_enabled(inner), |ui, _| {
                    if let Some(value) = menu_rows(ui, inner) {
                        chosen = Some(value);
                    }
                });
                if chosen.is_some() {
                    picked = chosen;
                }
            }
        }
    }
    picked
}

/// Whether any row of `rows`, at any depth, is an enabled entry.
fn has_enabled<T>(rows: &[MenuRow<T>]) -> bool {
    rows.iter().any(|row| match row {
        MenuRow::Entry(entry) => entry.is_enabled() && !entry.is_separator() && !entry.is_heading(),
        MenuRow::Submenu(_, inner) => has_enabled(inner),
    })
}

/// Shows one row per entry, a caption for a heading, a rule for a separator, and returns the index of the entry that
/// ran. In a value menu
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
        if entry.is_heading() {
            MenuItem::heading(entry.label().clone()).show(ui);
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
/// - A menu taller than the room its layer has scrolls: strips above and below its rows scroll while the pointer
///   rests on them, the wheel scrolls it, and it keeps the width of its rows.
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
    /// The bar shown above the menu in its own modeless layer.
    companion: Option<&'a mut dyn FnMut(&mut Ui)>,
}

impl<'a> Menu<'a> {
    /// A modeless bar layer above the menu, in its press family; it closes with the menu, and Escape closes the top
    /// layer first. The bar runs in the menu's end region, which the layer leaves empty in flow, so it takes no room.
    #[must_use]
    pub fn companion(mut self, bar: &'a mut dyn FnMut(&mut Ui)) -> Self {
        self.companion = Some(bar);
        self.props = self.props.end(EndRegion::new());
        self
    }

    /// A menu named `label`. Its open state is the menu's slot until [`open`](Menu::open) binds it.
    pub fn new(label: impl Into<Label>) -> Self {
        let props = ContainerProps::new()
            .surface(SurfaceKind::Popup)
            .pad(POPUP_PAD)
            .body_gap(Gap::None)
            .least_width(LeastWidth::Popup)
            .roving(Roving::new(NavShape::Flat).arrows(Arrows::Vertical).wrap(true))
            .label_column(true)
            .scroll(Scroll::vertical().strips())
            .mount(Mount::Layer(layer(false)));
        Menu {
            common: Common::new(),
            props,
            open: None,
            closed: None,
            submenu: false,
            values: false,
            opener: None,
            back: false,
            step: None,
            companion: None,
        }
        .label(label)
    }

    /// Marks the menu as a value menu: every row is a radio item, whether or not an entry is checked, and the checked
    /// entry holds the lead when the menu opens.
    #[must_use]
    pub fn values(mut self) -> Self {
        self.values = true;
        self
    }

    /// The rows of a menu in the flow of the layer that shows them, with no close rules of their own: the menu of a
    /// control that owns its popup shows its entries through it, on the layer's surface and inside its padding.
    pub fn inline(label: impl Into<Label>) -> Self {
        let props = ContainerProps::new()
            .body_gap(Gap::None)
            .least_width(LeastWidth::Popup)
            .roving(Roving::new(NavShape::Flat).arrows(Arrows::Vertical).wrap(true))
            .label_column(true)
            .scroll(Scroll::vertical().strips());
        Menu {
            common: Common::new(),
            props,
            open: None,
            closed: None,
            submenu: false,
            values: false,
            opener: None,
            back: false,
            step: None,
            companion: None,
        }
        .label(label)
    }

    /// A context menu named `label`, hanging from where the pointer was when it opened, with `opener` getting the
    /// focus back when it closes by key. An item's menu hangs from its collection's response and shows while the
    /// collection reports the item's request (`CollectionResponse::menu`), whose key its rows act on.
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
        Menu::submenu_row_lettered(ui, label, enabled, None, body)
    }

    /// [`submenu_row_enabled`](Menu::submenu_row_enabled) whose row takes `mnemonic` in place of the level's pick
    /// ([`MenuItem::mnemonic`]).
    pub fn submenu_row_lettered(
        ui: &mut Ui,
        label: impl Into<Label>,
        enabled: bool,
        mnemonic: Option<char>,
        body: impl FnMut(&mut Ui, &mut BodyCx<'_>),
    ) -> MenuItemOutput {
        let label = label.into();
        let mut head = MenuItem::submenu(label.clone()).enabled(enabled);
        if let Some(letter) = mnemonic {
            head = head.mnemonic(letter);
        }
        let head = head.show(ui);
        let mut open = head.submenu_open;
        let show = |ui: &mut Ui| {
            Menu::submenu(label, &head.response)
                .id(head.response.id.with("submenu"))
                .anchor(Anchor::Rect(head.frame))
                .open(Bind::bound(&mut open))
                .show(ui, body)
        };
        let shown = if head.sibling_submenu_open() { with_env(ui, |env| env.motion = Some(MotionMode::Off), show) } else { show(ui) };
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
    /// The moves a field that drives the menu routes to it.
    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(Self::INPUT).with_moves();
    type Style = MenuStyle;
    type Output = MenuOutput;

    fn style(cx: &StyleCx<'_>, _: MenuStyleKey) -> MenuStyle {
        MenuStyle { metrics: Metrics::default(), pad: crate::contract::pad_of(cx.tokens, POPUP_PAD) }
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    /// A pointer move onto an enabled row of the level makes it the active row, before the rows run. A row of a
    /// submenu level that took Back closes the menu. A typed letter runs the row its mnemonic names; the
    /// container claims the letters, so a letter works whichever row is focused. A level the pointer opened with no active row has no lead:
    /// the first key enters it at an end. The active row is the lead, so the first key after the pointer lit a row
    /// moves from that row; a row lit in this pass leads from the next, so only a key in the same frame as the pointer
    /// move that lit it starts from the row lit before.
    ///
    /// A field that drives the menu (Help's search) hands it Up, Down, Page keys, Home and End, which move the active
    /// row, and Enter, which runs the active row, or else the first enabled one; the focus stays in the field.
    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let level = MenuLevel::of(cx.memory::<LevelMemory>());
        level.resume(cx.core.ctx.cumulative_pass_nr());
        while let Some((mv, _)) = cx.intents().take_move() {
            if level.step_active(mv) {
                frame::request_repaint(&cx.core.ctx);
            }
        }
        // Enter routed from the field arrives as the menu's own activation by key: the row it names runs, and the run
        // closes the menus, not the activation.
        if cx.activation().is_some_and(|activation| activation.by_key)
            && let Some(row) = level.active_or_first()
        {
            cx.keep_open();
            level.run(row);
            frame::request_repaint(&cx.core.ctx);
        }
        // The rows run after this, so the pass that moved the pointer onto a row paints it, and the row it left, as the
        // level now has them.
        if crate::runtime::pointer::frame(&cx.core.ctx).delta != egui::Vec2::ZERO {
            let ctx = cx.core.ctx.clone();
            level.follow_pointer(|row| crate::runtime::pointer::hovers(&ctx, row));
        }
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
        // The letters the rows' mnemonics name are the level's claims while focus is in it; any other key reaches the
        // app.
        for (letter, row) in level.lettered_rows() {
            let Some(key) = Key::from_char(letter) else { continue };
            let chord = Chord::plain(key);
            cx.claim(chord);
            if cx.take_claimed(chord) {
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

    /// The companion bar, shown as a child layer of the menu's: it is in the menu's press family and closes with the
    /// menu.
    fn end(&mut self, ui: &mut Ui, cx: &mut BodyCx<'_>) {
        let Some(bar) = self.companion.as_mut() else { return };
        if cx.is_exiting() {
            return;
        }
        let label = Env::of(ui).words::<MenuWords>().bar_companion.clone();
        let _ = CompanionLayer::new(label).show(ui, |ui, _| bar(ui));
    }
}

/// The style of a menu's companion layer: the surface comes from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct CompanionLayerStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// What a companion layer reports.
#[derive(Debug)]
struct CompanionOutput {
    response: Response,
}

impl Outcome for CompanionOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The modeless layer a menu's companion bar shows in, above the menu's panel. It takes no focus on open and has no
/// opener of its own: it is open while the menu's body runs it.
#[derive(crate::contract::Component)]
#[component(container, unregistered)]
struct CompanionLayer {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    open: Bind<'static, bool>,
}

impl CompanionLayer {
    fn new(label: Label) -> Self {
        let overlay = Overlay::floating()
            .modeless()
            .body_scroll(false)
            .focus(FocusPolicy::NONE)
            .dismiss(Dismiss::new().outside(Outside::Keep))
            .place(Place::anchored(Anchor::Parent, Side::Above, Align::Start));
        let props = ContainerProps::new().mount(Mount::Layer(overlay)).surface(SurfaceKind::Floating).pad(Pad::Control).sizing(Sizing::Content);
        CompanionLayer { common: Common::new(), props, open: Bind::given(true) }.label(label).id_salt("menu-companion")
    }
}

impl Component for CompanionLayer {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = CompanionLayerStyle;
    type Output = CompanionOutput;

    fn style(_: &StyleCx<'_>, _: CompanionLayerStyleKey) -> CompanionLayerStyle {
        CompanionLayerStyle::default()
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn content_env(&self, _: &Cx<'_, Self>, values: &mut crate::runtime::env::AppValues) {
        values.set(InBar(true));
    }

    fn finish(self, cx: Cx<'_, Self>) -> CompanionOutput {
        CompanionOutput { response: cx.response.clone() }
    }
}

impl Container for CompanionLayer {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }
}

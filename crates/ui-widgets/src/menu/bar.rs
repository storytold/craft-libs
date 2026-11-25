//! `MenuBar`: a row of menu titles, each opening a menu the caller writes.
//!
//! The titles are parts of one component in one roving group, so the bar is one Tab stop. Each title owns a popup
//! (the indexed popup `"menu"`) that its part toggles on the press, and the popup shows a `Menu` below the title.
//! While a menu is open the pointer moving onto another title switches to that menu, and Left and Right at the top
//! of the menu move to the previous and next one. Titles that do not fit go into a trailing "…" menu that holds them
//! as submenus, or all of them into one "Menu". Under `MenuPlacement::ScreenTop` the bar shows nothing: the OS menu
//! bar takes the same items.
//!
//! Keys follow spec 9.7: Alt and a letter open the menu with that mnemonic (also while another is open). F10 or a
//! lone Alt closes an open menu; with none open it moves focus to the first title, and the next F10 or lone Alt, an
//! Escape, a shortcut (Space, a canvas's momentary Hand, among them) or running a row returns focus to where it was.
//! On the focused bar a letter without Alt is a mnemonic too: it opens the title it marks, and one no title marks does
//! nothing, so it runs no shortcut.
//! Down and Enter open the focused title's menu; Space opens none. A `&` in a title's words row marks its mnemonic; without
//! one the title takes the first letter no earlier title holds. The mnemonic is underlined while Alt is held.
//!
//! The Help menu can start with a search field over a list of `HelpEntry` commands: typing lists up to `search::MAX_RESULTS`
//! matches by label, English label, menu path or command id, each as a command row with its path as detail.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::search::{HelpEntry, MAX_RESULTS, SearchIndex, SearchWords, Synonyms, rank};
use super::words::MenuWords;
use super::{Menu, MenuItem, MenuStep};
use crate::buttons::face::{self, FaceLook, FaceSpec};
use crate::contract::{
    Access, Bind, Common, Component, Cx, Focus as _, MeasureCx, MenuKey, Name, NoValue, Outcome, OverlayUi, Overlays as _, Pad, PaintUi, PartsCx,
    Pointer as _, PopupHandle, PopupKind, Props as _, Role, SetPosition, Show as _, ShowContainer as _,
};
use crate::display::{Divider, Shade, Text};
use crate::layout_components::Stack;
use crate::list::RowStyleCx as _;
use crate::runtime::commands::{self, CommandRef};
use crate::runtime::env::{EnvView, Label};
use crate::runtime::focus::{self, FocusTarget};
use crate::runtime::memory::{Facts, Remember as _};
use crate::runtime::words::common::CommonWords;
use crate::style::{ComponentStyle, FocusLook, IconLook, StyleCx, SurfaceLook, TextLook, TypeRole, UiTokens};
use crate::text::TextField;
use craft_behavior::fit::{FitItem, FitKeep, fit_items};
use craft_behavior::{Arrows, Flow, Intent, Metrics, NavMove, NavShape, PartKey, PartRef, Proposal, Roving, Sense};
use craft_host::target::MenuPlacement;
use craft_keymap::mnemonic;
use craft_theme::Flags;
use egui::{Context, Pos2, Rect, Response, Ui, Vec2};

const TITLE: PartKey = PartKey::new("title");
const MORE: PartKey = PartKey::new("more");

/// How much room a menu bar's titles take on each side of their text, and so how far apart they stand. An app sets it
/// once at its root (`RootEnv::values`); the default is the regular padding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::runtime::env::EnvValue, crate::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum MenuTitlePad {
    /// The padding inside panels (`space.pad.regular`).
    #[default]
    Regular,
    /// The padding inside controls (`space.pad.control`): titles set closer, as in an app bar whose menus share the
    /// line with many controls.
    Control,
}

/// How a menu bar's titles stand. An app sets it once at its root (`RootEnv::values`); the default is a menu row's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, crate::runtime::env::EnvValue, crate::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum MenuTitleLook {
    /// A menu row's: square corners, a control high, [`MenuTitlePad`] beside the text.
    #[default]
    Row,
    /// A bare push button's, as egui's menu bar draws its titles: a push button high with the button's corners
    /// (`radius.button`) and `space.gap.tight` beside the text, the titles `space.pad.control` apart.
    Button,
}

/// The style of a title's face: a menu row's surface and ink (flat, square corners, no border), with the command
/// highlight (`CommandHighlight`) under the pointer and while its menu is open or the keyboard is on it.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct TitleFaceStyle {
    #[style(surface)]
    surface: SurfaceLook,
    #[style(metrics)]
    metrics: Metrics,
    #[style(focus)]
    focus: FocusLook,
    #[style(text)]
    text: TextLook,
    #[style(icon)]
    icon: IconLook,
    face: FaceLook,
}

/// The face of a menu title: lit while its menu is open or the keyboard is on it, its mnemonic underlined while Alt
/// is held or the bar has focus.
#[derive(crate::contract::Component)]
#[component(sample = TitleFace::sample)]
struct TitleFace {
    #[common]
    common: Common,
    lit: bool,
    mnemonic: Option<usize>,
}

impl TitleFace {
    fn new(label: impl Into<Label>, lit: bool, mnemonic: Option<usize>) -> Self {
        TitleFace { common: Common::new(), lit, mnemonic }.label(label)
    }

    fn sample() -> Self {
        TitleFace::new("Title", false, None)
    }
}

impl Component for TitleFace {
    const ROLE: Role = Role::MENU_ITEM;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    type Style = TitleFaceStyle;
    type Output = egui::Response;

    fn style(cx: &StyleCx<'_>, _: TitleFaceStyleKey) -> TitleFaceStyle {
        let tokens = cx.tokens;
        let surface = cx.command_row_look();
        let ink = cx.command_row_ink(tokens.color.text.normal);
        let control = cx.control();
        let (surface, pad, height) = match (cx.env::<MenuTitleLook>(), cx.env::<MenuTitlePad>()) {
            (MenuTitleLook::Button, _) => (surface.radius(UiTokens::corner(tokens.shape.radius.button)), tokens.space.gap.tight, cx.button_height()),
            (MenuTitleLook::Row, MenuTitlePad::Regular) => (surface, tokens.space.pad.regular, control.height),
            (MenuTitleLook::Row, MenuTitlePad::Control) => (surface, tokens.space.pad.control, control.height),
        };
        TitleFaceStyle {
            surface,
            metrics: Metrics::new().padding(craft_behavior::Insets::horizontal(pad)).min_size(Vec2::new(0.0, height)).gap(control.gap),
            focus: cx.focus_look(),
            text: TextLook::new(TypeRole::Label, ink.clone()),
            icon: IconLook::new(control.icon, ink),
            face: FaceLook::of(cx),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        face::measure(
            cx,
            &FaceSpec {
                arrangement: crate::buttons::Arrangement::Inline,
                label: cx.resolved.label,
                role: None,
                lead: None,
                picture: None,
                trail: None,
                loading: false,
                mnemonic: None,
                align: crate::primitives::HAlign::Center,
                chord: None,
                detail: None,
            },
        )
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn flags(&self) -> Flags {
        if self.lit { Flags::LEAD } else { Flags::empty() }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let spec = FaceSpec {
            arrangement: crate::buttons::Arrangement::Inline,
            label: cx.resolved.label,
            role: None,
            lead: None,
            picture: None,
            trail: None,
            loading: false,
            mnemonic: self.mnemonic,
            align: crate::primitives::HAlign::Center,
            chord: None,
            detail: None,
        };
        face::paint(ui, cx, (&cx.style.text, &cx.style.icon), &cx.style.face, &spec, cx.content_rect);
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

/// The style of a menu bar: the titles wear row faces, which underline their mnemonics.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct MenuBarStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A title's text without its marker, and its mnemonic: the letter and its character index in the text, none for a
/// letter the text does not show.
type TitleText = (String, Option<(char, Option<usize>)>);

/// A menu of the bar: its title and the body that fills the menu when it opens.
pub struct MenuTitle<'a> {
    label: Label,
    body: Box<dyn FnMut(&mut Ui) + 'a>,
    holds: Vec<CommandRef>,
    /// The letter that opens the menu in place of the bar's own pick.
    mnemonic: Option<char>,
}

impl<'a> MenuTitle<'a> {
    /// A menu titled `label` (a `&` marks the mnemonic) whose rows `body` shows.
    pub fn new(label: impl Into<Label>, body: impl FnMut(&mut Ui) + 'a) -> Self {
        MenuTitle { label: label.into(), body: Box::new(body), holds: Vec::new(), mnemonic: None }
    }

    /// The letter that opens the menu, in place of the bar's own pick. A letter the title lacks still opens it, with
    /// nothing underlined.
    #[must_use]
    pub fn mnemonic(mut self, letter: char) -> Self {
        self.mnemonic = Some(letter);
        self
    }

    /// Records the title's rect for `commands` with `commands::holds` in every pass it is shown.
    #[must_use]
    pub fn holds(mut self, commands: Vec<CommandRef>) -> Self {
        self.holds = commands;
        self
    }
}

/// The Help menu's search: the index of the menu that starts with the field, and the commands it searches.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct MenuHelp<'a> {
    menu: usize,
    entries: &'a dyn Fn() -> Vec<HelpEntry>,
}

impl std::fmt::Debug for MenuHelp<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuHelp").field("menu", &self.menu).finish_non_exhaustive()
    }
}

impl<'a> MenuHelp<'a> {
    /// A search field at the top of the menu `menu` (its position among the titles) over the commands `entries`
    /// lists. The bar runs it only in a pass that shows the Help menu, so an app whose command list is costly to
    /// collect (every row of its menus) pays nothing while the menu is closed.
    pub fn new(menu: usize, entries: &'a dyn Fn() -> Vec<HelpEntry>) -> Self {
        MenuHelp { menu, entries }
    }
}

/// What a menu bar reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct MenuBarOutput {
    /// The bar's response.
    pub response: Response,
    /// A menu of the bar is open.
    pub open: bool,
    /// The titles alone (with "…" when it shows), from the first title's reading start to the last title's end.
    pub titles: Rect,
    /// True in the one pass where a menu of the bar opened while no menu of the bar was open when the pass began. A
    /// rerun pass in the same frame and a switch from one open title to another report false.
    pub opened: bool,
}

impl Outcome for MenuBarOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A title of the bar: a menu, or the overflow menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    Title(usize),
    More,
}

/// Bar state between passes.
#[derive(Clone, Debug, Default)]
struct BarState {
    /// Where the pointer last was over a title, so that only a moving pointer switches menus.
    hover: Option<Pos2>,
}

/// The help field's text, the passes the Help menu has been open, and whether the field has taken the key focus.
#[derive(Clone, Debug, Default)]
struct Query(String, u32, bool);

craft_ui_contract::stays!(BarFact);

/// What the bar published for `is_open` and `is_focused`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BarFact {
    open: bool,
    focused: bool,
}

/// Whether a menu of a bar is open. The app leaves the keys to it meanwhile: its shortcuts wait until the menu
/// closes.
pub fn is_open(ctx: &Context) -> bool {
    Facts::<BarFact>::of_last_pass(ctx).iter().any(|fact| fact.open)
}

/// Whether the keyboard is on a menu bar's titles.
pub fn is_focused(ctx: &Context) -> bool {
    Facts::<BarFact>::of_last_pass(ctx).iter().any(|fact| fact.focused)
}

/// The titles a bar shows in a given room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TitleFit {
    /// How many titles show, from the first.
    pub shown: usize,
    /// The rest are in an overflow menu: "…" after the shown titles, or "Menu" when none is shown.
    pub overflow: bool,
}

/// The titles to show in `room`, given each title's `widths` and the width of the "…" title: every title when they
/// fit; else the titles that fit beside "…"; else none, with one "Menu" holding them all.
pub fn fit_titles(widths: &[f32], more: f32, room: f32) -> TitleFit {
    let titles: Vec<FitItem> = widths.iter().map(|width| FitItem::new(*width, 0, FitKeep::Free)).collect();
    let fit = fit_items(&titles, room, 0.0, 0.0, more);
    TitleFit { shown: fit.shown.len(), overflow: !fit.overflow.is_empty() }
}

/// Which titles a pass declared as parts.
#[derive(Clone, Copy, Debug)]
struct Declared {
    shown: usize,
    more: bool,
}

fn declared(cx: &impl crate::contract::Pointer) -> Declared {
    let shown = (0..).take_while(|at| cx.part_if_declared(TITLE.at(*at)).is_some()).count();
    Declared { shown, more: cx.part_if_declared(MORE).is_some() }
}

/// A menu bar. Titles are the caller's `MenuTitle`s; the bar itself shows no menu rows.
///
/// AccessKit: the bar is a `MenuBar`; titles are `MenuItem`s that have a menu popup, report `expanded` while their
/// menu is open and carry their access key.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(overlay)]
pub struct MenuBar<'a> {
    #[common]
    common: Common,
    titles: Vec<MenuTitle<'a>>,
    /// The Help menu's search.
    #[prop]
    help: Option<MenuHelp<'a>>,
    /// Whether the bar takes F10, Alt and Alt+letter. Off while a dialog or the shortcut recorder owns the keys.
    #[prop]
    keyboard: bool,
    open: bool,
    /// A menu opened in this pass while none was open when it began.
    opened: bool,
}

impl<'a> MenuBar<'a> {
    /// A bar named `label` with one title per menu, in reading order.
    pub fn new(label: impl Into<Label>, titles: Vec<MenuTitle<'a>>) -> Self {
        MenuBar { common: Common::new(), titles, help: None, keyboard: true, open: false, opened: false }.label(label)
    }

    fn in_window(env: &EnvView<'_>) -> bool {
        env.shell().menu() == MenuPlacement::InWindow
    }

    /// Each title's text without its marker, and its mnemonic: the letter and its character index in the text, none
    /// for a letter the text does not show. A Mac menu has no access letters.
    fn texts(&self, env: &EnvView<'_>) -> Vec<TitleText> {
        let labels: Vec<Label> = self.titles.iter().map(|title| env.resolve(&title.label)).collect();
        let refs: Vec<&str> = labels.iter().map(Label::as_str).collect();
        let own: Vec<Option<char>> = self.titles.iter().map(|title| title.mnemonic).collect();
        let folder = env.folder();
        let marks = if env.shell().os.is_mac() { vec![None; refs.len()] } else { mnemonic::assign_with(&refs, &own, &folder) };
        refs.iter().zip(marks).map(|(label, mark)| (mnemonic::marked(label, &folder).0, mark)).collect()
    }

    fn face(text: &str, lit: bool, mnemonic: Option<usize>) -> TitleFace {
        TitleFace::new(Label::from(text.to_owned()), lit, mnemonic)
    }

    /// The text of the overflow title: "…" beside shown titles, "Menu" when it holds them all.
    fn more_text(words: &MenuWords, shown: usize) -> String {
        if shown == 0 { words.bar_all.as_str().to_owned() } else { words.bar_more.as_str().to_owned() }
    }

    /// Each title's width as its face measures, the face height, and the width of the "…" title, each with the gap
    /// after it ([`Self::gap`]).
    fn widths(&self, cx: &MeasureCx<'_, Self>) -> (Vec<f32>, f32, f32) {
        let texts = self.texts(&cx.env);
        let gap = Self::gap(cx);
        let sizes: Vec<Vec2> = texts.iter().map(|(text, _)| cx.face(Self::face(text, false, None))).collect();
        let more = cx.face(Self::face(&Self::more_text(&cx.env.words::<MenuWords>(), 1), false, None));
        let height = sizes.iter().map(|size| size.y).fold(more.y, f32::max);
        (sizes.iter().map(|size| size.x + gap).collect(), height, more.x + gap)
    }

    /// The room after each title: `space.pad.control` between titles that stand as buttons, none between rows.
    fn gap(cx: &MeasureCx<'_, Self>) -> f32 {
        match cx.env.get::<MenuTitleLook>() {
            MenuTitleLook::Button => cx.tokens.space.pad.control,
            MenuTitleLook::Row => 0.0,
        }
    }

    fn slot_part(open: Open) -> PartRef {
        match open {
            Open::Title(at) => TITLE.at(at as u64),
            Open::More => MORE.into(),
        }
    }

    /// The parts a pass declared, as menus.
    fn slots(declared: Declared) -> Vec<Open> {
        (0..declared.shown).map(Open::Title).chain(declared.more.then_some(Open::More)).collect()
    }

    /// The index of a slot's popup: the title's position, and the overflow menu after the last title.
    fn popup_index(slot: Open, declared: Declared) -> usize {
        match slot {
            Open::Title(at) => at,
            Open::More => declared.shown,
        }
    }

    fn popup(cx: &mut Cx<'_, Self>, slot: Open, declared: Declared) -> PopupHandle {
        cx.popup("menu").at(Self::popup_index(slot, declared))
    }

    /// The menu that is open, if any.
    fn opened(cx: &mut Cx<'_, Self>, slots: &[Open], declared: Declared) -> Option<Open> {
        slots.iter().copied().find(|slot| Self::popup(cx, *slot, declared).is_open(cx))
    }

    /// Opens `target`'s menu and closes the others.
    fn open_only(cx: &mut Cx<'_, Self>, slots: &[Open], declared: Declared, target: Open) {
        for slot in slots {
            let popup = Self::popup(cx, *slot, declared);
            if *slot == target {
                popup.open(cx);
            } else if popup.is_open(cx) {
                popup.close(cx);
            }
        }
    }

    /// F10 or a lone Alt: closes an open menu; else moves focus to the first title, or gives it back to the
    /// component that held it when the bar already has it.
    fn focus_key(cx: &mut Cx<'_, Self>, slots: &[Open], declared: Declared, opened: Option<Open>) {
        if opened.is_some() {
            for slot in slots {
                let popup = Self::popup(cx, *slot, declared);
                if popup.is_open(cx) {
                    popup.close(cx);
                }
            }
        } else if cx.is_focused() {
            focus::give_back_bar(&cx.core.ctx);
        } else {
            let target = slots.first().map_or(FocusTarget::Id(cx.id), |slot| FocusTarget::Part(cx.id, Self::slot_part(*slot)));
            focus::take_for_bar(&cx.core.ctx, cx.id, target);
            cx.core.sync_focus();
        }
    }

    /// Left or Right at the top of the open menu `from`: the previous or next menu opens, wrapping at the ends. The
    /// directions are the keys', so they mirror in a right-to-left bar.
    fn step_menu(cx: &mut Cx<'_, Self>, slots: &[Open], declared: Declared, from: Open, step: MenuStep) {
        let Some(at) = slots.iter().position(|slot| *slot == from) else { return };
        let forward = matches!(step, MenuStep::Next) != cx.flow.is_rtl();
        let to = if forward { (at + 1) % slots.len() } else { (at + slots.len() - 1) % slots.len() };
        Self::open_only(cx, slots, declared, slots[to]);
    }
}

impl Component for MenuBar<'_> {
    const ROLE: Role = Role::MENU_BAR;
    // Enter and Down open the focused title's menu; Space does not. Space is a canvas's momentary Hand, so it goes on
    // to the app's shortcuts, and a shortcut pressed on the bar gives the keyboard back to where it was.
    const KEYS: crate::runtime::focus::KeyUse =
        crate::runtime::focus::KeyUse::of(crate::contract::Input::Pointer).with_moves().with_activate(crate::runtime::focus::ActivateKeys::Enter);
    type Style = MenuBarStyle;
    type Output = MenuBarOutput;

    fn style(_: &StyleCx<'_>, _: MenuBarStyleKey) -> MenuBarStyle {
        MenuBarStyle { metrics: Metrics::new() }
    }

    /// Every title's width; at its least (`Proposal::Min`) the width of the "…" title alone. A row that has less room
    /// than every title needs narrows the bar, and the titles that no longer fit go into "…".
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        if !Self::in_window(&cx.env) {
            return Vec2::ZERO;
        }
        let (widths, height, more) = self.widths(cx);
        let width = if cx.proposal == Proposal::Min { more } else { widths.iter().sum() };
        Vec2::new(width, height)
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        if !Self::in_window(&cx.env) {
            return;
        }
        let (widths, _, more) = self.widths(cx);
        let fit = fit_titles(&widths, more, rect.width());
        let gap = Self::gap(cx);
        let mut flow = Flow::new(rect, cx.flow.is_rtl());
        for (at, width) in widths.iter().take(fit.shown).enumerate() {
            let (cell, rest) = flow.take_start(*width);
            flow = rest;
            let cell = Flow::new(cell, cx.flow.is_rtl()).take_start(width - gap).0;
            cx.part(TITLE.at(at as u64), cell).sense(Sense::CLICK | Sense::FOCUS).focusable().toggles_at("menu", at);
        }
        if fit.overflow {
            let cell = flow.take_start(more - gap).0;
            cx.part(MORE, cell).sense(Sense::CLICK | Sense::FOCUS).focusable().toggles_at("menu", fit.shown);
        }
        cx.roving(Roving::new(NavShape::Flat).arrows(Arrows::Horizontal));
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let found = declared(cx);
        let slots = Self::slots(found);
        let texts = self.texts(&cx.env);
        let opened = Self::opened(cx, &slots, found);
        let keys = if self.keyboard && Self::in_window(&cx.env) {
            cx.register_menu_bar();
            cx.take_menu_keys()
        } else {
            Vec::new()
        };
        if opened.is_none() && !cx.is_focused() {
            focus::forget_bar_return(&cx.core.ctx);
        }
        for key in keys {
            match key {
                MenuKey::Letter(letter) => {
                    let held = |(_, mark): &TitleText| mark.is_some_and(|(held, _)| held == letter);
                    if let Some(at) = texts.iter().take(found.shown).position(held) {
                        Self::open_only(cx, &slots, found, Open::Title(at));
                    }
                }
                MenuKey::Focus => Self::focus_key(cx, &slots, found, opened),
            }
        }
        while let Some((mv, _)) = cx.intents().take_move() {
            // Down on a focused title opens its menu; the bar's roving group leaves Up and Down to the part.
            let title = cx.active_part().and_then(|part| slots.iter().copied().find(|slot| Self::slot_part(*slot) == part));
            if let (NavMove::Down, Some(slot), true) = (mv, title, opened.is_none()) {
                Self::open_only(cx, &slots, found, slot);
            }
        }
        if opened.is_none() && cx.is_focused() && cx.intents().take(Intent::Cancel) {
            focus::give_back_bar(&cx.core.ctx);
        }
        let mut state = cx.memory::<BarState>().clone();
        let hovered = slots.iter().find_map(|slot| cx.hover_pos(Self::slot_part(*slot)).map(|pos| (*slot, pos)));
        match hovered {
            Some((slot, pos)) => {
                if opened.is_some_and(|open| open != slot) && state.hover.is_some_and(|before| before != pos) {
                    Self::open_only(cx, &slots, found, slot);
                }
                state.hover = Some(pos);
            }
            None => state.hover = None,
        }
        let opened = Self::opened(cx, &slots, found);
        // A popup began open when its open state differs from `changed`: switching titles closes one popup and opens
        // another, so the edge is that some menu is open now and none began open.
        let began_open = slots.iter().any(|slot| {
            let popup = Self::popup(cx, *slot, found);
            popup.is_open(cx) != popup.changed(cx)
        });
        self.opened = opened.is_some() && !began_open;
        self.open = opened.is_some();
        Facts::<BarFact>::register(&cx.ctx, BarFact { open: self.open, focused: cx.is_focused() });
        *cx.memory::<BarState>() = state;
    }

    /// The open title's menu below it: the caller's body, with the help search above it in the Help menu. The
    /// overflow menu lists each hidden title as a submenu. Every menu and submenu stays below the bar. Running a row returns focus to the component that held
    /// it when F10 or Alt took it for the bar, as any activation that closes the menus does (`focus::return_from_pick`).
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        // The menus and their submenus stay below the bar: a tall one scrolls instead of covering the titles.
        ui.contain_popups_below();
        let found = declared(cx);
        let slots = Self::slots(found);
        let opened = Self::opened(cx, &slots, found);
        let words = cx.env.words::<MenuWords>();
        let separator = cx.env.words::<CommonWords>().path_separator.as_str().to_owned();
        let texts = self.texts(&cx.env);
        let help = self.help;
        // Only a pass that shows the Help menu needs its commands: the entries function runs here and nowhere else.
        let help_entries = help.filter(|help| opened == Some(Open::Title(help.menu))).map(|help| (help.entries)());
        // The search starts empty each time the Help menu opens.
        let mut query = if help_entries.is_some() { cx.memory::<Query>().clone() } else { Query::default() };
        let mut stepped = None;
        for slot in &slots {
            let popup = Self::popup(cx, *slot, found);
            let layer = super::layer(false).anchor(ui.anchor_part(Self::slot_part(*slot)));
            let name = match slot {
                Open::Title(at) => texts[*at].0.clone(),
                Open::More => Self::more_text(&words, found.shown),
            };
            let titles = &mut self.titles;
            let step = ui.show(popup, layer, |ui, _| {
                Menu::inline(Label::from(name.clone()))
                    .show(ui, |ui, menu| match slot {
                        Open::Title(at) => {
                            if let (Some(_), Some(entries)) = (help.filter(|help| help.menu == *at), help_entries.as_deref()) {
                                // The field sits in a padded frame: its edge lines up with the row labels and it keeps clear of the first row.
                                // The field drives the menu: Up, Down, Home and End move its active row and Enter runs
                                // it, or the first enabled match, while typing stays in the field.
                                let drives = menu.resolved.id;
                                let field = Stack::new()
                                    .pad(Pad::Regular)
                                    .fill_width()
                                    .show(ui, |ui, _| {
                                        TextField::search(words.bar_search.as_str().to_owned(), Bind::bound(&mut query.0))
                                            .placeholder(words.bar_search.as_str().to_owned())
                                            .drives(drives)
                                            .show(ui)
                                            .response
                                    })
                                    .inner;
                                // Opening Help puts the key focus in its field, from the pass after the menu opened, once
                                // the layer has recorded where focus returns to.
                                if let Some(field) = field {
                                    if field.has_focus() {
                                        query.2 = true;
                                    } else if !query.2 && query.1 > 0 {
                                        focus::request(ui.ctx(), FocusTarget::Id(field.id));
                                    }
                                }
                                query.1 = query.1.saturating_add(1);
                                // The index is built only for a query, from the entries this pass collected.
                                let mut seen: Vec<&CommandRef> = Vec::new();
                                let listed: Vec<&HelpEntry> = if query.0.trim().is_empty() {
                                    Vec::new()
                                } else {
                                    let synonyms = Synonyms::of(&cx.env.words::<SearchWords>());
                                    let index = SearchIndex::new(&cx.env, entries, &synonyms);
                                    rank(&index, &query.0, &[], &|_| 0)
                                        .into_iter()
                                        .filter_map(|at| entries.get(at))
                                        .filter(|entry| {
                                            let fresh = !seen.contains(&entry.command());
                                            seen.push(entry.command());
                                            fresh
                                        })
                                        .take(MAX_RESULTS)
                                        .collect()
                                };
                                for entry in &listed {
                                    let row = MenuItem::command(entry.command().clone()).label(entry.label().clone());
                                    if entry.path().is_empty() {
                                        row.show(ui)
                                    } else {
                                        let path: Vec<String> = entry.path().iter().map(|label| cx.env.resolve(label).as_str().to_owned()).collect();
                                        row.detail(path.join(&separator)).show(ui)
                                    };
                                }
                                if listed.is_empty() && !query.0.trim().is_empty() {
                                    Text::new(words.bar_no_matches.as_str().to_owned()).shade(Shade::Muted).show(ui);
                                }
                                Divider::horizontal().show(ui);
                            }
                            (titles[*at].body)(ui);
                        }
                        Open::More => {
                            for title in titles.iter_mut().skip(found.shown) {
                                super::popup::Menu::submenu_row(ui, title.label.clone(), |ui, _| (title.body)(ui));
                            }
                        }
                    })
                    .output
                    .step
            });
            if let Some(Some(step)) = step {
                stepped = Some((*slot, step));
            }
        }
        if let Some((from, step)) = stepped {
            Self::step_menu(cx, &slots, found, from, step);
        }
        *cx.memory::<Query>() = query;
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let found = declared(cx);
        let texts = self.texts(&cx.env);
        let words = cx.env.words::<MenuWords>();
        let slots = Self::slots(found);
        for (at, slot) in slots.iter().enumerate() {
            let name = match slot {
                Open::Title(title) => texts[*title].0.clone(),
                Open::More => Self::more_text(&words, found.shown),
            };
            let node = access.node(Self::slot_part(*slot), Role::MENU_ITEM);
            node.name = Name::Text(name);
            node.has_popup(PopupKind::Menu);
            if let Open::Title(title) = slot
                && let Some((letter, _)) = texts[*title].1
            {
                node.access_key(letter);
            }
            node.position = Some(SetPosition::new(at).of(slots.len()));
        }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let found = declared(cx);
        let texts = self.texts(&cx.env);
        let words = cx.env.words::<MenuWords>();
        let underlined = cx.modifiers().alt || cx.is_focused();
        for slot in Self::slots(found) {
            let part = Self::slot_part(slot);
            let text = match slot {
                Open::Title(at) => texts[at].0.clone(),
                Open::More => Self::more_text(&words, found.shown),
            };
            let open = cx.core.toggles.iter().any(|toggle| toggle.part == part && toggle.open);
            // Under the pointer a title is the active row, as it is while its menu is open or the keyboard is on it.
            let lit = open || cx.hover_pos(part).is_some() || cx.part_state(part).flags.contains(Flags::FOCUS_VISIBLE);
            let mnemonic = match slot {
                Open::Title(at) if underlined => texts[at].1.and_then(|(_, index)| index),
                _ => None,
            };
            ui.face(part, Self::face(&text, lit, mnemonic));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> MenuBarOutput {
        let found = declared(&cx);
        let texts = self.texts(&cx.env);
        // A title in the overflow menu has no part rect, so it records nothing.
        for (at, title) in self.titles.iter().enumerate().take(found.shown) {
            if let (false, Some(result), Some((text, _))) =
                (title.holds.is_empty(), cx.part_if_declared(Self::slot_part(Open::Title(at))), texts.get(at))
            {
                commands::holds(&cx.ctx, cx.response.layer_id, result.rect, &title.holds, Label::from(text.clone()));
            }
        }
        let rects = Self::slots(found).into_iter().filter_map(|slot| cx.part_if_declared(Self::slot_part(slot)).map(|result| result.rect));
        let start = if cx.flow.is_rtl() { cx.rect.right_top() } else { cx.rect.left_top() };
        let titles = rects.reduce(Rect::union).unwrap_or_else(|| Rect::from_min_size(start, Vec2::ZERO));
        MenuBarOutput { response: cx.response.clone(), open: self.open, titles, opened: self.opened }
    }
}

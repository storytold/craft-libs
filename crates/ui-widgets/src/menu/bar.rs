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
//! Escape, or running a row returns focus to where it was. A `&` in a title's words row marks its mnemonic; without
//! one the title takes the first letter no earlier title holds. The mnemonic is underlined while Alt is held.
//!
//! The Help menu can start with a search field over a list of `HelpEntry` commands: typing lists up to `search::MAX_RESULTS`
//! matches by label, English label or menu path, each as a command row with its path as detail.

#[cfg(feature = "testing")]
mod stories;

use super::search::{HelpEntry, search_order};
use super::words::MenuWords;
use super::{Menu, MenuItem, MenuStep};
use crate::buttons::face::{self, FaceLook, FaceSpec};
use crate::contract::{
    Access, Bind, Common, Component, Cx, Focus as _, MeasureCx, MenuKey, Name, NoValue, Outcome, OverlayUi, Overlays as _, Pad, PaintUi, PartsCx,
    Pointer as _, PopupHandle, PopupKind, Props as _, Role, SetPosition, Show as _, ShowContainer as _, Styled as _,
};
use crate::display::{Divider, Text};
use crate::layout_components::Stack;
use crate::list::RowStyleCx as _;
use crate::primitives::{PaintShape, StrokeWidth, TextRun};
use crate::runtime::commands::CommandRef;
use crate::runtime::env::{EnvView, Label};
use crate::runtime::focus::{self, FocusTarget};
use crate::runtime::memory::{Facts, Remember as _};
use crate::runtime::words::common::CommonWords;
use crate::style::{ComponentStyle, FocusLook, IconLook, StyleCx, SurfaceLook, TextLook, TypeRole};
use crate::text::TextField;
use craft_behavior::fit::{FitItem, FitKeep, fit_items};
use craft_behavior::{Arrows, Axis, Flow, Intent, Metrics, NavMove, NavShape, PartKey, PartRef, Roving, Sense};
use craft_host::target::MenuPlacement;
use craft_keymap::mnemonic;
use craft_theme::{Flags, Stateful};
use egui::{Context, Pos2, Rect, Response, Ui, Vec2, pos2};

const TITLE: PartKey = PartKey::new("title");
const MORE: PartKey = PartKey::new("more");

/// The style of a title's face: a menu row's surface and ink (flat, square corners, no border), with the accent fill
/// and `text.on_accent` under the pointer and while its menu is open or the keyboard is on it.
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

/// The face of a menu title: lit while its menu is open or the keyboard is on it.
#[derive(crate::contract::Component)]
#[component(sample = TitleFace::sample)]
struct TitleFace {
    #[common]
    common: Common,
    lit: bool,
}

impl TitleFace {
    fn new(label: impl Into<Label>, lit: bool) -> Self {
        TitleFace { common: Common::new(), lit }.label(label)
    }

    #[cfg(feature = "testing")]
    fn sample() -> Self {
        TitleFace::new("Title", false)
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
        TitleFaceStyle {
            surface,
            metrics: Metrics::new()
                .padding(craft_behavior::Insets::horizontal(tokens.space.pad.regular))
                .min_size(Vec2::new(0.0, control.height))
                .gap(control.gap),
            focus: cx.focus_look(),
            text: TextLook::new(TypeRole::Label, ink.clone()),
            icon: IconLook::new(control.icon, ink),
            face: FaceLook::of(cx),
        }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        face::measure(cx, &FaceSpec { label: cx.resolved.label, role: None, lead: None, trail: None, loading: false })
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    fn flags(&self) -> Flags {
        if self.lit { Flags::LEAD } else { Flags::empty() }
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let spec = FaceSpec { label: cx.resolved.label, role: None, lead: None, trail: None, loading: false };
        face::paint(ui, cx, (&cx.style.text, &cx.style.icon), &cx.style.face, &spec, cx.content_rect);
    }

    fn finish(self, cx: Cx<'_, Self>) -> egui::Response {
        cx.response.clone()
    }
}

/// The style of a menu bar: the titles wear row faces; the bar itself draws the mnemonic underline in the
/// faces' label role and ink.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct MenuBarStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The mnemonic underline's ink and the role the underlined run is measured in.
    #[style(text)]
    text: TextLook,
}

/// A menu of the bar: its title and the body that fills the menu when it opens.
pub struct MenuTitle<'a> {
    label: Label,
    body: Box<dyn FnMut(&mut Ui) + 'a>,
}

impl<'a> MenuTitle<'a> {
    /// A menu titled `label` (a `&` marks the mnemonic) whose rows `body` shows.
    pub fn new(label: impl Into<Label>, body: impl FnMut(&mut Ui) + 'a) -> Self {
        MenuTitle { label: label.into(), body: Box::new(body) }
    }
}

/// The Help menu's search: the index of the menu that starts with the field, and the commands it searches.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct MenuHelp<'a> {
    menu: usize,
    entries: &'a [HelpEntry],
}

impl<'a> MenuHelp<'a> {
    /// A search field at the top of the menu `menu` (its position among the titles) over `entries`.
    pub fn new(menu: usize, entries: &'a [HelpEntry]) -> Self {
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

/// The help field's text.
#[derive(Clone, Debug, Default)]
struct Query(String);

/// What the bar published for `is_open` and `is_focused`.
#[derive(Clone, Copy, Debug)]
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
}

impl<'a> MenuBar<'a> {
    /// A bar named `label` with one title per menu, in reading order.
    pub fn new(label: impl Into<Label>, titles: Vec<MenuTitle<'a>>) -> Self {
        MenuBar { common: Common::new(), titles, help: None, keyboard: true, open: false }.label(label)
    }

    fn in_window(env: &EnvView<'_>) -> bool {
        env.shell().menu() == MenuPlacement::InWindow
    }

    /// Each title's text without its marker, and its mnemonic: the letter and its character index in the text. A Mac
    /// menu has no access letters.
    fn texts(&self, env: &EnvView<'_>) -> Vec<(String, Option<(char, usize)>)> {
        let labels: Vec<Label> = self.titles.iter().map(|title| env.resolve(&title.label)).collect();
        let refs: Vec<&str> = labels.iter().map(Label::as_str).collect();
        let marks = if env.shell().os.is_mac() { vec![None; refs.len()] } else { mnemonic::assign(&refs) };
        refs.iter().zip(marks).map(|(label, mark)| (mnemonic::marked(label).0, mark)).collect()
    }

    fn face(text: &str, lit: bool) -> TitleFace {
        TitleFace::new(Label::from(text.to_owned()), lit)
    }

    /// The text of the overflow title: "…" beside shown titles, "Menu" when it holds them all.
    fn more_text(words: &MenuWords, shown: usize) -> String {
        if shown == 0 { words.bar_all.as_str().to_owned() } else { words.bar_more.as_str().to_owned() }
    }

    /// Each title's width as its face measures, the face height, and the width of the "…" title.
    fn widths(&self, cx: &MeasureCx<'_, Self>) -> (Vec<f32>, f32, f32) {
        let texts = self.texts(&cx.env);
        let sizes: Vec<Vec2> = texts.iter().map(|(text, _)| cx.face(Self::face(text, false))).collect();
        let more = cx.face(Self::face(&Self::more_text(&cx.env.words::<MenuWords>(), 1), false));
        let height = sizes.iter().map(|size| size.y).fold(more.y, f32::max);
        (sizes.iter().map(|size| size.x).collect(), height, more.x)
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

    /// The help search's entries as `[label, English label, path]` texts.
    fn help_texts(entries: &[HelpEntry], env: &EnvView<'_>, separator: &str) -> Vec<[String; 3]> {
        entries
            .iter()
            .map(|entry| {
                let path: Vec<String> = entry.path().iter().map(|label| env.resolve(label).as_str().to_owned()).collect();
                let english = entry.english().map(|label| env.resolve(label).as_str().to_owned());
                [env.resolve(entry.label()).as_str().to_owned(), english.unwrap_or_default(), path.join(separator)]
            })
            .collect()
    }

    /// The line under the mnemonic letter of the title in `part`, shown while Alt is held.
    fn underline(ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>, part: PartRef, text: &str, at: usize) {
        let Some(cell) = cx.part_if_declared(part).map(|result| result.rect) else { return };
        let draw = ui.draw();
        let role = cx.style.text.role;
        let size_of = |run: &str| if run.is_empty() { Vec2::ZERO } else { draw.layout(&TextRun::new(run).role(role), f32::INFINITY).size() };
        let before: String = text.chars().take(at).collect();
        let letter: String = text.chars().skip(at).take(1).collect();
        let (whole, before, letter) = (size_of(text), size_of(&before).x, size_of(&letter).x);
        let line = cell.center().y + whole.y / 2.0;
        let from = if cx.flow.is_rtl() { cell.center().x + whole.x / 2.0 - before - letter } else { cell.center().x - whole.x / 2.0 + before };
        draw.stroke(&PaintShape::Path(vec![pos2(from, line), pos2(from + letter, line)]), StrokeWidth::Thin, cx.resolve(&cx.style.text.color));
    }
}

impl Component for MenuBar<'_> {
    const ROLE: Role = Role::MENU_BAR;
    const KEYS: crate::runtime::focus::KeyUse = crate::runtime::focus::KeyUse::of(crate::contract::Input::Pointer).with_moves();
    type Style = MenuBarStyle;
    type Output = MenuBarOutput;

    fn style(cx: &StyleCx<'_>, _: MenuBarStyleKey) -> MenuBarStyle {
        let ink = cx.color(cx.tokens.color.text.normal);
        MenuBarStyle { metrics: Metrics::new(), text: TextLook::new(TypeRole::Label, Stateful::new(ink)) }
    }

    /// Every title's width when the room allows it; the bar fills the row, so what is left over is room for the
    /// overflow.
    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        if !Self::in_window(&cx.env) {
            return Vec2::ZERO;
        }
        let (widths, height, _) = self.widths(cx);
        Vec2::new(widths.iter().sum(), height)
    }

    fn fills(&self) -> Option<Axis> {
        Some(Axis::X)
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
        let mut flow = Flow::new(rect, cx.flow.is_rtl());
        for (at, width) in widths.iter().take(fit.shown).enumerate() {
            let (cell, rest) = flow.take_start(*width);
            flow = rest;
            cx.part(TITLE.at(at as u64), cell).sense(Sense::CLICK | Sense::FOCUS).focusable().toggles_at("menu", at).on_press();
        }
        if fit.overflow {
            let (cell, _) = flow.take_start(more);
            cx.part(MORE, cell).sense(Sense::CLICK | Sense::FOCUS).focusable().toggles_at("menu", fit.shown).on_press();
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
                    let held = |(_, mark): &(String, Option<(char, usize)>)| mark.is_some_and(|(held, _)| held == letter);
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
        self.open = opened.is_some();
        Facts::<BarFact>::register(&cx.ctx, BarFact { open: self.open, focused: cx.is_focused() });
        *cx.memory::<BarState>() = state;
    }

    /// The open title's menu below it: the caller's body, with the help search above it in the Help menu. The
    /// overflow menu lists each hidden title as a submenu. Running a row returns focus to the component that held
    /// it when F10 or Alt took it for the bar, as any activation that closes the menus does (`focus::return_from_menus`).
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let found = declared(cx);
        let slots = Self::slots(found);
        let opened = Self::opened(cx, &slots, found);
        let words = cx.env.words::<MenuWords>();
        let folder = cx.env.folder();
        let texts = self.texts(&cx.env);
        let help = self.help;
        let help_rows = help
            .filter(|help| opened == Some(Open::Title(help.menu)))
            .map(|help| Self::help_texts(help.entries, &cx.env, cx.env.words::<CommonWords>().path_separator.as_str()));
        // The search starts empty each time the Help menu opens.
        let mut query = if help_rows.is_some() { cx.memory::<Query>().clone() } else { Query::default() };
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
                    .show(ui, |ui, _| match slot {
                        Open::Title(at) => {
                            if let (Some(help), Some(rows)) = (help.filter(|help| help.menu == *at), help_rows.as_ref()) {
                                // The field sits in a padded frame: its edge lines up with the row labels and it keeps clear of the first row.
                                Stack::new().pad(Pad::Regular).fill_width().show(ui, |ui, _| {
                                    TextField::search(words.bar_search.as_str().to_owned(), Bind::bound(&mut query.0))
                                        .placeholder(words.bar_search.as_str().to_owned())
                                        .show(ui);
                                });
                                let refs: Vec<[&str; 3]> =
                                    rows.iter().map(|[label, english, path]| [label.as_str(), english.as_str(), path.as_str()]).collect();
                                let mut seen: Vec<&CommandRef> = Vec::new();
                                let listed: Vec<usize> = search_order(&folder, &query.0, &refs)
                                    .into_iter()
                                    .filter(|at| {
                                        let command = help.entries[*at].command();
                                        let fresh = !seen.contains(&command);
                                        seen.push(command);
                                        fresh
                                    })
                                    .collect();
                                for at in &listed {
                                    let entry = &help.entries[*at];
                                    let row = MenuItem::command(entry.command().clone()).label(entry.label().clone());
                                    if entry.path().is_empty() {
                                        row.show(ui)
                                    } else {
                                        row.detail(rows[*at][2].clone()).show(ui)
                                    };
                                }
                                if listed.is_empty() && !query.0.trim().is_empty() {
                                    Text::new(words.bar_no_matches.as_str().to_owned()).show(ui);
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
            ui.face(part, Self::face(&text, lit));
            if let Open::Title(at) = slot
                && underlined
                && let Some((_, index)) = texts[at].1
            {
                Self::underline(ui, cx, part, &text, index);
            }
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> MenuBarOutput {
        let found = declared(&cx);
        let rects = Self::slots(found).into_iter().filter_map(|slot| cx.part_if_declared(Self::slot_part(slot)).map(|result| result.rect));
        let start = if cx.flow.is_rtl() { cx.rect.right_top() } else { cx.rect.left_top() };
        let titles = rects.reduce(Rect::union).unwrap_or_else(|| Rect::from_min_size(start, Vec2::ZERO));
        MenuBarOutput { response: cx.response.clone(), open: self.open, titles }
    }
}

//! Right-to-left menus: the row and popup helpers every menu surface should call, so a
//! right-to-left interface mirrors them like a native one.
//!
//! egui's `Style::right_to_left` (the suite's egui fork) mirrors the chrome — a popup's default
//! align, a submenu's open side, the arrow glyph — but not the bodies and row atoms: popup bodies
//! are pinned `top_down_justified(Align::Min)` and a plain [`egui::Button`] packs its atoms
//! left-to-right whatever the flag. These helpers do the mirroring in one place each:
//! [`popup_body`] (re-exported from [`craft_ui::direction`]) scopes a popup body,
//! [`menu_row_button_atom`] orders a row's atoms, and [`menu_row`], [`submenu_row`],
//! [`context_menu`] and [`menu_button`] are the one-call primitives every popup site uses.

use egui::containers::menu::{MenuState, SubMenu, SubMenuButton};
use egui::{Color32, Response, RichText, Ui, WidgetText};

pub use craft_ui::direction::popup_body;

/// A menu row's button atoms in reading order, whatever the direction: a plain [`egui::Button`]
/// lays its atoms out left-to-right whatever the direction flag, so `trailing` (a shortcut, a
/// submenu arrow) moves to the physical left via [`egui::Button::left_text`] in a right-to-left
/// row, and a row without one gets a leading grow so its label hugs the row's reading start.
pub fn menu_row_button_atom<'a>(text: WidgetText, trailing: Option<egui::Atom<'a>>, rtl: bool) -> egui::Button<'a> {
    match (rtl, trailing) {
        (true, Some(t)) => egui::Button::new(text).left_text(t),
        (true, None) => egui::Button::new((egui::Atom::grow(), text)),
        (false, Some(t)) => egui::Button::new(text).right_text(t),
        (false, None) => egui::Button::new(text),
    }
}

/// A menu row's button: `text` with its `shortcut` at the row's reading end, weak-tinted as
/// [`egui::Button::shortcut_text`] does. The tint is applied to the atom here, so the
/// left-to-right pixels are unchanged and the right-to-left atom order comes from
/// [`menu_row_button_atom`].
pub fn menu_row_button<'a>(text: WidgetText, shortcut: Option<&str>, rtl: bool) -> egui::Button<'a> {
    let trailing = shortcut.map(|s| egui::Atom::from(RichText::new(s).weak()));
    menu_row_button_atom(text, trailing, rtl)
}

/// A hand-built multi-atom row's atoms in reading order: as laid out left-to-right, reversed in a
/// right-to-left one (grow included).
pub fn mirror_atoms<'a>(mut atoms: egui::Atoms<'a>, rtl: bool) -> egui::Atoms<'a> {
    if rtl {
        atoms.reverse();
    }
    atoms
}

/// The checkmark menu rows draw in their gutter.
pub const CHECK_MARK: &str = "✓ ";

/// Width of the checkmark column every menu row reserves: the width of [`CHECK_MARK`] in the
/// button font, so checked, unchecked and plain rows (and submenu rows) all start their label at
/// the same x, whatever the script of the label.
pub fn check_gutter(ui: &Ui) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    ui.painter().layout_no_wrap(CHECK_MARK.to_owned(), font, Color32::PLACEHOLDER).size().x
}

/// Which end of a menu row the checkmark column sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GutterSide {
    /// The row's reading start: its left edge in a left-to-right row, its right edge in a
    /// right-to-left one, where native menus keep the column.
    ReadingStart,
    /// The row's reading end.
    ReadingEnd,
}

/// Splits a trailing ellipsis off a label, where a key or transient mark goes before it:
/// `("فتح", "…")`, `("Open", "...")`, or `("Open", "")` when there is none.
fn split_trailing_ellipsis(label: &str) -> (&str, &str) {
    label.strip_suffix('…').map(|base| (base, "…")).or_else(|| label.strip_suffix("...").map(|base| (base, "..."))).unwrap_or((label, ""))
}

/// A label split into its base and the transient `" (M)"` mark that goes before a trailing
/// ellipsis while the mnemonics show: `("فتح", " (M)…")`.
fn transient_mark(label: &str, m: char) -> (String, String) {
    let (base, tail) = split_trailing_ellipsis(label);
    (base.to_owned(), format!(" ({m}){tail}"))
}

/// A menu row's text: the label behind a fixed-width checkmark column ([`check_gutter`]) on
/// `side`, with the check mark in the column when `checked`, and the mnemonic underlined while
/// `mnemonic_visible` — a label without its letter then shows a transient `" (M)"` before a
/// trailing ellipsis (a right-to-left label keeps its English key discoverable while Alt is
/// held).
pub fn menu_row_text(ui: &Ui, checked: bool, label: &str, mnemonic: Option<char>, mnemonic_visible: bool, side: GutterSide) -> WidgetText {
    let rtl = craft_ui::direction::is_rtl(ui);
    let font = egui::TextStyle::Button.resolve(ui.style());
    let plain = egui::TextFormat { font_id: font, color: Color32::PLACEHOLDER, ..Default::default() };
    let underlined = egui::TextFormat { underline: egui::Stroke::new(1.0, Color32::PLACEHOLDER), ..plain.clone() };
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    // A right-to-left row reads from the right: the base direction is pinned so the column stays
    // on the reading-start edge whatever script the label starts with (a Latin or digit-first
    // label would infer a left-to-right paragraph and move it back).
    if rtl {
        job.direction = egui::epaint::text::TextDirection::RightToLeft;
    }
    let at = mnemonic.and_then(|m| label.char_indices().find(|(_, c)| c.to_ascii_uppercase() == m).map(|(i, c)| (i, c.len_utf8())));
    // The label's pieces in reading order: the underlined letter, or the transient mark when the
    // label can't hold it and the mnemonics show.
    let mut pieces: Vec<(String, egui::TextFormat)> = match at.filter(|_| mnemonic_visible) {
        Some((i, len)) => vec![
            (label.get(..i).unwrap_or_default().to_owned(), plain.clone()),
            (label.get(i..i + len).unwrap_or_default().to_owned(), underlined),
            (label.get(i + len..).unwrap_or_default().to_owned(), plain.clone()),
        ],
        None => match mnemonic.filter(|_| mnemonic_visible).map(|m| transient_mark(label, m)) {
            Some((base, mark)) => vec![(base, plain.clone()), (mark, plain.clone())],
            None => vec![(label.to_owned(), plain.clone())],
        },
    };
    pieces.retain(|(text, _)| !text.is_empty());
    // The label starts after the column: a checked row draws the mark in it, the others leave it
    // empty (a leading space of the same width, not padding characters whose width varies).
    let mut lead = check_gutter(ui);
    let append_part = |job: &mut egui::text::LayoutJob, text: &str, format: &egui::TextFormat, lead: &mut f32| {
        if text.is_empty() {
            return;
        }
        job.append(text, *lead, format.clone());
        *lead = 0.0;
    };
    let gutter_first = side == GutterSide::ReadingStart;
    if gutter_first && checked {
        job.append(CHECK_MARK, 0.0, plain.clone());
        lead = 0.0;
    }
    for (text, format) in &pieces {
        append_part(&mut job, text, format, &mut lead);
    }
    if !gutter_first && checked {
        job.append(CHECK_MARK, 0.0, plain.clone());
        lead = 0.0;
    }
    if job.sections.is_empty() {
        // An empty label still keeps the row as wide as its column.
        job.append("", lead, plain);
    }
    job.into()
}

/// One row of any popup menu (a context menu, a flyout, a dropdown): `label` behind the
/// fixed-width checkmark column at the row's reading start ([`menu_row_text`]), `shortcut` at the
/// reading end, in a button that mirrors under a right-to-left interface — a plain `ui.button`
/// lays its atoms left-to-right whatever the direction flag, so popup bodies use this instead.
/// A row that is neither checked nor in a right-to-left interface keeps a plain label,
/// pixel-identical to a plain button.
pub fn menu_row(ui: &mut Ui, label: &str, shortcut: Option<&str>, checked: bool, enabled: bool) -> Response {
    let rtl = craft_ui::direction::is_rtl(ui);
    let text = if checked || rtl { menu_row_text(ui, checked, label, None, false, GutterSide::ReadingStart) } else { RichText::new(label).into() };
    ui.add_enabled(enabled, menu_row_button(text, shortcut, rtl))
}

/// A submenu row, drawn as `ui.menu_button` draws it, except that a row scrolled out of view
/// doesn't open its submenu: egui opens a submenu whenever the pointer is over its row, clipped
/// or not, and a menu reopened scrolled down would have a hidden row under the title open on the
/// press, the release on the title then closing the whole menu. An open submenu (opened from
/// the keyboard, say) is drawn as usual. The body goes through [`popup_body`] like every menu
/// popup body. Returns the row and its submenu's content, when shown.
pub fn submenu_row<R>(ui: &mut Ui, title: WidgetText, content: impl FnOnce(&mut Ui) -> R) -> (Response, Option<R>) {
    let sub = SubMenu::id_from_widget_id(ui.next_auto_id());
    let open = MenuState::from_ui(ui, |state, _| state.open_item == Some(sub));
    let inactive = ui.style().visuals.widgets.inactive;
    if open {
        ui.style_mut().visuals.widgets.inactive = ui.style().visuals.widgets.open;
    }
    let response = ui.add(menu_row_button_atom(title, Some(SubMenuButton::arrow_atom(None)), craft_ui::direction::is_rtl(ui)));
    ui.style_mut().visuals.widgets.inactive = inactive;
    // The rect egui tests the pointer against (`SubMenu::show`), against the part that can be seen.
    let reach = response.rect.expand2(ui.spacing().item_spacing / 2.0);
    let hidden = ui.ctx().pointer_hover_pos().is_some_and(|p| reach.contains(p) && !ui.clip_rect().contains(p));
    if hidden && !open {
        return (response, None);
    }
    let inner = SubMenu::new().show(ui, &response, |ui| popup_body(ui, content));
    (response, inner.map(|r| r.inner))
}

/// A right-click context menu whose body goes through [`popup_body`], so it mirrors in a
/// right-to-left interface like every other popup; left-to-right it is the stock call.
pub fn context_menu<R>(resp: &Response, add_contents: impl FnOnce(&mut Ui) -> R) -> Option<egui::InnerResponse<R>> {
    egui::Popup::context_menu(resp).show(|ui| popup_body(ui, add_contents))
}

/// A menu button whose submenu body goes through [`popup_body`], so it mirrors in a right-to-left
/// interface like every other popup; left-to-right it is the stock call.
pub fn menu_button<R>(ui: &mut Ui, label: impl Into<WidgetText>, add_contents: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<Option<R>> {
    ui.menu_button(label, |ui| popup_body(ui, add_contents))
}

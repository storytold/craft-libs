//! The contents of the preferences window: a search field with its match count over one scrolling page of cards.

use super::model::{PageSpec, PrefValue, PreferencesModel};
use super::rows::{RowsOut, ShownRow, reset_button, show_row};
use super::search::{self, Listed};
use super::window::PreferencesState;
use super::words::PreferencesWords;
use crate::contract::{Bind, Pad, Props as _, Scroll, Show as _, ShowContainer as _};
use crate::display::{Shade, Text};
use crate::form::FieldGroup;
use crate::layout_components::{Row, Stack};
use crate::notices::{Notice, NoticeLayout};
use crate::runtime::env::{Env, Label, Units};
use crate::sections::Section;
use crate::style::{SurfaceKind, TypeRole};
use crate::text::TextField;
use craft_i18n::matcher::Matcher;
use craft_theme::variant::Tone;
use egui::Ui;

/// A group of the shown page: its title and the rows the search keeps.
struct ShownGroup {
    title: Option<Label>,
    rows: Vec<ShownRow>,
}

/// The page to draw, copied from the host's model so that the host can be borrowed again to draw custom rows.
pub(super) struct ShownPage {
    pub id: String,
    title: Label,
    custom: bool,
    footnote: Option<Label>,
    groups: Vec<ShownGroup>,
}

/// What the host's pages look like under the search: the cards to show, in order, and how many matches the search
/// found.
pub(super) struct Snapshot {
    pub shown: Vec<ShownPage>,
    pub searching: bool,
    /// The settings the search matched: rows whose text matches, and a listed page that has none counts itself.
    pub matches: usize,
}

/// The cards of `host` under `matcher`: every listed page, each with the rows the search keeps.
pub(super) fn snapshot<H: PreferencesModel + ?Sized>(host: &H, matcher: &Matcher) -> Snapshot {
    let pages = host.pages();
    let listed = search::list_pages(&pages, matcher);
    let searching = !matcher.is_empty();
    let mut matches = 0;
    let shown = listed
        .iter()
        .map(|l| {
            let page = &pages[l.index];
            if searching {
                let rows = page.groups.iter().flat_map(|g| &g.rows).filter(|r| r.visible && search::row_matches(matcher, r)).count();
                matches += rows.max(usize::from(!page.has_content() || l.all_rows && rows == 0));
            }
            shown_page(matcher, l, page)
        })
        .collect();
    Snapshot { shown, searching, matches }
}

fn shown_page(matcher: &Matcher, listed: &Listed, page: &PageSpec<'_>) -> ShownPage {
    let groups = page
        .groups
        .iter()
        .filter_map(|group| {
            let rows: Vec<ShownRow> = group.rows.iter().filter(|row| search::row_shown(matcher, listed, row)).map(ShownRow::of).collect();
            (!rows.is_empty()).then(|| ShownGroup { title: group.title.map(|title| Label::from(title.to_owned())), rows })
        })
        .collect();
    ShownPage {
        id: page.id.to_owned(),
        title: Label::from(page.title.to_owned()),
        custom: page.custom,
        footnote: page.footnote.map(|note| Label::from(note.to_owned())),
        groups,
    }
}

/// What the body asks of the window: the values the rows edited and whether Reset Section was pressed.
#[derive(Default)]
pub(super) struct BodyOut {
    pub rows: RowsOut,
    /// The page whose Reset Section was pressed.
    pub reset: Option<String>,
    pub custom_changed: bool,
}

impl BodyOut {
    pub fn edits(&mut self) -> Vec<(String, PrefValue)> {
        std::mem::take(&mut self.rows.edits)
    }
}

/// Shows the search field with its match count over the cards: one scrolling page, a card per page. `state.search`
/// changes as the user types; a `state.selected` the page has not scrolled to yet is brought into view once.
pub(super) fn show_body<H: PreferencesModel + ?Sized>(
    ui: &mut Ui,
    host: &mut H,
    snap: &Snapshot,
    state: &mut PreferencesState,
    words: &PreferencesWords,
    units: &Units,
    out: &mut BodyOut,
) {
    Row::new().fill_width().show(ui, |ui, _| {
        TextField::search(words.search_label.clone(), Bind::bound(&mut state.search)).placeholder(words.search_placeholder.clone()).show(ui);
        if snap.searching {
            let env = Env::of(ui);
            let n = env.format().integer(snap.matches as u64);
            let text = if snap.matches == 1 { words.match_one.fill(&[("n", &n)]) } else { words.match_other.fill(&[("n", &n)]) };
            let shade = if snap.matches == 0 { Shade::Faint } else { Shade::Muted };
            Text::new(text).role(TypeRole::Caption).shade(shade).show(ui);
        }
    });
    if snap.shown.is_empty() {
        Notice::new(Tone::Info, words.no_matches.clone()).layout(NoticeLayout::Centred).show(ui);
        return;
    }
    // The scroll goes to the page the host named once, when it is first shown or changes.
    let jump = !state.selected.is_empty() && state.scrolled_to != state.selected && snap.shown.iter().any(|page| page.id == state.selected);
    Stack::new().fill().scroll(Scroll::vertical()).label(words.pages_label.clone()).show(ui, |ui, _| {
        for page in &snap.shown {
            let top = ui.cursor().min;
            show_card(ui, host, page, snap.searching, words, units, out);
            if jump && page.id == state.selected {
                ui.scroll_to_rect(egui::Rect::from_min_size(top, egui::vec2(1.0, 1.0)), Some(egui::Align::TOP));
            }
        }
    });
    if jump {
        state.scrolled_to = state.selected.clone();
    }
}

/// One page as a card: its title, groups, footnote and Reset Section.
fn show_card<H: PreferencesModel + ?Sized>(
    ui: &mut Ui,
    host: &mut H,
    page: &ShownPage,
    searching: bool,
    words: &PreferencesWords,
    units: &Units,
    out: &mut BodyOut,
) {
    Stack::new().fill_width().surface(SurfaceKind::Raised).pad(Pad::Regular).label(page.title.clone()).id_salt(page.id.as_str()).show(ui, |ui, _| {
        Text::new(page.title.clone()).role(TypeRole::Heading).show(ui);
        if page.custom {
            Stack::new().show(ui, |ui, _| out.custom_changed |= host.custom_page(ui, &page.id));
        } else if page.groups.is_empty() && !searching {
            Notice::new(Tone::Info, words.empty_page.clone()).layout(NoticeLayout::Centred).show(ui);
        }
        for (at, group) in page.groups.iter().enumerate() {
            let mut rows = |ui: &mut Ui| {
                let name = group.title.clone().unwrap_or_else(|| page.title.clone());
                FieldGroup::new(name).show(ui, |ui, _| {
                    for row in &group.rows {
                        show_row(ui, host, row, units, &mut out.rows);
                    }
                });
            };
            match &group.title {
                Some(title) => {
                    Section::fixed(title.clone()).id_salt(at).show(ui, |ui, _| rows(ui));
                }
                None => rows(ui),
            }
        }
        if let Some(note) = &page.footnote {
            Text::new(note.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
        }
        if !searching && (page.custom || !page.groups.is_empty()) && reset_button(ui, words.reset_section.clone()) {
            out.reset = Some(page.id.clone());
        }
    });
}

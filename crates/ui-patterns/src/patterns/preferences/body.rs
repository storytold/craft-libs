//! The contents of the preferences window: a search field over a page list and the shown page.

use super::model::{PageSpec, PrefValue, PreferencesModel};
use super::rows::{RowsOut, ShownRow, reset_button, show_row};
use super::search::{self, Listed};
use super::window::PreferencesState;
use super::words::PreferencesWords;
use crate::contract::{
    ActivateOn, Bind, CollectionEvent, Props as _, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing,
};
use crate::display::{Shade, Text};
use crate::form::FieldGroup;
use crate::layout_components::{Row, Stack};
use crate::list::{List, ListRow, RowInk};
use crate::notices::{Notice, NoticeLayout};
use crate::runtime::env::{Label, Units};
use crate::sections::Section;
use crate::style::TypeRole;
use crate::text::TextField;
use craft_behavior::{DisplayOrder, ItemSource, Selection, SelectionChange};
use craft_i18n::matcher::Matcher;
use craft_theme::variant::Tone;
use egui::Ui;

/// One entry of the page list.
pub(super) struct PageEntry {
    pub id: String,
    pub title: Label,
    pub dimmed: bool,
}

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

/// What the host's pages look like under the search: the pages to list and the page to show.
pub(super) struct Snapshot {
    pub list: Vec<PageEntry>,
    pub shown: Option<ShownPage>,
    pub searching: bool,
}

/// The listed pages of `host` under `matcher`, and the part of the selected page the search keeps (the first listed
/// page when the search hides the selected one).
pub(super) fn snapshot<H: PreferencesModel + ?Sized>(host: &H, matcher: &Matcher, selected: &str) -> Snapshot {
    let pages = host.pages();
    let listed = search::list_pages(&pages, matcher);
    let list = listed
        .iter()
        .map(|l| PageEntry { id: pages[l.index].id.to_owned(), title: Label::from(pages[l.index].title.to_owned()), dimmed: l.dimmed })
        .collect();
    let shown = search::shown_position(&listed, &pages, selected).map(|at| shown_page(matcher, &listed[at], &pages[listed[at].index]));
    Snapshot { list, shown, searching: !matcher.is_empty() }
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
    pub reset: bool,
}

impl BodyOut {
    pub fn edits(&mut self) -> Vec<(String, PrefValue)> {
        std::mem::take(&mut self.rows.edits)
    }
}

/// The page list's rows: one per listed page, keyed by the page id.
struct Pages<'a>(&'a [PageEntry]);

impl DisplayOrder<String> for Pages<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> String {
        self.0[at].id.clone()
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        Some(std::borrow::Cow::Borrowed(self.0[at].title.as_str()))
    }
}

impl ItemSource<String> for Pages<'_> {}

/// Shows the search field, the page list and the page. `state.search` and `state.selected` change as the user types
/// and picks.
pub(super) fn show_body<H: PreferencesModel + ?Sized>(
    ui: &mut Ui,
    host: &mut H,
    snap: &Snapshot,
    state: &mut PreferencesState,
    words: &PreferencesWords,
    units: &Units,
    out: &mut BodyOut,
) {
    let list_id = ui.scope_id().with("pages");
    TextField::search(words.search_label.clone(), Bind::bound(&mut state.search))
        .placeholder(words.search_placeholder.clone())
        .drives(list_id)
        .show(ui);
    let Some(page) = &snap.shown else {
        Notice::new(Tone::Info, words.no_matches.clone()).layout(NoticeLayout::Centred).show(ui);
        return;
    };
    Row::new().fill().show(ui, |ui, _| {
        let current = Selection::single(page.id.clone());
        let source = Pages(&snap.list);
        let listed = List::<String>::new(words.pages_label.clone())
            .id(list_id)
            .select(SelectMode::Single)
            .activate(ActivateOn::Click)
            .allow_empty(false)
            .header(false)
            .sizing(Sizing::Content)
            .selection(Bind::given(current))
            .show_items(ui, &source, &mut |key, _| {
                let entry = snap.list.iter().find(|entry| entry.id == *key);
                let row = ListRow::new(entry.map_or_else(Label::default, |entry| entry.title.clone()));
                if entry.is_some_and(|entry| entry.dimmed) { row.ink(RowInk::Dimmed) } else { row }
            });
        for event in listed.events {
            if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                && let Some(key) = keys.into_iter().last()
            {
                state.selected = key;
            }
        }
        Stack::new()
            .fill()
            .scroll(Scroll::vertical())
            .label(page.title.clone())
            .show(ui, |ui, _| show_page(ui, host, page, snap.searching, words, units, out));
    });
}

/// The page's title, groups, footnote and Reset Section.
fn show_page<H: PreferencesModel + ?Sized>(
    ui: &mut Ui,
    host: &mut H,
    page: &ShownPage,
    searching: bool,
    words: &PreferencesWords,
    units: &Units,
    out: &mut BodyOut,
) {
    Text::new(page.title.clone()).role(TypeRole::Title).show(ui);
    if page.custom {
        Stack::new().show(ui, |ui, _| host.custom_page(ui, &page.id));
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
        out.reset = true;
    }
}

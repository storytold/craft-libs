//! `PreferencesPage`: the preferences body without a window, applying each value at once.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::model::{PageSpec, PreferencesModel};
use super::rows::{RowsOut, ShownRow, reset_button, show_row};
use super::search::{self, Listed};
use super::window::PreferencesState;
use super::words::PreferencesWords;
use crate::contract::{
    ActivateOn, Bind, CollectionEvent, NoValue, Outcome, Pattern, Props, Scroll, SelectMode, Show as _, ShowCollection as _, ShowContainer as _,
    Sizing,
};
use crate::display::{Divider, Shade, Text};
use crate::form::FieldGroup;
use crate::layout_components::{Row, Stack};
use crate::list::{List, ListRow, RowInk};
use crate::notices::{Notice, NoticeLayout};
use crate::runtime::env::{Env, Label, Units};
use crate::runtime::memory::Remember as _;
use crate::runtime::requests::ColorRequest;
use crate::sections::Section;
use crate::style::TypeRole;
use crate::text::TextField;
use craft_behavior::{DisplayOrder, EditEvent, ItemSource, Mixed, Selection, SelectionChange};
use craft_i18n::matcher::Matcher;
use craft_theme::variant::Tone;
use egui::{Response, Ui};
use std::sync::Arc;

/// A preferences page that applies each value at once, with search and Reset Section. [`PreferencesWindow`] shows its
/// body through this page, so one page path remains.
///
/// - **Pages and search.** The page lists the host's pages that match the search (the empty ones dimmed) and shows
///   the picked one, or the first listed when the search hides it. Up, Down and Enter in the search field go to the
///   page list.
/// - **Edits.** Each edit reaches [`PreferencesModel::set`] in the pass it lands in, and Reset Section reaches
///   [`PreferencesModel::reset_page`]; the page has no OK, Cancel or Apply of its own.
/// - **State.** The search, the picked page and the error live in the bound [`PreferencesState`]. A page that has just
///   appeared starts with no search and no error; an edit or a Reset clears the error.
///
/// [`PreferencesWindow`]: super::PreferencesWindow
#[derive(Pattern, Props)]
pub struct PreferencesPage<'a> {
    state: Bind<'a, PreferencesState>,
}

/// What a [`PreferencesPage`] reports: its response, the colour the host should pick and the page shown.
#[derive(Debug)]
#[non_exhaustive]
pub struct PreferencesPageOutput {
    /// The page's response.
    pub response: Response,
    /// A colour row asked for the picker; the host answers through [`PreferencesModel::set_color`].
    pub pick_color: Option<ColorRequest>,
    /// The id of the page shown, `None` when the search lists no page.
    pub page: Option<String>,
}

impl Outcome for PreferencesPageOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What the rows of one run ask of the host: the values they edited and whether Reset Section was pressed.
#[derive(Default)]
struct BodyOut {
    rows: RowsOut,
    reset: bool,
}

/// What one run of the page's body did: the state it read and the state it leaves, and what the rows asked for.
struct PageRun {
    before: PreferencesState,
    work: PreferencesState,
    out: BodyOut,
    page: Option<String>,
}

impl<'a> PreferencesPage<'a> {
    /// A page over the host's `state`.
    pub fn new(state: Bind<'a, PreferencesState>) -> Self {
        PreferencesPage { state }
    }

    /// Shows the page for this pass over `host`. The values the rows edit and Reset Section reach the host once, after
    /// the body's real run, so a measuring run of the body applies nothing.
    pub fn show<H: PreferencesModel + ?Sized>(self, ui: &mut Ui, host: &mut H) -> PreferencesPageOutput {
        let PreferencesPage { mut state } = self;
        let env = Env::of(ui);
        let words = env.words::<PreferencesWords>();
        let units = env.units();
        let title: Arc<str> = words.title.as_str().into();
        let mut shown = Stack::new().fill().show(ui, |ui, cx| {
            let before = match state.reborrow().read(cx) {
                Some(Mixed::Same(held)) => held,
                _ => PreferencesState::default(),
            };
            let mut work = before.clone();
            if cx.appeared() {
                work.search.clear();
                work.error = None;
            }
            let matcher = Matcher::new(&env.folder(), work.search.trim());
            let snap = snapshot(&*host, &matcher, &work.selected);
            let mut out = BodyOut::default();
            show_body(ui, &mut *host, &snap, &mut work, &words, &units, &mut out);
            PageRun { before, work, out, page: snap.shown.map(|page| page.id) }
        });
        let pick_color = shown
            .settle(|cx, run| {
                let edits = std::mem::take(&mut run.out.rows.edits);
                if !edits.is_empty() || run.out.reset {
                    run.work.error = None;
                }
                for (key, value) in edits {
                    host.set(&key, value);
                }
                if run.out.reset
                    && let Some(page) = &run.page
                {
                    host.reset_page(page);
                }
                if run.work != run.before {
                    let event = EditEvent::set(Mixed::Same(run.before.clone()), run.work.clone(), title.clone(), None);
                    let _ = state.reborrow().write(cx, &event);
                }
                run.out.rows.pick_color.take()
            })
            .flatten();
        let page = shown.inner.and_then(|run| run.page);
        PreferencesPageOutput { response: shown.output, pick_color, page }
    }
}

/// One entry of the page list.
struct PageEntry {
    id: String,
    title: Label,
    dimmed: bool,
}

/// A group of the shown page: its title and the rows the search keeps.
struct ShownGroup {
    title: Option<Label>,
    rows: Vec<ShownRow>,
}

/// The page to draw, copied from the host's model so that the host can be borrowed again to draw custom rows.
struct ShownPage {
    id: String,
    title: Label,
    custom: bool,
    footnote: Option<Label>,
    groups: Vec<ShownGroup>,
}

/// What the host's pages look like under the search: the pages to list and the page to show.
struct Snapshot {
    list: Vec<PageEntry>,
    shown: Option<ShownPage>,
    searching: bool,
}

/// The listed pages of `host` under `matcher`, and the part of the selected page the search keeps (the first listed
/// page when the search hides the selected one).
fn snapshot<H: PreferencesModel + ?Sized>(host: &H, matcher: &Matcher, selected: &str) -> Snapshot {
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
/// and picks; the rows' edits and a press of Reset Section go to `out`.
fn show_body<H: PreferencesModel + ?Sized>(
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
        Divider::vertical().show(ui);
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
    if !page.groups.is_empty() {
        FieldGroup::new(page.title.clone()).show(ui, |ui, _| {
            for (at, group) in page.groups.iter().enumerate() {
                let mut rows = |ui: &mut Ui| {
                    Stack::new().id_salt(at).show(ui, |ui, _| {
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
        });
    }
    if let Some(note) = &page.footnote {
        Text::new(note.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    if !searching && (page.custom || !page.groups.is_empty()) && reset_button(ui, words.reset_section.clone()) {
        out.reset = true;
    }
}

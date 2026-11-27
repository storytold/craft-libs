//! `PreferencesPage`: the preferences body without a window, applying each value at once.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use super::model::{PageSpec, PreferencesModel};
use super::rows::{RowsOut, ShownRow, reset_button, show_row};
use super::search::{self, Listed};
use super::window::PreferencesState;
use super::words::PreferencesWords;
use crate::contract::{Bind, NoValue, Outcome, Pad, Pattern, Props, Scroll, Show as _, ShowContainer as _};
use crate::display::{Shade, Text};
use crate::form::FieldGroup;
use crate::layout_components::{Row, Stack};
use crate::list::{ListRow, RowInk};
use crate::notices::{Notice, NoticeLayout};
use crate::patterns::form_dialogs::SectionList;
use crate::runtime::env::{Env, Label, Units};
use crate::runtime::memory::Remember as _;
use crate::runtime::requests::ColorRequest;
use crate::sections::{GroupBox, Section};
use crate::style::{SurfaceKind, TypeRole};
use crate::text::TextField;
use craft_behavior::{DisplayOrder, EditEvent, ItemSource, Mixed};
use craft_i18n::matcher::Matcher;
use craft_theme::variant::Tone;
use egui::{Response, Ui};
use std::sync::Arc;

/// How a [`PreferencesPage`] lays out the host's pages. Both keep the same search, rows, edits and Reset Section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PreferencesLayout {
    /// A page list beside the shown page, each row's control in a field named by the row: WordCraft's preferences.
    #[default]
    Paged,
    /// One scrolling column of cards, a card per listed page with its groups, each row's label then its control on one
    /// line and a short choice as a segmented control ([`RowSpec::pills`](super::RowSpec::pills)); the search field
    /// shows its match count beside it, and the column brings the picked page into view once: PdfCraft's preferences.
    Cards,
    /// [`Paged`](Self::Paged) as a desktop preferences dialog draws it: the page list in a frame as tall as the page,
    /// no heading over the page, and each titled group a framed group box with its title on the frame; the window puts
    /// OK, Cancel, Prev and Next in a column at the end edge: PhotoCraft's preferences.
    Classic,
}

/// A preferences page that applies each value at once, with search and Reset Section. [`PreferencesWindow`] shows its
/// body through this page, so one page path remains.
///
/// - **Pages and search.** The page lists the host's pages that match the search (the empty ones dimmed) and shows
///   the picked one, or the first listed when the search hides it. Up, Down and Enter in the search field go to the
///   page list. With [`PreferencesLayout::Cards`] every listed page shows as a card in one scrolling column instead,
///   the search field shows how many settings match, and a picked page the column has not scrolled to is brought
///   into view once.
/// - **Edits.** Each edit reaches [`PreferencesModel::set`] in the pass it lands in, and Reset Section reaches
///   [`PreferencesModel::reset_page`]; the page has no OK, Cancel or Apply of its own.
/// - **State.** The search, the picked page and the error live in the bound [`PreferencesState`]. A page that has just
///   appeared starts with no search and no error; an edit, a custom row's or page's change, or a Reset clears the
///   error.
///
/// [`PreferencesWindow`]: super::PreferencesWindow
#[derive(Pattern, Props)]
pub struct PreferencesPage<'a> {
    state: Bind<'a, PreferencesState>,
    /// Lays the pages out as `layout` says.
    #[prop]
    layout: PreferencesLayout,
}

/// What a [`PreferencesPage`] reports: its response, the colour the host should pick and the page shown.
#[derive(Debug)]
#[non_exhaustive]
pub struct PreferencesPageOutput {
    /// The page's response.
    pub response: Response,
    /// A colour row asked for the picker; the host answers through [`PreferencesModel::set_color`].
    pub pick_color: Option<ColorRequest>,
    /// The id of the page shown, `None` when the search lists no page. With [`PreferencesLayout::Cards`], the picked
    /// page while the search lists it, else the first card.
    pub page: Option<String>,
    /// The ids of the pages the search lists, in order: the pages Prev and Next step through.
    pub pages: Vec<String>,
}

impl Outcome for PreferencesPageOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What the rows of one run ask of the host: the values they edited, the page whose Reset Section was pressed and
/// whether a custom page changed the draft.
#[derive(Default)]
struct BodyOut {
    rows: RowsOut,
    reset: Option<String>,
    custom_changed: bool,
}

/// What one run of the page's body did: the state it read and the state it leaves, and what the rows asked for.
struct PageRun {
    before: PreferencesState,
    work: PreferencesState,
    out: BodyOut,
    page: Option<String>,
    listed: Vec<String>,
}

impl<'a> PreferencesPage<'a> {
    /// A page over the host's `state`, laid out as [`PreferencesLayout::Paged`].
    pub fn new(state: Bind<'a, PreferencesState>) -> Self {
        PreferencesPage { state, layout: PreferencesLayout::default() }
    }

    /// Shows the page for this pass over `host`. The values the rows edit and Reset Section reach the host once, after
    /// the body's real run, so a measuring run of the body applies nothing.
    pub fn show<H: PreferencesModel + ?Sized>(self, ui: &mut Ui, host: &mut H) -> PreferencesPageOutput {
        let PreferencesPage { mut state, layout } = self;
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
            let mut out = BodyOut::default();
            let listed: Vec<String>;
            let page = match layout {
                PreferencesLayout::Paged | PreferencesLayout::Classic => {
                    let snap = snapshot(&*host, &matcher, &work.selected);
                    show_body(ui, &mut *host, &snap, &mut work, &Look { words: &words, units: &units, layout }, &mut out);
                    listed = snap.list.iter().map(|entry| entry.id.clone()).collect();
                    snap.shown.map(|page| page.id)
                }
                PreferencesLayout::Cards => {
                    let snap = cards_snapshot(&*host, &matcher);
                    show_cards(ui, &mut *host, &snap, &mut work, &words, &units, &mut out);
                    listed = snap.shown.iter().map(|page| page.id.clone()).collect();
                    let shown = snap.shown.iter().find(|page| page.id == work.selected).or(snap.shown.first());
                    shown.map(|page| page.id.clone())
                }
            };
            PageRun { before, work, out, page, listed }
        });
        let pick_color = shown
            .settle(|cx, run| {
                let edits = std::mem::take(&mut run.out.rows.edits);
                if !edits.is_empty() || run.out.reset.is_some() || run.out.custom_changed || run.out.rows.custom_changed {
                    run.work.error = None;
                }
                for (key, value) in edits {
                    host.set(&key, value);
                }
                if let Some(page) = &run.out.reset {
                    host.reset_page(page);
                }
                if run.work != run.before {
                    let event = EditEvent::set(Mixed::Same(run.before.clone()), run.work.clone(), title.clone(), None);
                    let _ = state.reborrow().write(cx, &event);
                }
                run.out.rows.pick_color.take()
            })
            .flatten();
        let (page, pages) = shown.inner.map_or((None, Vec::new()), |run| (run.page, run.listed));
        PreferencesPageOutput { response: shown.output, pick_color, page, pages }
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

/// What the host's pages look like under the search as cards: the cards to show, in order, and how many settings the
/// search matched.
struct CardsSnapshot {
    shown: Vec<ShownPage>,
    searching: bool,
    /// The settings the search matched: rows whose text matches, and a listed page that has none counts itself.
    matches: usize,
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

/// The cards of `host` under `matcher`: every listed page, each with the rows the search keeps.
fn cards_snapshot<H: PreferencesModel + ?Sized>(host: &H, matcher: &Matcher) -> CardsSnapshot {
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
    CardsSnapshot { shown, searching, matches }
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

/// What a paged body draws with: the words, the units and the layout.
#[derive(Clone, Copy)]
struct Look<'a> {
    words: &'a PreferencesWords,
    units: &'a Units,
    layout: PreferencesLayout,
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
    look: &Look<'_>,
    out: &mut BodyOut,
) {
    let words = look.words;
    let list_id = ui.scope_id().with("pages");
    TextField::search(words.search_label.clone(), Bind::bound(&mut state.search))
        .placeholder(words.search_placeholder.clone())
        .drives(list_id)
        .show(ui);
    let Some(page) = &snap.shown else {
        Notice::new(Tone::Info, words.no_matches.clone()).layout(NoticeLayout::Centred).show(ui);
        return;
    };
    let source = Pages(&snap.list);
    let mut sections = SectionList::new(words.pages_label.clone(), &source, page.id.clone()).id(list_id).body_label(page.title.clone());
    if look.layout == PreferencesLayout::Classic {
        sections = sections.framed();
    }
    let picked = sections.show(
        ui,
        |key| {
            let entry = snap.list.iter().find(|entry| entry.id == *key);
            let row = ListRow::new(entry.map_or_else(Label::default, |entry| entry.title.clone()));
            if entry.is_some_and(|entry| entry.dimmed) { row.ink(RowInk::Dimmed) } else { row }
        },
        |ui| show_page(ui, host, page, snap.searching, look, out),
    );
    if let Some(key) = picked {
        state.selected = key;
    }
}

/// The page's title, groups, footnote and Reset Section.
fn show_page<H: PreferencesModel + ?Sized>(ui: &mut Ui, host: &mut H, page: &ShownPage, searching: bool, look: &Look<'_>, out: &mut BodyOut) {
    let Look { words, units, layout } = *look;
    let classic = layout == PreferencesLayout::Classic;
    if !classic {
        Text::new(page.title.clone()).role(TypeRole::Title).show(ui);
    }
    if page.custom {
        Stack::new().show(ui, |ui, _| out.custom_changed |= host.custom_page(ui, &page.id));
    } else if page.groups.is_empty() && !searching {
        Notice::new(Tone::Info, words.empty_page.clone()).layout(NoticeLayout::Centred).show(ui);
    }
    if !page.groups.is_empty() {
        FieldGroup::new(page.title.clone()).show(ui, |ui, _| {
            for (at, group) in page.groups.iter().enumerate() {
                let mut rows = |ui: &mut Ui| {
                    Stack::new().id_salt(at).show(ui, |ui, _| {
                        for row in &group.rows {
                            show_row(ui, host, row, units, PreferencesLayout::Paged, &mut out.rows);
                        }
                    });
                };
                match &group.title {
                    Some(title) if classic => {
                        GroupBox::new(title.clone(), None).id_salt(at).show(ui, |ui, _| rows(ui));
                    }
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
        out.reset = Some(page.id.clone());
    }
}

/// Shows the search field with its match count over the cards: one scrolling column, a card per page. `state.search`
/// changes as the user types; a `state.selected` the column has not scrolled to yet is brought into view once.
fn show_cards<H: PreferencesModel + ?Sized>(
    ui: &mut Ui,
    host: &mut H,
    snap: &CardsSnapshot,
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
    Stack::new().fill().scroll(Scroll::vertical()).label(words.cards_label.clone()).show(ui, |ui, _| {
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
                        show_row(ui, host, row, units, PreferencesLayout::Cards, &mut out.rows);
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

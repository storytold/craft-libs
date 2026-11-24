//! `CommandPalette` (24.4): a search over the app's commands, in one centred modal box or in place under a search box.

use super::words::PanelWords;

use crate::contract::overlay::{Dismiss, Outside, OverlayFit, ViewportShare};
use crate::contract::{
    Bind, BodyCx, CollectionEvent, Overlay, Pattern, Politeness, Props, Show as _, ShowCollection as _, ShowContainer as _, Sizing, announce, clicked,
};
use crate::display::{Shade, Text};
use crate::layout_components::{Row, Stack};
use crate::list::{List, ListRow};
use crate::menu::search::{HelpEntry, SearchIndex, rank};
use crate::popover::{Popover, PopupList};
use crate::primitives::IconSpec;
use crate::runtime::commands::{self, CommandRef, FireSource, ResolvedCommand};
use crate::runtime::env::{Env, Label, WordTemplate};
use crate::runtime::focus::{self, FocusTarget};
use crate::runtime::memory::Remember as _;
use crate::runtime::overlay::{self, CloseReason};
use crate::runtime::words::common::CommonWords;
use crate::style::TypeRole;
use crate::text::TextField;
use callout::Pointed;
use craft_behavior::{DisplayOrder, EditEvent, IconId, ItemSource, Mixed, Selection};
use egui::{Id, Rect, Response, Ui};
use std::borrow::Cow;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

mod callout;
#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The most commands the empty query lists when the app has run none.
const FIRST_ROWS: usize = 12;
/// The commands the empty query lists, newest first, of those remembered.
pub const RECENT: usize = 8;
/// The commands `CommandPalette::remember` keeps.
pub const RECENT_KEPT: usize = 20;
/// The most results a query lists.
pub const RESULTS: usize = 50;
/// The box's widest share of the window; its height is capped at the same share of the window's height.
const SHARE: ViewportShare = ViewportShare::new(0.6, 0.7);

/// What the palette keeps in its container's memory while it is open: the query, the active row, the number of
/// results last announced and the stamp of the query the list was last built for.
#[derive(Clone, Default)]
struct Session {
    query: String,
    selection: Selection<Line>,
    announced: Option<usize>,
    stamp: u64,
}

/// The key of a row of the list: the suggested heading, a command by its position in the app's entries, the line that
/// sets the fallback row off from the commands, or the fallback row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Line {
    #[default]
    Heading,
    Entry(usize),
    Separator,
    Fallback,
}

/// What the dialog's footer says; the body decides it in the pass it lists, the footer reads it.
enum Status {
    /// Nothing to say: an empty query lists the recent commands.
    Quiet,
    /// The query matches no command.
    Empty,
    /// Why the active row does not run, or where its control is.
    Line(String),
    /// `shown` of the `total` matches are listed.
    Count { shown: usize, total: usize },
}

/// Where a [`CommandPalette`] shows its search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PaletteFace {
    /// A centred box.
    #[default]
    Dialog,
    /// A search box where it is laid out (the title bar) that opens to the field and its results in place.
    InPlace,
}

/// What a palette reports: its response, the command picked in this pass and how it closed.
#[derive(Debug)]
#[non_exhaustive]
pub struct PaletteOutput {
    /// The box's response.
    pub response: Response,
    /// The command the user picked in this pass. The palette has closed and fired it as `FireSource::Palette`.
    pub picked: Option<CommandRef>,
    /// How the palette closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The query the fallback row handed back, when the user picked it in this pass.
    pub fallback: Option<String>,
    /// The disabled command of the active row Enter was pressed on in this pass: nothing ran and the palette stays
    /// open, its footer saying why. An app that closes the palette there and reports the reason itself reads it.
    pub refused: Option<CommandRef>,
}

impl crate::contract::Outcome for PaletteOutput {
    type Value = crate::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A search over the app's commands: a search field over the matching commands, in a centred modal box or in place.
///
/// - Entries: [`HelpEntry`]s, found by label, English label, menu path, command id or words and ranked by [`rank`]
///   over the app's [`SearchIndex`], at most [`RESULTS`] of them; the active result keeps the last row when a better
///   one would push it past the end. A row is one line: the icon, the label, the entry's home or else its menu path
///   (dropped when the label would get too narrow) and the command's chord at the end. A command is enabled as the
///   window under the palette has it: the palette closes before the command runs, so its own modal layer blocks none.
///   A disabled command is listed and does not run.
/// - Faces: [`PaletteFace::Dialog`] is a centred modal box. [`PaletteFace::InPlace`] is a search box where it is laid
///   out, showing the `placeholder`; it opens to the field with the key focus in the pass that opens it and lists the
///   results in a popup under the field. The in-place field is the palette's face, so it is named by the palette's
///   `label`, closed and open; the dialog face's field, inside a box named `label`, keeps the generic search name.
/// - Empty query: with `suggested`, its heading and those commands; else the commands last run, newest first (the
///   `recents` binding), else the first entries.
/// - Fallback: with `fallback`, a last row follows the matches of a query, or stands alone when nothing matches. Picking
///   it closes the palette and reports the query in the output's `fallback`; no command runs.
/// - Keys: the field keeps the focus; Up, Down, Page Up, Page Down move the active row (Up and Down wrap at the ends),
///   Enter picks it, Escape clears the query and, on an empty query, closes the box; a press outside closes it. The
///   number of results is announced politely.
/// - Callout (dialog face): the control that shows the active row's command gets a ring and a box beside it; when the
///   control is out of view or under the palette, the footer names the entry's home instead. The footer also says why
///   a disabled active row does not run, else how many results there are. The in-place list says when nothing matches.
/// - Picking: the box closes, the command fires as `FireSource::Palette` (the app receives it as it receives a menu's)
///   and goes first in `recents`; [`CommandPalette::remember`] is the rule. The output's `picked` names it. Without a
///   binding the palette keeps `recents` for the session. The dialog face returns the focus to where it was before the
///   palette opened; the in-place face leaves it where the host puts it after the popup closes.
#[derive(Pattern, Props)]
pub struct CommandPalette<'a> {
    label: Label,
    entries: &'a [HelpEntry],
    index: &'a SearchIndex,
    open: Bind<'a, bool>,
    /// The pick counts that rank the results, 0 for a command never picked.
    #[prop]
    uses: Option<&'a dyn Fn(&CommandRef) -> u32>,
    /// The commands last run, newest first.
    #[prop]
    recents: Bind<'a, Vec<CommandRef>>,
    /// Where the search shows: a centred box or in place.
    #[prop]
    face: PaletteFace,
    /// The hint the in-place search box shows while it is closed.
    #[prop(into)]
    placeholder: Option<Label>,
    /// The rows listed on an empty query, under a heading.
    #[prop(into)]
    suggested: Option<(Label, Vec<CommandRef>)>,
    /// The row that hands the query back; its text is the template filled with the query as `{text}`.
    #[prop]
    fallback: Option<WordTemplate>,
}

impl<'a> CommandPalette<'a> {
    /// A palette named `label` over `entries`, ranked through `index` (which the app builds when its entries change),
    /// open while `open` is true.
    pub fn new(label: impl Into<Label>, entries: &'a [HelpEntry], index: &'a SearchIndex, open: Bind<'a, bool>) -> Self {
        CommandPalette {
            label: label.into(),
            entries,
            index,
            open,
            uses: None,
            recents: Bind::kept(),
            face: PaletteFace::default(),
            placeholder: None,
            suggested: None,
            fallback: None,
        }
    }

    /// Puts `command` first in `recents`, once, keeping the newest `RECENT_KEPT`.
    pub fn remember(recents: &mut Vec<CommandRef>, command: CommandRef) {
        recents.retain(|held| *held != command);
        recents.insert(0, command);
        recents.truncate(RECENT_KEPT);
    }

    /// Shows the palette while it is open.
    pub fn show(self, ui: &mut Ui) -> PaletteOutput {
        match self.face {
            PaletteFace::Dialog => self.show_dialog(ui),
            PaletteFace::InPlace => self.show_in_place(ui),
        }
    }

    fn show_dialog(self, ui: &mut Ui) -> PaletteOutput {
        let CommandPalette { label, entries, index, open, uses, mut recents, placeholder, suggested, fallback, .. } = self;
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let words = env.words::<PanelWords>();
        let list_id = ui.scope_id().with("palette-rows");
        let never_used = |_: &CommandRef| 0;
        let uses: &dyn Fn(&CommandRef) -> u32 = uses.unwrap_or(&never_used);
        let hint: Label = placeholder.unwrap_or_else(|| words.palette_placeholder.clone());
        let mut chosen = Chosen::default();
        let shown = Popover::new(label.clone(), open)
            .collection_body()
            .overlay(|_| {
                let mut overlay = Overlay::modal().dismiss(Dismiss::new().outside(Outside::Close));
                overlay.fit = OverlayFit::ContentCapped(SHARE);
                overlay
            })
            .show(ui, |ui, cx| {
                let recent = recent_of(&mut recents, cx);
                let palette_rect = cx.rect;
                let opening = overlay::opening(&ctx, cx.resolved.id);
                // The body decides the footer in the pass it lists; the footer region runs after it.
                let status = RefCell::new(Status::Quiet);
                // The field is the stack's header and the status its footer, so the list takes the room left in
                // the capped box and scrolls inside it. The three regions share the stack's memory, which is kept by
                // its id across a close, so each opening starts a fresh session in the header, the first region.
                Stack::new()
                    .header(|ui, stack| {
                        let state = stack.memory::<Session>();
                        if opening {
                            *state = Session::default();
                        }
                        TextField::search(words.palette_search.clone(), Bind::bound(&mut state.query))
                            .placeholder(hint.clone())
                            .drives(list_id)
                            .show(ui);
                    })
                    .footer(|ui, stack| {
                        let format = stack.env.format();
                        let text = match &*status.borrow() {
                            Status::Quiet => return,
                            Status::Empty => words.palette_empty.clone(),
                            Status::Line(line) => Label::from(line.clone()),
                            Status::Count { shown, total } if shown < total => Label::from(
                                words.palette_count.fill(&[("shown", &format.integer(*shown as u64)), ("n", &format.integer(*total as u64))]),
                            ),
                            Status::Count { total, .. } => {
                                let n = *total as u64;
                                Label::from(words.palette_results.text(n, &format.integer(n)))
                            }
                        };
                        Text::new(text).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                    })
                    .show(ui, |ui, stack| {
                        let state = stack.memory::<Session>();
                        let input = RowsInput {
                            ctx: &ctx,
                            entries,
                            index,
                            uses,
                            words: &words,
                            list_id,
                            suggested: suggested.as_ref(),
                            fallback: fallback.as_ref(),
                            recent: &recent,
                            fill: true,
                            callout: Some(palette_rect),
                        };
                        let (picks, said) = show_rows(ui, &input, state, &cx.env);
                        chosen = picks;
                        *status.borrow_mut() = said;
                    });
                conclude(cx, &ctx, &chosen, &label, recent, &mut recents);
            });
        let closed = shown.output.closed;
        PaletteOutput { response: shown.output.response, picked: chosen.picked, closed, fallback: chosen.fallback, refused: chosen.refused }
    }

    fn show_in_place(self, ui: &mut Ui) -> PaletteOutput {
        let CommandPalette { label, entries, index, mut open, uses, mut recents, placeholder, suggested, fallback, .. } = self;
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let words = env.words::<PanelWords>();
        let scope = ui.scope_id();
        let list_id = scope.with("palette-rows");
        let field_id = scope.with("palette-field");
        let never_used = |_: &CommandRef| 0;
        let uses: &dyn Fn(&CommandRef) -> u32 = uses.unwrap_or(&never_used);
        let face_hint: Label = placeholder.unwrap_or_else(|| words.palette_placeholder.clone());
        let mut chosen = Chosen::default();
        let mut closed = None;
        let shown = Row::new().show(ui, |ui, cx| {
            let mut is_open = open.reborrow().read(cx).and_then(|held| held.same().cloned()).unwrap_or(false);
            // A click on the face and an app that sets `open` take the same path: the open branch below runs in this
            // pass, and the field takes the key focus in it.
            // The closed face is the search field's own look at its own size, so what sits beside it does not shift on open.
            let closed_face = !is_open
                && clicked(
                    &TextField::search(label.clone(), Bind::given(String::new())).placeholder(face_hint.clone()).id(field_id).show(ui).response,
                );
            if closed_face {
                let name: Arc<str> = Arc::from(label.as_str());
                let _ = open.reborrow().write(cx, &EditEvent::set(Mixed::Same(false), true, name, None));
                is_open = true;
            }
            // The closed-to-open transition, whichever route made it: the field takes the key focus in this pass.
            let was_open = std::mem::replace(&mut cx.memory::<WasOpen>().0, is_open);
            let opening = is_open && !was_open;
            if !is_open {
                return;
            }
            // The container's memory is kept by its id across a close, so each opening starts a fresh session: an
            // empty query and no active row.
            Row::new().show(ui, |ui, inner| {
                if opening {
                    *inner.memory::<Session>() = Session::default();
                    focus::request(&ctx, FocusTarget::Id(field_id));
                }
                let session = inner.memory::<Session>();
                let field =
                    TextField::search(label.clone(), Bind::bound(&mut session.query)).placeholder(face_hint.clone()).id(field_id).show(ui).response;
                let popup = PopupList::new(label.clone(), open.reborrowed()).opened_by(&field).list(list_id).show(ui, |ui, popup| {
                    let recent = recent_of(&mut recents, popup);
                    let input = RowsInput {
                        ctx: &ctx,
                        entries,
                        index,
                        uses,
                        words: &words,
                        list_id,
                        suggested: suggested.as_ref(),
                        fallback: fallback.as_ref(),
                        recent: &recent,
                        fill: false,
                        callout: None,
                    };
                    chosen = show_rows(ui, &input, session, &popup.env).0;
                    conclude(popup, &ctx, &chosen, &label, recent, &mut recents);
                });
                closed = popup.output.closed;
            });
        });
        PaletteOutput { response: shown.output, picked: chosen.picked, closed, fallback: chosen.fallback, refused: chosen.refused }
    }
}

/// Whether the in-place face was open in the pass before, so the pass that opens it is known.
#[derive(Clone, Default)]
struct WasOpen(bool);

/// What a pass of the list chose.
#[derive(Default)]
struct Chosen {
    picked: Option<CommandRef>,
    fallback: Option<String>,
    refused: Option<CommandRef>,
}

/// What the list body reads, the same for both faces.
struct RowsInput<'a> {
    ctx: &'a egui::Context,
    entries: &'a [HelpEntry],
    index: &'a SearchIndex,
    uses: &'a dyn Fn(&CommandRef) -> u32,
    words: &'a PanelWords,
    list_id: Id,
    suggested: Option<&'a (Label, Vec<CommandRef>)>,
    fallback: Option<&'a WordTemplate>,
    recent: &'a [CommandRef],
    /// The list fills the room its box leaves (the capped dialog) instead of the width alone (a popup that sizes to it).
    fill: bool,
    /// The dialog's box: the active row's control gets a callout clear of it, and the footer says what the list does
    /// not. `None` in place, where the list itself says when nothing matches.
    callout: Option<Rect>,
}

/// The commands last run, read through the `recents` binding.
fn recent_of(recents: &mut Bind<'_, Vec<CommandRef>>, cx: &mut BodyCx<'_>) -> Vec<CommandRef> {
    recents.reborrow().read(cx).and_then(|held| held.same().cloned()).unwrap_or_default()
}

/// Shows the list of `state`'s query and reports what the user picked in it, and what the dialog's footer says.
fn show_rows(ui: &mut Ui, input: &RowsInput<'_>, state: &mut Session, env: &crate::runtime::env::EnvView<'_>) -> (Chosen, Status) {
    let found = state.query.trim().to_owned();
    let stamp = stamp_of(&state.query);
    // A new query lists from the top; under the same one the active result keeps its row.
    let active_before = match state.selection.lead {
        Some(Line::Entry(at)) if state.stamp == stamp => Some(at),
        _ => None,
    };
    let fresh = state.stamp != stamp;
    state.stamp = stamp;
    let suggested = input.suggested.map(|(_, commands)| commands.as_slice());
    let listed = layout(input, &state.query, suggested, active_before);
    // A new query's active row is its first match, enabled or not: Enter on a disabled one runs nothing, and the footer
    // says why, rather than Enter running the next match.
    if fresh {
        state.selection.lead = listed.keys.iter().copied().find(|key| matches!(key, Line::Entry(_)));
    }
    let heading = input.suggested.map(|(label, _)| env.resolve(label).as_str().to_owned()).unwrap_or_default();
    let fallback_text = input.fallback.map(|template| template.fill(&[("text", found.as_str())])).unwrap_or_default();
    let rows = row_data(input.ctx, input.entries, &listed.keys, &heading, &fallback_text, env);
    let source = Listed { keys: &listed.keys, rows: &rows };
    let mut list = List::options(input.words.palette_list.clone())
        .id(input.list_id)
        .arrows_wrap(true)
        .selection(Bind::bound(&mut state.selection))
        .rows_stamp(stamp)
        .sizing(if input.fill { Sizing::Fill } else { Sizing::FillWidth });
    if input.callout.is_none() {
        list = list.empty(input.words.palette_empty.clone());
    }
    let events = list.show_items(ui, &source, &mut |key, _| rows[position(&listed.keys, *key)].row()).events;
    let mut chosen = Chosen::default();
    let command = |at: usize| input.entries.get(at).map(|entry| entry.command().clone());
    for event in events {
        match event {
            CollectionEvent::Activate(Line::Entry(at)) => chosen.picked = command(at),
            CollectionEvent::Activate(Line::Fallback) => chosen.fallback = Some(found.clone()),
            CollectionEvent::Refused(Line::Entry(at)) => chosen.refused = command(at),
            _ => {}
        }
    }
    let status = status_of(ui, input, &listed, &rows, state.selection.lead, found.is_empty(), env);
    let results = listed.results();
    if state.announced != Some(results) && !found.is_empty() {
        let n = results as u64;
        announce(input.ctx, input.words.palette_results.text(n, &env.format().integer(n)), Politeness::Polite);
    }
    state.announced = Some(results);
    (chosen, status)
}

/// What the dialog's footer says about the active row, else the first listed command: why a disabled one does not run,
/// where its control is when the callout can mark none, else the count of results. Marks the control it can. In place
/// there is no footer and no callout.
fn status_of(
    ui: &mut Ui,
    input: &RowsInput<'_>,
    listed: &Listing,
    rows: &[RowData],
    lead: Option<Line>,
    query_empty: bool,
    env: &crate::runtime::env::EnvView<'_>,
) -> Status {
    let Some(palette) = input.callout else { return Status::Quiet };
    let results = listed.results();
    let active = lead.filter(|key| listed.keys.contains(key)).or_else(|| listed.keys.iter().copied().find(|key| matches!(key, Line::Entry(_))));
    let Some(Line::Entry(at)) = active else {
        return if query_empty {
            Status::Quiet
        } else if results == 0 {
            Status::Empty
        } else {
            Status::Count { shown: results, total: listed.total }
        };
    };
    let entry = &input.entries[at];
    let row = &rows[position(&listed.keys, Line::Entry(at))];
    let pointed = callout::show(ui, entry.command(), palette);
    let home = entry.home().map(|home| env.resolve(home).as_str().to_owned());
    match (row.hint.as_ref().filter(|_| !row.enabled), home) {
        (Some(hint), _) => Status::Line(hint.to_string()),
        (None, Some(home)) if pointed == Pointed::Nowhere => Status::Line(home),
        _ if query_empty => Status::Quiet,
        _ => Status::Count { shown: results, total: listed.total },
    }
}

/// Closes the palette for a pick or the fallback, fires the picked command and puts it first in `recents`.
fn conclude(
    cx: &mut BodyCx<'_>,
    ctx: &egui::Context,
    chosen: &Chosen,
    label: &Label,
    recent: Vec<CommandRef>,
    recents: &mut Bind<'_, Vec<CommandRef>>,
) {
    if chosen.fallback.is_some() {
        cx.request_close();
    }
    let Some(command) = &chosen.picked else { return };
    cx.request_close();
    commands::fire_beneath(ctx, command.clone(), FireSource::Palette);
    let before = recent.clone();
    let mut after = recent;
    CommandPalette::remember(&mut after, command.clone());
    let name: Arc<str> = Arc::from(label.as_str());
    let _ = recents.reborrow().write(cx, &EditEvent::set(Mixed::Same(before), after, name, None));
}

/// The rows the list shows, in display order, and how many commands matched in all.
struct Listing {
    keys: Vec<Line>,
    total: usize,
}

impl Listing {
    /// The commands listed: every row but the heading and the fallback.
    fn results(&self) -> usize {
        self.keys.iter().filter(|key| matches!(key, Line::Entry(_))).count()
    }
}

/// The entries the list shows, by position in `entries`, and how many matched in all: the matches for a query, best
/// first and at most `RESULTS`, else the newest `RECENT` commands last run that have an entry, else the first entries.
/// A command appears once. `active` is the entry the user is on: when better matches would push it past the cap it
/// takes the last row, so the list does not move under it.
fn listing(input: &RowsInput<'_>, query: &str, active: Option<usize>) -> (Vec<usize>, usize) {
    let entries = input.entries;
    let empty = query.trim().is_empty();
    let last: Vec<usize> =
        input.recent.iter().filter_map(|command| entries.iter().position(|entry| entry.command() == command)).take(RECENT).collect();
    let candidates: Vec<usize> = if !empty {
        rank(input.index, query, &last, &|at: usize| (input.uses)(entries[at].command()))
    } else if last.is_empty() {
        (0..entries.len()).collect()
    } else {
        last
    };
    let mut seen: Vec<&CommandRef> = Vec::new();
    let mut keys: Vec<usize> = candidates
        .into_iter()
        .filter(|at| {
            let command = entries[*at].command();
            let fresh = !seen.contains(&command);
            seen.push(command);
            fresh
        })
        .collect();
    if empty {
        keys.truncate(RECENT.max(FIRST_ROWS));
        let total = keys.len();
        return (keys, total);
    }
    let total = keys.len();
    let held = active.filter(|key| keys.iter().position(|at| at == key).is_some_and(|at| at >= RESULTS));
    keys.truncate(if held.is_some() { RESULTS - 1 } else { RESULTS });
    keys.extend(held);
    (keys, total)
}

/// The rows of the list in display order. An empty query with `suggested` lists the heading and the suggested commands
/// that are entries, and nothing else; a query lists its matches and, with a fallback, the fallback row last.
fn layout(input: &RowsInput<'_>, query: &str, suggested: Option<&[CommandRef]>, active: Option<usize>) -> Listing {
    let entries = input.entries;
    if query.trim().is_empty() {
        let mut at: Vec<usize> = Vec::new();
        for command in suggested.unwrap_or_default() {
            if let Some(found) = entries.iter().position(|entry| entry.command() == command)
                && !at.contains(&found)
            {
                at.push(found);
            }
        }
        // Suggestions that name no entry leave the empty query its usual list rather than "no results".
        if at.is_empty() {
            let (keys, total) = listing(input, query, active);
            return Listing { keys: keys.into_iter().map(Line::Entry).collect(), total };
        }
        let total = at.len();
        return Listing { keys: std::iter::once(Line::Heading).chain(at.into_iter().map(Line::Entry)).collect(), total };
    }
    let (keys, total) = listing(input, query, active);
    let mut keys: Vec<Line> = keys.into_iter().map(Line::Entry).collect();
    if input.fallback.is_some() {
        if !keys.is_empty() {
            keys.push(Line::Separator);
        }
        keys.push(Line::Fallback);
    }
    Listing { keys, total }
}

/// The position of the row `key` in the listing.
fn position(listed: &[Line], key: Line) -> usize {
    listed.iter().position(|at| *at == key).unwrap_or(0)
}

fn stamp_of(query: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    query.trim().hash(&mut hasher);
    hasher.finish()
}

/// What one listed row shows.
struct RowData {
    label: String,
    /// The entry's home, else its menu path.
    path: Option<String>,
    chord: Option<String>,
    enabled: bool,
    icon: Option<IconSpec<'static>>,
    /// Why the command does not run now, when the app says.
    hint: Option<Arc<str>>,
    /// The row heads the rows after it.
    heading: bool,
    /// The row is the line between the commands and the fallback row.
    separator: bool,
}

impl RowData {
    /// A row of `label` alone: the heading, the separator or the fallback row.
    fn plain(label: &str, heading: bool, separator: bool) -> RowData {
        RowData { label: label.to_owned(), path: None, chord: None, enabled: true, icon: None, hint: None, heading, separator }
    }

    /// One line: the icon, the label, then the home or menu path, which drops out when the label would get too
    /// narrow, and the chord at the end.
    fn row(&self) -> ListRow<'static> {
        if self.heading {
            return ListRow::new(self.label.clone()).heading();
        }
        if self.separator {
            return ListRow::separator();
        }
        let row = ListRow::new(self.label.clone()).option().command_row().enabled(self.enabled);
        let row = match &self.icon {
            Some(icon) => row.icon(icon.clone()),
            None => row,
        };
        // The first trailing item is the outermost: the chord ends the row and the path sits before it.
        let row = match &self.chord {
            Some(chord) => row.trailing(chord.clone()),
            None => row,
        };
        match &self.path {
            Some(path) => row.optional(path.clone()),
            None => row,
        }
    }
}

/// The icon a command names, when it names one with a static name.
fn command_icon(command: &ResolvedCommand) -> Option<IconSpec<'static>> {
    match command.icon.as_ref()? {
        Cow::Borrowed(name) => Some(IconSpec::svg(IconId::new(name))),
        Cow::Owned(_) => None,
    }
}

fn row_data(
    ctx: &egui::Context,
    entries: &[HelpEntry],
    keys: &[Line],
    heading: &str,
    fallback: &str,
    env: &crate::runtime::env::EnvView<'_>,
) -> Vec<RowData> {
    let (os, layout) = (env.shell().os, env.keyboard_layout());
    let separator = env.words::<CommonWords>().path_separator.as_str().to_owned();
    keys.iter()
        .map(|key| match key {
            Line::Heading => RowData::plain(heading, true, false),
            Line::Separator => RowData::plain("", false, true),
            Line::Fallback => RowData::plain(fallback, false, false),
            Line::Entry(at) => {
                let entry = &entries[*at];
                let resolved = commands::resolve_beneath(ctx, entry.command().clone());
                let path = match entry.home() {
                    Some(home) => env.resolve(home).as_str().to_owned(),
                    None => entry.path().iter().map(|label| env.resolve(label).as_str().to_owned()).collect::<Vec<_>>().join(&separator),
                };
                RowData {
                    label: env.resolve(entry.label()).as_str().to_owned(),
                    path: Some(path).filter(|path| !path.is_empty()),
                    chord: resolved.chord.as_ref().map(|chord| chord.display_for(os, layout)),
                    enabled: resolved.enabled,
                    icon: entry.icon().cloned().or_else(|| command_icon(&resolved)),
                    hint: resolved.hint.clone(),
                    heading: false,
                    separator: false,
                }
            }
        })
        .collect()
}

/// The listed rows as the options of the list.
struct Listed<'a> {
    keys: &'a [Line],
    rows: &'a [RowData],
}

impl DisplayOrder<Line> for Listed<'_> {
    fn len(&self) -> usize {
        self.keys.len()
    }

    fn key(&self, at: usize) -> Line {
        self.keys[at]
    }

    fn is_section(&self, at: usize) -> bool {
        self.rows[at].heading || self.rows[at].separator
    }

    fn enabled(&self, at: usize) -> bool {
        self.rows[at].enabled
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        (!self.is_section(at)).then(|| std::borrow::Cow::Borrowed(self.rows[at].label.as_str()))
    }
}

impl ItemSource<Line> for Listed<'_> {}

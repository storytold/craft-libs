//! `CommandPalette` (24.4): a search over the app's commands in one centred modal box.

use super::words::PanelWords;
use crate::contract::overlay::{Dismiss, Outside, OverlayFit, ViewportShare};
use crate::contract::{
    Bind, CollectionEvent, Overlay, Pattern, Politeness, Props, Show as _, ShowCollection as _, ShowContainer as _, Sizing, announce,
};
use crate::display::{Shade, Text};
use crate::layout_components::Stack;
use crate::list::{List, ListRow};
use crate::menu::search::{HelpEntry, SearchIndex, rank};
use crate::popover::Popover;
use crate::primitives::IconSpec;
use crate::runtime::commands::{self, CommandRef, FireSource, ResolvedCommand};
use crate::runtime::env::{Env, Label};
use crate::runtime::memory::Remember as _;
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::common::CommonWords;
use crate::style::TypeRole;
use crate::text::TextField;
use callout::Pointed;
use craft_behavior::{DisplayOrder, EditEvent, IconId, ItemSource, Mixed, Selection};
use egui::{Response, Ui};
use std::borrow::Cow;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

mod callout;
#[cfg(feature = "testing")]
mod stories;

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

/// What the palette keeps in its stack's memory while it is open: the query, the active row, the number of
/// results last announced and the stamp of the query the list was last built for.
#[derive(Clone, Default)]
struct Session {
    query: String,
    selection: Selection<usize>,
    announced: Option<usize>,
    stamp: u64,
}

/// What the footer says; the body decides it in the pass it lists, the footer reads it.
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
}

impl crate::contract::Outcome for PaletteOutput {
    type Value = crate::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A search over the app's commands in a centred modal box: a search field over the matching commands.
///
/// - Entries: [`HelpEntry`]s, found by label, English label, menu path or words and ranked by [`rank`] over the
///   app's [`SearchIndex`], at most [`RESULTS`] of them; the active result keeps the last row when a better one would
///   push it past the end. A row is one line: the icon, the label, the entry's home or else its menu path (dropped
///   when the label would get too narrow) and the command's chord at the end. A command is enabled as the window
///   under the palette has it: the palette closes before the command runs, so its own modal layer blocks none. A
///   disabled command is listed and does not run.
/// - Empty query: the commands last run, newest first (the `recents` binding), else the first entries.
/// - Keys: the field keeps the focus; Up, Down, Page Up, Page Down move the active row (Up and Down wrap at the ends),
///   Enter picks it, Escape clears the query and, on an empty query, closes the box; a press outside closes it.
///   The number of results is announced politely.
/// - Callout: the control that shows the active row's command gets a ring and a box beside it; when the control is
///   out of view or under the palette, the footer names the entry's home instead. The footer also says why a
///   disabled active row does not run, else how many results there are.
/// - Picking: the box closes, focus returns to where it was before the palette opened, the command fires as
///   `FireSource::Palette` (the app receives it as it receives a menu's) and goes first in `recents`;
///   [`CommandPalette::remember`] is the rule. The output's `picked` names it. Without a binding the palette keeps `recents` for the session.
#[derive(Pattern, Props)]
pub struct CommandPalette<'a> {
    label: Label,
    entries: &'a [HelpEntry],
    index: &'a SearchIndex,
    open: Bind<'a, bool>,
    uses: Option<&'a dyn Fn(&CommandRef) -> u32>,
    /// The commands last run, newest first.
    #[prop]
    recents: Bind<'a, Vec<CommandRef>>,
}

impl<'a> CommandPalette<'a> {
    /// A palette named `label` over `entries`, ranked through `index` (which the app builds when its entries change),
    /// open while `open` is true.
    pub fn new(label: impl Into<Label>, entries: &'a [HelpEntry], index: &'a SearchIndex, open: Bind<'a, bool>) -> Self {
        CommandPalette { label: label.into(), entries, index, open, uses: None, recents: Bind::kept() }
    }

    /// The pick counts that rank the results, 0 for a command never picked.
    #[must_use]
    pub fn uses(mut self, uses: &'a dyn Fn(&CommandRef) -> u32) -> Self {
        self.uses = Some(uses);
        self
    }

    /// Puts `command` first in `recents`, once, keeping the newest `RECENT_KEPT`.
    pub fn remember(recents: &mut Vec<CommandRef>, command: CommandRef) {
        recents.retain(|held| *held != command);
        recents.insert(0, command);
        recents.truncate(RECENT_KEPT);
    }

    /// Shows the palette while it is open.
    pub fn show(self, ui: &mut Ui) -> PaletteOutput {
        let CommandPalette { label, entries, index, open, uses, mut recents } = self;
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let words = env.words::<PanelWords>();
        let list_id = ui.scope_id().with("palette-rows");
        let never_used = |_: &CommandRef| 0;
        let uses: &dyn Fn(&CommandRef) -> u32 = uses.unwrap_or(&never_used);
        let mut picked: Option<CommandRef> = None;
        let shown = Popover::new(label.clone(), open)
            .collection_body()
            .overlay(|_| {
                let mut overlay = Overlay::modal().dismiss(Dismiss::new().outside(Outside::Close));
                overlay.fit = OverlayFit::ContentCapped(SHARE);
                overlay
            })
            .show(ui, |ui, cx| {
                let recent: Vec<CommandRef> = recents.reborrow().read(cx).and_then(|held| held.same().cloned()).unwrap_or_default();
                let palette_rect = cx.rect;
                // The body decides the footer in the pass it lists; the footer region runs after it.
                let status = RefCell::new(Status::Quiet);
                // The field is the stack's header and the status its footer, so the list takes the room left in
                // the capped box and scrolls inside it. The three regions share the stack's memory.
                Stack::new()
                    .header(|ui, stack| {
                        let state = stack.memory::<Session>();
                        TextField::search(words.palette_search.clone(), Bind::bound(&mut state.query))
                            .placeholder(words.palette_placeholder.clone())
                            .drives(list_id)
                            .show(ui);
                    })
                    .footer(|ui, stack| {
                        let format = stack.env.format();
                        let text = match &*status.borrow() {
                            Status::Quiet => return,
                            Status::Empty => Label::from(words.palette_empty.clone()),
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
                        let stamp = stamp_of(&state.query);
                        // A new query lists from the top; under the same one the active result keeps its row.
                        let active_before = state.selection.lead.filter(|_| state.stamp == stamp);
                        state.stamp = stamp;
                        let listed = listing(entries, index, &state.query, &recent, uses, active_before);
                        let rows = row_data(&ctx, entries, &listed.keys, &cx.env);
                        let source = Listed { keys: &listed.keys, rows: &rows };
                        let events = List::options(words.palette_list.clone())
                            .id(list_id)
                            .arrows_wrap(true)
                            .selection(Bind::bound(&mut state.selection))
                            .rows_stamp(stamp)
                            .sizing(Sizing::Fill)
                            .show_items(ui, &source, &mut |key, _| rows[position(&listed.keys, *key)].row())
                            .events;
                        for event in events {
                            if let CollectionEvent::Activate(key) = event {
                                picked = Some(entries[key].command().clone());
                            }
                        }
                        let query_empty = state.query.trim().is_empty();
                        let active = state.selection.lead.filter(|key| listed.keys.contains(key)).or_else(|| listed.keys.first().copied());
                        *status.borrow_mut() = match active {
                            None if query_empty => Status::Quiet,
                            None => Status::Empty,
                            Some(key) => {
                                let row = &rows[position(&listed.keys, key)];
                                let pointed = callout::show(ui, entries[key].command(), palette_rect);
                                let home = entries[key].home().map(|home| cx.env.resolve(home).as_str().to_owned());
                                match (row.hint.as_ref().filter(|_| !row.enabled), home) {
                                    (Some(hint), _) => Status::Line(hint.to_string()),
                                    (None, Some(home)) if pointed == Pointed::Nowhere => Status::Line(home),
                                    _ if query_empty => Status::Quiet,
                                    _ => Status::Count { shown: listed.keys.len(), total: listed.total },
                                }
                            }
                        };
                        if state.announced != Some(listed.keys.len()) && !query_empty {
                            let n = listed.keys.len() as u64;
                            announce(&ctx, words.palette_results.text(n, &cx.env.format().integer(n)), Politeness::Polite);
                        }
                        state.announced = Some(listed.keys.len());
                    });
                if let Some(command) = &picked {
                    cx.request_close();
                    commands::fire_beneath(&ctx, command.clone(), FireSource::Palette);
                    let before = recent.clone();
                    let mut after = recent;
                    Self::remember(&mut after, command.clone());
                    let name: Arc<str> = Arc::from(label.as_str());
                    let _ = recents.reborrow().write(cx, &EditEvent::set(Mixed::Same(before), after, name, None));
                }
            });
        let closed = shown.output.closed;
        PaletteOutput { response: shown.output.response, picked, closed }
    }
}

/// The entries the list shows, by position in `entries`, and how many matched in all.
struct Listing {
    keys: Vec<usize>,
    total: usize,
}

/// The entries the list shows: the matches for a query, best first and at most `RESULTS`, else the newest `RECENT`
/// commands last run that have an entry, else the first entries. A command appears once. `active` is the entry the
/// user is on: when better matches would push it past the cap it takes the last row, so the list does not move
/// under it.
fn listing(
    entries: &[HelpEntry],
    index: &SearchIndex,
    query: &str,
    recent: &[CommandRef],
    uses: &dyn Fn(&CommandRef) -> u32,
    active: Option<usize>,
) -> Listing {
    let empty = query.trim().is_empty();
    let last: Vec<usize> =
        recent.iter().filter_map(|command| entries.iter().position(|entry| entry.command() == command)).take(RECENT).collect();
    let candidates: Vec<usize> = if !empty {
        rank(index, query, &last, &|at: usize| uses(entries[at].command()))
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
        return Listing { total: keys.len(), keys };
    }
    let total = keys.len();
    let held = active.filter(|key| keys.iter().position(|at| at == key).is_some_and(|at| at >= RESULTS));
    keys.truncate(if held.is_some() { RESULTS - 1 } else { RESULTS });
    keys.extend(held);
    Listing { keys, total }
}

/// The position of the entry `key` in the listing.
fn position(listed: &[usize], key: usize) -> usize {
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
}

impl RowData {
    /// One line: the icon, the label, then the home or menu path, which drops out when the label would get too
    /// narrow, and the chord at the end.
    fn row(&self) -> ListRow<'static> {
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

fn row_data(ctx: &egui::Context, entries: &[HelpEntry], listed: &[usize], env: &crate::runtime::env::EnvView<'_>) -> Vec<RowData> {
    let (os, layout) = (env.shell().os, env.keyboard_layout());
    let separator = env.words::<CommonWords>().path_separator.as_str().to_owned();
    listed
        .iter()
        .map(|at| {
            let entry = &entries[*at];
            let resolved = commands::resolve_beneath(ctx, entry.command().clone());
            let path: Vec<String> = entry.path().iter().map(|label| env.resolve(label).as_str().to_owned()).collect();
            let path = match entry.home() {
                Some(home) => env.resolve(home).as_str().to_owned(),
                None => path.join(&separator),
            };
            RowData {
                label: env.resolve(entry.label()).as_str().to_owned(),
                path: Some(path).filter(|path| !path.is_empty()),
                chord: resolved.chord.as_ref().map(|chord| chord.display_for(os, layout)),
                enabled: resolved.enabled,
                icon: entry.icon().cloned().or_else(|| command_icon(&resolved)),
                hint: resolved.hint.clone(),
            }
        })
        .collect()
}

/// The listed entries as the options of the list, keyed by their position in the app's entries.
struct Listed<'a> {
    keys: &'a [usize],
    rows: &'a [RowData],
}

impl DisplayOrder<usize> for Listed<'_> {
    fn len(&self) -> usize {
        self.keys.len()
    }

    fn key(&self, at: usize) -> usize {
        self.keys[at]
    }

    fn enabled(&self, at: usize) -> bool {
        self.rows[at].enabled
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        Some(std::borrow::Cow::Borrowed(self.rows[at].label.as_str()))
    }
}

impl ItemSource<usize> for Listed<'_> {}

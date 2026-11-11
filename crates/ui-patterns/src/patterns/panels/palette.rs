//! `CommandPalette` (24.4): a search over the app's commands in one centred modal box.

use super::words::PanelWords;
use crate::contract::overlay::{OverlayFit, ViewportShare};
use crate::contract::{Bind, CollectionEvent, Overlay, Pattern, Politeness, Props, Show as _, ShowCollection as _, ShowContainer as _, announce};
use crate::display::{Shade, Text};
use crate::layout_components::Stack;
use crate::list::{List, ListRow};
use crate::menu::search::{HelpEntry, search_order};
use crate::popover::Popover;
use crate::runtime::commands::{self, CommandRef};
use crate::runtime::env::{Env, Label};
use crate::runtime::memory::Remember as _;
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::common::CommonWords;
use crate::style::TypeRole;
use crate::text::TextField;
use craft_behavior::{DisplayOrder, EditEvent, ItemSource, Mixed, Selection};
use egui::{Response, Ui};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[cfg(feature = "testing")]
mod stories;

/// The most commands the empty query lists when the app has run none.
const FIRST_ROWS: usize = 12;
/// The commands remembered for the empty query.
pub const RECENT: usize = 8;
/// The box's widest share of the window; its height is capped at the same share of the window's height.
const SHARE: ViewportShare = ViewportShare::new(0.6, 0.7);

/// What the palette keeps in its stack's memory while it is open: the query, the active row and the number of
/// results last announced.
#[derive(Clone, Default)]
struct Session {
    query: String,
    selection: Selection<usize>,
    announced: Option<usize>,
}

/// What a palette reports: its response, the command picked in this pass and how it closed.
#[derive(Debug)]
#[non_exhaustive]
pub struct PaletteOutput {
    /// The box's response.
    pub response: Response,
    /// The command the user picked in this pass. The palette has closed; the app runs it (a modal layer blocks the
    /// commands that must not run under one while it is open).
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
/// - Entries: [`HelpEntry`]s, found by label, English label or menu path and ranked by [`search_order`]. A row shows
///   the label, the menu path under it and the command's chord at the end; a disabled command is listed and does
///   not run.
/// - Empty query: the commands last run, newest first (the `recents` binding), else the first entries.
/// - Keys: the field keeps the focus; Up, Down, Page Up, Page Down move the active row, Enter picks it, Escape clears
///   the query and, on an empty query, closes the box; a press outside closes it. A caption under the list names the keys. The number of results is announced politely.
/// - Picking: the output's `picked` names the command and the box closes. With `recents` bound the command is also
///   put first in the list; [`CommandPalette::remember`] is the rule.
#[derive(Pattern, Props)]
pub struct CommandPalette<'a> {
    label: Label,
    entries: &'a [HelpEntry],
    open: Bind<'a, bool>,
    /// The commands last run, newest first.
    #[prop]
    recents: Option<Bind<'a, Vec<CommandRef>>>,
}

impl<'a> CommandPalette<'a> {
    /// A palette named `label` over `entries`, open while `open` is true.
    pub fn new(label: impl Into<Label>, entries: &'a [HelpEntry], open: Bind<'a, bool>) -> Self {
        CommandPalette { label: label.into(), entries, open, recents: None }
    }

    /// Puts `command` first in `recents`, once, keeping the newest `RECENT`.
    pub fn remember(recents: &mut Vec<CommandRef>, command: CommandRef) {
        recents.retain(|held| *held != command);
        recents.insert(0, command);
        recents.truncate(RECENT);
    }

    /// Shows the palette while it is open.
    pub fn show(self, ui: &mut Ui) -> PaletteOutput {
        let CommandPalette { label, entries, open, mut recents } = self;
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let words = env.words::<PanelWords>();
        let list_id = ui.scope_id().with("palette-rows");
        let mut picked: Option<CommandRef> = None;
        let shown = Popover::new(label.clone(), open)
            .collection_body()
            .overlay(|_| {
                let mut overlay = Overlay::modal();
                overlay.fit = OverlayFit::ContentCapped(SHARE);
                overlay
            })
            .show(ui, |ui, cx| {
                let recent: Vec<CommandRef> =
                    recents.as_mut().and_then(|bind| bind.reborrow().read(cx)).and_then(|held| held.same().cloned()).unwrap_or_default();
                let texts = texts_of(entries, &cx.env);
                let folder = cx.env.folder();
                // The field is the stack's header and the key hint its footer, so the list takes the room left in
                // the capped box and scrolls inside it. The three regions share the stack's memory.
                Stack::new()
                    .header(|ui, stack| {
                        let state = stack.memory::<Session>();
                        TextField::search(words.palette_search.clone(), Bind::bound(&mut state.query))
                            .placeholder(words.palette_placeholder.clone())
                            .drives(list_id)
                            .show(ui);
                    })
                    .footer(|ui, _| {
                        Text::new(words.palette_hint.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                    })
                    .show(ui, |ui, stack| {
                        let state = stack.memory::<Session>();
                        let listed = listing(entries, &state.query, &folder, &texts, &recent);
                        let rows = row_data(&ctx, entries, &texts, &listed, &cx.env);
                        let source = Listed { keys: &listed, rows: &rows };
                        let stamp = stamp_of(&state.query);
                        let events = List::options(words.palette_list.clone())
                            .id(list_id)
                            .empty(words.palette_empty.clone())
                            .selection(Bind::bound(&mut state.selection))
                            .rows_stamp(stamp)
                            .show_items(ui, &source, &mut |key, _| rows[position(&listed, *key)].row())
                            .events;
                        for event in events {
                            if let CollectionEvent::Activate(key) = event {
                                picked = Some(entries[key].command().clone());
                            }
                        }
                        if state.announced != Some(listed.len()) && !state.query.trim().is_empty() {
                            let n = listed.len() as u64;
                            announce(&ctx, words.palette_results.text(n, &cx.env.format().integer(n)), Politeness::Polite);
                        }
                        state.announced = Some(listed.len());
                    });
                if let Some(command) = &picked {
                    cx.request_close();
                    if let Some(bind) = recents.as_mut() {
                        let before = recent.clone();
                        let mut after = recent;
                        Self::remember(&mut after, command.clone());
                        let name: Arc<str> = Arc::from(label.as_str());
                        let _ = bind.reborrow().write(cx, &EditEvent::set(Mixed::Same(before), after, name, None));
                    }
                }
            });
        let closed = shown.output.closed;
        PaletteOutput { response: shown.output.response, picked, closed }
    }
}

/// The `[label, English label, menu path]` texts of `entries` for the matcher.
fn texts_of(entries: &[HelpEntry], env: &crate::runtime::env::EnvView<'_>) -> Vec<[String; 3]> {
    let separator = env.words::<CommonWords>().path_separator.as_str().to_owned();
    entries
        .iter()
        .map(|entry| {
            let path: Vec<String> = entry.path().iter().map(|label| env.resolve(label).as_str().to_owned()).collect();
            let english = entry.english().map(|label| env.resolve(label).as_str().to_owned()).unwrap_or_default();
            [env.resolve(entry.label()).as_str().to_owned(), english, path.join(&separator)]
        })
        .collect()
}

/// The entries the list shows, by position in `entries`: the matches for a query, best first, else the commands
/// last run, newest first, else the first entries. A command appears once.
fn listing(entries: &[HelpEntry], query: &str, folder: &craft_i18n::Folder, texts: &[[String; 3]], recent: &[CommandRef]) -> Vec<usize> {
    let refs: Vec<[&str; 3]> = texts.iter().map(|[label, english, path]| [label.as_str(), english.as_str(), path.as_str()]).collect();
    let candidates: Vec<usize> = if query.trim().is_empty() {
        let last: Vec<usize> = recent.iter().filter_map(|command| entries.iter().position(|entry| entry.command() == command)).collect();
        if last.is_empty() { (0..entries.len()).collect() } else { last }
    } else {
        search_order(folder, query, &refs)
    };
    let limit = if query.trim().is_empty() { RECENT.max(FIRST_ROWS) } else { usize::MAX };
    let mut seen: Vec<&CommandRef> = Vec::new();
    candidates
        .into_iter()
        .filter(|at| {
            let command = entries[*at].command();
            let fresh = !seen.contains(&command);
            seen.push(command);
            fresh
        })
        .take(limit)
        .collect()
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
    path: Option<String>,
    chord: Option<String>,
    enabled: bool,
}

impl RowData {
    fn row(&self) -> ListRow<'static> {
        let row = ListRow::new(self.label.clone()).option().enabled(self.enabled);
        let row = match &self.path {
            Some(path) => row.detail(path.clone()),
            None => row,
        };
        match &self.chord {
            Some(chord) => row.trailing(chord.clone()),
            None => row,
        }
    }
}

fn row_data(
    ctx: &egui::Context,
    entries: &[HelpEntry],
    texts: &[[String; 3]],
    listed: &[usize],
    env: &crate::runtime::env::EnvView<'_>,
) -> Vec<RowData> {
    let (os, layout) = (env.shell().os, env.keyboard_layout());
    listed
        .iter()
        .map(|at| {
            let resolved = commands::resolve(ctx, entries[*at].command().clone());
            RowData {
                label: texts[*at][0].clone(),
                path: Some(texts[*at][2].clone()).filter(|path| !path.is_empty()),
                chord: resolved.chord.as_ref().map(|chord| chord.display_for(os, layout)),
                enabled: resolved.enabled,
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

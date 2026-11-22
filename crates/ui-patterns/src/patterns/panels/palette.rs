//! `CommandPalette` (24.4): a search over the app's commands, in one centred modal box or in place under a search box.

use super::words::PanelWords;

use crate::contract::overlay::{OverlayFit, ViewportShare};
use crate::contract::{
    Bind, BodyCx, CollectionEvent, Overlay, Pattern, Politeness, Props, Show as _, ShowCollection as _, ShowContainer as _, Sizing, announce, clicked,
};
use crate::display::{Shade, Text};
use crate::layout_components::{Row, Stack};
use crate::list::{List, ListRow};
use crate::menu::search::{HelpEntry, search_order};
use crate::popover::{Popover, PopupList};

use crate::runtime::commands::{self, CommandRef, FireSource};
use crate::runtime::env::{Env, Label, WordTemplate};
use crate::runtime::focus::{self, FocusTarget};
use crate::runtime::memory::Remember as _;
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::common::CommonWords;
use crate::style::TypeRole;
use crate::text::TextField;
use craft_behavior::{DisplayOrder, EditEvent, ItemSource, Mixed, Selection};
use egui::{Id, Response, Ui};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// The most commands the empty query lists when the app has run none.
const FIRST_ROWS: usize = 12;
/// The commands remembered for the empty query.
pub const RECENT: usize = 8;
/// The box's widest share of the window; its height is capped at the same share of the window's height.
const SHARE: ViewportShare = ViewportShare::new(0.6, 0.7);

/// What the palette keeps in its container's memory while it is open: the query, the active row and the number of
/// results last announced.
#[derive(Clone, Default)]
struct Session {
    query: String,
    selection: Selection<Line>,
    announced: Option<usize>,
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
}

impl crate::contract::Outcome for PaletteOutput {
    type Value = crate::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A search over the app's commands: a search field over the matching commands, in a centred modal box or in place.
///
/// - Entries: [`HelpEntry`]s, found by label, English label, menu path or command id and ranked by [`search_order`].
///   A row is one line: the label, the menu path (dropped when the label would get too narrow) and the command's chord
///   at the end. A command is enabled as the window under the palette has it: the palette closes before the command
///   runs, so its own modal layer blocks none. A disabled command is listed and does not run.
/// - Faces: [`PaletteFace::Dialog`] is a centred modal box. [`PaletteFace::InPlace`] is a search box where it is laid
///   out, showing the `placeholder`; it opens to the field with the key focus in the pass that opens it and lists the
///   results in a popup under the field. The in-place field is the palette's face, so it is named by the palette's
///   `label`, closed and open; the dialog face's field, inside a box named `label`, keeps the generic search name.
/// - Empty query: with `suggested`, its heading and those commands; else the commands last run, newest first (the
///   `recents` binding), else the first entries.
/// - Fallback: with `fallback`, a last row follows the matches of a query, or stands alone when nothing matches. Picking
///   it closes the palette and reports the query in the output's `fallback`; no command runs.
/// - Keys: the field keeps the focus; Up, Down, Page Up, Page Down move the active row (Up and Down wrap at the ends),
///   Enter picks it, Escape clears the query and, on an empty query, closes the box; a press outside closes it. The dialog
///   face shows a caption under the list that names the keys. The number of results is announced politely.
/// - Picking: the box closes, the command fires as `FireSource::Palette` (the app receives it as it receives a menu's)
///   and goes first in `recents`; [`CommandPalette::remember`] is the rule. The output's `picked` names it. Without a
///   binding the palette keeps `recents` for the session. The dialog face returns the focus to where it was before the
///   palette opened; the in-place face leaves it where the host puts it after the popup closes.
#[derive(Pattern, Props)]
pub struct CommandPalette<'a> {
    label: Label,
    entries: &'a [HelpEntry],
    open: Bind<'a, bool>,
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
    /// A palette named `label` over `entries`, open while `open` is true.
    pub fn new(label: impl Into<Label>, entries: &'a [HelpEntry], open: Bind<'a, bool>) -> Self {
        CommandPalette {
            label: label.into(),
            entries,
            open,
            recents: Bind::kept(),
            face: PaletteFace::default(),
            placeholder: None,
            suggested: None,
            fallback: None,
        }
    }

    /// Puts `command` first in `recents`, once, keeping the newest `RECENT`.
    pub fn remember(recents: &mut Vec<CommandRef>, command: CommandRef) {
        recents.retain(|held| *held != command);
        recents.insert(0, command);
        recents.truncate(RECENT);
    }

    /// Shows the palette while it is open.
    pub fn show(self, ui: &mut Ui) -> PaletteOutput {
        match self.face {
            PaletteFace::Dialog => self.show_dialog(ui),
            PaletteFace::InPlace => self.show_in_place(ui),
        }
    }

    fn show_dialog(self, ui: &mut Ui) -> PaletteOutput {
        let CommandPalette { label, entries, open, mut recents, placeholder, suggested, fallback, .. } = self;
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let words = env.words::<PanelWords>();
        let list_id = ui.scope_id().with("palette-rows");
        let hint: Label = placeholder.unwrap_or_else(|| words.palette_placeholder.clone());
        let mut chosen = Chosen::default();
        let shown = Popover::new(label.clone(), open)
            .collection_body()
            .overlay(|_| {
                let mut overlay = Overlay::modal();
                overlay.fit = OverlayFit::ContentCapped(SHARE);
                overlay
            })
            .show(ui, |ui, cx| {
                let recent = recent_of(&mut recents, cx);
                // The field is the stack's header and the key hint its footer, so the list takes the room left in
                // the capped box and scrolls inside it. The three regions share the stack's memory.
                Stack::new()
                    .header(|ui, stack| {
                        let state = stack.memory::<Session>();
                        TextField::search(words.palette_search.clone(), Bind::bound(&mut state.query))
                            .placeholder(hint.clone())
                            .drives(list_id)
                            .show(ui);
                    })
                    .footer(|ui, _| {
                        Text::new(words.palette_hint.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                    })
                    .show(ui, |ui, stack| {
                        let state = stack.memory::<Session>();
                        let input = RowsInput {
                            ctx: &ctx,
                            entries,
                            words: &words,
                            list_id,
                            suggested: suggested.as_ref(),
                            fallback: fallback.as_ref(),
                            recent: &recent,
                            fill: true,
                        };
                        chosen = show_rows(ui, &input, state, &cx.env);
                    });
                conclude(cx, &ctx, &chosen, &label, recent, &mut recents);
            });
        let closed = shown.output.closed;
        PaletteOutput { response: shown.output.response, picked: chosen.picked, closed, fallback: chosen.fallback }
    }

    fn show_in_place(self, ui: &mut Ui) -> PaletteOutput {
        let CommandPalette { label, entries, mut open, mut recents, placeholder, suggested, fallback, .. } = self;
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let words = env.words::<PanelWords>();
        let scope = ui.scope_id();
        let list_id = scope.with("palette-rows");
        let field_id = scope.with("palette-field");
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
                let popup = PopupList::new(label.clone(), open.reborrowed()).opened_by(&field).show(ui, |ui, popup| {
                    let recent = recent_of(&mut recents, popup);
                    let input = RowsInput {
                        ctx: &ctx,
                        entries,
                        words: &words,
                        list_id,
                        suggested: suggested.as_ref(),
                        fallback: fallback.as_ref(),
                        recent: &recent,
                        fill: false,
                    };
                    chosen = show_rows(ui, &input, session, &popup.env);
                    conclude(popup, &ctx, &chosen, &label, recent, &mut recents);
                });
                closed = popup.output.closed;
            });
        });
        PaletteOutput { response: shown.output, picked: chosen.picked, closed, fallback: chosen.fallback }
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
}

/// What the list body reads, the same for both faces.
struct RowsInput<'a> {
    ctx: &'a egui::Context,
    entries: &'a [HelpEntry],
    words: &'a PanelWords,
    list_id: Id,
    suggested: Option<&'a (Label, Vec<CommandRef>)>,
    fallback: Option<&'a WordTemplate>,
    recent: &'a [CommandRef],
    /// The list fills the room its box leaves (the capped dialog) instead of the width alone (a popup that sizes to it).
    fill: bool,
}

/// The commands last run, read through the `recents` binding.
fn recent_of(recents: &mut Bind<'_, Vec<CommandRef>>, cx: &mut BodyCx<'_>) -> Vec<CommandRef> {
    recents.reborrow().read(cx).and_then(|held| held.same().cloned()).unwrap_or_default()
}

/// Shows the list of `state`'s query and reports what the user picked in it.
fn show_rows(ui: &mut Ui, input: &RowsInput<'_>, state: &mut Session, env: &crate::runtime::env::EnvView<'_>) -> Chosen {
    let texts = texts_of(input.entries, env);
    let folder = env.folder();
    let found = state.query.trim().to_owned();
    let suggested = input.suggested.map(|(_, commands)| commands.as_slice());
    let keys = layout(input.entries, &state.query, &folder, &texts, input.recent, suggested, input.fallback.is_some());
    let heading = input.suggested.map(|(label, _)| env.resolve(label).as_str().to_owned()).unwrap_or_default();
    let fallback_text = input.fallback.map(|template| template.fill(&[("text", found.as_str())])).unwrap_or_default();
    let rows = row_data(input.ctx, input.entries, &texts, &keys, &heading, &fallback_text, env);
    let source = Listed { keys: &keys, rows: &rows };
    let stamp = stamp_of(&state.query);
    let events = List::options(input.words.palette_list.clone())
        .id(input.list_id)
        .arrows_wrap(true)
        .empty(input.words.palette_empty.clone())
        .selection(Bind::bound(&mut state.selection))
        .rows_stamp(stamp)
        .sizing(if input.fill { Sizing::Fill } else { Sizing::FillWidth })
        .show_items(ui, &source, &mut |key, _| rows[position(&keys, *key)].row())
        .events;
    let mut chosen = Chosen::default();
    for event in events {
        if let CollectionEvent::Activate(key) = event {
            match key {
                Line::Entry(at) => chosen.picked = input.entries.get(at).map(|entry| entry.command().clone()),
                Line::Fallback => chosen.fallback = Some(found.clone()),
                Line::Heading | Line::Separator => {}
            }
        }
    }
    let results = keys.iter().filter(|key| matches!(key, Line::Entry(_))).count();
    if state.announced != Some(results) && !found.is_empty() {
        let n = results as u64;
        announce(input.ctx, input.words.palette_results.text(n, &env.format().integer(n)), Politeness::Polite);
    }
    state.announced = Some(results);
    chosen
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

/// The `[label, English label, menu path, command id]` texts of `entries` for the matcher.
fn texts_of(entries: &[HelpEntry], env: &crate::runtime::env::EnvView<'_>) -> Vec<[String; 4]> {
    let separator = env.words::<CommonWords>().path_separator.as_str().to_owned();
    entries
        .iter()
        .map(|entry| {
            let path: Vec<String> = entry.path().iter().map(|label| env.resolve(label).as_str().to_owned()).collect();
            let english = entry.english().map(|label| env.resolve(label).as_str().to_owned()).unwrap_or_default();
            [env.resolve(entry.label()).as_str().to_owned(), english, path.join(&separator), entry.command().id.clone()]
        })
        .collect()
}

/// The entries the list shows, by position in `entries`: the matches for a query, best first, else the commands
/// last run, newest first, else the first entries. A command appears once.
fn listing(entries: &[HelpEntry], query: &str, folder: &craft_i18n::Folder, texts: &[[String; 4]], recent: &[CommandRef]) -> Vec<usize> {
    let refs: Vec<[&str; 4]> =
        texts.iter().map(|[label, english, path, id]| [label.as_str(), english.as_str(), path.as_str(), id.as_str()]).collect();
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

/// The rows of the list in display order. An empty query with `suggested` lists the heading and the suggested commands
/// that are entries, and nothing else; a query lists its matches and, with a fallback, the fallback row last.
fn layout(
    entries: &[HelpEntry],
    query: &str,
    folder: &craft_i18n::Folder,
    texts: &[[String; 4]],
    recent: &[CommandRef],
    suggested: Option<&[CommandRef]>,
    with_fallback: bool,
) -> Vec<Line> {
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
            return listing(entries, query, folder, texts, recent).into_iter().map(Line::Entry).collect();
        }
        return std::iter::once(Line::Heading).chain(at.into_iter().map(Line::Entry)).collect();
    }
    let mut rows: Vec<Line> = listing(entries, query, folder, texts, recent).into_iter().map(Line::Entry).collect();
    if with_fallback {
        if !rows.is_empty() {
            rows.push(Line::Separator);
        }
        rows.push(Line::Fallback);
    }
    rows
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
    path: Option<String>,
    chord: Option<String>,
    enabled: bool,
    /// The row heads the rows after it.
    heading: bool,
    /// The row is the line between the commands and the fallback row.
    separator: bool,
}

impl RowData {
    /// One line: the label, then the menu path, which drops out when the label would get too narrow, and the chord
    /// at the end.
    fn row(&self) -> ListRow<'static> {
        if self.heading {
            return ListRow::new(self.label.clone()).heading();
        }
        if self.separator {
            return ListRow::separator();
        }
        let row = ListRow::new(self.label.clone()).option().command_row().enabled(self.enabled);
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

fn row_data(
    ctx: &egui::Context,
    entries: &[HelpEntry],
    texts: &[[String; 4]],
    keys: &[Line],
    heading: &str,
    fallback: &str,
    env: &crate::runtime::env::EnvView<'_>,
) -> Vec<RowData> {
    let (os, layout) = (env.shell().os, env.keyboard_layout());
    let line =
        |label: &str, heading: bool, separator: bool| RowData { label: label.to_owned(), path: None, chord: None, enabled: true, heading, separator };
    keys.iter()
        .map(|key| match key {
            Line::Heading => line(heading, true, false),
            Line::Separator => line("", false, true),
            Line::Fallback => line(fallback, false, false),
            Line::Entry(at) => {
                let resolved = commands::resolve_beneath(ctx, entries[*at].command().clone());
                RowData {
                    label: texts[*at][0].clone(),
                    path: Some(texts[*at][2].clone()).filter(|path| !path.is_empty()),
                    chord: resolved.chord.as_ref().map(|chord| chord.display_for(os, layout)),
                    enabled: resolved.enabled,
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

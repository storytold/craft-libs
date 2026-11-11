//! The text adapter: places the egui text widget at a text part's rect, applies the IME engine's edits and the
//! edit commands to it, and records what happened for the pass's typing run (18.2).

mod completion;

use crate::primitives::{TextLayout, TextOptions};
use crate::runtime::access::text::{Line, Run};
use crate::runtime::egui::{clipboard, focus as egui_focus, input as egui_input, text_edit};
use crate::runtime::focus::{KeepKeys, KeyUse};
use crate::runtime::keys::{self, Intents};
use crate::runtime::{commands, ime};
use craft_behavior::{EditCommand, EditorInput, Intent, PartKey, TextPos};
use craft_keymap::Mods;
use egui::{Context, Id, Pos2, Rect, Ui};
use std::any::Any;
use std::sync::Arc;

/// How a typing run ended, as the adapter saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    /// Enter, or Cmd/Ctrl+Enter in a multi-line field that submits.
    Submit,
    /// A commit-on character was typed.
    Separator,
    /// Focus left the part.
    Blur,
    /// Escape.
    Escape,
}

/// The composing text and where it sits in the draft, in characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Preedit {
    pub at: usize,
    pub text: String,
}

/// What the adapter keeps per text part between passes.
#[derive(Clone, Default)]
pub(crate) struct TextState {
    pub draft: Option<String>,
    /// The value when the run began (a `K::Value`).
    pub start: Option<Arc<dyn Any + Send + Sync>>,
    /// The value the run last showed or applied (a `K::Value`): a different value at the next pass was set outside
    /// the run, which ends it.
    pub seen: Option<Arc<dyn Any + Send + Sync>>,
    /// The draft changed since the typing run last read it.
    pub dirty: bool,
    pub end: Option<End>,
    /// The modifiers of the Enter that ended the run with `End::Submit`.
    pub submit_mods: Mods,
    pub invalid: bool,
    /// The text a commit left, shown for the pass after it.
    pub normalised: Option<String>,
    pub preedit: Option<Preedit>,
    /// The rows of last pass's text, for mapping an AccessKit selection back to characters; kept only while
    /// AccessKit runs.
    pub rows: Option<Rows>,
    pub caret: Option<usize>,
    /// The pass time the caret last moved or text was typed, for the blink.
    pub caret_moved: f64,
    pub focused: bool,
    /// The caret a click chose before the field took focus.
    pub click_caret: Option<usize>,
    pub completions: completion::Completions,
}

/// The id of a text part's egui widget, which holds key focus.
pub(crate) fn text_id(id: Id, key: PartKey) -> Id {
    crate::runtime::memory::sub(id, ("text", key))
}

/// The keys the text part keeps, from its options.
pub(crate) fn keys_of(base: KeyUse, options: &TextOptions) -> KeyUse {
    let keys = if options.tab_inserts {
        base.with_keep(KeepKeys::Tab)
    } else if options.multiline() {
        base.with_keep(KeepKeys::BlockArrows)
    } else {
        base
    };
    let keys = keys.with_value(base.value || options.steps_value);
    if options.submits { keys.with_submits() } else { keys }
}

pub(crate) struct AdapterIn<'a> {
    pub ctx: &'a Context,
    pub ui: &'a mut Ui,
    pub id: Id,
    pub key: PartKey,
    pub rect: Rect,
    pub shown: &'a str,
    pub options: &'a TextOptions,
    pub keys: KeyUse,
    pub read_only: bool,
    pub enabled: bool,
    /// The widget can hold key focus: an enabled part, or a disabled one whose reason shows on focus.
    pub focusable: bool,
    /// The widget takes the pointer (`text_edit::Edit::pointer`).
    pub pointer: bool,
    /// Where a click that enters the field went down: the caret goes to the character there.
    pub click: Option<Pos2>,
    pub rtl: bool,
    /// The text widget holds key focus.
    pub focused: bool,
    pub ink: egui::Color32,
    pub intents: &'a mut Intents,
    pub placeholder: Option<&'a str>,
    /// Whether the caret stroke shows, from the blink.
    pub caret_visible: bool,
    /// The width of the preedit underline.
    pub underline: f32,
    pub now: f64,
}

pub(crate) struct AdapterOut {
    pub layout: TextLayout,
    pub focused: bool,
    /// The screen y of the first row's baseline; `None` while the text is empty.
    pub baseline: Option<f32>,
    /// The character `AdapterIn::click` fell at.
    pub clicked_char: Option<usize>,
}

/// The rows of a laid-out text as AccessKit lines, with the byte where each starts in the text.
#[derive(Clone)]
pub(crate) struct Rows {
    pub lines: Vec<Line>,
    starts: Vec<usize>,
}

impl Rows {
    /// The rows of `layout` over `text`, placed from `origin`. A newline between two rows belongs to neither.
    pub(crate) fn of(layout: &TextLayout, text: &str, origin: Pos2) -> Rows {
        let (mut lines, mut starts, mut at) = (Vec::new(), Vec::new(), 0);
        for (row, rect, runs) in layout.row_lines() {
            starts.push(at);
            at += row.len();
            if text.get(at..).is_some_and(|rest| rest.starts_with('\n')) {
                at += 1;
            }
            let runs = runs.into_iter().map(|run| Run { bytes: run.bytes, rtl: run.rtl, positions: run.positions, widths: run.widths }).collect();
            lines.push(Line { text: row, rect: rect.translate(origin.to_vec2()), runs });
        }
        Rows { lines, starts }
    }

    /// The position of character `chars` of `text`.
    pub(crate) fn pos(&self, text: &str, chars: usize) -> TextPos {
        let byte = byte_of(text, chars);
        let line = self.starts.iter().rposition(|start| *start <= byte).unwrap_or(0);
        TextPos::new(line, byte.saturating_sub(self.starts.get(line).copied().unwrap_or(0)))
    }

    /// The character index of `pos` in `text`.
    pub(crate) fn chars(&self, text: &str, pos: TextPos) -> usize {
        let byte = self.starts.get(pos.line).copied().unwrap_or(0) + pos.byte;
        text.get(..byte).map_or_else(|| text.chars().count(), |head| head.chars().count())
    }
}

fn byte_of(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(byte, _)| byte)
}

/// Inserts `insert` over the selection `(a, b)` (characters) of `text`; returns the caret after it.
fn replace(text: &mut String, selection: (usize, usize), insert: &str) -> usize {
    let (lo, hi) = (selection.0.min(selection.1), selection.0.max(selection.1));
    let (start, end) = (byte_of(text, lo), byte_of(text, hi));
    text.replace_range(start..end, insert);
    lo + insert.chars().count()
}

/// Replaces `selection` (characters) of `text` with `insert` and puts the caret after it, in the widget as well.
fn replace_selection(ctx: &Context, widget: Id, text: &mut String, selection: (usize, usize), insert: &str) {
    let caret = replace(text, selection, insert);
    text_edit::select(ctx, widget, caret, caret);
}

/// Applies the IME engine's edits and this pass's edit commands to the draft; returns whether the text changed.
fn apply(input: &mut AdapterIn<'_>, state: &mut TextState, text: &mut String, widget: Id) -> bool {
    let ctx = input.ctx;
    let mut changed = false;
    let selection = |text: &str| text_edit::selection(ctx, widget).unwrap_or((text.chars().count(), text.chars().count()));
    if let Some(preedit) = ime::take_leaving(ctx, widget).filter(|preedit| !preedit.is_empty()) {
        let at = state.preedit.take().map_or_else(|| text.chars().count(), |p| p.at);
        replace_selection(ctx, widget, text, (at, at), &preedit);
        changed = true;
    }
    if let Some(insert) = &input.options.replace_selection {
        replace_selection(ctx, widget, text, selection(text), insert);
        changed = true;
    }
    let edits = ime::take(ctx, widget, input.read_only);
    if !input.focused {
        return changed;
    }
    for edit in edits {
        match edit {
            EditorInput::Preedit { text: composing, .. } => {
                state.preedit = (!composing.is_empty())
                    .then(|| Preedit { at: state.preedit.as_ref().map_or_else(|| selection(text).1, |p| p.at), text: composing.to_string() });
            }
            EditorInput::ImeCommit => {}
            EditorInput::Text(typed) => {
                let at = state.preedit.take().map_or_else(|| selection(text), |p| (p.at, p.at));
                replace_selection(ctx, widget, text, at, &typed);
                changed = true;
            }
            _ => {}
        }
    }
    while let Some(command) = input.intents.take_edit() {
        if input.read_only && !EditorInput::Edit(command).is_read_only_safe() {
            continue;
        }
        match command {
            EditCommand::SelectAll => text_edit::select(ctx, widget, 0, text.chars().count()),
            EditCommand::Copy | EditCommand::Cut => {
                let (a, b) = selection(text);
                if a != b {
                    let (lo, hi) = (a.min(b), a.max(b));
                    clipboard::copy_text(ctx, text[byte_of(text, lo)..byte_of(text, hi)].to_owned());
                    if command == EditCommand::Cut {
                        replace_selection(ctx, widget, text, (lo, hi), "");
                        changed = true;
                    }
                }
            }
            EditCommand::Paste => {
                if let Some(pasted) = keys::pasted_text(ctx, input.id) {
                    let pasted = if input.options.multiline() { pasted.to_string() } else { pasted.replace(['\n', '\r'], " ") };
                    replace_selection(ctx, widget, text, selection(text), &pasted);
                    changed = true;
                }
            }
            EditCommand::Undo | EditCommand::Redo => {
                let redo = command == EditCommand::Redo;
                match text_edit::undo(ctx, widget, text, redo) {
                    Some(previous) => {
                        *text = previous;
                        changed = true;
                    }
                    None => commands::fall_through(ctx, command),
                }
            }
            _ => {}
        }
    }
    changed
}

/// The selection of the text widget `id` as character indices, `(caret, anchor)`.
pub(crate) fn selection(ctx: &Context, id: Id) -> Option<(usize, usize)> {
    text_edit::selection(ctx, id)
}

/// Sets the selection of the text widget `id`: `anchor` and `caret` are character indices.
pub(crate) fn select(ctx: &Context, id: Id, anchor: usize, caret: usize) {
    text_edit::select(ctx, id, anchor, caret);
}

/// The modifiers held in this pass, which are the Enter press's.
fn held_mods(ctx: &Context) -> Mods {
    let os = crate::runtime::windows::os_of(ctx);
    ctx.input(|input| Mods::from_egui(&input.modifiers, os))
}

/// Runs the text widget for one pass.
pub(crate) fn run(state: &mut TextState, mut input: AdapterIn<'_>) -> AdapterOut {
    let (ctx, widget) = (input.ctx, text_id(input.id, input.key));
    let focused_before = input.focused;
    let mut base = state.draft.clone().or_else(|| state.normalised.take()).unwrap_or_else(|| input.shown.to_owned());
    let mut edited = apply(&mut input, state, &mut base, widget);
    if let Some((range, insert)) = state.completions.take(&base) {
        let chars = |byte: usize| base.get(..byte).map_or_else(|| base.chars().count(), |head| head.chars().count());
        let selection = (chars(range.start), chars(range.end));
        replace_selection(ctx, widget, &mut base, selection, &insert);
        edited = true;
    }
    // A clearing field takes Escape while it shows text, with or without a draft, and passes it on once it is empty.
    let takes_escape =
        if input.options.clear_on_escape { !base.is_empty() || state.preedit.is_some() } else { state.draft.is_some() || state.preedit.is_some() };
    if focused_before && takes_escape && !state.completions.open && input.intents.take(Intent::Cancel) {
        // Escape over a composition alone ends the composition and leaves the text.
        if state.draft.is_some() || state.preedit.is_none() {
            state.end = Some(End::Escape);
        }
        state.preedit = None;
    }
    let mut display = base.clone();
    let preedit = state.preedit.as_ref().map(|p| {
        let at = byte_of(&display, p.at);
        display.insert_str(at, &p.text);
        at..at + p.text.len()
    });
    let separated = input.focused && !input.read_only && state.preedit.is_none() && egui_input::take_separator(ctx, input.options.commit_on);
    let filter = egui_focus::event_filter(input.keys.keep, false);
    let edit = text_edit::Edit {
        id: widget,
        rect: input.rect,
        text: &mut display,
        read_only: input.read_only,
        enabled: input.enabled,
        focusable: input.focusable,
        pointer: input.pointer,
        multiline: input.options.multiline(),
        password: input.options.mask,
        hint: input.placeholder,
        mono: input.options.mono,
        role: input.options.role,
        ink: input.ink,
        filter,
        char_limit: input.options.limit,
        preedit: preedit.clone(),
        rtl: input.rtl,
        centred: input.options.centred,
        caret_visible: input.caret_visible,
        underline: input.underline,
    };
    let shown = text_edit::show(input.ui, edit);
    // egui's own IME output is dropped: the engine writes the pass's.
    if let Some(caret) = shown.caret.filter(|_| shown.focused) {
        ime::note_caret(ctx, widget, input.rect, caret, input.options.mask);
    }
    let typed = shown.changed && state.preedit.is_none();
    if typed || edited {
        state.draft = Some(if typed { display.clone() } else { base });
        state.dirty = true;
        state.caret_moved = input.now;
    }
    let caret = text_edit::selection(ctx, widget).map(|selection| selection.0);
    if caret != state.caret {
        state.caret = caret;
        state.caret_moved = input.now;
    }
    let typing = shown.focused && state.preedit.is_none() && !input.read_only;
    let at = state.draft.as_deref().filter(|_| typing).map(|text| (text, caret.unwrap_or_else(|| text.chars().count())));
    state.completions.refresh(input.options.complete.as_deref(), at);
    let submitted =
        (shown.entered && shown.focused).then(|| held_mods(ctx)).filter(|mods| !input.options.multiline() || (input.options.submits && mods.cmd));
    if let Some(mods) = submitted {
        state.end = Some(End::Submit);
        state.submit_mods = mods;
        if input.options.multiline() {
            input.intents.take(Intent::SubmitForce);
        }
    } else if separated {
        state.end = Some(End::Separator);
    } else if shown.lost_focus && state.draft.is_some() && state.end.is_none() {
        state.end = Some(End::Blur);
    }
    state.focused = shown.focused;
    let full = display;
    let clicked_char = input.click.map(|pos| shown.char_at(pos));
    let layout = TextLayout::new(shown.galley, false, full);
    let baseline = (!layout.full.is_empty()).then(|| shown.galley_pos.y + layout.first_baseline);
    AdapterOut { layout, focused: shown.focused, baseline, clicked_char }
}

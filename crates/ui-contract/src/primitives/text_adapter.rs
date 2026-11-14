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
    /// The component put the text back as it was before the run (`Cx::set_draft` with `None`): the run ends as
    /// Escape ends it, but the Escape is not the field's.
    Revert,
}

/// A change of the draft the component asked for, applied in the next pass (`Cx::set_draft`).
#[derive(Clone, Debug)]
pub(crate) enum DraftChange {
    /// The draft becomes this text, as a change of the typing run.
    Text(String),
    /// The run ends with the text from before it.
    Revert,
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
    pub completions: completion::Completions,
    /// The draft change `Cx::set_draft` asked for, applied at the start of the next pass.
    pub change: Option<DraftChange>,
    /// The selection, `(caret, anchor)`, the text part had when its edit menu took the focus: egui collapses the
    /// selection of a widget without focus, and the menu's command acts on it once the focus is back.
    pub held_selection: Option<(usize, usize)>,
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
    // A multi-line part makes a plain Enter a line break.
    if options.submits || options.multiline() { keys.with_submits() } else { keys }
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
    pub rtl: bool,
    /// The text widget holds key focus.
    pub focused: bool,
    /// The text part's own edit menu is open and holds the focus: losing the focus to it ends no typing run, and
    /// getting it back is no new focus.
    pub held: bool,
    pub ink: egui::Color32,
    pub intents: &'a mut Intents,
    /// A list the component opened on the text is shown (a dropdown, the completions): Escape is the list's.
    pub list_open: bool,
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
            EditCommand::SelectAll | EditCommand::Copy => select_or_copy(ctx, widget, text, command),
            EditCommand::Cut => {
                let (a, b) = selection(text);
                if a != b {
                    let (lo, hi) = (a.min(b), a.max(b));
                    clipboard::copy_text(ctx, text[byte_of(text, lo)..byte_of(text, hi)].to_owned());
                    replace_selection(ctx, widget, text, (lo, hi), "");
                    changed = true;
                }
            }
            EditCommand::Delete => {
                let (a, b) = selection(text);
                if a != b {
                    replace_selection(ctx, widget, text, (a.min(b), a.max(b)), "");
                    changed = true;
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

/// Select All or Copy on the text widget `widget` showing `text`: the edit commands read-only text takes.
pub(crate) fn select_or_copy(ctx: &Context, widget: Id, text: &str, command: EditCommand) {
    match command {
        EditCommand::SelectAll => text_edit::select(ctx, widget, 0, text.chars().count()),
        EditCommand::Copy => {
            let Some((a, b)) = text_edit::selection(ctx, widget).filter(|(a, b)| a != b) else { return };
            let (lo, hi) = (a.min(b), a.max(b));
            clipboard::copy_text(ctx, text[byte_of(text, lo)..byte_of(text, hi)].to_owned());
        }
        _ => {}
    }
}

/// The selection of the text widget `id` as character indices, `(caret, anchor)`.
pub(crate) fn selection(ctx: &Context, id: Id) -> Option<(usize, usize)> {
    text_edit::selection(ctx, id)
}

/// Sets the selection of the text widget `id`: `anchor` and `caret` are character indices.
pub(crate) fn select(ctx: &Context, id: Id, anchor: usize, caret: usize) {
    text_edit::select(ctx, id, anchor, caret);
}

/// `TextOptions::arrows_from_end`: before the widget reads a plain Left or Right, a selection of the whole text
/// collapses to its end.
fn arrows_from_end(ctx: &Context, widget: Id, len: usize) {
    let arrow = egui_input::plain_horizontal_arrow_pressed(ctx);
    let whole = selection(ctx, widget).is_some_and(|(caret, anchor)| caret.min(anchor) == 0 && caret.max(anchor) == len);
    if arrow && len > 0 && whole {
        select(ctx, widget, len, len);
    }
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
    if input.held {
        if state.held_selection.is_none() {
            state.held_selection = selection(ctx, widget);
        }
    } else if let Some((caret, anchor)) = state.held_selection.take()
        && input.focused
    {
        select(ctx, widget, anchor, caret);
    }
    let mut base = state.draft.clone().or_else(|| state.normalised.take()).unwrap_or_else(|| input.shown.to_owned());
    let mut edited = apply(&mut input, state, &mut base, widget);
    if let Some((range, insert)) = state.completions.take(&base) {
        let chars = |byte: usize| base.get(..byte).map_or_else(|| base.chars().count(), |head| head.chars().count());
        let selection = (chars(range.start), chars(range.end));
        replace_selection(ctx, widget, &mut base, selection, &insert);
        edited = true;
    }
    match state.change.take() {
        Some(DraftChange::Text(text)) => {
            let all = (0, base.chars().count());
            replace_selection(ctx, widget, &mut base, all, &text);
            edited = true;
        }
        Some(DraftChange::Revert) => state.end = Some(End::Revert),
        None => {}
    }
    // A clearing field takes Escape while it shows text, with or without a draft, and passes it on once it is empty.
    let takes_escape =
        if input.options.clear_on_escape { !base.is_empty() || state.preedit.is_some() } else { state.draft.is_some() || state.preedit.is_some() };
    if focused_before && takes_escape && !state.completions.open && !input.list_open && input.intents.take(Intent::Cancel) {
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
    if input.focused && input.options.arrows_from_end {
        arrows_from_end(ctx, widget, base.chars().count());
    }
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
        galley: None,
    };
    // A secondary press inside the selection keeps the selection for the edit menu it opens, as native fields do;
    // egui's widget moves the caret to any press, so the selection from before the press is put back.
    let secondary = crate::runtime::pointer::frame(ctx).press.is_some_and(|press| press.button == craft_behavior::PointerButton::Secondary);
    let kept = selection(ctx, widget).filter(|(caret, anchor)| secondary && caret != anchor);
    let shown = text_edit::show(input.ui, edit);
    if let Some((caret, anchor)) = kept
        && selection(ctx, widget).is_some_and(|(at, _)| (caret.min(anchor)..=caret.max(anchor)).contains(&at))
    {
        select(ctx, widget, anchor, caret);
    }
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
    } else if shown.lost_focus && !input.held && state.draft.is_some() && state.end.is_none() {
        state.end = Some(End::Blur);
    }
    state.focused = shown.focused || input.held;
    let full = display;
    let layout = TextLayout::new(shown.galley, false, full);
    let baseline = (!layout.full.is_empty()).then_some(shown.galley_pos.y + layout.first_baseline);
    AdapterOut { layout, focused: shown.focused, baseline }
}

/// What [`show_selectable`] shows: the read-only text of a selection part.
pub(crate) struct Selectable<'a> {
    /// The id of the selection widget.
    pub widget: Id,
    /// The component the widget's access node folds into.
    pub owner: Id,
    /// The room the text is painted in; a line wider than it is cut at its edge.
    pub rect: Rect,
    /// The laid-out text and the point it was placed at.
    pub layout: &'a TextLayout,
    pub at: Pos2,
    pub run: &'a crate::primitives::TextRun<'a>,
    pub enabled: bool,
    /// The text colour.
    pub ink: egui::Color32,
}

/// Shows `text` as a read-only selectable widget over its painted lines: the selection behind the characters, the copy
/// command and the pointer's selecting, without a caret.
pub(crate) fn show_selectable(ui: &mut Ui, text: Selectable<'_>) {
    let Selectable { widget, owner, rect, layout, at, run, enabled, ink } = text;
    let ctx = ui.ctx().clone();
    let multiline = run.wrap == crate::primitives::Wrap::Wrap;
    let keep = if multiline { KeepKeys::BlockArrows } else { KeepKeys::InlineArrows };
    let mut shown = layout.full.clone();
    // A line longer than its room is cut at the room's edge, as a field's is.
    let mut cut = (layout.size().x > rect.width()).then(|| {
        let mut cut = crate::runtime::egui::alloc::child(ui, egui::UiBuilder::new().max_rect(rect));
        cut.set_clip_rect(rect.intersect(cut.clip_rect()));
        cut
    });
    text_edit::show(
        cut.as_mut().unwrap_or(&mut *ui),
        text_edit::Edit {
            id: widget,
            rect: layout.galley.rect.translate(at.to_vec2()),
            text: &mut shown,
            read_only: true,
            enabled,
            focusable: enabled,
            pointer: true,
            multiline,
            password: false,
            hint: None,
            mono: false,
            role: run.role,
            ink,
            filter: egui_focus::event_filter(keep, false),
            char_limit: None,
            preedit: None,
            rtl: false,
            centred: false,
            caret_visible: false,
            underline: 0.0,
            galley: Some(layout.galley.clone()),
        },
    );
    if crate::runtime::egui::access::enabled(&ctx, widget) {
        crate::runtime::egui::access::fold_widget(&ctx, widget, owner);
    }
}

//! The types of text parts and editors (18.2, 18.4): keys, options, completion and the own-caret editor state.

use crate::primitives::TextLayout;
use crate::runtime::env::{Label, Message};
use crate::style::Ink;
use craft_behavior::{PartKey, TextPos};
use craft_theme::TypeRole;
use egui::Rect;
use std::marker::PhantomData;
use std::ops::Range;
use std::sync::Arc;

/// A text part's key, typed by the value its codec gives.
#[derive(Debug)]
pub struct TextKey<V>(pub(crate) PartKey, PhantomData<fn() -> V>);

impl<V> Clone for TextKey<V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<V> Copy for TextKey<V> {}

impl<V> TextKey<V> {
    /// The key of the text part `key`.
    pub const fn new(key: PartKey) -> Self {
        TextKey(key, PhantomData)
    }

    /// The part key.
    pub const fn key(self) -> PartKey {
        self.0
    }
}

/// What Tab does in a text field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TabKey {
    /// Tab moves focus.
    #[default]
    Moves,
    /// Tab inserts a tab.
    Inserts,
    /// Tab and Shift+Tab rewrite the draft (the text part selects it and refuses); with no draft they move focus.
    Rewrites,
    /// Tab keeps focus and is handed back as `EditResponse::tab`: `Some(true)` for Tab, `Some(false)` for Shift+Tab.
    Reports,
}

/// Whether a pasted text is whole ([`TextOptions::whole_paste`]).
type WholePaste = dyn Fn(&str) -> bool;

/// The colour ranges of a text ([`TextOptions::color_ranges`]): called with the text each time the part lays it out,
/// which is every frame the part shows. An app whose ranges are costly to find keeps its own cache.
pub type ColorRanges = dyn Fn(&str) -> Vec<ColorRange>;

/// The longest text, in bytes, a part colours: a longer one is laid out in its plain ink, and its
/// [`ColorRanges`] is not called. It bounds the cost of an app's colouring (a highlighter whose time grows with the
/// square of the length on crafted input) at the size of a long script.
pub const COLORED_TEXT_LIMIT: usize = 16 * 1024;

/// A theme colour for a stretch of a field's text. Each is the token's colour moved, where it has to be, to the text
/// contrast minimum on the field's fill, so every ink reads in every theme and contrast.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RangeInk {
    /// `text.muted`.
    Muted,
    /// `accent.text`.
    Accent,
    /// `status.good.fg`.
    Good,
    /// `status.warn.fg`.
    Warn,
    /// `status.bad.fg`.
    Bad,
    /// `status.info.fg`.
    Info,
}

/// One stretch of a text in an ink of its own: `range` is in bytes of the text the [`ColorRanges`] was given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorRange {
    /// The bytes the ink covers.
    pub range: Range<usize>,
    /// Their ink.
    pub ink: RangeInk,
}

impl ColorRange {
    /// The bytes `range` in `ink`.
    pub fn new(range: Range<usize>, ink: RangeInk) -> Self {
        ColorRange { range, ink }
    }
}

/// How a text part behaves beyond its codec.
#[derive(Clone)]
pub struct TextOptions {
    /// Shown while the text is empty.
    pub placeholder: Option<Label>,
    /// The field's look: a quiet one's placeholder takes the muted text colour instead of the faint one.
    pub look: crate::style::FieldLook,
    /// Lines: 1 is a single-line field.
    pub lines: Option<usize>,
    /// The most characters a draft holds.
    pub limit: Option<usize>,
    /// Characters show as dots.
    pub mask: bool,
    /// A parsable draft applies as typed whatever the codec says.
    pub live: bool,
    /// The owner of the current draft, when the same field can edit different values.
    pub edit_target: Option<egui::IdSalt>,
    /// Escape clears the text and commits the empty text while text shows, and goes on to the host once it is empty.
    pub clear_on_escape: bool,
    /// Escape keeps the text: an open typing run commits once, editing ends by leaving focus, and the same Escape
    /// goes on to the enclosing scope. A live filter keeps its query this way. Ignored with `clear_on_escape`, and on a
    /// disabled or read-only part.
    pub keep_on_escape: bool,
    /// Monospace text.
    pub mono: bool,
    /// The type role the text is set in; a monospace text takes the mono role's face and size instead.
    pub role: TypeRole,
    /// The text colour; `None` takes the surface's ink. A disabled part shows the disabled text colour whatever this is.
    pub ink: Option<Ink>,
    /// What Tab does in this field.
    pub tab: TabKey,
    /// The bound caret as a byte offset, read and written through its binding; the adapter keeps no caret of its own.
    pub caret: Option<usize>,
    /// Candidates for the text before the caret.
    pub complete: Option<Arc<dyn Completer>>,
    /// Enter accepts the highlighted candidate, as Tab does; off, Enter stays the field's own (a cell commits on it).
    pub complete_on_enter: bool,
    /// Text that replaces the selection in the pass it is set (the caret inserts it).
    pub replace_selection: Option<String>,
    /// Typing one of these characters commits the draft, as Enter does without submitting; the character is not
    /// inserted. Pasted and composed text is not typing.
    pub commit_on: &'static [char],
    /// Whether the part edits and takes focus. A disabled part shows its text in the disabled text colour while the
    /// rest of the component stays enabled.
    pub enabled: bool,
    /// The text is a value its arrows step: Alt with Up or Down is a fine move.
    pub steps_value: bool,
    /// The whole text is selected when the field takes focus, whatever the codec's caret rule says.
    pub select_on_focus: bool,
    /// A layer that opens on the field (`FocusOrigin::Open`) leaves the caret where the codec's rule puts it instead
    /// of selecting the whole text: a field that opens to add to its text (an edited comment) rather than replace it.
    pub keep_caret_on_open: bool,
    /// Selects the current text before this pass's typing, unless an IME composition is active.
    pub select_all: bool,
    /// Enter does something in the field itself (it saves or runs a search): the field takes the `Submit` intent, so the
    /// enclosing dialog's default button does not act on it.
    pub submits: bool,
    /// The component's value is mixed: the part shows no text, whatever its codec, and typing starts from empty text.
    /// The value handed to the part is the start the codec parses against; committing empty text that the codec
    /// rejects leaves the value mixed.
    pub mixed: bool,
    /// The text sits in the middle of the part instead of at its start (numbers).
    pub centred: bool,
    /// The text sits at the part's end edge, against what follows it (a number and its attached unit sign). Ignored
    /// while `centred`.
    pub end_aligned: bool,
    /// A plain Left or Right with the whole text selected starts from the end of the text, not from the selection's
    /// start: a field whose last digits are what the user edits (a timecode's frames).
    pub arrows_from_end: bool,
    /// Why a draft the codec cannot parse is no value ("Not a hex color"): while such a draft shows, it is the
    /// component's refusing validity message, unless its binding refuses or warns with one of its own.
    pub unparsed: Option<Message>,
    /// The control's caller hangs a context menu of its own on it: a context request opens that menu, not the text
    /// part's edit menu.
    pub own_menu: bool,
    /// A press on a part declared `holds_drafts`, or on a popup layer above the field, leaves the focused draft
    /// unapplied.
    pub holds_drafts: bool,
    /// Set from the codec by `PartsCx::text`, never by hosts: a pasted text that this says is whole replaces all the
    /// text instead of the selection. It gets the paste as copied; the codec trims it.
    // PLAN: no `Send + Sync`: `Codec` and `PartsCx::text`'s codec are not `Send`.
    pub whole_paste: Option<Arc<WholePaste>>,
    /// The key that submits a multi-line text: under `Enter`, plain, Cmd and Ctrl+Enter submit and Shift+Enter breaks
    /// the line.
    pub submit_key: SubmitKey,
    /// The text shown, and exposed as the part's accessible value, instead of the value's own text while the part has
    /// neither focus nor a draft (a derived number in parentheses). Focus, and any typing, start from the value's own
    /// text: this changes only what an idle part shows, never what the codec formats or parses.
    pub resting: Option<String>,
    /// Colours stretches of the text: the ranges it gives, in order, take their inks and the rest the part's ink. A range
    /// that is empty, overlaps the one before, ends past the text or splits a character is skipped. Colour is display
    /// only: editing, the caret, selection, wrapping and what assistive technology reads are as without it. A masked or
    /// disabled part, and a text past [`COLORED_TEXT_LIMIT`], show no colours.
    pub color_ranges: Option<Arc<ColorRanges>>,
}

/// Which Enter chord submits a multi-line text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubmitKey {
    /// Cmd or Ctrl+Enter submits; plain Enter breaks the line.
    #[default]
    CommandEnter,
    /// Plain, Cmd and Ctrl+Enter submit; Shift+Enter breaks the line.
    Enter,
}

impl Default for TextOptions {
    fn default() -> Self {
        TextOptions {
            placeholder: None,
            look: crate::style::FieldLook::Normal,
            lines: None,
            limit: None,
            mask: false,
            live: false,
            edit_target: None,
            clear_on_escape: false,
            keep_on_escape: false,
            mono: false,
            role: TypeRole::Body,
            ink: None,
            tab: TabKey::default(),
            caret: None,
            complete: None,
            complete_on_enter: false,
            replace_selection: None,
            commit_on: &[],
            enabled: true,
            steps_value: false,
            select_on_focus: false,
            keep_caret_on_open: false,
            select_all: false,
            submits: false,
            mixed: false,
            centred: false,
            end_aligned: false,
            arrows_from_end: false,
            unparsed: None,
            own_menu: false,
            holds_drafts: false,
            whole_paste: None,
            submit_key: SubmitKey::CommandEnter,
            resting: None,
            color_ranges: None,
        }
    }
}

impl TextOptions {
    /// Options with every default.
    pub fn new() -> Self {
        Self::default()
    }

    #[doc(hidden)]
    pub fn multiline(&self) -> bool {
        self.lines.is_some_and(|lines| lines > 1)
    }
}

impl std::fmt::Debug for TextOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextOptions")
            .field("lines", &self.lines)
            .field("live", &self.live)
            .field("complete", &self.complete.is_some())
            .field("tab", &self.tab)
            .field("caret", &self.caret)
            .field("holds_drafts", &self.holds_drafts)
            .finish_non_exhaustive()
    }
}

/// Completion candidates for a text part.
pub trait Completer: Send + Sync {
    /// The candidates for the text before the caret, and the byte range they replace.
    fn complete(&self, text: &str, caret: usize, out: &mut Vec<Completion>) -> Option<Range<usize>>;
}

/// One completion candidate.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Completion {
    /// The text shown in the list.
    pub label: Label,
    /// The text that replaces the range.
    pub insert: String,
    /// A short mark before the label (GridCraft's "ƒ" for a function).
    pub mark: Option<Label>,
    /// What the candidate is, shown under the list while its row is highlighted.
    pub description: Option<Label>,
}

impl Completion {
    /// A candidate shown as `label` that inserts `insert`.
    pub fn new(label: impl Into<Label>, insert: impl Into<String>) -> Self {
        Completion { label: label.into(), insert: insert.into(), mark: None, description: None }
    }

    /// Sets the mark before the label.
    #[must_use]
    pub fn mark(mut self, mark: impl Into<Label>) -> Self {
        self.mark = Some(mark.into());
        self
    }

    /// Sets the description shown while the candidate is highlighted.
    #[must_use]
    pub fn description(mut self, description: impl Into<Label>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// What an editor part edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditorPurpose {
    /// Prose.
    Text,
    /// Source code.
    Code,
    /// A number.
    Number,
    /// A secret: the OS shows no suggestions.
    Password,
}

/// One bidi run of a line: its bytes, direction, each character's start offset and advance.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct EditorRun<'a> {
    /// The run's bytes in the line's text.
    pub bytes: Range<usize>,
    /// The run reads right to left.
    pub rtl: bool,
    /// Each character's start offset, relative to the line's rect.
    pub positions: &'a [f32],
    /// Each character's advance.
    pub widths: &'a [f32],
}

impl<'a> EditorRun<'a> {
    /// A run over `bytes`.
    pub const fn new(bytes: Range<usize>, rtl: bool, positions: &'a [f32], widths: &'a [f32]) -> Self {
        EditorRun { bytes, rtl, positions, widths }
    }
}

/// One visible line of an own-caret editor.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct EditorLine<'a> {
    /// The line's text.
    pub text: &'a str,
    /// Where it is drawn.
    pub rect: Rect,
    /// The baseline's y.
    pub baseline: f32,
    /// The bidi runs; a single-direction line is one run.
    pub runs: &'a [EditorRun<'a>],
}

impl<'a> EditorLine<'a> {
    /// A line of `text` at `rect`.
    pub const fn new(text: &'a str, rect: Rect, baseline: f32, runs: &'a [EditorRun<'a>]) -> Self {
        EditorLine { text, rect, baseline, runs }
    }
}

/// What an own-caret editor reports once per frame for the IME and AccessKit.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct EditorState<'a> {
    /// The caret, for the IME candidate window.
    pub caret: Rect,
    /// The visible lines in document order.
    pub lines: &'a [EditorLine<'a>],
    /// The caret's line (an index into `lines`) and byte offset.
    pub caret_at: TextPos,
    /// The selection.
    pub selection: Option<(TextPos, TextPos)>,
    /// The editor shows marked text of an input method. An editor that ends the composition itself (a press in its
    /// text, a selection, an app edit over the marked text) reports `false` and the OS composition is interrupted;
    /// an editor that shows no marked text reports `false` throughout.
    pub composing: bool,
    /// The editor draws the marked text of the pre-edits it is given ([`EditorState::marked`]). Such an editor that
    /// reports `composing: false` while the input method still composes ended the composition itself, even in the pass
    /// the pre-edit arrived in (a pre-edit and a refused deletion in one frame), and the OS composition is interrupted.
    /// An editor that shows no marked text leaves it `false`, so the OS composes for it until the commit.
    pub shows_marked: bool,
    /// Whether undo and redo have something to do; false sends them to the app command (11.3).
    pub can_undo: bool,
    /// Whether redo has something to do.
    pub can_redo: bool,
    /// The committed text around the caret, or why there is none, with the input-method target's epoch.
    pub surrounding: Option<(u64, EditorSurrounding<'a>)>,
}

pub use crate::runtime::ime::EditorSurrounding;

/// The input method's mutation stamp, which an editor compares to tell its own edits from the input method's.
pub use egui::ime_context::ImeMutationStamp as ImeStamp;

impl<'a> EditorState<'a> {
    /// Reports `s` as the committed text around the caret, for the input-method target at `epoch`.
    #[must_use]
    pub fn surrounding(mut self, epoch: u64, s: EditorSurrounding<'a>) -> Self {
        self.surrounding = Some((epoch, s));
        self
    }

    /// A state with no selection, no marked text and nothing to undo.
    pub const fn new(caret: Rect, lines: &'a [EditorLine<'a>], caret_at: TextPos) -> Self {
        EditorState {
            caret,
            lines,
            caret_at,
            selection: None,
            composing: false,
            shows_marked: false,
            can_undo: false,
            can_redo: false,
            surrounding: None,
        }
    }

    /// Reports an editor that draws the input method's marked text, `composing` while it shows some
    /// ([`EditorState::shows_marked`]).
    #[must_use]
    pub const fn marked(mut self, composing: bool) -> Self {
        self.shows_marked = true;
        self.composing = composing;
        self
    }
}

impl TextLayout {
    /// The layout's runs as the borrowed runs an editor line takes.
    pub fn editor_runs(&self) -> Vec<EditorRun<'_>> {
        self.runs().iter().map(|run| EditorRun::new(run.bytes.clone(), run.rtl, &run.positions, &run.widths)).collect()
    }
}

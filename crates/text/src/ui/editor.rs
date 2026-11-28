//! [`TextEditor`]: typing on a canvas, the editing session of a type tool or of a text frame, cell or caption typed
//! in place.
//!
//! The app owns the text, its layout and its undo; the editor owns the click run, the marked text and the keys, and
//! reads and writes the selection through its `Bind`. It reads and changes the text through the app's
//! [`TextEditorHost`], one `replace` per edit, and draws the selection, the caret and the marked text's underline
//! over the canvas. The text itself is painted by the app.
//!
//! The editor is its own root with one editor part over the rect it is shown at (the text's screen bounds): a press
//! inside it is the editor's, a press outside it is not. A primary press on the canvas background (one no canvas
//! control claims, [`CanvasSpace::background`](craft_ui::canvas::CanvasSpace::background)) commits the session
//! (Photoshop); the press still reaches the canvas's tool. A secondary press leaves the session open for the canvas
//! menu, which acts on the live edit.
//!
//! Offsets are chars of the host's text.
//!
//! The marked (preedit) text of an input method is written into the host's text with [`EditKind::Composing`] and
//! replaced by every update, so it lays out, wraps and takes the character style exactly as typed text does. The
//! editor remembers its range and underlines it, the converted segment thicker. A commit replaces the range with
//! the result; a click keeps the marked text as typed. Whenever the editor ends the composition itself (a press, a
//! selection, an app edit over the marked text), it reports that it no longer composes and the input method's
//! composition is interrupted.

mod commands;
mod edits;
#[cfg(feature = "suite")]
pub(crate) mod stories;

use std::ops::Range;

use craft_behavior::kurbo::Point;
use craft_behavior::{
    CancelReason, CanvasView, CursorIcon, CursorPriority, EditCommand, EditorInput, GesturePhase, Metrics, PressPhase, Sense, TextPos,
};
use craft_ui::canvas::{canvas_space, context_click};
use craft_ui::contract::{
    Animate as _, Bind, BindMut, CommandSet as _, Commands as _, Common, Component, Cx, Edit as _, EditResponse, EditorStep, FacesCx, Focus as _,
    MeasureCx, Outcome, PaintUi, PartsCx, Pointer as _, Props as _, Remember as _, Role,
};
use craft_ui::fonts::{FaceBytes, FaceState};
use craft_ui::primitives::{EditorLine, EditorPurpose, EditorRun, EditorState, PaintShape, StrokeWidth};
use craft_ui::runtime::commands::ClipItem;
use craft_ui::runtime::env::Label;
use craft_ui::runtime::focus::{FocusPolicy, KeyUse};
use craft_ui::runtime::pending::Pending;
use craft_ui::style::{ComponentStyle, StyleColor, StyleCx};
use egui::{Pos2, Rect, Response, Vec2};

pub use commands::TextEditorCommand;
pub use edits::{click_count, click_selection, word_boundary};

use crate::quotes::QuoteStyle;
use craft_keymap::{Chord, Key, Mods};
use edits::{Edits, Session, char_slice, fingerprint};

const ESCAPE: Chord = Chord::plain(Key::Escape);
const ENTER: Chord = Chord::plain(Key::Enter);
const TAB: Chord = Chord::plain(Key::Tab);
const SHIFT_TAB: Chord = Chord::new(Mods::SHIFT, Key::Tab);

/// The keys that end a one-line field's session: Enter keeps the text, Tab and Shift+Tab keep it and move on.
const FIELD_LINE_ENDS: [(Chord, EditorEnd); 3] = [(ENTER, EditorEnd::Commit), (TAB, EditorEnd::Next), (SHIFT_TAB, EditorEnd::Previous)];
/// The keys that end a multi-line field's session; Enter is a line break there.
const FIELD_LINES_ENDS: [(Chord, EditorEnd); 2] = [(TAB, EditorEnd::Next), (SHIFT_TAB, EditorEnd::Previous)];

/// The selection: `anchor` stays where it started, `caret` moves. Equal: a caret with nothing selected. Char
/// offsets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct TextSelection {
    /// Char offset of the caret, the end that moves.
    pub caret: usize,
    /// Char offset of the anchor, the end that stays.
    pub anchor: usize,
}

impl TextSelection {
    /// A selection from `anchor` to `caret`.
    pub fn new(anchor: usize, caret: usize) -> Self {
        TextSelection { caret, anchor }
    }

    /// A caret at `at` with nothing selected.
    pub fn caret_at(at: usize) -> Self {
        TextSelection { caret: at, anchor: at }
    }

    /// The selected chars, start before end.
    pub fn range(self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    /// Whether nothing is selected: the caret is at the anchor.
    pub fn is_empty(self) -> bool {
        self.caret == self.anchor
    }
}

/// The caret as a segment in document units, from the line's ascent to its descent: vertical in horizontal text,
/// horizontal in vertical text, slanted when the layer is rotated or skewed.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CaretLine {
    /// The caret's top end, at the line's ascent.
    pub top: Point,
    /// The caret's bottom end, at the line's descent.
    pub bottom: Point,
}

impl CaretLine {
    /// A caret from `top` to `bottom`.
    pub fn new(top: Point, bottom: Point) -> Self {
        CaretLine { top, bottom }
    }
}

/// What kind of edit a [`TextEditorHost::replace`] call is, for the app's history and styling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditKind {
    /// Typed text, a line break, or smart-quoted text.
    Typed,
    /// Marked text of a composition replacing the previous marked text. The app keeps it out of spelling marks and
    /// autocorrection; history coalesces it with the commit.
    Composing,
    /// The text a composition committed, replacing its marked text.
    Committed,
    /// Pasted text (no smart quotes).
    Pasted,
    /// Backspace, Delete, Cut, or an input method's delete-surrounding.
    Deleted,
}

/// A character style toggle on the selection (or the insertion style at a caret).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CharToggle {
    /// All Caps.
    AllCaps,
    /// Underline.
    Underline,
    /// Faux Bold.
    FauxBold,
    /// Faux Italic.
    FauxItalic,
    /// Strikethrough.
    Strikethrough,
}

/// A formatting key the editor recognised, for the app to apply to [`TextEditorHost::style_key`]'s selection. The
/// chords are those of [`TextEditorCommand`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum StyleKey {
    /// All Caps, Underline, Faux Bold, Faux Italic or Strikethrough.
    Toggle(CharToggle),
    /// Hyphenate on the selected paragraphs.
    Hyphenate,
    /// Leading by the given points (negative: less).
    Leading(f32),
    /// Baseline shift by the given points (positive: up).
    BaselineShift(f32),
    /// Tracking of the selection by the given thousandths of an em.
    Tracking(f32),
    /// Kerning of the pair around the caret, as [`StyleKey::Tracking`]. At a line or text edge there is no pair:
    /// the host does nothing and returns `true` (Photoshop), or returns `false` and the key moves by word.
    Kern(f32),
}

/// One bidi run of a [`HostLine`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HostRun {
    /// The run's bytes in the line's text.
    pub bytes: Range<usize>,
    /// The run reads right to left.
    pub rtl: bool,
    /// Each char's start offset along the line, in document units.
    pub positions: Vec<f32>,
    /// Each char's advance, in document units.
    pub widths: Vec<f32>,
}

impl HostRun {
    /// A run over `bytes` of the line, left to right unless `rtl`, with each char's start `positions` and advance
    /// `widths`.
    pub fn new(bytes: Range<usize>, rtl: bool, positions: Vec<f32>, widths: Vec<f32>) -> Self {
        HostRun { bytes, rtl, positions, widths }
    }
}

/// One laid-out line of the host's text, in document units, for the input method and assistive technology.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HostLine {
    /// The line's text without its line break.
    pub text: String,
    /// The line's chars in the host's text.
    pub chars: Range<usize>,
    /// The line's box; a rotated or skewed frame gives the box of its bounds.
    pub rect: Rect,
    /// The baseline's y.
    pub baseline: f32,
    /// The bidi runs; a single-direction line is one run.
    pub runs: Vec<HostRun>,
}

impl HostLine {
    /// A line of `text` covering `chars` of the host's text, in the box `rect` with its `baseline` y and bidi `runs`.
    pub fn new(text: String, chars: Range<usize>, rect: Rect, baseline: f32, runs: Vec<HostRun>) -> Self {
        HostLine { text, chars, rect, baseline, runs }
    }
}

/// What the editor gives its host to lay the text out with ([`TextEditorHost::lay_out`]): the document faces of the
/// editor's context, through [`FacesCx`].
pub struct HostCx<'a> {
    faces: &'a dyn Faces,
}

impl FacesCx for HostCx<'_> {
    fn document_face<L>(&self, key: &str, job: impl FnOnce() -> L) -> FaceState
    where
        L: FnOnce() -> Option<FaceBytes> + Send + 'static,
    {
        let mut job = Some(job);
        self.faces.face(key, &mut || job.take().map(|job| Box::new(job()) as Load))
    }

    fn document_face_bytes(&self, key: &str) -> Option<FaceBytes> {
        self.faces.bytes(key)
    }
}

/// A face load, as [`FacesCx::document_face`]'s job makes it.
type Load = Box<dyn FnOnce() -> Option<FaceBytes> + Send>;

/// [`FacesCx`] behind a reference: the editor's context, whose type names the editor's lifetimes.
trait Faces {
    fn face(&self, key: &str, job: &mut dyn FnMut() -> Option<Load>) -> FaceState;
    fn bytes(&self, key: &str) -> Option<FaceBytes>;
}

impl<T: FacesCx> Faces for T {
    fn face(&self, key: &str, job: &mut dyn FnMut() -> Option<Load>) -> FaceState {
        self.document_face(key, || {
            let load = job();
            move || load.and_then(|load| load())
        })
    }

    fn bytes(&self, key: &str) -> Option<FaceBytes> {
        self.document_face_bytes(key)
    }
}

/// The app's text as the editor sees it. Implemented over the edited text object (a type layer, a text frame, a
/// cell) for one editing session.
///
/// Geometry is in document units; the editor maps it to the screen with the canvas view of the pass. Layout calls
/// take `&mut self` so the host may cache its layout between them.
pub trait TextEditorHost {
    /// The whole text, including the marked text while composing.
    fn text(&self) -> &str;

    /// Lays the text out for the pass, before the editor asks for anything else in it. `cx` reaches the document
    /// faces of the editor's context ([`FacesCx`]): a host whose text is set in a document's own fonts asks for them
    /// here, wherever the editor is shown (over a canvas, in a page view's body). The default lays out nothing.
    fn lay_out(&mut self, _cx: &HostCx<'_>) {}

    /// Replace the chars `range` with `text`. The host applies its insertion style (the style the Character panel
    /// set at the caret), records history (one step per editing session) and lays the text out again. It returns the
    /// chars it inserted in place of `text`: fewer when it kept only part of it (a field's maximum length), and the
    /// editor puts the caret after them. `None` when it refused (a locked layer): the editor leaves its selection as
    /// it was.
    fn replace(&mut self, range: Range<usize>, text: &str, kind: EditKind) -> Option<usize>;

    /// The char offset nearest `at`, and the laid-out line it is on.
    fn hit(&mut self, at: Point) -> (usize, usize);

    /// The caret at `offset` on line `line` (`None`: the line the offset starts, which differs only at a soft wrap).
    /// `None` while the text has no layout yet.
    fn caret(&mut self, offset: usize, line: Option<usize>) -> Option<CaretLine>;

    /// The selection's highlight for `range`: one quadrilateral per line piece, corners in document units, in
    /// drawing order (bidi text gives several per line).
    fn selection_quads(&mut self, range: Range<usize>) -> Vec<[Point; 4]>;

    /// Where the marked text's underline runs for `range`: one segment per line piece, below the baseline, in
    /// document units.
    fn underline_segments(&mut self, range: Range<usize>) -> Vec<[Point; 2]>;

    /// The laid-out lines in document order, for the input method's candidate window and assistive technology.
    fn lines(&mut self) -> Vec<HostLine>;

    /// The chars of laid-out line `line`, without its line break.
    fn line_range(&mut self, line: usize) -> Range<usize>;

    /// The offset `lines` lines above (negative) or below `offset`, nearest the caret's position along the line
    /// (columns in vertical text).
    fn line_step(&mut self, offset: usize, lines: i32) -> usize;

    /// The start or end of the laid-out line holding `offset` (Home / End, Cmd+Backspace).
    fn line_edge(&mut self, offset: usize, end: bool) -> usize;

    /// Whether the text runs vertically: the arrow keys turn with it.
    fn vertical(&self) -> bool {
        false
    }

    /// Apply a formatting key to `selection`. `false` (the default) when the app has no such formatting.
    fn style_key(&mut self, _key: StyleKey, _selection: TextSelection) -> bool {
        false
    }
}

/// The style of the editor: the selection fill, the caret's two strokes and the marked text's underline.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TextEditorStyle {
    #[style(metrics)]
    metrics: Metrics,
    selection: StyleColor,
    caret: StyleColor,
    halo: StyleColor,
    underline: StyleColor,
}

/// How an editing session ended ([`TextEditorOutput::ended`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditorEnd {
    /// The app keeps the text.
    Commit,
    /// The app undoes the session.
    Cancel,
    /// Tab: the app keeps the text and moves to the next field.
    Next,
    /// Shift+Tab: the app keeps the text and moves to the previous field.
    Previous,
}

/// What the editor is: which keys end its session and which are text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum EditorMode {
    /// The canvas text editor: Enter is a line break and Tab a tab character.
    #[default]
    Canvas,
    /// A one-line form field: Enter commits, Tab and Shift+Tab move to the next or previous field, and pasted line
    /// breaks become spaces.
    FieldLine,
    /// A multi-line form field: Enter is a line break, Tab and Shift+Tab move to the next or previous field.
    FieldLines,
}

impl EditorMode {
    /// Whether this mode accepts newlines.
    fn multiline(self) -> bool {
        self != EditorMode::FieldLine
    }

    /// Whether this mode inserts tab characters rather than moving focus.
    fn takes_tab(self) -> bool {
        self == EditorMode::Canvas
    }

    /// The chords that end a session in this mode and how each ends it. The editor claims them while focused, so
    /// they reach it rather than focus navigation or its own text.
    fn ending_keys(self) -> &'static [(Chord, EditorEnd)] {
        match self {
            EditorMode::Canvas => &[],
            EditorMode::FieldLine => &FIELD_LINE_ENDS,
            EditorMode::FieldLines => &FIELD_LINES_ENDS,
        }
    }
}

/// `text` on one line: each line ending (`\r\n`, `\r` or `\n`) becomes one space.
fn one_line(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

/// What the editor did this pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "end the session on `ended`"]
pub struct TextEditorOutput {
    /// The editor's response.
    pub response: Response,
    /// The text changed (typing, a composition, a paste, a delete).
    pub changed: bool,
    /// The selection or caret moved.
    pub selection_changed: bool,
    /// The text changes of the session as one gesture over the selection. The first text change gives `Begin`, later
    /// changes `Update`; the session's end ([`ended`](Self::ended)) commits or cancels the gesture when one runs.
    pub edit: EditResponse<TextSelection>,
    /// The session ended this pass, whether or not its text changed. `Commit` is Cmd+Enter, Enter in a
    /// [`EditorMode::FieldLine`], a primary press on the canvas background, or Esc when Esc commits
    /// ([`TextEditor::esc_commits`], Photoshop's Use ESC Key to Commit Text, on by default): the app ends the
    /// session, keeps the text and records one undo step when it changed. `Next` and `Previous` are Tab and
    /// Shift+Tab in a field mode: the app keeps the text as for `Commit` and moves to the next or previous field.
    /// `Cancel` is Cmd+., or Esc when it doesn't commit: the app ends the session and undoes it. While a composition
    /// is open the keys give none of them.
    pub ended: Option<EditorEnd>,
    /// The Character panel command was pressed: the host shows or hides the panel. Set for the one pass.
    pub character_panel: bool,
    /// Where a secondary press on the editor was released, for the host's context menu (the type edit menu, which acts
    /// on the live edit). The editor's part takes the press, so the canvas's tool never sees it. Set for the one pass.
    pub context: Option<Pos2>,
}

impl Outcome for TextEditorOutput {
    type Value = TextSelection;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "screen-testing")]
    fn story_facts(&self) -> craft_ui::testing::StoryOutput {
        self.edit.story_facts()
    }
}

/// A text editing session on a canvas: selection, click runs, the input method and the keys. The app shows one over
/// the text's screen bounds while the session is open and drops it when the session ends.
///
/// What the user sees and does:
/// - A caret drawn as two strokes (light under dark), so it shows on any image. It blinks in Photoshop's rhythm for
///   as long as the session runs, also while another control holds the keys, solid while dragging and restarting at
///   each edit or move. Marked text is underlined, the converted segment
///   thicker.
/// - Clicks: one places the caret, Shift-click extends; quick clicks in one spot select more: two a word, three the
///   laid-out line, four the paragraph, five all text, the sixth starts over (Photoshop). Dragging extends by char,
///   and by word or line after a double or triple click. A primary press on the canvas outside the text commits.
/// - Keys: Left and Right by char (a selection collapses to its edge), with Cmd or Alt by word, Up and Down by
///   line, Cmd+Up and Cmd+Down to the text's start or end, Home and End to the line's edges, Shift extends any of
///   them. Backspace and Delete by char, Alt by word, Cmd+Backspace to the line start. Cmd+A all, Cmd+C, Cmd+X and
///   Cmd+V the clipboard. Enter a line break; Cmd+Enter commits; Esc commits or cancels; Cmd+. cancels;
///   Cmd+T asks the host for its Character panel. In a form field ([`mode`](Self::mode)) Tab and Shift+Tab end the
///   session toward the next or previous field, a press that moves focus away commits, and in a one-line field Enter commits. The
///   [`TextEditorCommand`]s format. In vertical text the arrows turn with the flow, the formatting keys too: Alt+Up
///   and Alt+Down track, Alt+Left and Alt+Right set the leading. A formatting key the host does not take moves the
///   caret as its arrow does: a line, a word, or with Cmd to the text's start or end; Shift extends.
/// - Typed straight quotes become the language's marks while [`smart_quotes`](Self::smart_quotes) is set; pasted
///   text is kept as it is. Pasted line endings become `\n`, in a one-line field a space.
/// - A [`masked`](Self::masked) editor is a password field: the host draws its text as bullets, and Copy and Cut
///   put nothing on the clipboard.
#[derive(craft_ui::contract::Component, craft_ui::contract::Props)]
pub struct TextEditor<'a> {
    #[common]
    common: Common,
    host: &'a mut dyn TextEditorHost,
    selection: Bind<'a, TextSelection>,
    /// Convert typed straight quotes with this style (unset: off). The app sets it from its Use Smart Quotes
    /// preference and the text's language at the caret.
    #[prop]
    smart_quotes: Option<QuoteStyle>,
    /// Whether Esc commits (Photoshop's default) or cancels the session.
    #[prop]
    esc_commits: bool,
    /// Whether focus goes back to the stop it came from when the session ends (on by default: the canvas text editor
    /// hands it back to the canvas). Off, the editor opens no focus scope, and an ended session leaves focus where the
    /// removal of a stop leaves it: for a form field whose own part handed it the focus, returning there would open the
    /// field again.
    #[prop]
    returns_focus: bool,
    /// What the editor is: the canvas text editor, or a one-line or multi-line form field.
    #[prop]
    mode: EditorMode,
    /// Whether the text is a secret, for a password field: the host draws it as bullets, the input method and
    /// assistive technology treat it as a password, and Copy and Cut put nothing on the clipboard.
    #[prop(flag)]
    masked: bool,
    changed: bool,
    selection_changed: bool,
    character_panel: bool,
    context: Option<Pos2>,
    ended: Option<EditorEnd>,
}

impl<'a> TextEditor<'a> {
    /// An editor named `label` over the app's text `host`, with the selection bound to `selection`.
    pub fn new(label: impl Into<Label>, host: &'a mut dyn TextEditorHost, selection: Bind<'a, TextSelection>) -> Self {
        TextEditor {
            common: Common::new(),
            host,
            selection,
            smart_quotes: None,
            esc_commits: true,
            returns_focus: true,
            mode: EditorMode::Canvas,
            masked: false,
            changed: false,
            selection_changed: false,
            character_panel: false,
            context: None,
            ended: None,
        }
        .label(label)
        .command_scope(TextEditorCommand::SCOPE)
    }
}

/// The canvas view of the pass, with the identity as the fallback outside a canvas.
struct Space(Option<CanvasView>);

impl Space {
    fn of(env: &craft_ui::runtime::env::EnvView<'_>) -> Self {
        Space(canvas_space(env).map(|space| space.view))
    }

    fn screen(&self, p: Point) -> Pos2 {
        self.0.as_ref().map_or_else(|| Pos2::new(p.x as f32, p.y as f32), |view| view.to_screen(p))
    }

    fn doc(&self, p: Pos2) -> Point {
        self.0.as_ref().map_or_else(|| Point::new(f64::from(p.x), f64::from(p.y)), |view| view.to_doc(p))
    }

    fn rect(&self, rect: Rect) -> Rect {
        Rect::from_two_pos(
            self.screen(Point::new(f64::from(rect.min.x), f64::from(rect.min.y))),
            self.screen(Point::new(f64::from(rect.max.x), f64::from(rect.max.y))),
        )
    }

    /// A screen rect as document units.
    fn doc_rect(&self, rect: Rect) -> Rect {
        let (min, max) = (self.doc(rect.min), self.doc(rect.max));
        Rect::from_two_pos(Pos2::new(min.x as f32, min.y as f32), Pos2::new(max.x as f32, max.y as f32))
    }

    fn zoom(&self) -> f32 {
        self.0.as_ref().map_or(1.0, |view| view.zoom as f32)
    }
}

/// The lines the host reports, or the whole text as one line when it has no layout yet.
fn lines_of(host: &mut dyn TextEditorHost, fallback: Rect) -> Vec<HostLine> {
    let lines = host.lines();
    if !lines.is_empty() {
        return lines;
    }
    let text = host.text().to_owned();
    let chars = 0..text.chars().count();
    vec![HostLine { text, chars, rect: fallback, baseline: fallback.bottom(), runs: Vec::new() }]
}

/// The position of char `at` among `lines`: the line that holds it (a soft wrap belongs to the line it ends) and
/// the byte in that line.
fn text_pos(lines: &[HostLine], at: usize) -> TextPos {
    let index = lines
        .iter()
        .position(|line| line.chars.start <= at && at < line.chars.end)
        .or_else(|| lines.iter().rposition(|line| line.chars.start <= at))
        .unwrap_or(0);
    let Some(line) = lines.get(index) else { return TextPos::new(0, 0) };
    let within = at.saturating_sub(line.chars.start);
    let byte = line.text.char_indices().nth(within).map_or(line.text.len(), |(byte, _)| byte);
    TextPos::new(index, byte)
}

/// The char at a position AccessKit sent, relative to `lines`.
fn char_at(lines: &[HostLine], pos: TextPos) -> usize {
    let Some(line) = lines.get(pos.line.min(lines.len().saturating_sub(1))) else { return 0 };
    let byte = pos.byte.min(line.text.len());
    line.chars.start + line.text[..byte].chars().count()
}

/// The I-beam is the editor's over its text and while its own press is held; elsewhere the canvas's tool or the rest
/// of the UI sets the pointer.
fn show_i_beam(cx: &mut Cx<'_, TextEditor<'_>>, phase: &PressPhase, vertical: bool) {
    if !matches!(phase, PressPhase::Idle) {
        cx.cursor(if vertical { CursorIcon::VerticalText } else { CursorIcon::Text }, CursorPriority::Hover);
    }
}

/// Applies one pass's editor steps to `edits` in order: typing, the marked text, moves, deletes, selections, the edit
/// commands and the commands keys fired. Returns the text a Copy or Cut put on the clipboard, and whether a command
/// asked for the character panel.
fn apply_steps(
    edits: &mut Edits<'_>,
    steps: Vec<EditorStep<TextEditorCommand>>,
    mode: EditorMode,
    masked: bool,
    pasted: Option<&str>,
    fallback: Rect,
) -> (Option<String>, bool) {
    let mut copied: Option<String> = None;
    let mut ending: Option<Range<usize>> = None;
    let mut character_panel = false;
    for step in steps {
        // A commit or cancel ends the session; an input method's committed text still lands.
        if (edits.done.commit || edits.done.cancel) && ending.is_none() {
            break;
        }
        let input = match step {
            EditorStep::Input(input) => input,
            EditorStep::Command(command, _) => {
                edits.command(command, &mut character_panel);
                continue;
            }
            _ => continue,
        };
        if let Some(range) = ending.take() {
            if let EditorInput::Text(text) = &input {
                edits.commit_marked(range, text);
                continue;
            }
            edits.drop_marked(range);
        }
        match input {
            EditorInput::Preedit { text, cursor, .. } => {
                let active = cursor.map(|bytes| byte_range_to_chars(&text, bytes));
                ending = edits.preedit(&text, active);
            }
            EditorInput::ImeCommit => ending = edits.session.preedit.take(),
            EditorInput::Text(text) => edits.done.changed |= edits.insert_typed(&text),
            EditorInput::Newline { .. } if edits.session.preedit.is_none() && mode.multiline() => {
                edits.done.changed |= edits.insert_text("\n", EditKind::Typed);
            }
            EditorInput::Tab if mode.takes_tab() => edits.done.changed |= edits.insert_text("\t", EditKind::Typed),
            EditorInput::Move(step, mods) => edits.motion(step, mods),
            EditorInput::Delete { forward, by, .. } => edits.done.changed |= edits.delete(forward, by),
            EditorInput::DeleteSurrounding { before, after, unit, .. } => edits.done.changed |= edits.delete_around(before, after, unit),
            EditorInput::Select { anchor, focus, .. } => {
                let lines = lines_of(&mut *edits.host, fallback);
                edits.end_composition();
                edits.selection = TextSelection { anchor: char_at(&lines, anchor), caret: char_at(&lines, focus) };
            }
            EditorInput::Edit(EditCommand::SelectAll) => edits.select_all(),
            EditorInput::Edit(EditCommand::Copy) if !masked => copied = edits.selected_text(),
            EditorInput::Edit(EditCommand::Cut) if !masked => {
                copied = edits.selected_text();
                edits.done.changed |= edits.delete_selection();
            }
            EditorInput::Edit(EditCommand::Paste) => {
                if let Some(text) = pasted {
                    let text = if mode.multiline() { text.to_string() } else { one_line(text) };
                    edits.done.changed |= edits.insert_text(&text, EditKind::Pasted);
                }
            }
            _ => {}
        }
    }
    if let Some(range) = ending {
        edits.drop_marked(range);
    }
    (copied, character_panel)
}

/// The clipboard's text when the pass's input asks to paste and the clipboard has it ready.
fn pasted_text(cx: &mut Cx<'_, TextEditor<'_>>, steps: &[EditorStep<TextEditorCommand>]) -> Option<std::sync::Arc<str>> {
    if !steps.iter().any(|step| matches!(step, EditorStep::Input(EditorInput::Edit(EditCommand::Paste)))) {
        return None;
    }
    match cx.clipboard().paste() {
        Pending::Ready(content) => content.text.clone(),
        _ => None,
    }
}

impl Component for TextEditor<'_> {
    const ROLE: Role = Role::MULTILINE_TEXT_INPUT;
    const INPUT: craft_ui::contract::Input = craft_ui::contract::Input::Editor;
    /// Escape always ends the session, so the editor keeps it from its row and from the shortcuts while focused.
    const KEYS: KeyUse = KeyUse::CODE.with_claims(&[ESCAPE]).with_holds_drafts();
    /// When the session ends, focus goes back to the stop it came from (the canvas), not to the stop before the editor.
    const FOCUS_SCOPE: FocusPolicy = FocusPolicy::TAB_LAYER;
    type Style = TextEditorStyle;
    type Output = TextEditorOutput;

    fn focus_scope(&self) -> FocusPolicy {
        if self.returns_focus { Self::FOCUS_SCOPE } else { FocusPolicy::NONE }
    }

    fn style(cx: &StyleCx<'_>, _: TextEditorStyleKey) -> TextEditorStyle {
        let overlay = &cx.tokens.color.overlay;
        TextEditorStyle {
            metrics: Metrics::new(),
            selection: cx.color(cx.tokens.color.state.text_selection),
            caret: cx.color(overlay.stroke),
            halo: cx.color(overlay.halo),
            underline: cx.color(cx.tokens.color.accent.solid),
        }
    }

    fn sense(&self) -> Sense {
        Sense::CLICK | Sense::DRAG | Sense::CONTEXT_CLICK | Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn measure(&mut self, _cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn bind(&mut self) -> Option<BindMut<'_, TextSelection>> {
        Some(self.selection.reborrow())
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.editor(EDITOR, rect, if self.masked { EditorPurpose::Password } else { EditorPurpose::Text });
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        // Update runs before paint, so one layout serves the pass's edits and its paint.
        self.host.lay_out(&HostCx { faces: &*cx });
        let space = Space::of(&cx.env);
        let before = cx.value().copied().unwrap_or_default();
        // The inputs and the commands keys fired, in the order their events came: typing after a commit's chord is
        // not the session's.
        let steps = cx.editor_steps::<TextEditorCommand>();
        let press = *cx.press();
        let now = cx.now();
        let pasted = pasted_text(cx, &steps);
        let (mode, masked) = (self.mode, self.masked);
        let mut session = std::mem::take(cx.memory::<Session>());
        let mut edits = Edits {
            host: &mut *self.host,
            session: &mut session,
            selection: before,
            smart_quotes: self.smart_quotes,
            esc_commits: self.esc_commits,
            done: Default::default(),
        };
        if edits.session.last_selection.is_some_and(|last| last != before) {
            edits.end_composition();
        }
        edits.reconcile();
        let fallback = space.doc_rect(cx.rect);

        let (copied, character_panel) = apply_steps(&mut edits, steps, mode, masked, pasted.as_deref(), fallback);
        // A press is the pass's last pointer event (H0), so the pass's editor input came before it.
        match press.phase {
            PressPhase::Down { start, .. } if start.button == craft_behavior::PointerButton::Primary => {
                edits.press(space.doc(start.origin), press.mods.shift, start.clicks, now);
            }
            PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => edits.drag(space.doc(pos)),
            PressPhase::Release { .. } | PressPhase::Cancel { .. } => edits.session.dragging = None,
            _ => {}
        }
        // A primary press elsewhere on the canvas keeps the marked text as typed, as a press on the text does. A
        // secondary press leaves the session open: the canvas menu it opens acts on the live edit.
        if canvas_space(&cx.env).is_some_and(|space| space.background().pressed == Some(craft_behavior::PointerButton::Primary)) {
            edits.end_composition();
            edits.session.dragging = None;
            edits.done.commit = true;
        }
        if mode != EditorMode::Canvas && cx.editor_commit_requested() {
            edits.end_composition();
            edits.session.dragging = None;
            edits.done.commit = true;
        }

        // Commands and the claimed keys arrive with the pass's own input: a second pass of the frame takes none, so
        // a commit, a cancel, a move to another field or the panel toggle is reported by one pass only. No key ends
        // the session while a composition is open: the key belongs to the input method then.
        if cx.take_claimed(ESCAPE) && edits.session.preedit.is_none() {
            if edits.esc_commits {
                edits.done.commit = true;
            } else {
                edits.done.cancel = true;
            }
        }
        for &(chord, end) in mode.ending_keys() {
            cx.claim(chord);
            if cx.take_claimed(chord) && edits.session.preedit.is_none() {
                match end {
                    EditorEnd::Commit => edits.done.commit = true,
                    EditorEnd::Next => edits.done.next = true,
                    EditorEnd::Previous => edits.done.previous = true,
                    EditorEnd::Cancel => edits.done.cancel = true,
                }
            }
        }

        let selection = edits.selection;
        let done = edits.done;
        if done.changed || selection != before {
            edits.session.blink_start = now;
        }
        edits.session.seen = Some(fingerprint(edits.host.text()));
        edits.session.last_selection = Some(selection);
        let vertical = edits.host.vertical();
        *cx.memory::<Session>() = session;

        if let Some(text) = copied {
            cx.clipboard().put(&[ClipItem::Text(text.into())]);
        }
        show_i_beam(cx, &press.phase, vertical);
        if done.changed {
            cx.propose(selection);
        } else if selection != before {
            if cx.gesture().is_some_and(|phase| matches!(phase, GesturePhase::Begin | GesturePhase::Update)) {
                cx.propose(selection);
            } else {
                cx.set(selection);
            }
        }
        if done.commit || done.next || done.previous {
            cx.commit();
        } else if done.cancel {
            cx.cancel(CancelReason::Escape);
        }
        self.ended = if done.commit {
            Some(EditorEnd::Commit)
        } else if done.next {
            Some(EditorEnd::Next)
        } else if done.previous {
            Some(EditorEnd::Previous)
        } else if done.cancel {
            Some(EditorEnd::Cancel)
        } else {
            None
        };
        self.changed = done.changed;
        self.selection_changed = selection != before;
        self.character_panel = character_panel;
        self.context = context_click(&press);
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        let space = Space::of(&cx.env);
        let session = cx.recall::<Session>();
        let text = self.host.text().to_owned();
        let chars = text.chars().count();
        let selection = cx.value().copied().unwrap_or_default();
        let range = selection.range();
        let range = range.start.min(chars)..range.end.min(chars);
        let screen = |p: Point| space.screen(p);

        if let Some(marked) = session.preedit.clone().filter(|r| r.end <= chars && !r.is_empty() && char_slice(&text, r) == session.preedit_text) {
            let mut underline = |r: Range<usize>, width: StrokeWidth| {
                for [a, b] in self.host.underline_segments(r) {
                    draw.stroke(&PaintShape::Path(vec![screen(a), screen(b)]), width, cx.style.underline);
                }
            };
            underline(marked.clone(), StrokeWidth::Medium);
            if let Some(active) = session.preedit_active.clone().filter(|a| !a.is_empty() && a.end <= marked.len()) {
                underline(marked.start + active.start..marked.start + active.end, StrokeWidth::Thick);
            }
        }
        if !range.is_empty() {
            for quad in self.host.selection_quads(range.clone()) {
                draw.fill(&PaintShape::Polygon(quad.iter().map(|&p| screen(p)).collect()), cx.style.selection);
            }
        }
        let caret_at = selection.caret.min(chars);
        let caret = self.host.caret(caret_at, None);
        let caret_rect = caret.map(|CaretLine { top, bottom }| Rect::from_two_pos(screen(top), screen(bottom)).expand(1.0));
        if range.is_empty()
            && (session.dragging.is_some() || cx.blink(session.blink_start))
            && let Some(CaretLine { top, bottom }) = caret
        {
            let stroke = PaintShape::Path(vec![screen(top), screen(bottom)]);
            draw.stroke(&stroke, StrokeWidth::Thick, cx.style.halo);
            draw.stroke(&stroke, StrokeWidth::Thin, cx.style.caret);
        }

        let caret_rect = caret_rect.unwrap_or(Rect::from_min_size(cx.rect.min, Vec2::ZERO));
        let host_lines = lines_of(&mut *self.host, space.doc_rect(cx.rect));
        let zoom = space.zoom();
        type Run = (std::ops::Range<usize>, bool, Vec<f32>, Vec<f32>);
        let runs: Vec<Vec<Run>> = host_lines
            .iter()
            .map(|line| {
                if line.runs.is_empty() {
                    let positions: Vec<f32> = line.text.chars().scan(0.0, |x, _| Some(std::mem::replace(x, *x + 1.0))).collect();
                    let widths = vec![1.0; positions.len()];
                    return vec![(0..line.text.len(), false, positions, widths)];
                }
                line.runs
                    .iter()
                    .map(|run| {
                        (run.bytes.clone(), run.rtl, run.positions.iter().map(|x| x * zoom).collect(), run.widths.iter().map(|w| w * zoom).collect())
                    })
                    .collect()
            })
            .collect();
        let editor_runs: Vec<Vec<EditorRun<'_>>> = runs
            .iter()
            .map(|line| line.iter().map(|(bytes, rtl, positions, widths)| EditorRun::new(bytes.clone(), *rtl, positions, widths)).collect())
            .collect();
        let lines: Vec<EditorLine<'_>> = host_lines
            .iter()
            .zip(&editor_runs)
            .map(|(line, runs)| {
                let rect = space.rect(line.rect);
                EditorLine::new(&line.text, rect, rect.top() + (line.baseline - line.rect.top()) * zoom, runs)
            })
            .collect();
        let mut state = EditorState::new(caret_rect, &lines, text_pos(&host_lines, caret_at));
        state.composing = session.preedit.is_some();
        state.selection = (!range.is_empty()).then(|| (text_pos(&host_lines, selection.anchor.min(chars)), text_pos(&host_lines, caret_at)));
        cx.editor_state(state);
    }

    fn finish(self, cx: Cx<'_, Self>) -> TextEditorOutput {
        TextEditorOutput {
            response: cx.response.clone(),
            changed: self.changed,
            selection_changed: self.selection_changed,
            edit: cx.edit_response(),
            character_panel: self.character_panel,
            context: self.context,
            ended: self.ended,
        }
    }
}

/// The part key of the editor part.
const EDITOR: craft_behavior::PartKey = craft_behavior::PartKey::new("editor");

/// A byte range of `text` as a char range.
fn byte_range_to_chars(text: &str, bytes: Range<usize>) -> Range<usize> {
    let chars = |at: usize| text[..at.min(text.len())].chars().count();
    chars(bytes.start)..chars(bytes.end)
}

//! The note pop-up: a modeless window over a note's messages that keeps its size on screen at every zoom.

use std::collections::HashMap;

use craft_behavior::{GesturePhase, IconId, Metrics, Sense};
use craft_theme::variant::{RadiusStep, Variant};
use craft_theme::{ControlSize, CustomVariant, Mode, Rgba, States, Tokens, VariantLook};
use craft_ui::buttons::{Button, IconButton, PressFocus};
use craft_ui::contract::overlay::{PlacementChange, PlacementKind, ResizeEdges};
use craft_ui::contract::{
    Bind, BindMut, BodyCx, Common, Component, Container, ContainerProps, Cx, DragBand, Gap, HeaderRegion, MeasureCx, Mount, NoValue, Outcome,
    Overlay, Pad, PaintUi, Pattern, Props as _, Role, Show as _, ShowContainer as _, SizeLimits, Sizing, ThemeScope, clicked,
};
use craft_ui::display::{Shade, Text};
use craft_ui::layout::RowAlign;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::primitives::{IconSpec, PaintShape, TextRun, Wrap};
use craft_ui::runtime::env::{Env, EnvView, Label};
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::style::{ComponentStyle, DataColor, StyleCx, SurfaceKind, TypeRole, UiTokens};
use craft_ui::text::TextField;
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Ui, Vec2, pos2, vec2};

use crate::ui::annotate::DEFAULT_COLOR;
use crate::ui::avatar::Avatar;
use crate::ui::comments::age_text;

use super::ThreadWords;

/// One message of a note pop-up.
#[derive(Clone, Debug, PartialEq)]
pub struct NotePopupMessage<'a> {
    /// The message's id.
    pub id: u64,
    /// The message's title line.
    pub title: &'a str,
    /// The author's name.
    pub author: &'a str,
    /// The message text.
    pub text: &'a str,
    /// Unix seconds of the last edit or of the creation; a message with no date shows none.
    pub modified: Option<i64>,
    /// How deep a reply sits; 0 is the comment.
    pub depth: usize,
    /// Whether the message can be edited.
    pub editable: bool,
}

/// The pop-ups' drafts, by note: the typed text not yet committed, the open reply box and its text, and the window's
/// rect while a move or resize runs. The host keeps it, so a pop-up remembers nothing of its own across passes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NoteDrafts {
    texts: HashMap<u64, String>,
    replies: HashMap<u64, (u64, String)>,
    placing: HashMap<u64, Rect>,
}

impl NoteDrafts {
    /// These drafts with the reply box of the note `note` open under its message `message`, on `text`, for a story that
    /// shows the box.
    #[cfg(feature = "screen-testing")]
    pub(super) fn with_reply(mut self, note: u64, message: u64, text: &str) -> Self {
        self.replies.insert(note, (message, text.to_owned()));
        self
    }

    /// Forgets everything of the note `note`: the pop-up closed.
    fn forget(&mut self, note: u64) {
        self.texts.remove(&note);
        self.replies.remove(&note);
        self.placing.remove(&note);
    }

    /// Takes this pass's placement `changes` of the note `note`, whose window the host put at `window`: a running
    /// gesture holds the window where it is, and a commit that changed the window by at least [`LEAST_CHANGE`] is the
    /// report. Each pass applies only the changes its own input made, so a second pass in a frame changes nothing.
    fn settle(&mut self, note: u64, window: Rect, changes: &[PlacementChange]) -> Option<(Rect, PlacementKind)> {
        let mut placed = None;
        for change in changes {
            match change.phase {
                GesturePhase::Begin | GesturePhase::Update => {
                    self.placing.insert(note, change.rect);
                }
                GesturePhase::Commit => {
                    self.placing.remove(&note);
                    if changed(window, change.rect) {
                        placed = Some((change.rect, change.kind));
                    }
                }
                _ => {
                    self.placing.remove(&note);
                }
            }
        }
        placed
    }
}

/// The pop-up's body: the note's text and Reply, then each reply as a panel holding the panels of its answers. It reads
/// and writes the host's drafts and gathers what the pass committed.
struct NoteBody<'a, 'm> {
    id: u64,
    messages: &'a [NotePopupMessage<'m>],
    drafts: &'a mut NoteDrafts,
    allowed: bool,
    now: i64,
    /// The body's tint: the note's colour thinned with white to a third, as production's.
    tint_color: Color32,
    /// The flat Reply's edge ([`NoteReplyLook::Flat`]): the note's colour thinned with white to seven tenths, as
    /// production's panel edge.
    edge_color: Color32,
    body: Option<String>,
    reply: Option<(u64, String)>,
}

impl NoteBody<'_, '_> {
    fn show(&mut self, ui: &mut Ui, words: &ThreadWords, view: &EnvView<'_>) {
        let messages = self.messages;
        let (answers, replies) = nest(messages);
        // The tint goes in behind the body's content, over the window's surface.
        let tint = ui.painter().add(egui::Shape::Noop);
        // The body sits on the note's pale tint, so it reads in the light palette whatever the theme: light fields, dark ink.
        Stack::new().fill_width().theme(ThemeScope::new().mode(Mode::Light)).show(ui, |ui, _| {
            if let Some(note) = messages.first() {
                if self.allowed && note.editable {
                    self.note_text(ui, note, words);
                } else {
                    Text::new(note.text.to_owned()).show(ui);
                }
                if self.allowed {
                    self.reply_box(ui, words, note.id);
                }
            }
            for &at in &replies {
                self.reply_panel(ui, words, view, &answers, at);
            }
        });
        let area = ui.min_rect().union(ui.clip_rect());
        let radius = UiTokens::corner(Env::of(ui).tokens.shape.radius.surface);
        let radius = CornerRadius { nw: 0, ne: 0, ..radius };
        ui.painter().set(tint, egui::Shape::rect_filled(area, radius, self.tint_color));
    }

    /// The panel of reply `at`: its author line, text and Reply, then the panels of its answers (`answers[at]`), one
    /// indent step in. A reply's answers sit deeper than it and no deeper than [`MAX_NESTING`], so this recurses no
    /// deeper than that.
    fn reply_panel(&mut self, ui: &mut Ui, words: &ThreadWords, view: &EnvView<'_>, answers: &[Vec<usize>], at: usize) {
        let messages = self.messages;
        let Some(message) = messages.get(at) else { return };
        let name = words.pin.fill(&[("author", message.author)]);
        Stack::new().surface(SurfaceKind::Frame).pad(Pad::Control).fill_width().label(name).show(ui, |ui, _| {
            byline(ui, message, view, self.now);
            Text::new(message.text.to_owned()).show(ui);
            if self.allowed {
                self.reply_box(ui, words, message.id);
            }
            let own = answers.get(at).map_or(&[][..], Vec::as_slice);
            if !own.is_empty() {
                // The indent step is the theme's, from the start edge.
                Stack::new().fill_width().indent(1).show(ui, |ui, _| {
                    for &answer in own {
                        self.reply_panel(ui, words, view, answers, answer);
                    }
                });
            }
        });
    }

    /// The note's text, typed in place: it commits on focus loss, Escape drops what was typed.
    fn note_text(&mut self, ui: &mut Ui, note: &NotePopupMessage<'_>, words: &ThreadWords) {
        let id = self.id;
        let mut text = self.drafts.texts.get(&id).cloned().unwrap_or_else(|| note.text.to_owned());
        let field = TextField::new(words.add_comment.clone(), Bind::bound(&mut text)).lines(NOTE_ROWS).show(ui);
        if let Some(committed) = field.events.committed().filter(|_| !field.escaped) {
            self.body = edited(note.text, committed).or(self.body.take());
            self.drafts.texts.remove(&id);
        } else if field.escaped || text == note.text {
            self.drafts.texts.remove(&id);
        } else {
            self.drafts.texts.insert(id, text);
        }
    }

    /// The Reply button under the message `to`, or once it is pressed the box to answer that message in: Post or
    /// Cmd/Ctrl+Enter posts the trimmed text, Cancel or Escape drops it. One box is open per note.
    fn reply_box(&mut self, ui: &mut Ui, words: &ThreadWords, to: u64) {
        let id = self.id;
        let open = self.drafts.replies.get(&id).filter(|(target, _)| *target == to).map(|(_, text)| text.clone());
        let Some(mut typed) = open else {
            let flat = Env::of(ui).get::<NoteReplyLook>() == NoteReplyLook::Flat;
            Row::new().fill_width().align(RowAlign::End).show(ui, |ui, _| {
                let reply = Button::new(words.reply.clone()).size(ControlSize::Sm);
                let reply = if flat { reply.variant(Variant::Custom(FLAT_REPLY)) } else { reply };
                let shown = reply.show(ui);
                // The flat face's edge at rest is the note's colour, which no token holds: the variant leaves it clear
                // and it is drawn here. Under the pointer (and so while pressed) the variant's own edge shows.
                let response = &shown.response;
                if flat && !response.hovered() {
                    let tokens = &Env::of(ui).tokens;
                    let (radius, width) = (UiTokens::corner(tokens.shape.radius.control), tokens.stroke.thin);
                    let edge = egui::Stroke::new(width, self.edge_color);
                    ui.painter().rect_stroke(response.rect, radius, edge, egui::StrokeKind::Inside);
                }
                if clicked(&shown) {
                    self.drafts.replies.insert(id, (to, String::new()));
                }
            });
            return;
        };
        let field = TextField::new(words.reply.clone(), Bind::bound(&mut typed)).lines(NOTE_ROWS).placeholder(words.reply.clone()).submits().show(ui);
        let text = typed.trim().to_owned();
        // The multi-line field submits on Cmd/Ctrl+Enter; plain Enter adds a line.
        let mut post = field.submitted && field.submit_mods.cmd;
        let mut cancel = field.escaped;
        Row::new().fill_width().align(RowAlign::End).show(ui, |ui, _| {
            cancel |= clicked(&Button::new(words.cancel.clone()).size(ControlSize::Sm).show(ui));
            post |= clicked(&Button::new(words.post.clone()).size(ControlSize::Sm).variant(Variant::PRIMARY).enabled(!text.is_empty()).show(ui));
        });
        let posted = post && !text.is_empty();
        if posted {
            self.reply = Some((to, text));
        }
        if posted || cancel {
            self.drafts.replies.remove(&id);
        } else {
            self.drafts.replies.insert(id, (to, typed));
        }
    }
}

/// The thread of `messages` (the note first, each reply after the message it answers, by depth): for each message the
/// replies whose panels sit in its panel, and the replies to the note. A reply deeper than [`MAX_NESTING`] counts as at
/// that depth, and one at depth 0 after the note as a reply to it.
fn nest(messages: &[NotePopupMessage<'_>]) -> (Vec<Vec<usize>>, Vec<usize>) {
    let mut answers: Vec<Vec<usize>> = vec![Vec::new(); messages.len()];
    let mut replies = Vec::new();
    // The replies open at each depth above the one being placed.
    let mut open: Vec<usize> = Vec::new();
    for (at, message) in messages.iter().enumerate().skip(1) {
        let depth = message.depth.clamp(1, MAX_NESTING);
        open.truncate(depth - 1);
        match open.last().and_then(|&parent| answers.get_mut(parent)) {
            Some(own) => own.push(at),
            None => replies.push(at),
        }
        open.push(at);
    }
    (answers, replies)
}

/// A reply's author line: avatar, name and age.
fn byline(ui: &mut Ui, message: &NotePopupMessage<'_>, view: &EnvView<'_>, now: i64) {
    Row::new().fill_width().show(ui, |ui, _| {
        let _ = Avatar::new(message.author).show(ui);
        Text::new(message.author.to_owned()).role(TypeRole::BodyStrong).show(ui);
        let age = message.modified.map_or_else(String::new, |then| age_text(view, then, now));
        Text::new(age).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    });
}

/// The size a note pop-up keeps to, in points: 200 to 800 wide and 120 to 800 high, whatever size the file gives it.
pub const NOTE_POPUP_LIMITS: SizeLimits = SizeLimits::new(vec2(200.0, 120.0), vec2(800.0, 800.0));

/// The rows the note's text and a reply show.
const NOTE_ROWS: usize = 2;
/// Past this depth a reply indents no further.
const MAX_NESTING: usize = 8;
/// The least a move or resize changes the window by to be reported, in points: a press that ends where it began
/// changes nothing.
const LEAST_CHANGE: f32 = 2.0;
/// How opaque the connector wedge is: production's 90 of 255.
const CONNECTOR_SHARE: f32 = 90.0 / 255.0;

/// What a note pop-up did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct NotePopupOutput {
    /// The pop-up's response.
    pub response: Response,
    /// The pop-up was closed this pass.
    pub closed: bool,
    /// The note's edited text, once it is committed.
    pub body: Option<String>,
    /// A reply posted this pass: the id of the message it answers and the text.
    pub reply: Option<(u64, String)>,
    /// The window's new rectangle and the gesture that made it, reported when a move or resize commits.
    pub placed: Option<(egui::Rect, PlacementKind)>,
}

/// A modeless note pop-up window over a note's messages: a title band in the note's colour with its kind, author and
/// age, and Close; the note's text, typed in place while the note may be edited; its replies, each a bordered panel
/// named for its author, an answer to a reply inside that reply's panel; and a Reply button under each message that opens a box to answer it (Post or Cmd/Ctrl+Enter posts, Cancel
/// or Escape drops it).
///
/// - Place: at the host's `rect`, its size kept within [`NOTE_POPUP_LIMITS`] on screen at every zoom, so it takes no
///   zoom scale. The title band moves it and the lower end corner resizes it while commenting is allowed; a move or
///   resize is reported once as [`NotePopupOutput::placed`] when it commits, and one under 2 points is not.
/// - Text: the note's text commits when its field loses focus or the pop-up closes, with trailing whitespace trimmed.
/// - Connector: with [`anchor`](Self::anchor), a faint wedge joins the mark to the window while either is hovered or
///   the note is selected, and only while the two do not overlap.
#[must_use = "show the pop-up with `show`"]
#[derive(Pattern)]
pub struct NotePopup<'a> {
    id: u64,
    label: Label,
    rect: egui::Rect,
    messages: &'a [NotePopupMessage<'a>],
    drafts: &'a mut NoteDrafts,
    color: Option<[u8; 3]>,
    anchor: Option<(egui::Rect, bool)>,
    selected: bool,
    now: i64,
    allowed: bool,
}

impl<'a> NotePopup<'a> {
    /// The pop-up `id` named `label` at `rect` over `messages`, keeping its reply drafts in `drafts`.
    pub fn new(id: u64, label: impl Into<Label>, rect: egui::Rect, messages: &'a [NotePopupMessage<'a>], drafts: &'a mut NoteDrafts) -> Self {
        NotePopup { id, label: label.into(), rect, messages, drafts, color: None, anchor: None, selected: false, now: 0, allowed: true }
    }

    /// The note's colour, straight RGB; `None` takes the default note colour.
    pub fn color(mut self, color: Option<[u8; 3]>) -> Self {
        self.color = color;
        self
    }

    /// The annotation's screen rect (the first quad or the icon) and whether the host's page part is hovered: the
    /// pop-up paints the faint connector from the mark to its window while `mark_hovered`, the window is hovered or the
    /// note is selected, and only when the two do not overlap.
    pub fn anchor(mut self, mark: egui::Rect, mark_hovered: bool) -> Self {
        self.anchor = Some((mark, mark_hovered));
        self
    }

    /// Whether the note is the selected one.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// The current time in Unix seconds, which the ages count from.
    pub fn now(mut self, unix_seconds: i64) -> Self {
        self.now = unix_seconds;
        self
    }

    /// Whether the document allows commenting: replying and editing.
    pub fn allowed(mut self, allowed: bool) -> Self {
        self.allowed = allowed;
        self
    }

    /// Shows the pop-up.
    pub fn show(self, ui: &mut Ui) -> NotePopupOutput {
        let NotePopup { id, label, rect, messages, drafts, color, anchor, selected, now, allowed } = self;
        let env = Env::of(ui);
        let words = env.words::<ThreadWords>();
        let view = env.view();
        let window = window_rect(rect);
        let [r, g, b] = color.unwrap_or(DEFAULT_COLOR);
        let fill = DataColor::new(Color32::from_rgb(r, g, b));
        let note = messages.first();
        let title = note.map_or_else(String::new, |note| {
            if note.author.is_empty() { note.title.to_owned() } else { words.note_title.fill(&[("kind", note.title), ("author", note.author)]) }
        });
        let age = note.and_then(|note| note.modified).map_or_else(String::new, |then| age_text(&view, then, now));
        // The grip sits at the lower end corner: the edges are physical, so it moves to the left under right to left.
        let end = if env.direction.is_rtl() { ResizeEdges::LEFT } else { ResizeEdges::RIGHT };
        let grip = if allowed { ResizeEdges::BOTTOM.union(end) } else { ResizeEdges::NONE };
        // A running move or resize shows where the gesture holds the window; the host's rect changes once it commits.
        let shown_at = drafts.placing.get(&id).copied().unwrap_or(window);
        let overlay =
            Overlay::floating().modeless().movable(allowed).resize_edges(grip).body_scroll(true).size_limits(NOTE_POPUP_LIMITS).placed(shown_at);
        let layer = NoteWindow::new(label, overlay, NoteLook { title, age, fill }, words.close.clone(), allowed, anchor, selected)
            .id_salt(("note-popup", id));
        let mut content = NoteBody {
            id,
            messages,
            drafts,
            allowed,
            now,
            tint_color: Color32::from_rgb(pale(r), pale(g), pale(b)),
            edge_color: Color32::from_rgb(edge_tone(r), edge_tone(g), edge_tone(b)),
            body: None,
            reply: None,
        };
        let shown = layer.show(ui, |ui, _| content.show(ui, &words, &view));
        let NoteBody { drafts, mut body, reply, .. } = content;
        let out = shown.output;
        let placed = drafts.settle(id, window, &out.changes);
        if out.close {
            // Close commits the typed text as a focus loss would.
            if let (Some(note), Some(text)) = (note, drafts.texts.get(&id)) {
                body = edited(note.text, text).or(body);
            }
            drafts.forget(id);
        }
        NotePopupOutput { response: out.response, closed: out.close, body, reply, placed }
    }
}

/// How a note pop-up draws its Reply button. An app sets it once at its root (`RootEnv::values`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, craft_ui::runtime::env::EnvValue, craft_ui::contract::MemoProps)]
#[env(style)]
#[non_exhaustive]
pub enum NoteReplyLook {
    /// The suite's small secondary button: an outlined pill.
    #[default]
    Standard,
    /// PdfCraft's: a small flat rectangle, a translucent white face with a thin edge over the note's tint, as
    /// production's pop-up footers draw it.
    Flat,
}

/// The flat Reply face ([`NoteReplyLook::Flat`]).
const FLAT_REPLY: CustomVariant = CustomVariant::new::<craft_ui::buttons::button::ButtonStyleKey>("craft_review.flat-reply", flat_reply_look);

/// White at 60% over the note's tint, an edge in the note's colour (drawn by the pop-up) and control corners; the
/// theme's state fills under the pointer and the press.
fn flat_reply_look(tokens: &Tokens) -> VariantLook {
    let c = &tokens.color;
    let face = Rgba([255, 255, 255, 153]);
    let text = States::new(c.text.normal, c.text.normal, c.text.normal, c.text.disabled);
    // The edge at rest is the note's colour, drawn by the pop-up (`NoteBody::reply_box`); clear here.
    let clear = Rgba([255, 255, 255, 0]);
    VariantLook::new(States::new(face, c.state.hover, c.state.pressed, face), text)
        .edge(States::new(clear, c.text.normal, c.text.normal, c.line.divider))
        .radius(RadiusStep::Control)
}

/// The text a commit of `typed` reports for a note that reads `text`: trailing whitespace trimmed, `None` when that
/// leaves the text as it was.
fn edited(text: &str, typed: &str) -> Option<String> {
    let typed = typed.trim_end();
    (typed != text).then(|| typed.to_owned())
}

/// The window's rect on screen for the host's `rect`: its size kept within [`NOTE_POPUP_LIMITS`] (a side that is not
/// finite takes the least size) and its top left where the host put it (the screen's origin when that is not finite).
fn window_rect(rect: Rect) -> Rect {
    let limits = NOTE_POPUP_LIMITS;
    let most = limits.max.unwrap_or(Vec2::splat(SizeLimits::BOUND));
    let side = |value: f32, least: f32, most: f32| if value.is_finite() { value.clamp(least, most) } else { least };
    let size = vec2(side(rect.width(), limits.min.x, most.x), side(rect.height(), limits.min.y, most.y));
    let min = if rect.min.is_finite() { rect.min } else { Pos2::ZERO };
    Rect::from_min_size(min, size)
}

/// Whether a gesture that left the window at `to` from `from` is worth reporting: it moved a corner or changed a side
/// by at least [`LEAST_CHANGE`].
fn changed(from: Rect, to: Rect) -> bool {
    (to.min - from.min).length() >= LEAST_CHANGE || (to.size() - from.size()).length() >= LEAST_CHANGE
}

/// A channel of the note's colour thinned with white to seven tenths: the flat Reply's edge.
fn edge_tone(channel: u8) -> u8 {
    ((u16::from(channel) * 7 + 255 * 3) / 10) as u8
}

/// A channel of the note's colour thinned with white to a third.
fn pale(channel: u8) -> u8 {
    ((u16::from(channel) + 2 * 255) / 3) as u8
}

/// The connector wedge from the mark to the window: its point on the mark where it is nearest the window, and the two
/// ends of its base (`half` wide each side of the middle) on the window's side that faces the mark. `None` without an
/// anchor, while neither the mark (`anchor`'s flag) nor the window (`hovered`) is hovered and the note is not
/// `selected`, and while the mark and the window overlap.
fn connector(anchor: Option<(Rect, bool)>, selected: bool, window: Rect, hovered: bool, half: f32) -> Option<[Pos2; 3]> {
    let (mark, mark_hovered) = anchor?;
    if !(mark_hovered || hovered || selected) || !mark.is_finite() || !window.is_positive() || mark.intersects(window) {
        return None;
    }
    let from = nearest_on(mark, window.center());
    let to = nearest_on(window, from);
    let [start, end] = wedge_base(window, from, to, half);
    Some([from, start, end])
}

/// The two ends of a connector's base on `window`: centred on `to` (the point of the window nearest the mark) along
/// the side that faces `from`, `half` each way and kept on that side. At a corner, the side is the one the mark lies
/// further beyond.
fn wedge_base(window: Rect, from: Pos2, to: Pos2, half: f32) -> [Pos2; 2] {
    // `half` along [low, high] around `centre`, moved in to stay on the side (a side shorter than the base gives all
    // of it).
    let spread = |centre: f32, low: f32, high: f32| {
        let half = half.min((high - low).max(0.0) / 2.0);
        let centre = centre.clamp(low + half, high - half);
        (centre - half, centre + half)
    };
    let on_horizontal = (to.y - window.top()).abs() < 0.5 || (to.y - window.bottom()).abs() < 0.5;
    let on_vertical = (to.x - window.left()).abs() < 0.5 || (to.x - window.right()).abs() < 0.5;
    let beyond_x = (from.x - to.x).abs() / window.width().max(1.0);
    let beyond_y = (from.y - to.y).abs() / window.height().max(1.0);
    if on_horizontal && !(on_vertical && beyond_x > beyond_y) {
        let (a, b) = spread(to.x, window.left(), window.right());
        [pos2(a, to.y), pos2(b, to.y)]
    } else {
        let (a, b) = spread(to.y, window.top(), window.bottom());
        [pos2(to.x, a), pos2(to.x, b)]
    }
}

/// The point of `rect`'s boundary nearest `point` (a point inside is pushed to the nearer vertical edge).
fn nearest_on(rect: Rect, point: Pos2) -> Pos2 {
    let x = point.x.clamp(rect.left(), rect.right());
    let y = point.y.clamp(rect.top(), rect.bottom());
    if rect.left() < x && x < rect.right() && rect.top() < y && y < rect.bottom() {
        pos2(if (point.x - rect.left()).abs() <= (point.x - rect.right()).abs() { rect.left() } else { rect.right() }, y)
    } else {
        pos2(x, y)
    }
}

/// What the title band shows: the note's kind and author, its age, on the note's colour.
#[derive(Clone, Debug)]
struct NoteLook {
    title: String,
    age: String,
    fill: DataColor,
}

/// The style of the note window's layer: the surface comes from the container.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct NoteWindowStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// Half the width of the connector wedge's base.
    wedge: f32,
}

/// What the note window reports: its response, Close and the placement changes of this pass.
#[derive(Debug)]
struct NoteWindowOutput {
    response: Response,
    close: bool,
    changes: Vec<PlacementChange>,
}

impl Outcome for NoteWindowOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The note pop-up's layer: a floating surface tinted with the note's colour, whose header is the title band (it
/// moves the layer) with Close at its end, under which it paints the connector wedge.
#[derive(craft_ui::contract::Component)]
#[component(container, unregistered)]
struct NoteWindow {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    /// Open while the host shows the pop-up; the host shows it again each pass it wants it.
    open: Bind<'static, bool>,
    look: NoteLook,
    close_label: Label,
    allowed: bool,
    anchor: Option<(Rect, bool)>,
    selected: bool,
    close: bool,
    changes: Vec<PlacementChange>,
}

impl NoteWindow {
    fn new(label: Label, overlay: Overlay, look: NoteLook, close_label: Label, allowed: bool, anchor: Option<(Rect, bool)>, selected: bool) -> Self {
        let props =
            ContainerProps::new().mount(Mount::Layer(overlay)).surface(SurfaceKind::Floating).header(HeaderRegion::new().band(DragBand::Move));
        NoteWindow {
            common: Common::new(),
            props,
            open: Bind::given(true),
            look,
            close_label,
            allowed,
            anchor,
            selected,
            close: false,
            changes: Vec::new(),
        }
        .label(label)
    }

    /// The wedge this pass, for the window at `response`'s rect: the window counts as hovered while the pointer is
    /// over it, its children included (egui's `contains_pointer` of the topmost layer under the pointer).
    fn wedge(&self, response: &Response, half: f32) -> Option<[Pos2; 3]> {
        connector(self.anchor, self.selected, response.rect, response.contains_pointer(), half)
    }
}

impl Component for NoteWindow {
    const ROLE: Role = Role::GROUP;
    const PARTS: bool = false;
    const STOPS: bool = false;
    type Style = NoteWindowStyle;
    type Output = NoteWindowOutput;

    fn style(cx: &StyleCx<'_>, _: NoteWindowStyleKey) -> NoteWindowStyle {
        NoteWindowStyle { metrics: Metrics::new(), wedge: cx.tokens.space.gap.inline }
    }

    fn sense(&self) -> Sense {
        Sense::HOVER
    }

    /// The body's tint over the surface, and the connector wedge, under the body.
    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let draw = ui.draw();
        let fill = self.look.fill;
        if let Some(wedge) = self.wedge(&cx.response, cx.style.wedge) {
            draw.faded(CONNECTOR_SHARE, |draw| draw.data_fill(&PaintShape::Polygon(wedge.to_vec()), fill));
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> NoteWindowOutput {
        NoteWindowOutput { response: cx.response.clone(), close: self.close, changes: self.changes }
    }
}

impl Container for NoteWindow {
    fn open(&mut self) -> Option<BindMut<'_, bool>> {
        Some(self.open.reborrow())
    }

    /// The title band: the note's kind and author and its age, with Close at its end, all on the note's colour. Close
    /// is the band's own control, which needs the document to allow commenting, as closing changes the note.
    fn header(&mut self, ui: &mut Ui, _: &mut BodyCx<'_>) {
        let look = self.look.clone();
        let band = ui.painter().add(egui::Shape::Noop);
        let shown = Row::new().fill_width().gap(Gap::None).show(ui, |ui, _| {
            let _ = NoteTitle { common: Common::new(), look: look.clone() }.label(look.title.clone()).sizing(Sizing::FillWidth).show(ui);
            let close = IconButton::new(IconSpec::svg(IconId::new("close")))
                .label(self.close_label.clone())
                .enabled(self.allowed)
                .press_focus(PressFocus::Keep)
                .show(ui);
            self.close |= clicked(&close);
        });
        let radius = UiTokens::corner(Env::of(ui).tokens.shape.radius.surface);
        let radius = CornerRadius { sw: 0, se: 0, ..radius };
        let DataColor(fill) = look.fill;
        ui.painter().set(band, egui::Shape::rect_filled(shown.output.rect, radius, fill));
    }

    fn closed(&mut self, _: CloseReason) {
        self.close = true;
    }

    fn placement_changed(&mut self, change: &PlacementChange) {
        self.changes.push(*change);
    }
}

/// The style of the title band.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
struct NoteTitleStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The band's height.
    height: f32,
    /// The padding at the start and the end.
    pad_x: f32,
    /// Between the title and the age.
    gap: f32,
}

/// The title band of a note pop-up: the title (cut short to fit) at the start and the age at the end, in the ink that
/// reads on the note's colour.
#[derive(craft_ui::contract::Component)]
#[component(unregistered)]
struct NoteTitle {
    #[common]
    common: Common,
    look: NoteLook,
}

impl Component for NoteTitle {
    const ROLE: Role = Role::HEADING;
    const PARTS: bool = false;
    const OVERLAYS: bool = false;
    type Style = NoteTitleStyle;
    type Output = Response;

    fn style(cx: &StyleCx<'_>, _: NoteTitleStyleKey) -> NoteTitleStyle {
        let control = cx.control();
        NoteTitleStyle { metrics: Metrics::new(), height: control.height, pad_x: control.pad_x, gap: cx.tokens.space.gap.inline }
    }

    fn measure(&mut self, cx: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::new(cx.room.unwrap_or(0.0), cx.style.height)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let style = cx.style;
        let draw = ui.draw();
        let rect = cx.response.rect;
        draw.data_fill(&PaintShape::Rect(rect, CornerRadius::ZERO), self.look.fill);
        let ink = ui.with_style(Variant::SECONDARY, |styles| styles.ink_on(self.look.fill));
        let inner = rect.shrink2(vec2(style.pad_x, 0.0));
        let age = TextRun::new(self.look.age.as_str()).role(TypeRole::Caption).color(ink);
        let age_size = if self.look.age.is_empty() { Vec2::ZERO } else { draw.layout(&age, f32::INFINITY).size() };
        let room = (inner.width() - age_size.x - if age_size.x > 0.0 { style.gap } else { 0.0 }).max(0.0);
        let title = TextRun::new(self.look.title.as_str()).role(TypeRole::BodyStrong).wrap(Wrap::Elide).color(ink);
        let rtl = cx.env.direction().is_rtl();
        let (title_x, age_x) = if rtl { (inner.right() - room, inner.left()) } else { (inner.left(), inner.right() - age_size.x) };
        let line = draw.layout(&title, f32::INFINITY).size().y;
        draw.text(Rect::from_min_size(pos2(title_x, inner.center().y - line / 2.0), Vec2::new(room, line)), &title);
        if age_size.x > 0.0 {
            let top = inner.center().y - age_size.y / 2.0;
            draw.text(Rect::from_min_size(pos2(age_x, top), age_size), &age);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

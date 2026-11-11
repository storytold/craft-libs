//! Cursor chat: your box at the pointer, and everyone's bubbles.

use craft_theme::variant::Variant;
use craft_ui::contract::{Bind, NoValue, Outcome, PaintUi, Pattern, Props, Remember as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::popover::Popover;
use craft_ui::primitives::{PaintShape, TextRun, Wrap};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::overlay::CloseReason;
use craft_ui::style::{DataColor, StyleCx, TypeRole};
use craft_ui::text::TextField;
use egui::{Color32, CornerRadius, Pos2, Rect, Response, Ui, Vec2};

use crate::collab::CollabRequest;

#[cfg(feature = "screen-testing")]
mod stories;

/// How much of the ink the name over a bubble's text keeps.
const NAME_INK: f32 = 0.8;

craft_ui::words! {
    /// The chat box's words, English by default.
    #[non_exhaustive]
    pub struct ChatWords {
        name: "chat.name" = "Cursor chat";
        hint: "chat.hint" = "Say something";
    }
}

/// The sizes of a bubble, from the style context of the component that paints it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BubbleLook {
    /// Room round the text.
    pub pad: Vec2,
    /// Between the name and the text.
    pub gap: f32,
    /// The corner radius, square at the pointer.
    pub radius: u8,
    /// From the pointer's tip to the bubble's corner.
    pub offset: Vec2,
    /// The width the text wraps at.
    pub width: f32,
    /// The body text size and line height.
    pub body: (f32, f32),
    /// The name's size and line height.
    pub caption: (f32, f32),
}

impl BubbleLook {
    /// The look the style context gives.
    pub fn of(cx: &StyleCx<'_>) -> Self {
        let (control, tokens) = (cx.control(), cx.tokens);
        let (body, caption) = (tokens.typography.get(TypeRole::Body), tokens.typography.get(TypeRole::Caption));
        BubbleLook {
            pad: Vec2::new(control.pad_x, tokens.space.gap.inline),
            gap: tokens.space.gap.tight,
            radius: tokens.shape.radius.surface.round().clamp(0.0, 255.0) as u8,
            offset: Vec2::new(tokens.size.pointer_tip.x, tokens.size.pointer_tip.y),
            width: tokens.size.length.field,
            body: (body.size, body.line_height),
            caption: (caption.size, caption.line_height),
        }
    }
}

/// A chat bubble with its corner beside the pointer's `tip`: the name over the text when given, on `fill` (the
/// person's colour) at `alpha`. As wide as its text, which wraps at the look's width; rounded except the corner at
/// the pointer, which is square.
pub fn paint_chat_bubble(ui: &PaintUi<'_>, look: &BubbleLook, tip: Pos2, name: Option<&str>, text: &str, fill: [u8; 3], alpha: f32) {
    let draw = ui.draw();
    let [r, g, b] = fill;
    let ink = ui.with_style(Variant::SECONDARY, |cx| cx.ink_on(DataColor::new(Color32::from_rgb(r, g, b))));
    let body = TextRun::new(text).role(TypeRole::Body).wrap(Wrap::Wrap).color(ink);
    let title = name.map(|name| TextRun::new(name).role(TypeRole::Caption).color(ink));
    let measured = draw.layout(&body, look.width).size();
    let head = title.as_ref().map(|title| draw.layout(title, look.width).size());
    let head_height = head.map_or(0.0, |size| size.y + look.gap);
    let width = head.map_or(0.0, |size| size.x).max(measured.x);
    let rect = Rect::from_min_size(tip + look.offset, Vec2::new(width, head_height + measured.y) + 2.0 * look.pad);
    let round = look.radius;
    draw.faded(alpha, |draw| {
        let background = Color32::from_rgb(r, g, b);
        draw.data_fill(&PaintShape::Rect(rect, CornerRadius { nw: 0, ne: round, sw: round, se: round }), DataColor::new(background));
        let mut at = rect.min + look.pad;
        if let Some(title) = &title {
            draw.faded(NAME_INK, |draw| {
                draw.text(Rect::from_min_size(at, Vec2::new(width, look.caption.1)), title);
            });
            at.y += head_height;
        }
        draw.text(Rect::from_min_size(at, Vec2::new(width, measured.y)), &body);
    });
}

/// The review commands outside a widget: `/` opens the cursor chat box. The app binds the chord and sets
/// [`CursorChatBox`]'s `open` flag when the command runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui::contract::CommandSet)]
#[command(prefix = "review")]
#[non_exhaustive]
pub enum ReviewCommand {
    /// Open the cursor chat box at the pointer.
    #[command(label = "Cursor chat", default = "/")]
    CursorChat,
}

/// Where the box was last shown, kept while it is open.
#[derive(Clone, Default)]
struct Last(Option<Pos2>);

/// Your cursor chat: while `open`, a text box at the pointer; each change comes back as a [`CollabRequest::Chat`] for
/// the app to send. Enter sends and closes the box, Escape or an empty Enter closes it and clears the message (an
/// empty `Chat`). The app opens it from [`ReviewCommand::CursorChat`].
#[derive(Pattern, Props)]
pub struct CursorChatBox<'a> {
    label: Label,
    text: &'a mut String,
    open: &'a mut bool,
}

impl<'a> CursorChatBox<'a> {
    /// A box that edits `text` while `open`.
    pub fn new(text: &'a mut String, open: &'a mut bool) -> Self {
        CursorChatBox { label: Label::Empty, text, open }
    }

    /// Shows the box and returns the requests it made. `pointer` is the pointer position in screen points, `None` off
    /// the canvas; an open box keeps its last position then.
    pub fn show(self, ui: &mut Ui, pointer: Option<Pos2>) -> CursorChatBoxOutput {
        let CursorChatBox { label, text, open } = self;
        let words = Env::of(ui).words::<ChatWords>();
        let mut requests = Vec::new();
        let shown = Stack::new().label(label).show(ui, |ui, cx| {
            if !*open {
                cx.keep::<Last>().0 = None;
                return;
            }
            let last = cx.keep::<Last>();
            last.0 = pointer.or(last.0);
            let Some(at) = last.0 else { return };
            let before = text.clone();
            let mut shown = true;
            let mut submitted = false;
            let mut field_escaped = false;
            let popover = Popover::new(words.name.clone(), Bind::bound(&mut shown)).at_point(at).show(ui, |ui, _| {
                let field = TextField::new(words.name.clone(), Bind::bound(&mut *text)).placeholder(words.hint.clone()).show(ui);
                submitted = field.submitted;
                field_escaped = field.escaped;
            });
            if *text != before {
                requests.push(CollabRequest::Chat(text.clone()));
            }
            let escaped = field_escaped || popover.output.closed == Some(CloseReason::Escape);
            if escaped || (submitted && text.trim().is_empty()) {
                text.clear();
                requests.push(CollabRequest::Chat(String::new()));
                *open = false;
            } else if submitted {
                *open = false;
            }
        });
        CursorChatBoxOutput { response: shown.output, requests }
    }
}

/// What the chat box did this pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct CursorChatBoxOutput {
    /// The box's response.
    pub response: Response,
    /// The chat messages to send, in order.
    pub requests: Vec<CollabRequest>,
}

impl Outcome for CursorChatBoxOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

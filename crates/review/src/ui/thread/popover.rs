//! The thread popover and the comment composer.

use craft_behavior::IconId;
use craft_theme::variant::Tone;
use craft_ui::buttons::{Button, IconButton, ToggleButton};
use craft_ui::contract::overlay::Anchor;
use craft_ui::contract::{Bind, Pattern, Props as _, Remember as _, Show as _, ShowContainer as _, Trigger, clicked};
use craft_ui::display::{Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::menu::{Menu, MenuItem};
use craft_ui::popover::Popover;
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::Env;
use craft_ui::runtime::focus::Initial;
use craft_ui::style::TypeRole;
use craft_ui::text::TextField;
use egui::{Pos2, Rect, Response, Ui};

use crate::comments::CommentThread;
use crate::ui::avatar::Avatar;
use crate::ui::comments::{CommentEdit, age_text};

use super::ThreadWords;
use super::mention::{MentionCompleter, mention_runs, message_lines};

/// The rows of the new comment's field.
const COMPOSER_ROWS: usize = 3;

/// The reactions a message offers.
pub const REACTIONS: [&str; 8] = ["👍", "❤", "😂", "🎉", "👀", "🙏", "🔥", "✅"];

/// The inline editor of the popover: the message and its draft.
#[derive(Clone, Default)]
struct Editing(Option<(usize, String)>);

/// The thread popover next to a pin: messages with avatar, author, age and text (@mentions in the accent),
/// reactions under each (click to toggle yours), your messages' Edit and Delete, the thread's Resolve /
/// Reopen button and a "..." menu (Mark as unread, Copy link and, on your own thread, Delete thread), and the
/// reply field at the bottom with @mention completion. Escape closes the mention list first, then an inline
/// edit, then the popover. Focus goes to the reply field when it opens.
#[must_use = "show the popover with `show`"]
#[derive(Pattern)]
pub struct ThreadPopover<'a> {
    thread: &'a CommentThread,
    opener: &'a Response,
    anchor: Option<Rect>,
    me: &'a str,
    people: &'a [String],
    draft: &'a mut String,
    now: i64,
}

impl<'a> ThreadPopover<'a> {
    /// A popover for `thread` hanging from `opener` (the pin's response). `me` is the viewer's name and
    /// `people` the names offered for @mentions; `draft` holds the reply text.
    pub fn new(thread: &'a CommentThread, opener: &'a Response, me: &'a str, people: &'a [String], draft: &'a mut String) -> Self {
        ThreadPopover { thread, opener, anchor: None, me, people, draft, now: 0 }
    }

    /// Hangs the popover from `rect` (in the opener's layer space) instead of from the opener's rect: for a
    /// popover opened from the Comments panel.
    pub fn anchor(mut self, rect: Rect) -> Self {
        self.anchor = Some(rect);
        self
    }

    /// The current time in Unix seconds, which the ages count from.
    pub fn now(mut self, unix_seconds: i64) -> Self {
        self.now = unix_seconds;
        self
    }
}

/// What the thread popover did this pass.
#[must_use]
#[non_exhaustive]
pub struct ThreadPopoverOutput {
    /// This pass's edits, in order.
    pub edits: Vec<CommentEdit>,
    /// It closed this pass (Escape, a click outside).
    pub closed: bool,
}

impl ThreadPopover<'_> {
    /// Shows the popover and reports the edits made.
    pub fn show(self, ui: &mut Ui) -> ThreadPopoverOutput {
        let ThreadPopover { thread, opener, anchor, me, people, draft, now } = self;
        let env = Env::of(ui);
        let words = env.words::<ThreadWords>();
        let view = env.view();
        let name = thread.messages.first().map_or_else(String::new, |first| words.pin.fill(&[("author", &first.author)]));
        let mut names: Vec<&str> = people.iter().map(String::as_str).collect();
        names.push(me);
        let mut edits = Vec::new();
        let mut open = true;
        let mut escaped = false;
        let mut close = false;
        let mut popover = Popover::new(name, Bind::bound(&mut open)).opened_by(opener).initial_focus(Initial::Default);
        if let Some(rect) = anchor {
            popover = popover.overlay(|overlay| overlay.opener_anchor(Anchor::Rect(rect)));
        }
        let shown = popover.show(ui, |ui, cx| {
            let Editing(mut editing) = cx.keep::<Editing>().clone();
            Stack::new().show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    let resolved = thread.resolved;
                    let label = if resolved { &words.reopen } else { &words.resolve };
                    let toggle = ToggleButton::new(label.clone(), Bind::given(resolved)).show(ui);
                    if let Some(&now_resolved) = toggle.value() {
                        edits.push(CommentEdit::Resolve { thread: thread.id, resolved: now_resolved });
                    }
                    let more = IconButton::new(IconSpec::svg(IconId::new("more"))).label(words.more.clone()).show(ui);
                    let mut ran = None;
                    Menu::new(words.more.clone()).opened_by(&more, Trigger::Click).show(ui, |ui, _| {
                        if MenuItem::new(words.mark_unread.as_str().to_owned()).icon(IconSpec::svg(IconId::new("eye_off"))).show(ui).ran {
                            ran = Some(CommentEdit::MarkUnread { thread: thread.id });
                        }
                        if MenuItem::new(words.copy_link.as_str().to_owned()).icon(IconSpec::svg(IconId::new("link"))).show(ui).ran {
                            ran = Some(CommentEdit::CopyLink { thread: thread.id });
                        }
                        if thread.messages.first().is_some_and(|first| first.author == me)
                            && MenuItem::new(words.delete_thread.as_str().to_owned()).icon(IconSpec::svg(IconId::new("trash"))).show(ui).ran
                        {
                            ran = Some(CommentEdit::Delete { thread: thread.id, index: 0 });
                        }
                    });
                    edits.extend(ran);
                    close |= clicked(&IconButton::new(IconSpec::svg(IconId::new("close"))).label(words.close.clone()).show(ui));
                });
                for (index, message) in thread.messages.iter().enumerate() {
                    Row::new().show(ui, |ui, _| {
                        let _ = Avatar::new(message.author.clone()).show(ui);
                        Stack::new().show(ui, |ui, _| {
                            Row::new().show(ui, |ui, _| {
                                Text::new(message.author.clone()).role(TypeRole::BodyStrong).show(ui);
                                let age = age_text(&view, message.modified.unwrap_or(message.created), now);
                                Text::new(age).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
                                if message.author == me {
                                    let delete = IconButton::new(IconSpec::svg(IconId::new("trash"))).label(words.delete.clone()).show(ui);
                                    if clicked(&delete) {
                                        edits.push(CommentEdit::Delete { thread: thread.id, index });
                                    }
                                    let edit = IconButton::new(IconSpec::svg(IconId::new("pencil"))).label(words.edit.clone()).show(ui);
                                    if clicked(&edit) {
                                        editing = Some((index, message.text.clone()));
                                    }
                                }
                                let react = IconButton::new(IconSpec::svg(IconId::new("plus"))).label(words.react.clone()).show(ui);
                                Menu::new(words.react.clone()).opened_by(&react, Trigger::Click).show(ui, |ui, _| {
                                    for emoji in REACTIONS {
                                        let mine = message.reactions.iter().any(|r| r.emoji == emoji && r.people.iter().any(|p| p == me));
                                        if MenuItem::new(emoji.to_owned()).check(mine).show(ui).ran {
                                            edits.push(CommentEdit::React { thread: thread.id, index, emoji: emoji.to_owned() });
                                        }
                                    }
                                });
                            });
                            match editing.as_mut().filter(|(editing, _)| *editing == index) {
                                Some((_, text)) => {
                                    let field = TextField::new(words.edit.clone(), Bind::given(text.clone())).lines(COMPOSER_ROWS).show(ui);
                                    if let Some(value) = field.value() {
                                        text.clone_from(value);
                                    }
                                    let escaped = field.escaped;
                                    let trimmed = text.trim().to_owned();
                                    let post = Button::new(words.post.clone()).enabled(!trimmed.is_empty()).show(ui);
                                    let post_clicked = clicked(&post);
                                    if post_clicked && !trimmed.is_empty() {
                                        edits.push(CommentEdit::Edit { thread: thread.id, index, text: trimmed });
                                    }
                                    if post_clicked || escaped {
                                        editing = None;
                                    }
                                }
                                None => {
                                    Stack::new().show(ui, |ui, _| {
                                        for line in message_lines(mention_runs(&message.text, &names)) {
                                            Row::new().wrap(true).show(ui, |ui, _| {
                                                for (run, mention) in &line {
                                                    for word in run.split_inclusive(' ') {
                                                        let text = Text::new(word.to_owned());
                                                        if *mention {
                                                            text.role(TypeRole::BodyStrong).tone(Tone::Accent).show(ui)
                                                        } else {
                                                            text.show(ui)
                                                        };
                                                    }
                                                }
                                            });
                                        }
                                    });
                                }
                            }
                            Row::new().wrap(true).show(ui, |ui, _| {
                                for reaction in &message.reactions {
                                    let on = reaction.people.iter().any(|p| p == me);
                                    let chip = ToggleButton::new(format!("{} {}", reaction.emoji, reaction.people.len()), Bind::given(on));
                                    if chip.tooltip(reaction.people.join(", ")).show(ui).value().is_some() {
                                        edits.push(CommentEdit::React { thread: thread.id, index, emoji: reaction.emoji.clone() });
                                    }
                                }
                            });
                        });
                    });
                }
                let kept = draft.clone();
                let field = TextField::new(words.reply.clone(), Bind::bound(&mut *draft))
                    .lines(COMPOSER_ROWS)
                    .placeholder(words.reply.clone())
                    .complete(MentionCompleter { people: people.to_vec(), me: me.to_owned() })
                    .submits()
                    .default_stop()
                    .show(ui);
                escaped |= field.escaped;
                // Escape closes the popover and leaves the typed reply for the next time it opens.
                if field.escaped {
                    *draft = kept;
                }
                let text = draft.trim().to_owned();
                let post = Button::new(words.post.clone()).enabled(!text.is_empty()).show(ui);
                if (clicked(&post) || field.submitted) && !text.is_empty() {
                    edits.push(CommentEdit::Reply { thread: thread.id, text });
                    draft.clear();
                }
            });
            *cx.keep::<Editing>() = Editing(editing);
        });
        ThreadPopoverOutput { edits, closed: !open || escaped || close || shown.output.closed.is_some() }
    }
}

/// The floating composer card for a new comment at a point: a multi-line field with the focus, Post and
/// Cancel (Escape).
#[must_use = "show the composer with `show`"]
#[derive(Pattern)]
pub struct CommentComposer<'a> {
    anchor: Option<Pos2>,
    draft: &'a mut String,
}

impl<'a> CommentComposer<'a> {
    /// A composer at `anchor` (screen points; `None` hides it) that edits `draft`.
    pub fn new(anchor: Option<Pos2>, draft: &'a mut String) -> Self {
        CommentComposer { anchor, draft }
    }
}

/// What the composer did this pass.
#[must_use]
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct CommentComposerOutput {
    /// The text on Post.
    pub posted: Option<String>,
    /// It closed without posting (Cancel, Escape, a click outside).
    pub cancelled: bool,
}

impl CommentComposer<'_> {
    /// Shows the composer and reports whether the user posted or closed it.
    pub fn show(self, ui: &mut Ui) -> CommentComposerOutput {
        let CommentComposer { anchor, draft } = self;
        let mut output = CommentComposerOutput::default();
        let Some(anchor) = anchor else { return output };
        let words = Env::of(ui).words::<ThreadWords>();
        let mut open = true;
        let mut posted = None;
        let mut cancel = false;
        let shown =
            Popover::new(words.add_comment.clone(), Bind::bound(&mut open)).at_point(anchor).initial_focus(Initial::Default).show(ui, |ui, _| {
                Stack::new().show(ui, |ui, _| {
                    let field = TextField::new(words.add_comment.clone(), Bind::bound(&mut *draft))
                        .lines(COMPOSER_ROWS)
                        .placeholder(words.add_comment.clone())
                        .submits()
                        .default_stop()
                        .show(ui);
                    cancel |= field.escaped;
                    let text = draft.trim().to_owned();
                    // The multi-line field submits on Cmd/Ctrl+Enter; plain Enter adds a line.
                    if field.submitted && field.submit_mods.cmd && !text.is_empty() {
                        posted = Some(text.clone());
                    }
                    Row::new().show(ui, |ui, _| {
                        let post = Button::new(words.post.clone()).enabled(!text.is_empty()).show(ui);
                        if clicked(&post) && !text.is_empty() {
                            posted = Some(text.clone());
                        }
                        let close = Button::new(words.cancel.clone()).show(ui);
                        cancel |= clicked(&close);
                    });
                });
            });
        let closed = !open || shown.output.closed.is_some();
        if posted.is_some() || cancel || closed {
            draft.clear();
        }
        output.cancelled = posted.is_none() && (cancel || closed);
        output.posted = posted;
        output
    }
}

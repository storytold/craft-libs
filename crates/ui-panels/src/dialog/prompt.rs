//! `Prompt` (24.4): a question with answers on the modal preset of `Dialog`.

use super::frame::DialogOutput;
use super::modal::Dialog;
use crate::choice::Checkbox;
use crate::contract::{Bind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use crate::display::{Icon, Shade, Text};
use crate::layout_components::{Row, Stack};
use crate::notices::{Notice, NoticeLayout};
use crate::primitives::IconSpec;
use crate::runtime::env::{Env, Label};
use crate::runtime::focus::Initial;
use crate::runtime::overlay::CloseReason;
use crate::sections::{DialogRole, FooterButton, Section};
use craft_behavior::{IconId, Mixed};
use craft_keymap::{Chord, mnemonic};
use craft_theme::variant::Tone;
use egui::{Response, Ui};

#[cfg(feature = "suite")]
pub(crate) mod stories;

/// One answer of a prompt before its button exists: the label, what the button means, whether it has a letter and
/// whether it is available.
pub(super) struct Reply {
    label: Label,
    role: DialogRole,
    lettered: bool,
    chord: Option<Chord>,
    enabled: bool,
    reason: Option<Label>,
}

impl Reply {
    /// An answer with a letter.
    pub(super) fn new(label: impl Into<Label>, role: DialogRole) -> Self {
        Reply { label: label.into(), role, lettered: true, chord: None, enabled: true, reason: None }
    }

    /// The answer's label.
    pub(super) fn label(&self) -> &Label {
        &self.label
    }

    /// An answer without a letter.
    pub(super) fn plain(label: impl Into<Label>, role: DialogRole) -> Self {
        Reply { lettered: false, ..Reply::new(label, role) }
    }

    /// A modified chord that also gives this answer while the prompt is the top modal (Cmd+D for Don't Save on
    /// macOS), whatever control has the focus.
    pub(super) fn chord(mut self, chord: Chord) -> Self {
        self.chord = Some(chord);
        self
    }

    /// Whether the answer is available; `reason` is what the disabled button tells.
    pub(super) fn enabled(mut self, on: bool, reason: Option<Label>) -> Self {
        self.enabled = on;
        self.reason = reason;
        self
    }
}

/// The buttons of `replies`, in the given order. The letters are assigned over the labels in the scope's language and
/// compared under its text folding (`craft_keymap::mnemonic::assign` with [`Env::folder`]), so every language gets
/// letters from its own words; the footer shows them.
pub(super) fn buttons(env: &Env, replies: Vec<Reply>) -> Vec<FooterButton> {
    let view = env.view();
    let labels: Vec<Label> = replies.iter().map(|reply| view.resolve(&reply.label)).collect();
    let lettered: Vec<&str> = replies.iter().zip(&labels).filter(|(reply, _)| reply.lettered).map(|(_, label)| label.as_str()).collect();
    let mut letters = mnemonic::assign(&lettered, &env.folder()).into_iter();
    replies
        .into_iter()
        .zip(labels)
        .map(|(reply, label)| {
            let mut button = FooterButton::new(label, reply.role).enabled(reply.enabled);
            if let Some(reason) = reply.reason {
                button = button.disabled_reason(reason);
            }
            if let Some(chord) = reply.chord {
                button = button.chord(chord);
            }
            match reply.lettered.then(|| letters.next().flatten()).flatten() {
                Some((letter, _)) => button.letter(letter),
                None => button,
            }
        })
        .collect()
}

/// Adds the "Apply to All" box (`label`) under `prompt`'s message while `remaining`, the documents still asked about
/// this one included, is above one. `hosted` is the dialog's own field on the dialog host; without one the box keeps
/// its state in memory. `ticked` starts at the hosted state and holds the box's state once the prompt is shown.
pub(super) fn apply_to_all<'a>(prompt: Prompt<'a>, label: Label, remaining: usize, hosted: Option<&'a mut bool>, ticked: &'a mut bool) -> Prompt<'a> {
    if remaining <= 1 {
        return prompt;
    }
    let mut hosted = hosted;
    prompt.extras(move |ui| {
        let bind = hosted.as_mut().map_or_else(Bind::kept, |value| Bind::bound(&mut **value));
        if let Some(Mixed::Same(shown)) = Checkbox::new(bind).label(label.clone()).show(ui).shown {
            *ticked = *shown;
        }
    })
}

/// What a prompt reports: its response, how it closed in this pass and the answer given.
#[derive(Debug)]
#[non_exhaustive]
pub struct PromptOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the prompt closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The index, in the order given to `answers`, of the answer given in this pass: the button clicked or lettered,
    /// the Accept answer for Enter, the escape answer for Escape and the close action.
    pub picked: Option<usize>,
}

impl Outcome for PromptOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The index Escape and the close action answer with: the answer of `chosen` when there is one, else Cancel, else
/// the first Alternate answer, else the first Other answer, else the default answer.
fn escape_answer(answers: &[FooterButton], chosen: Option<DialogRole>) -> Option<usize> {
    let find = |wanted: DialogRole| answers.iter().position(|answer| answer.role() == wanted);
    chosen
        .and_then(find)
        .or_else(|| find(DialogRole::Cancel))
        .or_else(|| find(DialogRole::Alternate))
        .or_else(|| find(DialogRole::Other))
        .or_else(|| default_answer(answers))
}

/// The first Accept answer, else the first Destructive one, whether it is enabled or not.
fn default_position(answers: &[FooterButton]) -> Option<usize> {
    let find = |wanted: DialogRole| answers.iter().position(|answer| answer.role() == wanted);
    find(DialogRole::Accept).or_else(|| find(DialogRole::Destructive))
}

/// The index Enter answers with: the default answer, when it is available.
fn default_answer(answers: &[FooterButton]) -> Option<usize> {
    default_position(answers).filter(|at| answers[*at].is_enabled())
}

type Below<'a> = Box<dyn FnMut(&mut Ui) + 'a>;

/// What a prompt shows under its message, in this order: a tone line, paragraphs, a list of lines, a collapsible
/// section, and the caller's own content.
#[derive(Default)]
pub(super) struct Extras<'a> {
    tone: Option<(Tone, Label)>,
    paragraphs: Vec<Label>,
    /// A muted note after the paragraphs.
    note: Option<Label>,
    lines: Vec<Label>,
    details: Option<(Label, Label)>,
    below: Option<Below<'a>>,
}

impl Extras<'_> {
    fn is_empty(&self) -> bool {
        self.tone.is_none()
            && self.paragraphs.is_empty()
            && self.note.is_none()
            && self.lines.is_empty()
            && self.details.is_none()
            && self.below.is_none()
    }

    fn show(&mut self, ui: &mut Ui) {
        let Extras { tone, paragraphs, note, lines, details, below } = self;
        if let Some((tone, text)) = tone {
            Notice::new(*tone, text.clone()).layout(NoticeLayout::Inline).show(ui);
        }
        for paragraph in paragraphs.iter() {
            Text::new(paragraph.clone()).show(ui);
        }
        if let Some(note) = note {
            Text::new(note.clone()).shade(Shade::Muted).show(ui);
        }
        for line in lines.iter() {
            Text::new(line.clone()).show(ui);
        }
        if let Some((label, text)) = details {
            Section::new(label.clone()).initially_open(false).show(ui, |ui, _| {
                Text::new(text.clone()).show(ui);
            });
        }
        if let Some(below) = below.as_mut() {
            below(ui);
        }
    }
}

/// A question with answers: a dialog on the modal preset, centred and not movable, with a message, optional extras
/// and one footer button per answer.
///
/// - Answers: `FooterButton`s with a `DialogRole` each, placed in the platform's order. A bare letter given with
///   `FooterButton::letter` answers while the focus is inside the prompt and no text part owns keys; on Windows and
///   Linux, Alt with the letter answers while the prompt is the top modal. A button's `FooterButton::chord` (Cmd+D
///   for Don't Save on macOS) answers while the prompt is the top modal.
/// - Escape and the close action give the escape answer (Cancel, else the first Other answer, else the default
///   answer); Enter, when no part takes it, gives the Accept answer while that button is enabled. Both come back as
///   `PromptOutput::picked`.
/// - The open state is the `opened` binding, else the prompt's own, which starts open.
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct Prompt<'a> {
    title: Label,
    message: Label,
    answers: Vec<FooterButton>,
    extras: Extras<'a>,
    /// An icon of the catalog beside the message, in the accent tone.
    #[prop]
    icon: Option<IconId>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// The role whose answer Escape and the close action give, instead of the default choice.
    #[prop]
    escape: Option<DialogRole>,
    /// Whether the dialog reports the alert-dialog role, described by the message and any tone.
    alert: bool,
}

impl<'a> Prompt<'a> {
    /// A prompt titled `title` asking `message`; add its answers with `answers`.
    pub fn new(title: impl Into<Label>, message: impl Into<Label>) -> Self {
        Prompt {
            title: title.into(),
            message: message.into(),
            answers: Vec::new(),
            extras: Extras::default(),
            icon: None,
            open: None,
            escape: None,
            alert: false,
        }
    }

    /// The prompt is an alert: its node has the alert-dialog role and the message (including any tone) as its
    /// description.
    #[must_use]
    pub(super) fn alert(mut self) -> Self {
        self.alert = true;
        self
    }

    /// The answers, one footer button each.
    #[must_use]
    pub fn answers(mut self, answers: impl IntoIterator<Item = FooterButton>) -> Self {
        self.answers = answers.into_iter().collect();
        self
    }

    /// A line under the message in the colour of `tone`, with the tone's icon.
    #[must_use]
    pub fn tone(mut self, tone: Tone, text: impl Into<Label>) -> Self {
        self.extras.tone = Some((tone, text.into()));
        self
    }

    /// A muted note after the paragraphs: secondary text.
    #[must_use]
    pub fn note(mut self, text: impl Into<Label>) -> Self {
        self.extras.note = Some(text.into());
        self
    }

    /// A paragraph after the message, a label of its own.
    #[must_use]
    pub fn paragraph(mut self, text: impl Into<Label>) -> Self {
        self.extras.paragraphs.push(text.into());
        self
    }

    /// Lines under the message, one text each (a file's warnings).
    #[must_use]
    pub fn lines(mut self, lines: impl IntoIterator<Item = Label>) -> Self {
        self.extras.lines = lines.into_iter().collect();
        self
    }

    /// A collapsible section under the message; `label` names its header ("Details").
    #[must_use]
    pub fn details(mut self, label: impl Into<Label>, text: impl Into<Label>) -> Self {
        self.extras.details = Some((label.into(), text.into()));
        self
    }

    /// The caller's content under the message and the extras: a "Don't show again" check box.
    #[must_use]
    pub fn extras(mut self, body: impl FnMut(&mut Ui) + 'a) -> Self {
        self.extras.below = Some(Box::new(body));
        self
    }

    /// Shows the prompt.
    pub fn show(self, ui: &mut Ui) -> PromptOutput {
        let Prompt { title, message, answers, extras, icon, open, escape, alert } = self;
        let (escape_at, enter_at) = (escape_answer(&answers, escape), default_answer(&answers));
        // With no available default answer (none, or a blocked one) focus stays off the buttons: landing on the first
        // other one would let Enter answer with it (Discard under a blocked Recover, Cancel in a job window).
        let initial = if enter_at.is_some() { Initial::Default } else { Initial::Keep };
        let mut dialog = Dialog::prompt(title).buttons(answers).initial_focus(initial).close_on_enter(enter_at.is_some());
        if alert {
            let env = Env::of(ui);
            let view = env.view();
            let message_str = view.resolve(&message);
            let description = match &extras.tone {
                Some((_, tone_label)) => {
                    let tone_str = view.resolve(tone_label);
                    let tone = tone_str.as_str().trim();
                    let msg = message_str.as_str().trim();
                    if tone.is_empty() {
                        msg.to_owned()
                    } else if msg.is_empty() {
                        tone.to_owned()
                    } else if msg.ends_with(['.', '!', '?']) {
                        format!("{msg} {tone}")
                    } else {
                        format!("{msg}. {tone}")
                    }
                }
                None => message_str.as_str().to_owned(),
            };
            dialog = dialog.alert().description(description);
        }
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let mut extras = extras;
        let shown = dialog.show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                match &icon {
                    Some(icon) => {
                        Row::new().show(ui, |ui, _| {
                            Icon::new(IconSpec::svg(icon.clone())).tone(Tone::Accent).show(ui);
                            Text::new(message.clone()).show(ui);
                        });
                    }
                    None => {
                        Text::new(message.clone()).show(ui);
                    }
                }
                if !extras.is_empty() {
                    extras.show(ui);
                }
            });
        });
        let DialogOutput { response, closed, picked } = shown.output;
        let picked = picked.or(match closed {
            Some(CloseReason::Enter) => enter_at,
            Some(_) => escape_at,
            None => None,
        });
        PromptOutput { response, closed, picked }
    }
}

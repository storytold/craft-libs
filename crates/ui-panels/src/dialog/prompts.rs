//! `StandardPrompt` (24.4): the confirmations every app asks the same way, on the dialog host, as configurations of
//! `Prompt`.

use super::host::{DialogResult, StackDialog};
use super::prompt::{Prompt, Reply, buttons};
use super::stack::{DialogId, DialogStack, dialog_id};
use super::words::DialogWords;
use crate::choice::Checkbox;
use crate::contract::{Bind, NoValue, Outcome, Props as _, Show as _};
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::DialogRole;
use craft_behavior::Mixed;
use craft_i18n::{Count, Format};
use craft_theme::variant::Tone;
use egui::{Context, Response, Ui};
use std::any::Any;
use std::fmt;

/// Which question a `StandardPrompt` asks.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PromptKind {
    /// "Delete layer ‘Background’?" with Delete and Cancel. `names` are up to four display names; `count` the whole
    /// number.
    #[non_exhaustive]
    ConfirmDelete {
        /// Display names of the first few items.
        names: Vec<String>,
        /// How many items the action affects.
        count: usize,
    },
    /// A file with damaged data: Open Anyway or Cancel.
    #[non_exhaustive]
    DamagedFile {
        /// The file's name.
        name: String,
    },
    /// The document's own file changed on disk since it was opened or saved: Replace or Cancel (Save As keeps both;
    /// the message says so).
    #[non_exhaustive]
    ChangedOnDisk {
        /// The document's file name.
        name: String,
    },
    /// A save would overwrite a file the save dialog did not ask about: Replace or Cancel. `answered` is the name
    /// the dialog answered, when it differs.
    #[non_exhaustive]
    ReplaceFile {
        /// The existing file's name.
        name: String,
        /// The name the save dialog answered, when it differs from `name`.
        answered: Option<String>,
    },
    /// Trashing a layer mask: Apply, Delete (the Alternate) or Cancel.
    TrashMask,
    /// Trashing a vector mask: "Delete the vector mask?" with OK and Cancel.
    TrashVectorMask,
    /// A prompt whose message and buttons the app gives (`StandardPrompt::custom`).
    Custom,
}

/// The answer to a `StandardPrompt`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PromptChoice {
    /// The default button: Delete, Open Anyway, Replace, Apply.
    Proceed,
    /// The Alternate button where the prompt has one (Trash mask's Delete).
    Alternate,
    /// Dismiss without acting.
    Cancel,
}

impl PromptChoice {
    fn role(self) -> DialogRole {
        match self {
            PromptChoice::Proceed => DialogRole::Accept,
            PromptChoice::Alternate => DialogRole::Other,
            PromptChoice::Cancel => DialogRole::Cancel,
        }
    }
}

/// What a `StandardPrompt` answers with: the choice, whether "Don't show again" was ticked with Proceed, and the
/// `tag` the app gave the prompt, so a result says which question it answers (the document, the path, the layers).
#[non_exhaustive]
pub struct PromptAnswer {
    /// The button the user pressed.
    pub choice: PromptChoice,
    /// "Don't show again" was ticked and the choice is Proceed: the app stores it and skips the prompt next time.
    pub skip: bool,
    tag: Option<Box<dyn Any + Send>>,
}

impl PromptAnswer {
    /// The tag, when the prompt had one of type `T`.
    pub fn tag<T: Any>(&self) -> Option<&T> {
        self.tag.as_deref()?.downcast_ref()
    }

    /// Takes the tag out when it is a `T`; a tag of another type stays.
    pub fn take_tag<T: Any>(&mut self) -> Option<T> {
        match self.tag.take()?.downcast::<T>() {
            Ok(tag) => Some(*tag),
            Err(other) => {
                self.tag = Some(other);
                None
            }
        }
    }
}

impl fmt::Debug for PromptAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PromptAnswer").field("choice", &self.choice).field("skip", &self.skip).field("tag", &self.tag.is_some()).finish()
    }
}

/// What a standard prompt reports: its response, how it closed, the answer given and the "Don't show again" box.
#[derive(Debug)]
#[non_exhaustive]
pub struct StandardPromptOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the prompt closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The answer given in this pass; `None` while the prompt stays open.
    pub answer: Option<PromptAnswer>,
    /// The "Don't show again" box is ticked. Storing it is the app's: it skips the prompt next time when the answer
    /// is Proceed.
    pub skipped: bool,
}

impl Outcome for StandardPromptOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A standard confirmation on the dialog host: a prompt with the platform's button order, a letter on every answer
/// and, with `dont_show_again`, a check box under the message. Escape and the close action cancel.
///
/// The question comes from the `PromptKind` constructor; `title`, `message`, `paragraph` and `button` replace or add
/// to it. The app's own texts are labels, translated by the scope's catalog before they reach the prompt. The skip
/// choice is the app's to store: the output says whether the box was ticked.
///
/// `show` is the pattern entry: the prompt keeps its open state and the box in memory. On the dialog host the
/// prompt is a `StackDialog` that holds both itself.
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct StandardPrompt {
    id: DialogId,
    kind: PromptKind,
    skip_box: bool,
    skip: bool,
    open: bool,
    /// What a programmatic OK answers.
    chosen: PromptChoice,
    /// Carries a typed tag back in the `PromptAnswer` (a document id with the layer ids); read it with
    /// `PromptAnswer::tag` or `PromptAnswer::take_tag`.
    #[prop(boxed)]
    tag: Option<Box<dyn Any + Send>>,
    /// The counted noun of the delete question, as a count word with `{n}` (one and other forms); the default is
    /// "{n} item" and "{n} items".
    #[prop]
    noun: Option<Count>,
    /// Replaces the title of any kind.
    #[prop(into)]
    title: Option<Label>,
    /// Replaces the message of any kind.
    #[prop(into)]
    message: Option<Label>,
    paragraphs: Vec<Label>,
    buttons: Vec<(PromptChoice, Label)>,
    details: Option<(Label, Label)>,
    tone: Option<(Tone, Label)>,
    lines: Vec<Label>,
    /// Whether the host can apply the mask (`PromptKind::TrashMask`); with `false` the Apply button is disabled and
    /// takes no click, letter or Enter. Other kinds ignore it.
    #[prop]
    can_apply: bool,
    /// Set by `instance`.
    instance: Option<DialogId>,
}

/// A prompt's title, message and answers.
type Content = (Label, Label, Vec<(PromptChoice, Reply)>);

impl StandardPrompt {
    /// A prompt asking `kind`, with no tag and no "Don't show again" box.
    pub fn new(kind: PromptKind) -> Self {
        Self {
            id: dialog_id(("standard-prompt", std::mem::discriminant(&kind))),
            kind,
            skip_box: false,
            skip: false,
            open: true,
            chosen: PromptChoice::Proceed,
            tag: None,
            noun: None,
            title: None,
            message: None,
            paragraphs: Vec::new(),
            buttons: Vec::new(),
            details: None,
            tone: None,
            lines: Vec::new(),
            can_apply: true,
            instance: None,
        }
    }

    /// Makes this prompt one of several of its kind, for a library feature that asks its own question beside the
    /// app's prompts. Its result is kept apart: only `DialogHost::take_by_id` with the id `open` returns reaches it,
    /// so the app's `take::<StandardPrompt>()` never takes it, and opening another prompt doesn't drop it.
    #[must_use]
    pub fn instance(mut self, instance: impl std::hash::Hash + fmt::Debug) -> Self {
        self.id = dialog_id(("standard-prompt", std::mem::discriminant(&self.kind), instance));
        self.instance = Some(self.id);
        self
    }

    /// The answer a host-side `confirm` gives, as pressing that button would; Proceed (the default button) until
    /// set.
    pub fn set_choice(&mut self, choice: PromptChoice) {
        self.chosen = choice;
    }

    /// The delete confirmation for `count` items; `names` are the first few display names.
    pub fn confirm_delete(names: impl IntoIterator<Item = impl Into<String>>, count: usize) -> Self {
        Self::new(PromptKind::ConfirmDelete { names: names.into_iter().map(Into::into).collect(), count })
    }

    /// The changed-on-disk question for the file `name`.
    pub fn changed_on_disk(name: impl Into<String>) -> Self {
        Self::new(PromptKind::ChangedOnDisk { name: name.into() })
    }

    /// The replace-file question for `name`; `answered` is the name the save dialog answered, when it differs.
    pub fn replace_file(name: impl Into<String>, answered: Option<String>) -> Self {
        Self::new(PromptKind::ReplaceFile { name: name.into(), answered })
    }

    /// The layer-mask trash question.
    pub fn trash_mask() -> Self {
        Self::new(PromptKind::TrashMask)
    }

    /// A prompt with the app's own wording and buttons: rasterize a layer, or a GPU warning. Add the buttons with
    /// `button`; the choice of the pressed one is in the `PromptAnswer`. The title is the library's "Confirm" until
    /// `title` sets it.
    pub fn custom(message: impl Into<Label>) -> Self {
        Self::new(PromptKind::Custom).message(message)
    }

    /// Adds a paragraph after the message, a label of its own.
    #[must_use]
    pub fn paragraph(mut self, paragraph: impl Into<Label>) -> Self {
        self.paragraphs.push(paragraph.into());
        self
    }

    /// Adds a button with the app's label; its letter comes from the translated labels of the prompt. The first call
    /// replaces the kind's own buttons. `PromptChoice::Proceed` is the default button (Enter), `PromptChoice::Cancel`
    /// answers Escape and the close action, and `PromptChoice::Alternate` is a third answer; each is listed once.
    #[must_use]
    pub fn button(mut self, choice: PromptChoice, label: impl Into<Label>) -> Self {
        debug_assert!(self.buttons.iter().all(|(listed, _)| *listed != choice), "a prompt answers each choice once");
        self.buttons.push((choice, label.into()));
        self
    }

    /// A collapsible section under the message; `label` names its header ("Details").
    #[must_use]
    pub fn details(mut self, label: impl Into<Label>, text: impl Into<Label>) -> Self {
        self.details = Some((label.into(), text.into()));
        self
    }

    /// A line under the message in the colour of `tone`.
    #[must_use]
    pub fn tone(mut self, tone: Tone, text: impl Into<Label>) -> Self {
        self.tone = Some((tone, text.into()));
        self
    }

    /// Lines under the message, one text each (a damaged file's problems).
    #[must_use]
    pub fn lines(mut self, lines: impl IntoIterator<Item = Label>) -> Self {
        self.lines = lines.into_iter().collect();
        self
    }

    /// Adds the "Don't show again" box.
    #[must_use]
    pub fn dont_show_again(mut self) -> Self {
        self.skip_box = true;
        self
    }

    /// The question this prompt asks.
    pub fn kind(&self) -> &PromptKind {
        &self.kind
    }

    /// The title, message and answers of the kind, with the app's replacements applied.
    fn content(&self, words: &DialogWords, common: &CommonWords, format: &Format) -> Content {
        let (mut title, mut message, mut answers) = self.kind_content(words, common, format);
        if let Some(own) = &self.title {
            title = own.clone();
        }
        if let Some(own) = &self.message {
            message = own.clone();
        }
        if !self.buttons.is_empty() {
            answers = self.buttons.iter().map(|(choice, label)| (*choice, Reply::new(label.clone(), choice.role()))).collect();
        }
        if title.as_str().is_empty() {
            title = words.custom_title.clone();
        }
        (title, message, answers)
    }

    fn kind_content(&self, words: &DialogWords, common: &CommonWords, format: &Format) -> Content {
        let cancel = (PromptChoice::Cancel, Reply::new(common.cancel.clone(), DialogRole::Cancel));
        let proceed = |label: &Label, role: DialogRole| (PromptChoice::Proceed, Reply::new(label.clone(), role));
        match &self.kind {
            PromptKind::ConfirmDelete { names, count } => {
                let message = if *count <= 1 && names.len() == 1 {
                    words.confirm_delete_one.fill(&[("name", names[0].as_str())])
                } else {
                    let mut listed = names.join(", ");
                    if *count > names.len() {
                        listed.push('…');
                    }
                    let n = u64::try_from(*count).unwrap_or(u64::MAX);
                    let noun = self.noun.clone().unwrap_or_else(|| words.items.clone());
                    let things = noun.text(n, &format.integer(n));
                    words.confirm_delete_many.fill(&[("things", things.as_str()), ("names", listed.as_str())])
                };
                (words.confirm_delete_title.clone(), message.into(), vec![proceed(&words.delete, DialogRole::Destructive), cancel])
            }
            PromptKind::DamagedFile { name } => (
                words.damaged_title.clone(),
                words.damaged_message.fill(&[("name", name.as_str())]).into(),
                vec![proceed(&words.open_anyway, DialogRole::Accept), cancel],
            ),
            PromptKind::ChangedOnDisk { name } => {
                let message =
                    format!("{}\n\n{}", words.changed_on_disk_message.fill(&[("name", name.as_str())]), words.changed_on_disk_detail.as_str());
                (words.changed_on_disk_title.clone(), message.into(), vec![proceed(&words.replace, DialogRole::Destructive), cancel])
            }
            PromptKind::ReplaceFile { name, answered } => {
                let mut message = words.replace_file_message.fill(&[("name", name.as_str())]);
                if let Some(answered) = answered {
                    message = format!("{message}\n\n{}", words.replace_file_detail.fill(&[("answered", answered.as_str())]));
                }
                (words.replace_file_title.clone(), message.into(), vec![proceed(&words.replace, DialogRole::Destructive), cancel])
            }
            PromptKind::Custom => (Label::Empty, Label::Empty, Vec::new()),
            PromptKind::TrashVectorMask => (
                words.trash_vector_mask_title.clone(),
                words.trash_vector_mask_message.clone(),
                vec![proceed(&common.ok, DialogRole::Accept), cancel],
            ),
            PromptKind::TrashMask => (
                words.trash_mask_title.clone(),
                words.trash_mask_message.clone(),
                vec![
                    (PromptChoice::Proceed, Reply::new(words.apply.clone(), DialogRole::Accept).enabled(self.can_apply, None)),
                    (PromptChoice::Alternate, Reply::new(words.discard.clone(), DialogRole::Other)),
                    cancel,
                ],
            ),
        }
    }

    /// The answer of `choice`; Proceed reports the skip box when it is `ticked`.
    fn answer(&mut self, choice: PromptChoice, ticked: bool) -> PromptAnswer {
        PromptAnswer { choice, skip: choice == PromptChoice::Proceed && self.skip_box && ticked, tag: self.tag.take() }
    }

    /// Shows the prompt; `hosted` holds the open state and the box in the prompt's own fields, else in memory.
    fn present(&mut self, ui: &mut Ui, hosted: bool) -> StandardPromptOutput {
        let env = Env::of(ui);
        let (words, common) = (env.words::<DialogWords>(), env.words::<CommonWords>());
        let (title, message, answers) = self.content(&words, &common, env.format());
        let choices: Vec<PromptChoice> = answers.iter().map(|(choice, _)| *choice).collect();
        let replies: Vec<Reply> = answers.into_iter().map(|(_, reply)| reply).collect();
        let mut prompt = Prompt::new(title, message).answers(buttons(&env, replies));
        for paragraph in &self.paragraphs {
            prompt = prompt.paragraph(paragraph.clone());
        }
        if let Some((tone, text)) = &self.tone {
            prompt = prompt.tone(*tone, text.clone());
        }
        if !self.lines.is_empty() {
            prompt = prompt.lines(self.lines.iter().cloned());
        }
        if let Some((label, text)) = &self.details {
            prompt = prompt.details(label.clone(), text.clone());
        }
        let mut ticked = hosted && self.skip;
        if self.skip_box {
            let label = common.dont_show_again.clone();
            let tick = &mut ticked;
            let mut value = hosted.then_some(&mut self.skip);
            prompt = prompt.extras(move |ui| {
                if let Some(Mixed::Same(shown)) =
                    Checkbox::new(value.as_mut().map_or_else(Bind::kept, |value| Bind::bound(&mut **value))).label(label.clone()).show(ui).shown
                {
                    *tick = *shown;
                }
            });
        }
        if hosted {
            prompt = prompt.opened(Bind::bound(&mut self.open));
        }
        let shown = prompt.show(ui);
        let answer = shown.picked.map(|at| self.answer(choices[at], ticked));
        StandardPromptOutput { response: shown.response, closed: shown.closed, answer, skipped: ticked }
    }

    /// Shows the prompt; `answer` is `Some` the pass a button, Enter, Escape or the close action answers.
    pub fn show(mut self, ui: &mut Ui) -> StandardPromptOutput {
        self.present(ui, false)
    }
}

impl StackDialog for StandardPrompt {
    type Result = PromptAnswer;

    fn id(&self) -> DialogId {
        self.id
    }

    fn instance(&self) -> Option<DialogId> {
        self.instance
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<PromptAnswer>> {
        self.present(ui, true).answer.map(DialogResult::Ok)
    }

    /// The answer set with `set_choice`: the default button's, Proceed, unless set.
    fn confirm(&mut self, _: &Context) -> Option<PromptAnswer> {
        Some(self.answer(self.chosen, self.skip))
    }
}

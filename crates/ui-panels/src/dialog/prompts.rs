//! `StandardPrompt` (24.4): the confirmations every app asks the same way, on the dialog host, as configurations of
//! `Prompt`.

use super::host::{DialogResult, FieldRefusal, StackDialog};
use super::prompt::{Prompt, Reply, buttons};
use super::stack::{DialogId, DialogStack, dialog_id};
use super::words::DialogWords;
use crate::choice::Checkbox;
use crate::contract::{Bind, NoValue, Outcome, Props as _, Show as _};
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::DialogRole;
use craft_behavior::{IconId, Mixed};
use craft_host::Host;
use craft_i18n::{Count, Digits, Format, Region, Words};
use craft_theme::variant::Tone;
use egui::{Context, Response, Ui};
use serde_json::{Value, json};
use std::any::Any;
use std::fmt;
use std::sync::Arc;

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
    /// The document's own file changed on disk since it was opened or saved: Replace (Proceed), Save As… (the
    /// Alternate, which keeps both files) or Cancel.
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

/// The answer to a `StandardPrompt`. Automation names it in camel case (`"proceed"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum PromptChoice {
    /// The main action, and the default button unless `StandardPrompt::default_choice` names another: Delete, Open
    /// Anyway, Replace, Apply.
    Proceed,
    /// The Alternate button where the prompt has one (Trash mask's Delete, changed on disk's Save As…).
    Alternate,
    /// Dismiss without acting.
    Cancel,
}

impl PromptChoice {
    fn role(self) -> DialogRole {
        match self {
            PromptChoice::Proceed => DialogRole::Accept,
            PromptChoice::Alternate => DialogRole::Alternate,
            PromptChoice::Cancel => DialogRole::Cancel,
        }
    }
}

/// What a `StandardPrompt` answers with: the choice, whether "Don't show again" was ticked with an answer other than
/// Cancel, and the `tag` the app gave the prompt, so a result says which question it answers (the document, the path,
/// the layers).
#[non_exhaustive]
pub struct PromptAnswer {
    /// The button the user pressed.
    pub choice: PromptChoice,
    /// "Don't show again" was ticked and the choice is not Cancel: the app stores the choice and gives it next time
    /// without asking. A cancelled prompt is asked again.
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
    /// is not Cancel.
    pub skipped: bool,
}

impl Outcome for StandardPromptOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A standard confirmation on the dialog host: a prompt with the platform's button order, a letter on every answer
/// and, with `dont_show_again`, a check box under the message. Escape and the close action cancel, or give the
/// choice `escape` names.
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
    /// What a programmatic OK answers; the default answer until `set_choice` names another.
    chosen: Option<PromptChoice>,
    /// The answer Enter gives and the button drawn as default: Proceed unless set. A choice with no button leaves
    /// Proceed. It is also the answer a host-side `confirm` gives until `set_choice` names another. It is kept apart
    /// from `chosen`, so automation setting the answer does not move the default the user sees.
    #[prop(name = "default_choice")]
    default: PromptChoice,
    /// Carries a typed tag back in the `PromptAnswer` (a document id with the layer ids); read it with
    /// `PromptAnswer::tag` or `PromptAnswer::take_tag`.
    #[prop(boxed)]
    tag: Option<Box<dyn Any + Send>>,
    /// Escape and the close action answer `choice` in place of Cancel, for a prompt with no Cancel button ("Switched
    /// to CPU"). A choice the prompt has no enabled button for leaves the default: Cancel, else the Alternate, else
    /// the default button.
    #[prop]
    escape: PromptChoice,
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
    /// A muted note after the paragraphs: secondary text.
    #[prop(into)]
    note: Option<Label>,
    /// An icon of the catalog beside the message, in the accent tone.
    #[prop]
    icon: Option<IconId>,
    buttons: Vec<(PromptChoice, Label)>,
    details: Option<(Label, Label)>,
    tone: Option<(Tone, Label)>,
    lines: Vec<Label>,
    /// The role of the Alternate button in place of the kind's: by default `DialogRole::Alternate` (an answer that
    /// closes the question: No, Don't Save, Discard), and `DialogRole::Alternative` for a changed file's Save As….
    /// `DialogRole::Alternative` gives production's order: a second way to answer ("Save as…", "Keep editable", a
    /// script's "No") between Cancel and the accepting button.
    #[prop]
    alternate_role: Option<DialogRole>,
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
    /// The tag attached with the `tag` builder, when it is a `T`: tells which family's prompt is open.
    pub fn tag_as<T: Any>(&self) -> Option<&T> {
        self.tag.as_deref()?.downcast_ref()
    }

    /// A prompt asking `kind`, with no tag and no "Don't show again" box.
    pub fn new(kind: PromptKind) -> Self {
        Self {
            id: dialog_id(("standard-prompt", std::mem::discriminant(&kind))),
            kind,
            skip_box: false,
            skip: false,
            open: true,
            chosen: None,
            default: PromptChoice::Proceed,
            tag: None,
            escape: PromptChoice::Cancel,
            noun: None,
            title: None,
            message: None,
            paragraphs: Vec::new(),
            note: None,
            icon: None,
            buttons: Vec::new(),
            details: None,
            tone: None,
            lines: Vec::new(),
            alternate_role: None,
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

    /// The answer a host-side `confirm` gives, as pressing that button would; the default choice until set. A choice
    /// the prompt has no enabled button for gives none. The button drawn as default stays where `default_choice` put
    /// it.
    pub fn set_choice(&mut self, choice: PromptChoice) {
        self.chosen = Some(choice);
    }

    /// The choice Escape and the close action answer, when the prompt has a button that takes it; `None` otherwise.
    pub fn escape_choice(&self) -> Option<PromptChoice> {
        self.takes(self.escape).then_some(self.escape)
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

    /// What a programmatic OK answers: the choice `set_choice` named, else the default answer.
    fn chosen_answer(&self) -> PromptChoice {
        self.chosen.unwrap_or_else(|| self.default_answer())
    }

    /// The choice the default button answers: the one `default_choice` set when it has a button that takes a press,
    /// else Proceed.
    fn default_answer(&self) -> PromptChoice {
        if self.takes(self.default) { self.default } else { PromptChoice::Proceed }
    }

    /// Adds a paragraph after the message, a label of its own.
    #[must_use]
    pub fn paragraph(mut self, paragraph: impl Into<Label>) -> Self {
        self.paragraphs.push(paragraph.into());
        self
    }

    /// Adds a button with the app's label; its letter comes from the translated labels of the prompt. The first call
    /// replaces the kind's own buttons. `PromptChoice::Proceed` is the default button (Enter) unless `default_choice`
    /// names another, `PromptChoice::Cancel`
    /// answers Escape and the close action unless `escape` names another choice, and `PromptChoice::Alternate` is a
    /// third answer; each is listed once.
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

    /// The framework's damaged-file prompt for the file `name`: Open Anyway or Cancel.
    pub fn damaged_file(name: impl Into<String>) -> Self {
        Self::new(PromptKind::DamagedFile { name: name.into() })
    }

    /// The question this prompt asks.
    pub fn kind(&self) -> &PromptKind {
        &self.kind
    }

    /// The title the prompt shows under `host`: the text a scope under `host` shows, for code without a `Ui` (an app's
    /// command, a test without a frame).
    pub fn shown_title(&self, host: &Host) -> String {
        let (words, (title, _, _)) = self.content_under(host);
        title.resolved(&words).as_str().to_owned()
    }

    /// The message the prompt shows under `host`, as [`shown_title`](Self::shown_title).
    pub fn shown_message(&self, host: &Host) -> String {
        let (words, (_, message, _)) = self.content_under(host);
        message.resolved(&words).as_str().to_owned()
    }

    /// The paragraphs the prompt shows under its message under `host`, in order, as
    /// [`shown_title`](Self::shown_title).
    pub fn shown_paragraphs(&self, host: &Host) -> Vec<String> {
        let words = host.words();
        self.paragraphs.iter().map(|paragraph| paragraph.resolved(&words).as_str().to_owned()).collect()
    }

    /// The answers the prompt has buttons for under `host`, each with its button's label, in the prompt's order (the
    /// footer places them in the platform's order), as [`shown_title`](Self::shown_title).
    pub fn shown_buttons(&self, host: &Host) -> Vec<(PromptChoice, String)> {
        let (words, (_, _, answers)) = self.content_under(host);
        answers.into_iter().map(|(choice, reply)| (choice, reply.label().resolved(&words).as_str().to_owned())).collect()
    }

    /// [`content`](Self::content) in the words and number format of `host`, with those words.
    fn content_under(&self, host: &Host) -> (Words, Content) {
        let words = host.words();
        let format = host.i18n().map_or_else(|| Arc::new(Format::new(&Region::system(), Digits::Locale)), |i18n| i18n.format());
        let content = self.content(&words.set::<DialogWords>(), &words.set::<CommonWords>(), &format);
        (words, content)
    }

    /// The title, message and answers of the kind, with the app's replacements applied.
    fn content(&self, words: &DialogWords, common: &CommonWords, format: &Format) -> Content {
        let (mut title, mut message, (proceed, role)) = self.kind_content(words, common, format);
        if let Some(own) = &self.title {
            title = own.clone();
        }
        if let Some(own) = &self.message {
            message = own.clone();
        }
        // A changed file's Save as… is a second way to save, not an answer that closes the question.
        let (alternate, kind_role) = match self.kind {
            PromptKind::ChangedOnDisk { .. } => (&words.save_as, DialogRole::Alternative),
            _ => (&words.discard, DialogRole::Alternate),
        };
        let alternate_role = self.alternate_role.unwrap_or(kind_role);
        // `Prompt` and the footer take the first Accept answer as the default (Enter and the default stop), so the
        // default choice's reply is the Accept one and an Accept Proceed that is not the default steps back to Other.
        // A Destructive Proceed keeps its look.
        let default = self.default_answer();
        let answers = self
            .offered()
            .into_iter()
            .map(|(choice, enabled)| {
                let own = self.buttons.iter().find(|(button, _)| *button == choice).map(|(_, label)| {
                    let role = if choice == PromptChoice::Alternate { alternate_role } else { choice.role() };
                    (label.clone(), role)
                });
                let (label, role) = own.unwrap_or_else(|| match choice {
                    PromptChoice::Proceed => (proceed.clone(), role),
                    PromptChoice::Alternate => (alternate.clone(), alternate_role),
                    PromptChoice::Cancel => (common.cancel.clone(), DialogRole::Cancel),
                });
                let role = match role {
                    _ if default == PromptChoice::Proceed => role,
                    _ if choice == default => DialogRole::Accept,
                    DialogRole::Accept => DialogRole::Other,
                    role => role,
                };
                (choice, Reply::new(label, role).enabled(enabled, None))
            })
            .collect();
        if title.as_str().is_empty() {
            title = words.custom_title.clone();
        }
        (title, message, answers)
    }

    /// The answers the prompt has buttons for, in button order, with whether each takes a press: the app's own
    /// buttons when it gave any, else the kind's.
    fn offered(&self) -> Vec<(PromptChoice, bool)> {
        if !self.buttons.is_empty() {
            return self.buttons.iter().map(|(choice, _)| (*choice, true)).collect();
        }
        match self.kind {
            PromptKind::Custom => Vec::new(),
            PromptKind::TrashMask => vec![(PromptChoice::Proceed, self.can_apply), (PromptChoice::Alternate, true), (PromptChoice::Cancel, true)],
            PromptKind::ChangedOnDisk { .. } => vec![(PromptChoice::Proceed, true), (PromptChoice::Alternate, true), (PromptChoice::Cancel, true)],
            _ => vec![(PromptChoice::Proceed, true), (PromptChoice::Cancel, true)],
        }
    }

    /// Whether `choice` has a button that takes a press.
    fn takes(&self, choice: PromptChoice) -> bool {
        self.offered().contains(&(choice, true))
    }

    /// The kind's title, message, and the label and role of its Proceed button.
    fn kind_content(&self, words: &DialogWords, common: &CommonWords, format: &Format) -> (Label, Label, (Label, DialogRole)) {
        let proceed = |label: &Label, role: DialogRole| (label.clone(), role);
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
                (words.confirm_delete_title.clone(), message.into(), proceed(&words.delete, DialogRole::Destructive))
            }
            PromptKind::DamagedFile { name } => (
                words.damaged_title.clone(),
                words.damaged_message.fill(&[("name", name.as_str())]).into(),
                proceed(&words.open_anyway, DialogRole::Accept),
            ),
            PromptKind::ChangedOnDisk { name } => {
                let message =
                    format!("{}\n\n{}", words.changed_on_disk_message.fill(&[("name", name.as_str())]), words.changed_on_disk_detail.as_str());
                (words.changed_on_disk_title.clone(), message.into(), proceed(&words.replace, DialogRole::Destructive))
            }
            PromptKind::ReplaceFile { name, answered } => {
                let mut message = words.replace_file_message.fill(&[("name", name.as_str())]);
                if let Some(answered) = answered {
                    message = format!("{message}\n\n{}", words.replace_file_detail.fill(&[("answered", answered.as_str())]));
                }
                (words.replace_file_title.clone(), message.into(), proceed(&words.replace, DialogRole::Destructive))
            }
            PromptKind::Custom => (Label::Empty, Label::Empty, proceed(&common.ok, DialogRole::Accept)),
            PromptKind::TrashVectorMask => {
                (words.trash_vector_mask_title.clone(), words.trash_vector_mask_message.clone(), proceed(&common.ok, DialogRole::Accept))
            }
            PromptKind::TrashMask => (words.trash_mask_title.clone(), words.trash_mask_message.clone(), proceed(&words.apply, DialogRole::Accept)),
        }
    }

    /// The answer of `choice`; every choice but Cancel reports the skip box when it is `ticked`.
    fn answer(&mut self, choice: PromptChoice, ticked: bool) -> PromptAnswer {
        PromptAnswer { choice, skip: choice != PromptChoice::Cancel && self.skip_box && ticked, tag: self.tag.take() }
    }

    /// Shows the prompt; `hosted` holds the open state and the box in the prompt's own fields, else in memory.
    fn present(&mut self, ui: &mut Ui, hosted: bool) -> StandardPromptOutput {
        let env = Env::of(ui);
        let (words, common) = (env.words::<DialogWords>(), env.words::<CommonWords>());
        let (title, message, answers) = self.content(&words, &common, env.format());
        let choices: Vec<PromptChoice> = answers.iter().map(|(choice, _)| *choice).collect();
        let replies: Vec<Reply> = answers.into_iter().map(|(_, reply)| reply).collect();
        let footer = buttons(&env, replies);
        // The reply's own role, not `PromptChoice::role`: a kind's Proceed reply can be Destructive, and the prompt
        // finds the escape answer by its button's role.
        let escape = self
            .escape_choice()
            .and_then(|choice| choices.iter().position(|listed| *listed == choice))
            .and_then(|at| footer.get(at))
            .map(|button| button.role());
        // After the default remap a Proceed can share Other with the Alternate; Escape finds its answer by role, so a
        // shared role would answer whichever comes first. No prompt is built that way: this keeps it so.
        debug_assert!(
            escape.is_none_or(|role| footer.iter().filter(|button| button.role() == role).count() == 1),
            "the escape answer's role names one button"
        );
        let mut prompt = Prompt::new(title, message).answers(footer);
        if let Some(role) = escape {
            prompt = prompt.escape(role);
        }
        for paragraph in &self.paragraphs {
            prompt = prompt.paragraph(paragraph.clone());
        }
        if let Some((tone, text)) = &self.tone {
            prompt = prompt.tone(*tone, text.clone());
        }
        if let Some(note) = &self.note {
            prompt = prompt.note(note.clone());
        }
        if let Some(icon) = &self.icon {
            prompt = prompt.icon(icon.clone());
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

    /// The answer set with `set_choice`: the default button's, Proceed, unless set. None while that answer's
    /// button is missing or disabled.
    fn confirm(&mut self, _: &Context) -> Option<PromptAnswer> {
        let chosen = self.chosen_answer();
        self.takes(chosen).then(|| self.answer(chosen, self.skip))
    }

    /// `choice`, the answer a programmatic OK gives, and `skip`, the "Don't show again" box where the prompt has
    /// one.
    fn json_fields(&self) -> Option<Value> {
        let mut fields = json!({ "choice": self.chosen_answer() });
        if self.skip_box {
            fields["skip"] = Value::Bool(self.skip);
        }
        Some(fields)
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "choice" => {
                let choice = serde_json::from_value(value.clone()).ok().filter(|choice| self.takes(*choice));
                let Some(choice) = choice else {
                    let taken: Vec<Value> = self.offered().into_iter().filter(|(_, enabled)| *enabled).map(|(choice, _)| json!(choice)).collect();
                    return Err(format!("{}: the prompt answers {}", FieldRefusal::Rejects(&value).message(field), Value::from(taken)));
                };
                self.set_choice(choice);
            }
            "skip" if self.skip_box => {
                self.skip = value.as_bool().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
            }
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }
}

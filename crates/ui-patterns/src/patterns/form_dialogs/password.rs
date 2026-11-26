//! `PasswordDialog`: the password for a protected file, a permissions password, or a new password with its strength.

use crate::buttons::ToggleButton;
use crate::contract::{Bind, BodyCx, FieldLayout, NoValue, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _, clicked};
use crate::dialog::{Dialog, DialogOutput};
use crate::display::{Progress, Shade, Text};
use crate::form::{Field, FieldGroup, Help};
use crate::layout_components::{Row, Stack};
use crate::runtime::env::{Env, Label};
use crate::runtime::focus::{self, FocusTarget};
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use crate::text::TextField;
use craft_behavior::IconId;
use craft_theme::Variant;
use craft_theme::variant::{Emphasis, Tone};
use egui::{Response, Ui};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt;

#[cfg(feature = "suite")]
pub(crate) mod stories;

craft_ui::words! {
    /// The words of the password dialog beside the common Cancel. `{name}` is the file's name, `{strength}` one of
    /// the three strength words.
    pub struct PasswordWords {
        open_title: "passwordDialog.openTitle" = "Password required";
        open_message: template "passwordDialog.openMessage" = "\u{201c}{name}\u{201d} is protected. Enter a password to open it.";
        open: "passwordDialog.open" = "Open";
        permissions_title: "passwordDialog.permissionsTitle" = "Permissions password";
        permissions_message: template "passwordDialog.permissionsMessage" =
            "\u{201c}{name}\u{201d} has security restrictions. Enter its permissions password to lift them.";
        unlock: "passwordDialog.unlock" = "Unlock";
        set_title: "passwordDialog.setTitle" = "Set Password";
        type_password: "passwordDialog.type" = "Type Password";
        retype_password: "passwordDialog.retype" = "Re-type Password";
        mismatch: "passwordDialog.mismatch" = "The passwords don't match.";
        password: "passwordDialog.field" = "Password";
        show: "passwordDialog.show" = "Show password";
        hide: "passwordDialog.hide" = "Hide password";
        fields: "passwordDialog.fields" = "Password";
        strength_meter: "passwordDialog.strengthMeter" = "Password strength";
        strength: template "passwordDialog.strength" = "Strength: {strength}";
        weak: "passwordDialog.weak" = "Weak";
        medium: "passwordDialog.medium" = "Medium";
        strong: "passwordDialog.strong" = "Strong";
        wrong: "passwordDialog.wrong" = "That password is not correct.";
        open_several: count "passwordDialog.openSeveral" =
            "{n} file is protected. Enter a password to open it." | "{n} files are protected. Enter a password to open them.";
        permissions_several: count "passwordDialog.permissionsSeveral" =
            "{n} file has security restrictions. Enter its permissions password to lift them."
            | "{n} files have security restrictions. Enter their permissions password to lift them.";
        clear_hint: "passwordDialog.clearHint" = "Leave both fields empty to remove the password.";
    }
}

/// A password, kept in memory only: `Debug` never prints it (debug output ends up in logs), and its bytes are
/// overwritten when it is dropped. Only this buffer is overwritten: while the dialog is open the text is also in the
/// text field's `String` and the edit event's value, and those copies are freed without being zeroed.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wraps `text` as a secret.
    pub fn new(text: String) -> Self {
        Self(text)
    }

    /// The password, for handing to the decryptor. Don't store or log it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// True when no characters were typed.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_empty() { "\"\"" } else { "…" })
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

/// A rough strength of a password.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum PasswordStrength {
    /// Under 8 characters or a single character class.
    Weak,
    /// At least 8 characters and two classes.
    Medium,
    /// At least 12 characters and three classes.
    Strong,
}

impl PasswordStrength {
    /// Strong: at least 12 characters and three of the four classes (lower case, upper case, ASCII digits, anything not
    /// alphanumeric); Medium: at least 8 and two classes; else Weak.
    pub fn of(password: &str) -> PasswordStrength {
        let classes = [
            password.chars().any(char::is_lowercase),
            password.chars().any(char::is_uppercase),
            password.chars().any(|c| c.is_ascii_digit()),
            password.chars().any(|c| !c.is_alphanumeric()),
        ]
        .into_iter()
        .filter(|class| *class)
        .count();
        let length = password.chars().count();
        if length >= 12 && classes >= 3 {
            PasswordStrength::Strong
        } else if length >= 8 && classes >= 2 {
            PasswordStrength::Medium
        } else {
            PasswordStrength::Weak
        }
    }
}

/// The file or files a password is asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PasswordFiles {
    /// One file, by name.
    One(String),
    /// This many files.
    Several(usize),
}

/// What the dialog asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PasswordPurpose {
    /// The password that opens `files`; OK is Open.
    #[non_exhaustive]
    Open {
        /// The file or files, shown in the message.
        files: PasswordFiles,
    },
    /// The permissions (owner) password that lifts `files`' restrictions; OK is Unlock.
    #[non_exhaustive]
    Permissions {
        /// The file or files, shown in the message.
        files: PasswordFiles,
    },
    /// A new password typed twice, with its strength under the first field; OK stays disabled until both match and
    /// are not empty.
    Set,
    /// A new password typed twice; both fields empty is valid and clears the password.
    SetOrClear,
}

impl PasswordPurpose {
    /// The purpose of opening the file `name`.
    pub fn open(name: impl Into<String>) -> Self {
        PasswordPurpose::Open { files: PasswordFiles::One(name.into()) }
    }

    /// The purpose of lifting the restrictions of the file `name`.
    pub fn permissions(name: impl Into<String>) -> Self {
        PasswordPurpose::Permissions { files: PasswordFiles::One(name.into()) }
    }

    /// The purpose of opening `count` files with one password.
    pub fn open_several(count: usize) -> Self {
        PasswordPurpose::Open { files: PasswordFiles::Several(count) }
    }

    /// The purpose of lifting the restrictions of `count` files with one password.
    pub fn permissions_several(count: usize) -> Self {
        PasswordPurpose::Permissions { files: PasswordFiles::Several(count) }
    }
}

/// What a [`PasswordDialog`] answers: the password and the tag the app gave the dialog.
#[non_exhaustive]
pub struct PasswordAnswer {
    /// The password the user typed.
    pub secret: Secret,
    tag: Option<Box<dyn Any + Send>>,
}

impl PasswordAnswer {
    /// The tag the app gave the dialog, if it is a `T`.
    pub fn tag<T: Any>(&self) -> Option<&T> {
        self.tag.as_deref()?.downcast_ref()
    }

    /// Takes the tag out; `None` when it isn't a `T` (it stays).
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

impl fmt::Debug for PasswordAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PasswordAnswer").field("secret", &self.secret).field("tag", &self.tag.is_some()).finish()
    }
}

/// What a password dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PasswordOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The password and tag, the pass the dialog closed as OK.
    pub answer: Option<PasswordAnswer>,
}

impl Outcome for PasswordOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// What a [`PasswordFields`] reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PasswordFieldsOutput {
    /// The fields' response.
    pub response: Response,
    /// Whether the entries are valid: with a confirmation field, non-empty and equal, or both empty when
    /// [`clearable`](PasswordFields::clearable).
    pub valid: bool,
}

impl Outcome for PasswordFieldsOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The password fields: the masked field with the reveal toggle that hands the focus back, the Re-type field with the
/// mismatch help, the strength help and meter, and an error line.
#[must_use = "show the fields with `show`"]
#[derive(crate::contract::Pattern)]
pub struct PasswordFields<'a> {
    label: Label,
    input: Bind<'a, String>,
    confirm: Option<Bind<'a, String>>,
    error: Option<Label>,
    bare: bool,
    clearable: bool,
}

impl<'a> PasswordFields<'a> {
    /// The masked field named `label` over `input`.
    pub fn new(label: impl Into<Label>, input: Bind<'a, String>) -> Self {
        PasswordFields { label: label.into(), input, confirm: None, error: None, bare: false, clearable: false }
    }

    /// Adds the Re-type field over `confirm`, with the mismatch help and the strength help and meter.
    pub fn confirm(mut self, confirm: Bind<'a, String>) -> Self {
        self.confirm = Some(confirm);
        self
    }

    /// With a confirmation field, two empty entries are valid too: they clear the password. Off by default.
    pub fn clearable(mut self, on: bool) -> Self {
        self.clearable = on;
        self
    }

    /// The line under the field, in the error colour.
    pub fn error(mut self, error: Option<Label>) -> Self {
        self.error = error;
        self
    }

    /// The first field stands across the width with no caption before it, its name its placeholder: a prompt that asks
    /// for one password, as production's does.
    pub fn bare(mut self, bare: bool) -> Self {
        self.bare = bare;
        self
    }

    /// Shows the fields. With a confirmation field the first one takes the strength help once something is typed,
    /// else the error line; without one, the error line.
    pub fn show(self, ui: &mut Ui) -> PasswordFieldsOutput {
        let PasswordFields { label, mut input, mut confirm, error, bare, clearable } = self;
        let env = Env::of(ui);
        let words = env.words::<PasswordWords>();
        let mut valid = false;
        let shown = Stack::new().show(ui, |ui, cx| {
            let mut revealed = cx.memory::<Revealed>().0;
            let typed = bound_text(&mut input, cx);
            let retyped = confirm.as_mut().map(|confirm| bound_text(confirm, cx));
            let strength = (retyped.is_some() && !typed.is_empty()).then(|| PasswordStrength::of(&typed));
            FieldGroup::new(words.fields.clone()).show(ui, |ui, _| {
                let mut field = Field::new(label.clone());
                if bare {
                    field = field.layout(FieldLayout::Inside);
                }
                if let Some(strength) = strength {
                    let (word, tone) = match strength {
                        PasswordStrength::Weak => (&words.weak, Tone::Bad),
                        PasswordStrength::Medium => (&words.medium, Tone::Warn),
                        PasswordStrength::Strong => (&words.strong, Tone::Good),
                    };
                    field = field.help(Help::Status(tone, Label::from(words.strength.fill(&[("strength", word.as_str())]))));
                } else if let Some(error) = &error {
                    field = field.help(Help::Status(Tone::Bad, error.clone()));
                }
                field.show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        let mut text = TextField::new(label.clone(), input.reborrowed());
                        if bare {
                            text = text.placeholder(label.clone());
                        }
                        if !revealed {
                            text = text.masked();
                        }
                        let text = text.show(ui);
                        let name = if revealed { words.hide.clone() } else { words.show.clone() };
                        let reveal = ToggleButton::new(name, Bind::bound(&mut revealed))
                            .icon(IconId::new("eye"))
                            .on_icon(IconId::new("eye_off"))
                            .icon_only()
                            .show(ui);
                        // Typing goes on in the field: the reveal button hands the focus back, as does a press that left
                        // nothing focused.
                        if clicked(reveal.response()) || focus::focused_rec(ui.ctx()).is_none() {
                            focus::request(ui.ctx(), FocusTarget::Id(text.response().id));
                        }
                    });
                });
                if let (Some(confirm), Some(retyped)) = (confirm.as_mut(), &retyped) {
                    let mismatch = (!retyped.is_empty() && typed != *retyped).then(|| words.mismatch.clone());
                    let mut field = Field::new(words.retype_password.clone());
                    if let Some(mismatch) = mismatch {
                        field = field.help(Help::Status(Tone::Bad, mismatch));
                    }
                    field.show(ui, |ui, _| {
                        let mut text = TextField::new(words.retype_password.clone(), confirm.reborrowed());
                        if !revealed {
                            text = text.masked();
                        }
                        text.show(ui);
                    });
                }
            });
            if let Some(strength) = strength {
                let (filled, variant) = match strength {
                    PasswordStrength::Weak => (1.0 / 3.0, Variant::DANGER),
                    PasswordStrength::Medium => (2.0 / 3.0, Variant::look(Emphasis::Solid, Tone::Warn)),
                    PasswordStrength::Strong => (1.0, Variant::look(Emphasis::Solid, Tone::Good)),
                };
                Progress::new(filled).variant(variant).label(words.strength_meter.clone()).show(ui);
            }
            // The entries after this pass's edits: a bound or kept entry holds them now.
            let typed = bound_text(&mut input, cx);
            let retyped = confirm.as_mut().map(|confirm| bound_text(confirm, cx));
            valid = entries_valid(&typed, retyped.as_deref()) || (clearable && typed.is_empty() && retyped.as_deref().is_some_and(str::is_empty));
            *cx.memory::<Revealed>() = Revealed(revealed);
        });
        PasswordFieldsOutput { response: shown.output, valid }
    }
}

/// Whether the fields show their text, kept per [`PasswordFields`].
#[derive(Clone, Copy, Default)]
struct Revealed(bool);

/// The text `bind` holds this pass; empty while a kept binding holds nothing.
fn bound_text(bind: &mut Bind<'_, String>, cx: &mut BodyCx<'_>) -> String {
    bind.reborrow().read(cx).and_then(|value| value.same().cloned()).unwrap_or_default()
}

/// Whether password entries may be accepted: a non-empty entry, and with a confirmation one that it repeats.
fn entries_valid(input: &str, confirm: Option<&str>) -> bool {
    !input.is_empty() && confirm.is_none_or(|confirm| input == confirm)
}

/// What the dialog keeps while it is open: the entries and whether the fields found them valid after the last pass.
#[derive(Clone, Default)]
struct Entries {
    input: String,
    confirm: String,
    /// The fields' verdict; `None` before the first pass, when only a dialog that asks no confirmation may be accepted.
    valid: Option<bool>,
}

/// A question with a lock icon, the message, the masked field and the buttons. A new password adds a confirmation
/// field and the strength of the first entry.
///
/// - Keys: Enter submits only when the entries are valid; Escape and the close action cancel. The reveal button
///   beside the first field shows or hides the text, and its name says what it does. The first field takes the focus
///   back after a press on the reveal button and whenever nothing has it, so typing goes on.
/// - A wrong password: the app opens the dialog again with `error`; the field starts empty.
/// - The password comes back in the output's answer, the pass the dialog closed as OK.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct PasswordDialog<'a> {
    purpose: PasswordPurpose,
    /// Carried back in the answer, so the app knows which file the password is for.
    #[prop(boxed)]
    tag: Option<Box<dyn Any + Send>>,
    /// The line under the field, in the error colour.
    #[prop]
    error: Option<Label>,
    /// A muted note under the message.
    #[prop(into)]
    note: Option<Label>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
}

impl<'a> PasswordDialog<'a> {
    /// A dialog for `purpose`, with no error line and no tag.
    pub fn new(purpose: PasswordPurpose) -> Self {
        PasswordDialog { purpose, tag: None, error: None, note: None, open: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> PasswordOutput {
        let PasswordDialog { purpose, tag, error, note, open } = self;
        let env = Env::of(ui);
        let words = env.words::<PasswordWords>();
        let common = env.words::<CommonWords>();
        let setting = matches!(purpose, PasswordPurpose::Set | PasswordPurpose::SetOrClear);
        let clearing = matches!(purpose, PasswordPurpose::SetOrClear);
        let counted = |count: usize| (count as u64, env.format().integer(count as u64));
        let (title, message, ok) = match &purpose {
            PasswordPurpose::Open { files } => {
                let message = match files {
                    PasswordFiles::One(name) => words.open_message.fill(&[("name", name.as_str())]),
                    PasswordFiles::Several(count) => {
                        let (n, digits) = counted(*count);
                        words.open_several.text(n, &digits)
                    }
                };
                (words.open_title.clone(), Some(Label::from(message)), words.open.clone())
            }
            PasswordPurpose::Permissions { files } => {
                let message = match files {
                    PasswordFiles::One(name) => words.permissions_message.fill(&[("name", name.as_str())]),
                    PasswordFiles::Several(count) => {
                        let (n, digits) = counted(*count);
                        words.permissions_several.text(n, &digits)
                    }
                };
                (words.permissions_title.clone(), Some(Label::from(message)), words.unlock.clone())
            }
            PasswordPurpose::Set => (words.set_title.clone(), None, common.ok.clone()),
            PasswordPurpose::SetOrClear => (words.set_title.clone(), None, common.ok.clone()),
        };
        let mut dialog = Dialog::new(title)
            .title_icon(IconId::new("lock"))
            .buttons([FooterButton::new(ok, DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)])
            .close_on_enter(true)
            .close_guard(move |_, cx| {
                let entries = cx.memory::<Entries>();
                let valid = entries.valid.unwrap_or(false);
                if valid {
                    *entries = Entries::default();
                }
                valid
            })
            .on_pick(|button, cx| {
                if button.role() == DialogRole::Cancel {
                    *cx.memory::<Entries>() = Entries::default();
                }
            })
            .settle_buttons(move |cx, buttons| {
                let valid = cx.memory::<Entries>().valid.unwrap_or(false);
                if let Some(accept) = buttons.first_mut() {
                    accept.set_enabled(valid);
                }
            });
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let typed = RefCell::new(String::new());
        let valid = Cell::new(false);
        let shown = dialog.show(ui, |ui, cx| {
            let mut entries = cx.memory::<Entries>().clone();
            Stack::new().show(ui, |ui, _| {
                if let Some(message) = &message {
                    Text::new(message.clone()).show(ui);
                }
                if let Some(note) = &note {
                    Text::new(note.clone()).shade(Shade::Muted).show(ui);
                }
                let first = if setting { words.type_password.clone() } else { words.password.clone() };
                // A new password has no wrong-password line: its first field shows the strength instead.
                let mut fields =
                    PasswordFields::new(first, Bind::bound(&mut entries.input)).bare(!setting).error(if setting { None } else { error.clone() });
                if setting {
                    fields = fields.confirm(Bind::bound(&mut entries.confirm)).clearable(clearing);
                }
                let shown = fields.show(ui);
                if clearing {
                    Text::new(words.clear_hint.clone()).shade(Shade::Muted).show(ui);
                }
                valid.set(shown.valid);
                entries.valid = Some(shown.valid);
            });
            *typed.borrow_mut() = entries.input.clone();
            *cx.memory::<Entries>() = entries;
        });
        let DialogOutput { response, closed, .. } = shown.output;
        let answer = match closed {
            Some(CloseReason::Enter | CloseReason::Activated) if valid.get() => Some(PasswordAnswer { secret: Secret::new(typed.take()), tag }),
            _ => None,
        };
        PasswordOutput { response, closed, answer }
    }
}

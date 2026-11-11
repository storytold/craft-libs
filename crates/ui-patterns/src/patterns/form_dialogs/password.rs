//! `PasswordDialog`: the password for a protected file, a permissions password, or a new password with its strength.

use crate::buttons::ToggleButton;
use crate::contract::{Bind, NoValue, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _};
use crate::dialog::{Dialog, DialogOutput};
use crate::display::{Icon, Progress, Text};
use crate::form::{Field, FieldGroup, Help};
use crate::layout_components::{Row, Stack};
use crate::primitives::IconSpec;
use crate::runtime::env::{Env, Label};
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

#[cfg(feature = "testing")]
mod stories;

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

/// Strong: at least 12 characters and three of the four classes (lower case, upper case, ASCII digits, anything not
/// alphanumeric); Medium: at least 8 and two classes; else Weak.
pub(crate) fn password_strength(password: &str) -> PasswordStrength {
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

/// What the dialog asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PasswordPurpose {
    /// The password that opens `name`; OK is Open.
    #[non_exhaustive]
    Open {
        /// The file's name, shown in the message.
        name: String,
    },
    /// The permissions (owner) password that lifts `name`'s restrictions; OK is Unlock.
    #[non_exhaustive]
    Permissions {
        /// The file's name, shown in the message.
        name: String,
    },
    /// A new password typed twice, with its strength under the first field; OK stays disabled until both match and
    /// are not empty.
    Set,
}

impl PasswordPurpose {
    /// The purpose of opening the file `name`.
    pub fn open(name: impl Into<String>) -> Self {
        PasswordPurpose::Open { name: name.into() }
    }

    /// The purpose of lifting the restrictions of the file `name`.
    pub fn permissions(name: impl Into<String>) -> Self {
        PasswordPurpose::Permissions { name: name.into() }
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

/// What the dialog keeps while it is open: the entries and whether they show as text.
#[derive(Clone, Default)]
struct Entries {
    input: String,
    confirm: String,
    reveal: bool,
}

impl Entries {
    /// Whether OK may be pressed: a new password needs a non-empty entry that its confirmation repeats.
    fn valid(&self, setting: bool) -> bool {
        !setting || (!self.input.is_empty() && self.input == self.confirm)
    }
}

/// A question with a lock icon, the message, the masked field and the buttons. A new password adds a confirmation
/// field and the strength of the first entry.
///
/// - Keys: Enter submits only when the entries are valid; Escape and the close action cancel. The reveal button
///   beside the first field shows or hides the text, and its name says what it does.
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
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
}

impl<'a> PasswordDialog<'a> {
    /// A dialog for `purpose`, with no error line and no tag.
    pub fn new(purpose: PasswordPurpose) -> Self {
        PasswordDialog { purpose, tag: None, error: None, open: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> PasswordOutput {
        let PasswordDialog { purpose, tag, error, open } = self;
        let env = Env::of(ui);
        let words = env.words::<PasswordWords>();
        let common = env.words::<CommonWords>();
        let setting = matches!(purpose, PasswordPurpose::Set);
        let (title, message, ok) = match &purpose {
            PasswordPurpose::Open { name } => {
                (words.open_title.clone(), Some(Label::from(words.open_message.fill(&[("name", name.as_str())]))), words.open.clone())
            }
            PasswordPurpose::Permissions { name } => {
                (words.permissions_title.clone(), Some(Label::from(words.permissions_message.fill(&[("name", name.as_str())]))), words.unlock.clone())
            }
            PasswordPurpose::Set => (words.set_title.clone(), None, common.ok.clone()),
        };
        let mut dialog = Dialog::new(title)
            .buttons([FooterButton::new(ok, DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)])
            .close_guard(move |_, cx| cx.memory::<Entries>().valid(setting))
            .settle_buttons(move |cx, buttons| {
                let valid = cx.memory::<Entries>().valid(setting);
                if let Some(accept) = buttons.first_mut() {
                    accept.set_enabled(valid);
                }
            });
        if let Some(open) = open {
            dialog = dialog.opened(open);
        }
        let typed = RefCell::new(String::new());
        let valid = Cell::new(!setting);
        let shown = dialog.show(ui, |ui, cx| {
            let mut entries = cx.memory::<Entries>().clone();
            Stack::new().show(ui, |ui, _| {
                if let Some(message) = &message {
                    Row::new().show(ui, |ui, _| {
                        Icon::new(IconSpec::svg(IconId::new("lock"))).show(ui);
                        Text::new(message.clone()).show(ui);
                    });
                }
                let strength = (setting && !entries.input.is_empty()).then(|| password_strength(&entries.input));
                FieldGroup::new(words.fields.clone()).show(ui, |ui, _| {
                    let first = if setting { words.type_password.clone() } else { words.password.clone() };
                    let mut field = Field::new(first.clone());
                    if let Some(strength) = strength {
                        let (word, tone) = match strength {
                            PasswordStrength::Weak => (&words.weak, Tone::Bad),
                            PasswordStrength::Medium => (&words.medium, Tone::Warn),
                            PasswordStrength::Strong => (&words.strong, Tone::Good),
                        };
                        field = field.help(Help::Status(tone, Label::from(words.strength.fill(&[("strength", word.as_str())]))));
                    } else if let (false, Some(error)) = (setting, &error) {
                        field = field.help(Help::Status(Tone::Bad, error.clone()));
                    }
                    field.show(ui, |ui, _| {
                        Row::new().show(ui, |ui, _| {
                            let mut text = TextField::new(first.clone(), Bind::bound(&mut entries.input));
                            if !entries.reveal {
                                text = text.masked();
                            }
                            text.show(ui);
                            let name = if entries.reveal { words.hide.clone() } else { words.show.clone() };
                            ToggleButton::new(name, Bind::bound(&mut entries.reveal)).icon(IconId::new("eye")).icon_only().show(ui);
                        });
                    });
                    if setting {
                        let mismatch = (!entries.confirm.is_empty() && entries.input != entries.confirm).then(|| words.mismatch.clone());
                        let mut confirm = Field::new(words.retype_password.clone());
                        if let Some(mismatch) = mismatch {
                            confirm = confirm.help(Help::Status(Tone::Bad, mismatch));
                        }
                        confirm.show(ui, |ui, _| {
                            let mut text = TextField::new(words.retype_password.clone(), Bind::bound(&mut entries.confirm));
                            if !entries.reveal {
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
            });
            valid.set(entries.valid(setting));
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

//! Colour profile prompts (F04.12): the Embedded Profile Mismatch and Missing Profile questions
//! when a file opens ([`ProfileMismatchDialog`]), and a document's profile as the status bar and
//! the document's title say it ([`ProfileStatus`]).
//!
//! The answer is the app's to carry out (convert, assign, discard) and, with "Remember this
//! choice", to store as its Color Settings policy for that colour mode; the dialog only asks.

use craft_cms::ColorSpace;
use craft_cms::profile::Profile;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, Prompt, StackDialog, dialog_id};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use egui::Ui;

use crate::display::MonitorProblem;

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The profile prompts' words, English by default.
    #[non_exhaustive]
    pub struct ProfileWords {
        mismatch_title: "profile.mismatchTitle" = "Embedded Profile Mismatch";
        missing_title: "profile.missingTitle" = "Missing Profile";
        /// `{name}` the file name, `{mode}` the mode word ("RGB").
        mismatch_message: template "profile.mismatchMessage" = "The document \u{201c}{name}\u{201d} has an embedded color profile that does not match the current {mode} working space.";
        missing_message: template "profile.missingMessage" = "The document \u{201c}{name}\u{201d} does not have an embedded {mode} profile.";
        /// `{working}` is the working space's name.
        working: template "profile.working" = "Working: {working}";
        /// `{profile}` is the embedded profile's name.
        use_embedded: template "profile.useEmbedded" = "Use the embedded profile ({profile})";
        /// `{profile}` is the working space's name.
        convert: template "profile.convert" = "Convert to the working space ({profile})";
        discard: "profile.discard" = "Discard the embedded profile (don't color manage)";
        leave: "profile.leave" = "Leave as is (don't color manage)";
        assign_working: "profile.assignWorking" = "Assign working space";
        remember: "profile.remember" = "Remember this choice for future images";
        remember_tip: "profile.rememberTip" = "Turn the prompt back on with Ask On Mismatch in Edit \u{203a} Color Settings.";
        rgb: "profile.rgb" = "RGB";
        cmyk: "profile.cmyk" = "CMYK";
        gray: "profile.gray" = "Gray";
        /// `{mode}` the mode word.
        untagged: template "profile.untagged" = "Untagged {mode}";
        invalid: template "profile.invalid" = "Invalid {mode} profile";
        unnamed: template "profile.unnamed" = "Unnamed {mode} profile";
        /// A monitor profile that is not RGB; `{profile}` is its name, `{space}` its colour space.
        monitor_not_rgb: template "profile.monitorNotRgb" = "\u{201c}{profile}\u{201d} is a {space} profile, not an RGB profile.";
        /// A monitor profile that cannot be used; `{profile}` is its name, `{reason}` the engine's text.
        monitor_cannot_link: template "profile.monitorCannotLink" = "\u{201c}{profile}\u{201d} can't be used as a display profile: {reason}";
        /// A monitor profile that does not parse; `{reason}` is the parser's text.
        monitor_unreadable: template "profile.monitorUnreadable" = "The display profile can't be read: {reason}";
    }
}

/// A document's colour mode, as the prompts word it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProfileMode {
    #[allow(missing_docs)]
    Rgb,
    #[allow(missing_docs)]
    Cmyk,
    #[allow(missing_docs)]
    Gray,
}

impl ProfileMode {
    fn word(self, words: &ProfileWords) -> &str {
        match self {
            ProfileMode::Rgb => words.rgb.as_str(),
            ProfileMode::Cmyk => words.cmyk.as_str(),
            ProfileMode::Gray => words.gray.as_str(),
        }
    }
}

impl MonitorProblem {
    /// The reason a monitor profile was not used, in the words.
    pub fn label(&self, words: &ProfileWords) -> String {
        match self {
            MonitorProblem::NotRgb { description, space } => {
                let space = match space {
                    ColorSpace::Cmyk => words.cmyk.as_str().to_owned(),
                    ColorSpace::Gray => words.gray.as_str().to_owned(),
                    other => format!("{other:?}").to_uppercase(),
                };
                words.monitor_not_rgb.fill(&[("profile", description), ("space", &space)])
            }
            MonitorProblem::CannotLink { description, reason } => words.monitor_cannot_link.fill(&[("profile", description), ("reason", reason)]),
            MonitorProblem::Unreadable { reason } => words.monitor_unreadable.fill(&[("reason", reason)]),
        }
    }
}

/// A document's profile as Photoshop's status bar (Document Profile) says it: "Untagged RGB" (no
/// profile), "Invalid RGB profile" (one that does not parse or link), "Unnamed RGB profile" (no
/// description), else the profile's description.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileStatus {
    /// The document has no profile.
    Untagged,
    /// The profile does not parse or link.
    Invalid,
    /// The profile has no description.
    Unnamed,
    /// The profile's description.
    Named(String),
}

impl ProfileStatus {
    /// The status of a document whose embedded profile bytes are `icc` (`None` without one).
    pub fn of(icc: Option<&[u8]>) -> ProfileStatus {
        let Some(bytes) = icc else { return ProfileStatus::Untagged };
        let Ok(profile) = Profile::parse(bytes) else { return ProfileStatus::Invalid };
        let name: String = profile.description.chars().filter(|c| !c.is_control()).take(128).collect();
        let name = name.trim();
        if name.is_empty() { ProfileStatus::Unnamed } else { ProfileStatus::Named(name.to_owned()) }
    }

    /// The text: the words' template with the mode word, or the name.
    pub fn label(&self, mode: ProfileMode, words: &ProfileWords) -> String {
        let template = match self {
            ProfileStatus::Untagged => &words.untagged,
            ProfileStatus::Invalid => &words.invalid,
            ProfileStatus::Unnamed => &words.unnamed,
            ProfileStatus::Named(name) => return name.clone(),
        };
        template.fill(&[("mode", mode.word(words))])
    }
}

/// What the file asks.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileQuestion {
    /// The embedded profile differs from the working space; both names shown.
    #[non_exhaustive]
    Mismatch {
        /// The name of the profile embedded in the file.
        embedded: String,
        /// The name of the working space.
        working: String,
    },
    /// The file has no profile.
    #[non_exhaustive]
    Missing {
        /// The name of the working space.
        working: String,
    },
}

impl ProfileQuestion {
    /// The embedded profile `embedded` differs from the working space `working`.
    pub fn mismatch(embedded: impl Into<String>, working: impl Into<String>) -> Self {
        ProfileQuestion::Mismatch { embedded: embedded.into(), working: working.into() }
    }

    /// The file has no profile; `working` is the working space.
    pub fn missing(working: impl Into<String>) -> Self {
        ProfileQuestion::Missing { working: working.into() }
    }
}

/// The user's choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProfileAction {
    /// Mismatch: keep the embedded profile.
    UseEmbedded,
    /// Mismatch: convert the pixels to the working space.
    ConvertToWorking,
    /// Mismatch: drop the profile (not colour managed).
    Discard,
    /// Missing: leave it untagged.
    LeaveAsIs,
    /// Missing: tag it with the working space.
    AssignWorking,
}

/// The answer to a [`ProfileMismatchDialog`]: the action, and whether to remember it for this
/// mode (the app sets its policy and turns Ask On Mismatch off).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProfileAnswer {
    /// What the user chose.
    pub action: ProfileAction,
    /// Whether to apply the choice to later files of the same mode without asking.
    pub remember: bool,
}

/// Photoshop's Embedded Profile Mismatch and Missing Profile prompt on the
/// [`DialogHost`](craft_ui::dialog::DialogHost).
///
/// - The message (the file name last, so nothing in it reads as a placeholder; the missing form
///   adds "Working: …" on its own line), then a radio group: Use the embedded profile (name),
///   Convert to the working space (name), Discard the embedded profile; or for a missing
///   profile Leave as is, Assign working space. The first is chosen at open unless the app
///   gives its policy's action ([`action`](Self::action)).
/// - With [`rememberable`](Self::rememberable) (a mismatch in a mode that has a policy), the
///   "Remember this choice for future images" check box under the radios, with its tooltip.
/// - OK answers [`ProfileAnswer`]; Cancel answers `DialogResult::Cancel`: the app stops opening
///   this file and every file queued behind it (PhotoCraft).
/// - Keyboard: arrows move in the radio group, Space toggles the box, Enter is OK, Esc Cancel.
#[derive(craft_ui::contract::Pattern)]
pub struct ProfileMismatchDialog {
    id: DialogId,
    file_name: String,
    mode: ProfileMode,
    question: ProfileQuestion,
    action: ProfileAction,
    rememberable: bool,
    remember: bool,
    open: bool,
}

impl ProfileMismatchDialog {
    /// The prompt for `file_name` in `mode`.
    pub fn new(file_name: impl Into<String>, mode: ProfileMode, question: ProfileQuestion) -> ProfileMismatchDialog {
        let action = match question {
            ProfileQuestion::Mismatch { .. } => ProfileAction::UseEmbedded,
            ProfileQuestion::Missing { .. } => ProfileAction::LeaveAsIs,
        };
        ProfileMismatchDialog {
            id: dialog_id("profile-mismatch"),
            file_name: file_name.into(),
            mode,
            question,
            action,
            rememberable: false,
            remember: false,
            open: true,
        }
    }

    /// The action chosen at open (the app's policy for this mode).
    #[must_use]
    pub fn action(mut self, action: ProfileAction) -> ProfileMismatchDialog {
        self.action = action;
        self
    }

    /// Offers "Remember this choice"; only for a mismatch (ignored for a missing profile).
    #[must_use]
    pub fn rememberable(mut self, on: bool) -> ProfileMismatchDialog {
        self.rememberable = on;
        self
    }
}

impl StackDialog for ProfileMismatchDialog {
    type Result = ProfileAnswer;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ProfileAnswer>> {
        let env = Env::of(ui);
        let (words, common) = (env.words::<ProfileWords>(), env.words::<CommonWords>());
        let Self { file_name, mode, question, action, rememberable, remember, open, .. } = self;
        let mode_word = mode.word(&words);
        let name = file_name.as_str();
        let (title, message, choices, can_remember) = match &*question {
            ProfileQuestion::Mismatch { embedded, working } => {
                let message = words.mismatch_message.fill(&[("mode", mode_word), ("name", name)]);
                let choices = vec![
                    (ProfileAction::UseEmbedded, Label::from(words.use_embedded.fill(&[("profile", embedded)]))),
                    (ProfileAction::ConvertToWorking, Label::from(words.convert.fill(&[("profile", working)]))),
                    (ProfileAction::Discard, words.discard.clone()),
                ];
                (words.mismatch_title.clone(), message, choices, *rememberable)
            }
            ProfileQuestion::Missing { working } => {
                let what = words.missing_message.fill(&[("mode", mode_word), ("name", name)]);
                let message = format!("{what}\n{}", words.working.fill(&[("working", working)]));
                let choices = vec![(ProfileAction::LeaveAsIs, words.leave.clone()), (ProfileAction::AssignWorking, words.assign_working.clone())];
                (words.missing_title.clone(), message, choices, false)
            }
        };
        let group_label = title.clone();
        let (remember_label, remember_tip) = (words.remember.clone(), words.remember_tip.clone());
        let answers = [FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
        let held = &mut *action;
        let ticked = &mut *remember;
        let shown = Prompt::new(title, message)
            .answers(answers)
            .extras(move |ui| {
                RadioGroup::new(group_label.clone(), Bind::bound(&mut *held)).show(ui, |ui, _| {
                    for (value, text) in choices.iter().cloned() {
                        Radio::new(value).label(text).show(ui);
                    }
                });
                if can_remember {
                    Checkbox::new(Bind::bound(&mut *ticked)).label(remember_label.clone()).tooltip(remember_tip.clone()).show(ui);
                }
            })
            .opened(Bind::bound(open))
            .show(ui);
        match shown.picked {
            Some(0) => Some(DialogResult::Ok(ProfileAnswer { action: *action, remember: *remember && can_remember })),
            Some(_) => Some(DialogResult::Cancel),
            None => None,
        }
    }
}

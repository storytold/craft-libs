//! Colour profile prompts (F04.12): the Embedded Profile Mismatch and Missing Profile questions
//! when a file opens ([`ProfileMismatchDialog`]), and a document's profile as the status bar and
//! the document's title say it ([`ProfileStatus`]).
//!
//! The answer is the app's to carry out (convert, assign, discard) and, with "Remember this
//! choice", to store as its Color Settings policy for that colour mode; the dialog only asks.

use craft_cms::ColorSpace;
use craft_cms::profile::Profile;
use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, Prompt, StackDialog, dialog_id};
use craft_ui::pickers::Select;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::words::CommonWords;
use craft_ui::sections::{DialogRole, FooterButton};
use egui::{Context, Ui};
use serde_json::{Value, json};

use crate::display::MonitorProblem;

#[cfg(feature = "suite")]
pub(crate) mod stories;

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
        assign_profile: "profile.assignProfile" = "Assign profile:";
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
    /// Missing: tag it with the profile picked from the list the dialog was given.
    AssignProfile,
}

/// The answer to a [`ProfileMismatchDialog`]: the action, and whether to remember it for this
/// mode (the app sets its policy and turns Ask On Mismatch off).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProfileAnswer {
    /// What the user chose.
    pub action: ProfileAction,
    /// The profile's name for `AssignProfile`, else `None`.
    pub profile: Option<String>,
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
    profiles: Vec<String>,
    chosen: usize,
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
            profiles: Vec::new(),
            chosen: 0,
            open: true,
        }
    }

    /// The action chosen at open (the app's policy for this mode).
    #[must_use]
    pub fn action(mut self, action: ProfileAction) -> ProfileMismatchDialog {
        self.action = action;
        self
    }

    /// The profiles the Assign profile `Select` lists under its radio row, a Missing Profile prompt only; it starts on
    /// the first name.
    #[must_use]
    pub fn profiles(mut self, names: Vec<String>) -> ProfileMismatchDialog {
        self.profiles = names;
        self.chosen = 0;
        self
    }

    /// Offers "Remember this choice"; only for a mismatch (ignored for a missing profile).
    #[must_use]
    pub fn rememberable(mut self, on: bool) -> ProfileMismatchDialog {
        self.rememberable = on;
        self
    }

    /// Whether Assign profile is offered: a missing profile and a list of profiles to assign.
    fn offers_profile(&self) -> bool {
        !self.profiles.is_empty() && matches!(self.question, ProfileQuestion::Missing { .. })
    }

    /// The actions the question offers, in radio order.
    fn offered(&self) -> Vec<ProfileAction> {
        match self.question {
            ProfileQuestion::Mismatch { .. } => vec![ProfileAction::UseEmbedded, ProfileAction::ConvertToWorking, ProfileAction::Discard],
            ProfileQuestion::Missing { .. } => {
                let mut offered = vec![ProfileAction::LeaveAsIs, ProfileAction::AssignWorking];
                if self.offers_profile() {
                    offered.push(ProfileAction::AssignProfile);
                }
                offered
            }
        }
    }

    /// The answer the dialog's state gives: the profile's name only for Assign profile.
    fn answer(&self) -> ProfileAnswer {
        let profile = if self.action == ProfileAction::AssignProfile { self.profiles.get(self.chosen).cloned() } else { None };
        ProfileAnswer { action: self.action, profile, remember: self.remember && self.can_remember() }
    }

    /// Whether "Remember this choice" shows: a mismatch the app made rememberable.
    fn can_remember(&self) -> bool {
        self.rememberable && matches!(self.question, ProfileQuestion::Mismatch { .. })
    }
}

/// What automation calls `action`.
fn action_name(action: ProfileAction) -> &'static str {
    match action {
        ProfileAction::UseEmbedded => "useEmbedded",
        ProfileAction::ConvertToWorking => "convertToWorking",
        ProfileAction::Discard => "discard",
        ProfileAction::LeaveAsIs => "leaveAsIs",
        ProfileAction::AssignWorking => "assignWorking",
        ProfileAction::AssignProfile => "assignProfile",
    }
}

impl StackDialog for ProfileMismatchDialog {
    type Result = ProfileAnswer;

    fn id(&self) -> DialogId {
        self.id
    }

    /// The chosen action, as OK gives it; `None` for an action the question does not offer.
    fn confirm(&mut self, _: &Context) -> Option<ProfileAnswer> {
        self.offered().contains(&self.action).then(|| self.answer())
    }

    /// `action`, the chosen one of the question's actions, `profile` while Assign profile is offered, and `remember`
    /// while the box shows.
    fn json_fields(&self) -> Option<serde_json::Value> {
        let mut fields = json!({ "action": action_name(self.action) });
        if self.offers_profile() {
            fields["profile"] = Value::String(self.profiles.get(self.chosen).cloned().unwrap_or_default());
        }
        if self.can_remember() {
            fields["remember"] = Value::Bool(self.remember);
        }
        Some(fields)
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "action" => {
                let picked = self.offered().iter().copied().find(|action| value.as_str() == Some(action_name(*action)));
                let Some(picked) = picked else { return Err(FieldRefusal::Rejects(&value).message(field)) };
                self.action = picked;
            }
            "profile" if self.offers_profile() => {
                let at = value.as_str().and_then(|name| self.profiles.iter().position(|profile| profile == name));
                let Some(at) = at else { return Err(FieldRefusal::Rejects(&value).message(field)) };
                self.chosen = at;
            }
            "remember" if self.can_remember() => match value.as_bool() {
                Some(on) => self.remember = on,
                None => return Err(FieldRefusal::Rejects(&value).message(field)),
            },
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ProfileAnswer>> {
        let env = Env::of(ui);
        let (words, common) = (env.words::<ProfileWords>(), env.words::<CommonWords>());
        let (offered, can_remember, offers_profile) = (self.offered(), self.can_remember(), self.offers_profile());
        let entries: Vec<Entry<usize>> =
            if offers_profile { self.profiles.iter().enumerate().map(|(at, name)| Entry::new(at, name.clone())).collect() } else { Vec::new() };
        let Self { file_name, mode, question, action, remember, chosen, open, .. } = self;
        let mode_word = mode.word(&words);
        let name = file_name.as_str();
        let (title, message, embedded, working) = match &*question {
            ProfileQuestion::Mismatch { embedded, working } => {
                let message = words.mismatch_message.fill(&[("mode", mode_word), ("name", name)]);
                (words.mismatch_title.clone(), message, embedded.as_str(), working.as_str())
            }
            ProfileQuestion::Missing { working } => {
                let what = words.missing_message.fill(&[("mode", mode_word), ("name", name)]);
                let message = format!("{what}\n{}", words.working.fill(&[("working", working)]));
                (words.missing_title.clone(), message, "", working.as_str())
            }
        };
        let choices: Vec<(ProfileAction, Label)> = offered
            .iter()
            .map(|&choice| {
                let text = match choice {
                    ProfileAction::UseEmbedded => Label::from(words.use_embedded.fill(&[("profile", embedded)])),
                    ProfileAction::ConvertToWorking => Label::from(words.convert.fill(&[("profile", working)])),
                    ProfileAction::Discard => words.discard.clone(),
                    ProfileAction::LeaveAsIs => words.leave.clone(),
                    ProfileAction::AssignWorking => words.assign_working.clone(),
                    ProfileAction::AssignProfile => words.assign_profile.clone(),
                };
                (choice, text)
            })
            .collect();
        let group_label = title.clone();
        let select_label = words.assign_profile.clone();
        let (remember_label, remember_tip) = (words.remember.clone(), words.remember_tip.clone());
        let answers = [FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
        let held = &mut *action;
        let ticked = &mut *remember;
        let picked_profile = &mut *chosen;
        let shown = Prompt::new(title, message)
            .answers(answers)
            .extras(move |ui| {
                RadioGroup::new(group_label.clone(), Bind::bound(&mut *held)).show(ui, |ui, _| {
                    for (value, text) in choices.iter().cloned() {
                        Radio::new(value).label(text).show(ui);
                    }
                });
                if offers_profile {
                    Select::new(select_label.clone(), Bind::bound(&mut *picked_profile), &entries)
                        .enabled(*held == ProfileAction::AssignProfile)
                        .show(ui);
                }
                if can_remember {
                    Checkbox::new(Bind::bound(&mut *ticked)).label(remember_label.clone()).tooltip(remember_tip.clone()).show(ui);
                }
            })
            .opened(Bind::bound(open))
            .show(ui);
        match shown.picked {
            Some(0) => Some(DialogResult::Ok(self.answer())),
            Some(_) => Some(DialogResult::Cancel),
            None => None,
        }
    }
}

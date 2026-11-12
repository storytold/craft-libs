//! `NamedPresetRow`: a preset picker with a name prompt and the commands New, Rename, Delete and the file entries, as
//! buttons or one menu.
//!
//! The host keeps the library and performs every event the row reports. New and Rename open the prompt: Enter or OK
//! commits the name, Escape or Cancel closes it, and a name the host refuses shows its message under the field and
//! keeps the prompt open.

#[cfg(feature = "testing")]
mod stories;

use super::event::PresetEvent;
use super::files::{PresetFileRequest, PresetFiles};
use super::model::PresetError;
use super::panel_menu::PresetCommands;
use super::picker::PresetPicker;
use super::words::PresetWords;
use crate::buttons::{Button, IconButton};
use crate::contract::{Bind, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _, Trigger, Validity, clicked};
use crate::display::Divider;
use crate::form::Field;
use crate::layout_components::Row;
use crate::list::RowTest;
use crate::menu::{Menu, MenuItem};
use crate::runtime::commands::{CommandSet as _, CommandState, Commands as _};
use crate::runtime::env::{Env, Label, Message};
use crate::runtime::focus::{self, FocusTarget};
use crate::runtime::words::CommonWords;
use crate::text::TextField;
use craft_behavior::{IconId, Value};
use craft_theme::Variant;
use egui::{Response, Ui};
use std::hash::Hash;

/// The commands of a row, run on the row that holds the focus: its menu rows, its buttons and the chords the keymap
/// binds (F2 renames). The row reports each as the event of the same name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "namedPreset", scope = "namedPreset")]
#[non_exhaustive]
pub enum NamedPresetCommands {
    /// Ask for a name and make a preset of the current settings.
    #[command(label = "New…")]
    New,
    /// Ask for a new name of the current preset.
    #[command(label = "Rename…", default = "F2")]
    Rename,
    /// Delete the current preset.
    #[command(label = "Delete")]
    Delete,
}

/// How a [`NamedPresetRow`] lays out its commands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NamedPresetStyle {
    /// Buttons after the picker: the file entries, then New…. Rename… and Delete are the picker's context menu
    /// (Photoshop's Contour Editor).
    #[default]
    Buttons,
    /// One "Manage presets" button with a menu: Save as New Preset…, Rename…, Delete, then the file entries
    /// (Illustrator's Image Trace).
    Menu,
}

type NameCheckFn<'a> = Box<dyn Fn(&str) -> Option<PresetError> + 'a>;

/// Whether a name may be used: the host's answer for a typed name.
pub struct NameCheck<'a>(NameCheckFn<'a>);

impl<'a> NameCheck<'a> {
    /// The check that runs `check`.
    pub fn new(check: impl Fn(&str) -> Option<PresetError> + 'a) -> Self {
        NameCheck(Box::new(check))
    }
}

impl NameCheck<'_> {
    fn refuses(&self, name: &str) -> Option<PresetError> {
        (self.0)(name)
    }
}

/// What the row asks the host to do.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum NamedPresetEvent<K> {
    /// The user picked a preset: apply it.
    Apply(K),
    /// A name was committed for a new preset made of the current settings.
    #[non_exhaustive]
    Create {
        /// The name.
        name: String,
    },
    /// A name was committed for `key`.
    #[non_exhaustive]
    Rename {
        /// The preset.
        key: K,
        /// The new name.
        text: String,
    },
    /// Delete the preset.
    Delete(K),
    /// A file entry asks for its dialog.
    File(PresetFileRequest),
}

/// What a named preset row reports.
#[derive(Debug)]
#[non_exhaustive]
#[must_use = "the host carries out the row's events"]
pub struct NamedPresetOutput<K> {
    /// The row's group.
    pub response: Response,
    /// What the host carries out, in order.
    pub events: Vec<NamedPresetEvent<K>>,
    /// What the picker's browser reported besides the pick: renames, deletes and the footer's buttons.
    pub picker: Vec<PresetEvent<K>>,
}

impl<K> Outcome for NamedPresetOutput<K> {
    type Value = crate::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The name being typed: the preset it renames, or none for a new preset.
#[derive(Clone)]
struct Draft<K> {
    target: Option<K>,
    text: String,
    /// The prompt has just opened and asks for the focus.
    focus: bool,
}

/// What a command of the row asked for in this pass.
enum Ask {
    New,
    Rename,
    Delete,
    File(PresetCommands),
}

/// A preset picker with the commands that make, rename and delete presets.
///
/// - **Picker.** The host builds the `PresetPicker` and its bindings; a pick comes back as `Apply`.
/// - **Prompt.** New opens the name field, focused, with the suggestion; Rename opens it with the current name. The host's
///   `check_name` refuses a name, and the message shows through the field. A name equal to the current name, ignoring
///   case, is no clash with itself, and the same name changes nothing.
/// - **Built-in presets.** `built_in` presets show "(built-in)" after their name and cannot be renamed or deleted.
/// - **Files.** `files` adds its Load, Replace and Save entries.
/// - **Style.** `Buttons` (the default) puts the file entries and New… as buttons after the picker, with Rename… and
///   Delete as the picker's context menu; `Menu` puts them all in one "Manage presets" menu.
/// - **Keys.** F2 on the picker renames a user preset ([`NamedPresetCommands`] has the keys).
#[must_use = "show the row with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
#[pattern(probe(NamedPresetRow<'static, u32>))]
pub struct NamedPresetRow<'a, K: Value + Eq + Hash + 'static> {
    label: Label,
    picker: PresetPicker<'a, K>,
    /// The name the New prompt starts with, and the name an empty field stands for.
    #[prop(into)]
    suggest_name: Option<String>,
    /// Whether a typed name may be used.
    #[prop(func = "Fn(&str) -> Option<PresetError>")]
    check_name: Option<NameCheck<'a>>,
    /// Which presets are built in.
    #[prop(func = "Fn(&K) -> bool")]
    built_in: Option<RowTest<'a, K>>,
    /// The file entries of the menu.
    #[prop]
    files: Option<&'a PresetFiles<'a, K>>,
    /// How the commands are laid out.
    #[prop]
    style: NamedPresetStyle,
}

impl<'a, K: Value + Eq + Hash + 'static> NamedPresetRow<'a, K> {
    /// A row named `label` around `picker`.
    pub fn new(label: impl Into<Label>, picker: PresetPicker<'a, K>) -> Self {
        NamedPresetRow {
            label: label.into(),
            picker,
            suggest_name: None,
            check_name: None,
            built_in: None,
            files: None,
            style: NamedPresetStyle::default(),
        }
    }

    /// Shows the row and reports what the host carries out.
    pub fn show(self, ui: &mut Ui) -> NamedPresetOutput<K> {
        let NamedPresetRow { label, picker, suggest_name, check_name, built_in, files, style } = self;
        let env = Env::of(ui);
        let words = env.words::<PresetWords>();
        let common = env.words::<CommonWords>();
        let rows = picker.preset_rows();
        let is_built_in = |key: &K| built_in.as_ref().is_some_and(|test| test.passes(key));
        let suggestion = suggest_name.as_deref().unwrap_or("");
        let mut picker = Some(picker);
        let mut held: Option<K> = None;
        let mut events: Vec<NamedPresetEvent<K>> = Vec::new();
        let mut from_picker: Vec<PresetEvent<K>> = Vec::new();

        let shown = Field::new(label.clone()).command_scope(NamedPresetCommands::SCOPE).show(ui, |ui, body| {
            if let Some(shown) = picker.as_mut() {
                held = shown.held_key(body);
            }
            let current = held.as_ref().and_then(|key| rows.iter().find(|row| !row.is_group() && row.key == *key));
            let user = current.filter(|row| !is_built_in(&row.key));
            if let Some(row) = current.filter(|row| is_built_in(&row.key))
                && let Some(shown) = picker.take()
            {
                picker = Some(shown.display_name(words.built_in.fill(&[("name", row.label.as_str())])));
            }
            let naming = body.memory::<Option<Draft<K>>>();
            Row::new().wrap(true).show(ui, |ui, row| {
                let mut field: Option<Response> = None;
                if let Some(picker) = picker.take() {
                    let out = picker.show(ui);
                    field = Some(out.edit.response.clone());
                    if let Some(key) = out.picked {
                        if naming.as_ref().and_then(|draft| draft.target.as_ref()).is_some_and(|target| *target != key) {
                            *naming = None;
                        }
                        events.push(NamedPresetEvent::Apply(key));
                    }
                    from_picker.extend(out.events);
                }

                let mut done = false;
                if let Some(draft) = naming.as_mut() {
                    let fallback = draft.target.is_none().then_some(suggestion);
                    let target_label = draft.target.as_ref().and_then(|key| rows.iter().find(|row| row.key == *key)).map(|row| row.label.as_str());
                    let problem = |name: &str| -> Option<Label> {
                        let own = target_label.is_some_and(|label| label.to_lowercase() == name.to_lowercase());
                        if name.is_empty() || own {
                            return None;
                        }
                        let error = check_name.as_ref()?.refuses(name)?;
                        Some(match error {
                            PresetError::NameTaken => words.name_taken.clone(),
                            _ => words.name_invalid.clone(),
                        })
                    };
                    let edit = Field::new(words.name.clone())
                        .id_salt("name")
                        .show(ui, |ui, _| {
                            let check = |text: &String| match problem(&effective_name(text, fallback)) {
                                Some(message) => Validity::Refuse(Message::new(message)),
                                None => Validity::Ok,
                            };
                            TextField::new(words.name.clone(), Bind::given(draft.text.clone()).check(check))
                                .id_salt("name-text")
                                .select_on_focus()
                                .submits()
                                .show(ui)
                        })
                        .inner;
                    let (mut submitted, mut escaped) = (false, false);
                    if let Some(edit) = edit {
                        if draft.focus {
                            draft.focus = !focus::request(ui.ctx(), FocusTarget::Id(edit.response.id));
                        }
                        if let Some(value) = edit.value() {
                            draft.text.clone_from(value);
                        }
                        (submitted, escaped) = (edit.submitted, edit.escaped);
                    }
                    let ok = clicked(&Button::new(common.ok.clone()).id_salt("ok").variant(Variant::PRIMARY).show(ui));
                    let cancel = clicked(&Button::new(common.cancel.clone()).id_salt("cancel").show(ui));
                    let name = effective_name(&draft.text, fallback);
                    if cancel || escaped {
                        done = true;
                    } else if (ok || submitted) && !name.is_empty() && problem(&name).is_none() {
                        match draft.target.clone() {
                            None => events.push(NamedPresetEvent::Create { name }),
                            Some(key) if target_label != Some(name.as_str()) => events.push(NamedPresetEvent::Rename { key, text: name }),
                            Some(_) => {}
                        }
                        done = true;
                    }
                }
                if done {
                    *naming = None;
                }

                let idle = naming.is_none();
                for (command, enabled) in
                    [(NamedPresetCommands::New, idle), (NamedPresetCommands::Rename, user.is_some()), (NamedPresetCommands::Delete, user.is_some())]
                {
                    row.publish(command, CommandState::new().enabled(enabled));
                }
                let mut asked: Vec<Ask> = Vec::new();
                match style {
                    NamedPresetStyle::Buttons => {
                        for command in files.map(PresetFiles::commands).unwrap_or_default() {
                            let enabled = files.is_some_and(|files| files.enabled(command, held.as_ref()));
                            if clicked(&Button::new(Label::Empty).command(command).enabled(enabled).show(ui)) {
                                asked.push(Ask::File(command));
                            }
                        }
                        let _ = Button::new(Label::Empty).command(NamedPresetCommands::New).show(ui);
                        if let Some(field) = &field {
                            let _ = Menu::new(words.more.clone()).opened_by(field, Trigger::Context).show(ui, |ui, _| {
                                let _ = MenuItem::command(NamedPresetCommands::Rename).show(ui);
                                let _ = MenuItem::command(NamedPresetCommands::Delete).show(ui);
                            });
                        }
                    }
                    NamedPresetStyle::Menu => {
                        let more = IconButton::new(IconId::new("more")).label(words.more.clone()).show(ui);
                        let _ = Menu::new(words.more.clone()).opened_by(&more, Trigger::Click).show(ui, |ui, _| {
                            let _ = MenuItem::command(NamedPresetCommands::New).label(words.save_as_new.clone()).show(ui);
                            let _ = MenuItem::command(NamedPresetCommands::Rename).show(ui);
                            let _ = MenuItem::command(NamedPresetCommands::Delete).show(ui);
                            let Some(files) = files else { return };
                            let commands = files.commands();
                            if !commands.is_empty() {
                                Divider::horizontal().show(ui);
                            }
                            for command in commands {
                                if MenuItem::command(command).enabled(files.enabled(command, held.as_ref())).show(ui).ran {
                                    asked.push(Ask::File(command));
                                }
                            }
                        });
                    }
                }
                while let Some((command, _)) = row.take_command::<NamedPresetCommands>() {
                    asked.push(match command {
                        NamedPresetCommands::New => Ask::New,
                        NamedPresetCommands::Rename => Ask::Rename,
                        NamedPresetCommands::Delete => Ask::Delete,
                    });
                }
                for ask in asked {
                    match ask {
                        Ask::New => *naming = Some(Draft { target: None, text: suggestion.to_owned(), focus: true }),
                        Ask::Rename => {
                            if let Some(row) = user {
                                *naming = Some(Draft { target: Some(row.key.clone()), text: row.label.clone(), focus: true });
                            }
                        }
                        Ask::Delete => {
                            if let Some(row) = user {
                                if naming.as_ref().and_then(|draft| draft.target.as_ref()) == Some(&row.key) {
                                    *naming = None;
                                }
                                events.push(NamedPresetEvent::Delete(row.key.clone()));
                            }
                        }
                        Ask::File(command) => {
                            if let Some(request) = files.and_then(|files| files.request(command, held.as_ref())) {
                                events.push(NamedPresetEvent::File(request));
                            }
                        }
                    }
                }
            });
        });
        NamedPresetOutput { response: shown.output, events, picker: from_picker }
    }
}

/// The name a field stands for: what was typed, else the suggestion.
fn effective_name(text: &str, suggestion: Option<&str>) -> String {
    let typed = text.trim();
    if typed.is_empty() { suggestion.unwrap_or("").trim().to_owned() } else { typed.to_owned() }
}

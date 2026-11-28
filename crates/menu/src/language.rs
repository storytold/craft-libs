//! The Language menu: the interface languages as radio rows, with Automatic (the system's
//! language) first.
//!
//! The Mac layout puts the submenu in the app menu after Settings ([`crate::ItemRole::Language`]);
//! Windows and Linux keep it where the app puts it (Edit › Preferences or View). Choosing a row
//! reports [`LANGUAGE_COMMAND`] with the language code; the app stores it in its preferences and
//! installs the catalog. No restart: craft-i18n switches at once.

use craft_i18n::{I18n, Lang};
use craft_keymap::CommandRef;

use crate::model::{Item, ItemRole, Node};

/// The command every Language row runs, with `{"lang": "<code>"}` (`"auto"` for Automatic).
pub const LANGUAGE_COMMAND: &str = "app.language";

/// What a Language row chose.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LanguageChoice {
    /// Follow the system's language ([`I18n::system_lang`]).
    Automatic,
    /// A language by its code (`fr`, `zh-Hant`).
    Code(String),
}

impl LanguageChoice {
    /// The choice a command made, `None` for any other command or an unknown parameter.
    pub fn from_command(command: &CommandRef) -> Option<LanguageChoice> {
        if command.id != LANGUAGE_COMMAND {
            return None;
        }
        match command.params.get("lang")?.as_str()? {
            "auto" => Some(LanguageChoice::Automatic),
            "" => None,
            code => Some(LanguageChoice::Code(code.to_owned())),
        }
    }
}

/// One language row: its code and its name in itself ("Français").
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LanguageRow {
    /// The language code the app stores.
    pub code: String,
    /// The language's name in itself.
    pub name: String,
}

impl LanguageRow {
    /// A row for the language `code`, named `name` in itself.
    pub fn new(code: impl Into<String>, name: impl Into<String>) -> LanguageRow {
        LanguageRow { code: code.into(), name: name.into() }
    }
}

/// The Language submenu.
///
/// ```no_run
/// use craft_menu::{LanguageChoice, LanguageLabels, LanguageMenu};
/// # fn demo(i18n: &craft_i18n::I18n, command: &craft_menu::CommandRef) {
/// let menu = LanguageMenu::from_i18n(i18n, LanguageChoice::Automatic);
/// let node = menu.submenu(&LanguageLabels::default()); // into the app's View menu, or anywhere on a Mac
/// if let Some(choice) = LanguageChoice::from_command(command) { /* store it, install the language */ }
/// # }
/// ```
///
/// Rows: Automatic, a separator, then English and every offered language by code, each in its
/// own name ([`I18n::languages`], [`I18n::name`]). The current choice is the selected radio row
/// (a check mark on macOS). Names are never translated: a user who can't read the current
/// language still finds their own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageMenu {
    rows: Vec<LanguageRow>,
    current: LanguageChoice,
    automatic: bool,
}

impl LanguageMenu {
    /// The menu of `rows` (English first), with `current` selected and an Automatic row.
    pub fn new(rows: Vec<LanguageRow>, current: LanguageChoice) -> Self {
        LanguageMenu { rows, current, automatic: true }
    }

    /// The menu of the languages `i18n` offers.
    pub fn from_i18n(i18n: &I18n, current: LanguageChoice) -> Self {
        let rows = i18n.languages().into_iter().map(|l| LanguageRow { code: i18n.code(l).to_owned(), name: i18n.name(l).to_owned() }).collect();
        LanguageMenu::new(rows, current)
    }

    /// Whether the Automatic row and its separator show (on by default).
    pub fn automatic(mut self, on: bool) -> Self {
        self.automatic = on;
        self
    }

    /// The rows as radio items running [`LANGUAGE_COMMAND`].
    pub fn items(&self, labels: &LanguageLabels) -> Vec<Node> {
        let mut nodes = Vec::new();
        if self.automatic {
            let on = self.current == LanguageChoice::Automatic;
            let command = CommandRef::with_params(LANGUAGE_COMMAND, serde_json::json!({ "lang": "auto" }));
            nodes.push(Node::Item(Item::new(command, labels.automatic.as_str()).radio(on)));
            nodes.push(Node::Separator);
        }
        for row in &self.rows {
            let on = matches!(&self.current, LanguageChoice::Code(c) if c.eq_ignore_ascii_case(&row.code));
            let command = CommandRef::with_params(LANGUAGE_COMMAND, serde_json::json!({ "lang": row.code }));
            nodes.push(Node::Item(Item::new(command, row.name.as_str()).radio(on)));
        }
        nodes
    }

    /// The "Language" submenu, tagged [`crate::ItemRole::Language`].
    pub fn submenu(&self, labels: &LanguageLabels) -> Node {
        Node::submenu(labels.language.as_str(), self.items(labels)).role(ItemRole::Language)
    }

    /// The language a choice means: the system's for Automatic, English for a code `i18n`
    /// doesn't have.
    pub fn resolve(choice: &LanguageChoice, i18n: &I18n) -> Lang {
        match choice {
            LanguageChoice::Automatic => i18n.system_lang(),
            LanguageChoice::Code(code) => i18n.from_code(code).unwrap_or(Lang::EN),
        }
    }
}

craft_ui::words! {
    /// The Language menu's labels, English by default.
    #[non_exhaustive]
    pub struct LanguageLabels {
        language: "language.menu" = "Language";
        automatic: "language.automatic" = "Automatic";
    }
}

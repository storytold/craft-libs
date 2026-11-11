//! [`SettingsDocument`]: the preferences model over an app's settings struct, one row per setting.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use craft_host::Host;
use craft_host::target::Os;
use craft_i18n::{I18n, Lang, Words};
use egui::{Context, Ui};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::words::SettingsDocumentWords;

mod draft;
mod model;
mod pages;
mod rules;
mod texts;

pub use draft::SettingsDraft;
pub use pages::{PageInfo, StandardPage};
pub use rules::{RuleKind, SettingRule};
use texts::Texts;
pub use texts::setting_label;

/// The app's part of a settings document: rows and pages it draws itself, dialogs it opens from them, and settings that
/// live outside the struct (the appearance page writes the suite's theme selection). Every method has a default, so `()`
/// is a document without any.
pub trait CustomSettings<T> {
    /// Draws a [`RuleKind::Custom`] row's control; edits go into `draft`.
    fn row(&mut self, _ui: &mut Ui, _key: &str, _draft: &mut SettingsDraft) {}
    /// Draws a page listed with [`SettingsDocument::custom_page`].
    fn page(&mut self, _ui: &mut Ui, _page: &str, _draft: &mut SettingsDraft) {}
    /// Dialogs opened from a custom row (a colour picker), drawn last in the window's viewport. A dialog's answer goes
    /// into `draft`.
    fn nested(&mut self, _ctx: &Context, _draft: &mut SettingsDraft) {}
    /// Whether a dialog drawn by [`Self::nested`] is open.
    fn nested_open(&self) -> bool {
        false
    }
    /// Whether the app's own part differs from what is saved (enables Apply).
    fn differs(&self) -> bool {
        false
    }
    /// Commits the app's own part; `Err` keeps the window open with the message.
    fn apply(&mut self, _saved: &T) -> Result<(), String> {
        Ok(())
    }
    /// Drops the app's own draft.
    fn cancel(&mut self) {}
    /// Resets the app's own part of `page`.
    fn reset_page(&mut self, _page: &str) {}
}

impl<T> CustomSettings<T> for () {}

/// The preferences model built from the app's settings struct `T`.
///
/// Every top-level table of `T` is a page ([`StandardPage`], or [`PageInfo`] for the app's own), and every value in it
/// a row by its type: a bool is a check box, an integer an integer field (-1,000,000,000 to 1,000,000,000 without a
/// rule's range), a float a number field with three decimals, a string a text field (a list with
/// [`SettingRule::choices`], a swatch and hex field with [`SettingRule::color`]). Nested tables and arrays get no row
/// unless a rule makes them custom. Rows keep the struct's field order; titled groups follow in their first row's order.
///
/// Labels come from the key through the shared catalog: the id `setting.<key>` (`setting.performance.historyStates`)
/// with the rule's English label, or the key spelled out by [`setting_label`] ("History states"); a missing translation
/// shows the English. Search matches the key, the English label and the translated label; a page whose title matches
/// shows all its rows.
///
/// The window edits a draft. Apply turns the draft back into `T` through serde: a value that doesn't fit refuses Apply
/// with the `settingsDocument.invalid` word; otherwise the draft is the new saved value and
/// [`take_applied`](Self::take_applied) hands it to the app, which saves it. Cancel drops the draft. Reset Section puts
/// `T::default()`'s values of the page into the draft, except rules marked [`keep_on_reset`](SettingRule::keep_on_reset).
///
/// ```ignore
/// let mut prefs = SettingsDocument::new(store.get().clone())?
///     .pages(StandardPage::ALL.map(PageInfo::from).to_vec())
///     .rules(app_rules());
/// // each pass while the window is open:
/// let out = PreferencesWindow::new(label).show(ui, &mut prefs);
/// if let Some(value) = prefs.take_applied() { store.set(value); }
/// ```
pub struct SettingsDocument<T, C = ()> {
    saved: T,
    /// `saved` as a table: what the draft is compared with.
    saved_table: toml::Table,
    draft: SettingsDraft,
    defaults: toml::Table,
    /// The struct's top-level tables and their keys in field order (a `toml::Table` sorts them).
    order: FieldOrder,
    pages: Vec<PageInfo>,
    custom_pages: Vec<String>,
    rules: BTreeMap<String, SettingRule>,
    custom: C,
    /// The system `linux_only` rules compare with.
    os: Os,
    /// The catalogs and language of [`relabel`](Self::relabel); English until then.
    translator: Option<(Arc<I18n>, Lang)>,
    own: Arc<SettingsDocumentWords>,
    /// The page and row texts, built on the first [`PreferencesModel::pages`] after a change of pages, rules, language or
    /// struct.
    texts: OnceLock<Texts>,
    applied: Option<T>,
}

/// The order of the settings struct's fields: its tables, and the keys of each.
#[derive(Clone, Debug, Default)]
struct FieldOrder {
    pages: Vec<String>,
    keys: BTreeMap<String, Vec<String>>,
}

impl FieldOrder {
    fn of<T: Serialize>(value: &T) -> Self {
        let mut order = FieldOrder::default();
        if let Ok(document) = toml_edit::ser::to_document(value) {
            for (page, item) in document.iter() {
                if let Some(table) = item.as_table_like() {
                    order.pages.push(page.to_owned());
                    order.keys.insert(page.to_owned(), table.iter().map(|(key, _)| key.to_owned()).collect());
                }
            }
        }
        order
    }
}

fn table_of<T: Serialize>(value: &T) -> Result<toml::Table, toml::ser::Error> {
    toml::Table::try_from(value)
}

impl<T: Serialize + DeserializeOwned + Default + Clone + PartialEq> SettingsDocument<T> {
    /// A document over `saved`, with the draft equal to it. `Err` when `T` has no TOML form (a map with non-string keys,
    /// a top-level value that is not a table): an empty draft would reset every setting when Apply reads it back.
    pub fn new(saved: T) -> Result<Self, toml::ser::Error> {
        let saved_table = table_of(&saved)?;
        Ok(SettingsDocument {
            order: FieldOrder::of(&saved),
            draft: SettingsDraft { table: saved_table.clone() },
            saved_table,
            defaults: table_of(&T::default())?,
            saved,
            pages: Vec::new(),
            custom_pages: Vec::new(),
            rules: BTreeMap::new(),
            custom: (),
            os: Os::current(),
            translator: None,
            own: Arc::default(),
            texts: OnceLock::new(),
            applied: None,
        })
    }
}

impl<T: Serialize + DeserializeOwned + Default + Clone + PartialEq, C: CustomSettings<T>> SettingsDocument<T, C> {
    /// The page order; tables not listed follow in the struct's order, titled by [`setting_label`] of their key.
    #[must_use]
    pub fn pages(mut self, pages: Vec<PageInfo>) -> Self {
        self.pages = pages;
        self.texts = OnceLock::new();
        self
    }

    /// Pages the app draws whole ([`CustomSettings::page`]), by id.
    #[must_use]
    pub fn custom_page(mut self, page: impl Into<String>) -> Self {
        self.custom_pages.push(page.into());
        self.texts = OnceLock::new();
        self
    }

    /// The per-setting rules. A later rule for the same key replaces an earlier one.
    #[must_use]
    pub fn rules(mut self, rules: Vec<SettingRule>) -> Self {
        self.rules = rules.into_iter().map(|r| (r.key.clone(), r)).collect();
        self.texts = OnceLock::new();
        self
    }

    /// The system that [`SettingRule::linux_only`] compares with; the system this build runs on until set.
    #[must_use]
    pub fn os(mut self, os: Os) -> Self {
        self.os = os;
        self
    }

    /// The app's custom rows, pages and outside settings.
    pub fn custom<D: CustomSettings<T>>(self, custom: D) -> SettingsDocument<T, D> {
        SettingsDocument {
            saved: self.saved,
            saved_table: self.saved_table,
            draft: self.draft,
            defaults: self.defaults,
            order: self.order,
            pages: self.pages,
            custom_pages: self.custom_pages,
            rules: self.rules,
            custom,
            os: self.os,
            translator: self.translator,
            own: self.own,
            texts: self.texts,
            applied: self.applied,
        }
    }

    /// Translates the labels into the Host's language; call when the document is made and when the language changes (a
    /// language preview).
    pub fn relabel(&mut self, host: &Host) {
        self.translator = host.i18n_shared().map(|i18n| (Arc::clone(i18n), host.lang()));
        let words = match &self.translator {
            Some((i18n, lang)) => Words::from_i18n(Arc::clone(i18n), *lang),
            None => Words::english(),
        };
        self.own = words.set::<SettingsDocumentWords>();
        self.texts = OnceLock::new();
    }

    /// The saved value the window started from (or last applied).
    pub fn saved(&self) -> &T {
        &self.saved
    }

    /// Replaces the saved value and the draft (another instance changed the file). `Err` leaves the document as it was.
    pub fn reload(&mut self, saved: T) -> Result<(), toml::ser::Error> {
        self.saved_table = table_of(&saved)?;
        self.draft = SettingsDraft { table: self.saved_table.clone() };
        self.order = FieldOrder::of(&saved);
        self.saved = saved;
        self.texts = OnceLock::new();
        Ok(())
    }

    /// The value Apply or OK committed, once.
    pub fn take_applied(&mut self) -> Option<T> {
        self.applied.take()
    }
}

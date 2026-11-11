//! The translated texts of a settings document's pages and rows, and the control each row gets.

use super::draft::lookup;
use super::rules::{RuleKind, SettingRule};
use super::{CustomSettings, SettingsDocument};
use crate::contract::Entry;
use crate::patterns::preferences::model::{RowKind, RowSpec};
use crate::runtime::env::{Label, UnitKind};
use craft_host::target::Os;
use craft_theme::Rgba;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::ops::RangeInclusive;

/// The translated texts of the pages and rows, built from the draft's keys, the rules and the catalogs.
pub(super) struct Texts {
    pub(super) pages: Vec<PageText>,
}

pub(super) struct PageText {
    pub(super) id: String,
    pub(super) title: String,
    /// The English title, for the search.
    pub(super) keywords: String,
    pub(super) custom: bool,
    /// The titled groups in the order of their first row.
    pub(super) groups: Vec<String>,
    pub(super) rows: Vec<RowText>,
}

pub(super) struct RowText {
    /// The dotted path.
    key: String,
    label: String,
    /// The English label, for the search.
    keywords: String,
    help: Option<String>,
    /// The index in [`PageText::groups`].
    pub(super) group: Option<usize>,
    shape: Shape,
}

/// The control of a row, from its rule and the type of its value.
enum Shape {
    Bool,
    Int {
        range: RangeInclusive<i64>,
        unit: Option<UnitKind>,
    },
    Float {
        range: RangeInclusive<f64>,
        decimals: u8,
        unit: Option<UnitKind>,
    },
    Text,
    /// `(id, translated label)`, and what the ids count when the row labels them itself.
    Choice(Vec<(String, Label)>, Option<UnitKind>),
    Color,
    Custom,
}

/// The range of a number without a rule.
pub(super) const OPEN_RANGE: f64 = 1e9;

fn shape_of(rule: Option<&SettingRule>, value: Option<&toml::Value>) -> Option<Shape> {
    use toml::Value;
    let kind = rule.and_then(|rule| rule.kind.as_ref());
    if matches!(kind, Some(RuleKind::Custom)) {
        return Some(Shape::Custom);
    }
    Some(match (kind, value?) {
        (Some(RuleKind::Choice(options)), Value::String(_) | Value::Integer(_)) => {
            let counted = rule.and_then(|rule| rule.counted);
            Shape::Choice(options.iter().map(|(id, label)| (id.clone(), Label::from(label.clone()))).collect(), counted)
        }
        (Some(RuleKind::Color), Value::String(_)) => Shape::Color,
        (Some(RuleKind::Int { range, unit }), Value::Integer(_)) => Shape::Int { range: range.clone(), unit: *unit },
        (Some(RuleKind::Float { range, decimals, unit }), Value::Float(_)) => Shape::Float { range: range.clone(), decimals: *decimals, unit: *unit },
        (_, Value::Boolean(_)) => Shape::Bool,
        (_, Value::Integer(_)) => Shape::Int { range: -(OPEN_RANGE as i64)..=OPEN_RANGE as i64, unit: None },
        (_, Value::Float(_)) => Shape::Float { range: -OPEN_RANGE..=OPEN_RANGE, decimals: 3, unit: None },
        (_, Value::String(_)) => Shape::Text,
        _ => return None,
    })
}

/// A choice row of the translated `options`, with `value` chosen.
fn choice_kind<'a>(options: &'a [(String, Label)], counted: Option<UnitKind>, value: &'a str) -> RowKind<'a> {
    let kind = RowKind::choice(value, options.iter().map(|(id, label)| Entry::new(id.as_str(), label.clone())).collect());
    match counted {
        Some(unit) => kind.counting(unit),
        None => kind,
    }
}

/// What `shown_when` compares: a string as it is, a bool or number as Rust writes it.
fn value_text(value: &toml::Value) -> String {
    match value {
        toml::Value::String(text) => text.clone(),
        toml::Value::Boolean(on) => on.to_string(),
        toml::Value::Integer(n) => n.to_string(),
        toml::Value::Float(x) => x.to_string(),
        _ => String::new(),
    }
}

impl<T: Serialize + DeserializeOwned + Default + Clone + PartialEq, C: CustomSettings<T>> SettingsDocument<T, C> {
    /// The English text under the catalog `id`, translated into the language of [`relabel`](Self::relabel).
    fn translate(&self, id: &str, english: &str) -> String {
        match &self.translator {
            Some((i18n, lang)) => match i18n.tid(*lang, id, english) {
                "" => english.to_owned(),
                text => text.to_owned(),
            },
            None => english.to_owned(),
        }
    }

    pub(super) fn texts(&self) -> &Texts {
        self.texts.get_or_init(|| self.build_texts())
    }

    /// Every page with a table in the struct or a custom page, in the app's order and then the struct's: `(id, English
    /// title)`.
    fn page_list(&self) -> Vec<(String, String)> {
        let mut list: Vec<(String, String)> = Vec::new();
        let known = |id: &str| self.order.keys.contains_key(id) || self.custom_pages.iter().any(|page| page == id);
        let add = |id: &str, title: String, list: &mut Vec<(String, String)>| {
            if !list.iter().any(|(listed, _)| listed == id) {
                list.push((id.to_owned(), title));
            }
        };
        for page in self.pages.iter().filter(|page| known(&page.id)) {
            add(&page.id, page.title.clone(), &mut list);
        }
        for id in self.order.pages.iter().chain(&self.custom_pages) {
            add(id, setting_label(id), &mut list);
        }
        list
    }

    fn build_texts(&self) -> Texts {
        let pages = self
            .page_list()
            .into_iter()
            .map(|(id, english)| {
                let custom = self.custom_pages.contains(&id);
                let mut page = PageText {
                    title: self.translate(&format!("settingsPage.{id}"), &english),
                    keywords: english,
                    custom,
                    groups: Vec::new(),
                    rows: Vec::new(),
                    id,
                };
                if !custom {
                    self.build_rows(&mut page);
                }
                page
            })
            .collect();
        Texts { pages }
    }

    fn build_rows(&self, page: &mut PageText) {
        let table = self.saved_table.get(&page.id).and_then(toml::Value::as_table);
        let mut keys: Vec<String> = self.order.keys.get(&page.id).cloned().unwrap_or_default();
        // Custom rows of settings that live outside the struct.
        let prefix = format!("{}.", page.id);
        for (path, rule) in self.rules.range(prefix.clone()..).take_while(|(path, _)| path.starts_with(&prefix)) {
            let key = &path[prefix.len()..];
            if matches!(rule.kind, Some(RuleKind::Custom)) && !keys.iter().any(|k| k == key) {
                keys.push(key.to_owned());
            }
        }
        for key in keys {
            let path = format!("{}.{key}", page.id);
            let rule = self.rules.get(&path);
            let Some(mut shape) = shape_of(rule, table.and_then(|t| t.get(&key))) else { continue };
            if let Shape::Choice(options, None) = &mut shape {
                for (id, label) in options.iter_mut() {
                    *label = Label::from(self.translate(&format!("setting.{path}.{id}"), label.as_str()));
                }
            }
            let english = rule.and_then(|rule| rule.label.clone()).unwrap_or_else(|| setting_label(&path));
            let mut help = Vec::new();
            if let Some(text) = rule.and_then(|rule| rule.help.as_deref()) {
                help.push(self.translate(&format!("setting.{path}.help"), text));
            }
            if rule.is_some_and(|rule| rule.restart) {
                help.push(self.own.restart.as_str().to_owned());
            }
            let group = rule.and_then(|rule| rule.group.as_deref()).map(|english| {
                let title = self.translate(&format!("settingGroup.{english}"), english);
                page.groups.iter().position(|g| *g == title).unwrap_or_else(|| {
                    page.groups.push(title);
                    page.groups.len() - 1
                })
            });
            page.rows.push(RowText {
                label: self.translate(&format!("setting.{path}"), &english),
                keywords: english,
                help: (!help.is_empty()).then(|| help.join("\n")),
                key: path,
                group,
                shape,
            });
        }
    }

    /// Whether the rule lets the row show with the draft's values.
    fn row_visible(&self, key: &str) -> bool {
        let Some(rule) = self.rules.get(key) else { return true };
        if rule.hidden || (rule.linux_only && self.os != Os::Linux) {
            return false;
        }
        match &rule.shown_when {
            Some((other, wanted)) => lookup(&self.draft.table, other).is_some_and(|value| value_text(value) == *wanted),
            None => true,
        }
    }

    pub(super) fn row_spec<'a>(&'a self, row: &'a RowText) -> Option<RowSpec<'a>> {
        use toml::Value;
        let value = lookup(&self.draft.table, &row.key);
        let kind = match (&row.shape, value) {
            (Shape::Bool, Some(Value::Boolean(on))) => RowKind::Bool(*on),
            (Shape::Int { range, unit }, Some(Value::Integer(n))) => RowKind::Int { value: *n, range: range.clone(), unit: *unit },
            (Shape::Float { range, decimals, unit }, Some(Value::Float(x))) => {
                RowKind::Float { value: *x, range: range.clone(), decimals: *decimals, unit: *unit }
            }
            (Shape::Text, Some(Value::String(text))) => RowKind::Text(text),
            (Shape::Choice(options, counted), Some(Value::Integer(n))) => {
                let id = options.iter().map(|(id, _)| id.as_str()).find(|id| *id == n.to_string()).unwrap_or("");
                choice_kind(options, *counted, id)
            }
            (Shape::Choice(options, counted), Some(Value::String(id))) => choice_kind(options, *counted, id),
            (Shape::Color, Some(Value::String(text))) => RowKind::Color(Rgba::from_hex(text).unwrap_or(Rgba::rgb(128, 128, 128))),
            (Shape::Custom, _) => RowKind::Custom,
            _ => return None,
        };
        let spec = RowSpec::new(&row.key, &row.label, kind).keywords(&row.keywords).visible(self.row_visible(&row.key));
        Some(match &row.help {
            Some(help) => spec.help(help),
            None => spec,
        })
    }
}

/// A key's English label in sentence case: the last part of the dotted key with its first letter upper-cased and each
/// later capital turned into a space and the lower-case letter ("historyStates" becomes "History states"), then these
/// replacements in this order: "Psd" to "PSD", "Gpu" to "GPU", "Ui " to "UI ", "Mb" to "(MB)", "Exif" to "EXIF", "Hud" to
/// "HUD", "Lut " to "LUT ". A rule's label wins over this.
pub fn setting_label(key: &str) -> String {
    /// The spelling of acronyms and units in a label made from a key, applied in this order.
    const REPLACEMENTS: [(&str, &str); 7] =
        [("Psd", "PSD"), ("Gpu", "GPU"), ("Ui ", "UI "), ("Mb", "(MB)"), ("Exif", "EXIF"), ("Hud", "HUD"), ("Lut ", "LUT ")];
    let leaf = key.rsplit('.').next().unwrap_or(key);
    let mut label = String::new();
    for (at, ch) in leaf.chars().enumerate() {
        if at == 0 {
            label.extend(ch.to_uppercase());
        } else if ch.is_uppercase() {
            label.push(' ');
            label.extend(ch.to_lowercase());
        } else {
            label.push(ch);
        }
    }
    REPLACEMENTS.iter().fold(label, |label, (from, to)| label.replace(from, to))
}

//! SettingsDocument as a PreferencesModel.

use super::draft::{SettingsDraft, insert, lookup};
use super::rules::RuleKind;
use super::texts::OPEN_RANGE;
use super::{CustomSettings, SettingsDocument, table_of};
use crate::patterns::preferences::model::{GroupSpec, PageSpec, PrefValue, PreferencesModel};
use craft_theme::Rgba;
use egui::{Context, Ui};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::sync::Arc;

impl<T: Serialize + DeserializeOwned + Default + Clone + PartialEq, C: CustomSettings<T>> PreferencesModel for SettingsDocument<T, C> {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        self.texts()
            .pages
            .iter()
            .map(|page| {
                if page.custom {
                    return PageSpec::custom(&page.id, &page.title).keywords(&page.keywords);
                }
                let mut groups: Vec<GroupSpec<'_>> = std::iter::once(None)
                    .chain(page.groups.iter().map(|title| Some(title.as_str())))
                    .map(|title| GroupSpec::new(title, Vec::new()))
                    .collect();
                for row in &page.rows {
                    if let Some(spec) = self.row_spec(row) {
                        groups[row.group.map_or(0, |at| at + 1)].rows.push(spec);
                    }
                }
                groups.retain(|group| !group.rows.is_empty());
                let mut spec = PageSpec::new(&page.id, &page.title, Vec::new()).keywords(&page.keywords);
                spec.groups = groups;
                spec
            })
            .collect()
    }

    /// Writes the row's value into the draft, clamped to its range or kept to its choices.
    fn set(&mut self, key: &str, value: PrefValue) {
        use toml::Value;
        let Some(current) = lookup(&self.draft.table, key) else { return };
        let kind = self.rules.get(key).and_then(|rule| rule.kind.as_ref());
        let int_range = match kind {
            Some(RuleKind::Int { range, .. }) => range.clone(),
            _ => -(OPEN_RANGE as i64)..=OPEN_RANGE as i64,
        };
        let float_range = match kind {
            Some(RuleKind::Float { range, .. }) => range.clone(),
            _ => -OPEN_RANGE..=OPEN_RANGE,
        };
        let clamp_int = |n: i64| n.max(*int_range.start()).min(*int_range.end());
        let clamp_float = |x: f64| x.max(*float_range.start()).min(*float_range.end());
        let new = match (value, current) {
            (PrefValue::Bool(on), Value::Boolean(_)) => Value::Boolean(on),
            (PrefValue::Int(n), Value::Integer(_)) => Value::Integer(clamp_int(n)),
            (PrefValue::Int(n), Value::Float(_)) => Value::Float(clamp_float(n as f64)),
            (PrefValue::Float(x), Value::Float(_)) if x.is_finite() => Value::Float(clamp_float(x)),
            (PrefValue::Float(x), Value::Integer(_)) if x.is_finite() => Value::Integer(clamp_int(x.round() as i64)),
            (PrefValue::Text(text), Value::String(_)) if matches!(kind, None | Some(RuleKind::Custom)) => Value::String(text),
            (PrefValue::Choice(id), Value::String(_) | Value::Integer(_)) => match kind {
                Some(RuleKind::Choice(options)) if options.iter().any(|(option, _)| *option == id) => match id.parse::<i64>() {
                    Ok(n) if self.rules.get(key).is_some_and(|rule| rule.integer) => Value::Integer(n),
                    _ if matches!(current, Value::Integer(_)) => return,
                    _ => Value::String(id),
                },
                _ => return,
            },
            (PrefValue::Color(color), Value::String(_)) if matches!(kind, Some(RuleKind::Color)) => {
                Value::String(Rgba::rgb(color.0[0], color.0[1], color.0[2]).to_hex())
            }
            _ => return,
        };
        insert(&mut self.draft.table, key, new);
    }

    fn custom_row(&mut self, ui: &mut Ui, key: &str) {
        self.custom.row(ui, key, &mut self.draft);
    }

    fn custom_page(&mut self, ui: &mut Ui, page: &str) {
        self.custom.page(ui, page, &mut self.draft);
    }

    fn differs(&self) -> bool {
        self.draft.table != self.saved_table || self.custom.differs()
    }

    fn apply(&mut self) -> Result<(), String> {
        let words = Arc::clone(&self.own);
        let invalid = |error: String| words.invalid.fill(&[("error", &error)]);
        let value: T =
            toml::Value::Table(self.draft.table.clone()).try_into().map_err(|error: toml::de::Error| invalid(error.message().to_owned()))?;
        // The serialized value is the next draft: the struct's own reading of the values.
        let table = table_of(&value).map_err(|error| invalid(error.to_string()))?;
        self.custom.apply(&value)?;
        self.saved_table = table;
        self.draft = SettingsDraft { table: self.saved_table.clone() };
        self.saved = value.clone();
        self.applied = Some(value);
        Ok(())
    }

    fn cancel(&mut self) {
        self.draft = SettingsDraft { table: self.saved_table.clone() };
        self.custom.cancel();
    }

    fn reset_page(&mut self, page: &str) {
        if let Some(toml::Value::Table(defaults)) = self.defaults.get(page) {
            // The page is replaced, so a key the defaults leave out (an `Option` that is `None`) goes too; only the
            // `keep_on_reset` keys survive.
            let mut reset = defaults.clone();
            if let Some(toml::Value::Table(current)) = self.draft.table.get(page) {
                for (key, value) in current {
                    if self.rules.get(&format!("{page}.{key}")).is_some_and(|rule| rule.keep_on_reset) {
                        reset.insert(key.clone(), value.clone());
                    }
                }
            }
            self.draft.table.insert(page.to_owned(), toml::Value::Table(reset));
        }
        self.custom.reset_page(page);
    }

    fn nested(&mut self, ctx: &Context) {
        self.custom.nested(ctx, &mut self.draft);
    }

    fn nested_open(&self) -> bool {
        self.custom.nested_open()
    }
}

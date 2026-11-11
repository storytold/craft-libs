//! The condition builder: rows of field, operator and value, with nested groups.

use std::cell::{Cell, RefCell};

use craft_behavior::IconId;
use craft_theme::variant::Tone;
use craft_ui::buttons::{Button, IconButton};
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Entry, Held, Pad, Pattern, Props as _, RowAlign, Show as _, ShowContainer as _, clicked};
use craft_ui::display::Text;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::notices::{Notice, NoticeLayout};
use craft_ui::pickers::{DatePicker, PickedDate, Select};
use craft_ui::primitives::IconSpec;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::runtime::pointer;
use craft_ui::text::{NumberField, TextField};
use egui::{Response, Ui};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The builder's words, English by default.
    #[non_exhaustive]
    pub struct ConditionWords {
        match_word: "conditions.match" = "Match";
        of_rules: "conditions.ofRules" = "of the following rules:";
        all: "conditions.all" = "all";
        any: "conditions.any" = "any";
        none: "conditions.none" = "none";
        field: "conditions.field" = "Field";
        operator: "conditions.operator" = "Operator";
        value: "conditions.value" = "Value";
        and: "conditions.and" = "and";
        add_rule: "conditions.addRule" = "+ Rule";
        add_group: "conditions.addGroup" = "+ Group";
        add_rule_tip: "conditions.addRuleTip" = "Add a rule (⌥-click: a group)";
        remove_rule: "conditions.removeRule" = "Remove this rule";
        remove_group: "conditions.removeGroup" = "Remove this group";
        unit: "conditions.unit" = "Unit";
        bool_rule: "conditions.boolRule" = "Is set";
    }
}

/// How a group's rules combine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Match {
    #[default]
    /// Every rule must hold.
    All,
    /// At least one rule must hold.
    Any,
    /// No rule may hold.
    None,
}

/// A value editor kind; the field's kind picks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConditionKind {
    /// Free text.
    Text,
    /// A number.
    Number,
    /// A date, edited with the date picker.
    Date,
    /// A yes or no value.
    Bool,
    /// One of the field's choices (an album, a label).
    Choice,
}

/// A rule's value.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub enum ConditionValue {
    #[default]
    /// No value.
    None,
    /// A text value.
    Text(String),
    /// A number.
    Number(f64),
    /// Two numbers for "is between".
    Range(f64, f64),
    /// A date (a day, or a month or year for a partial date), the date picker's value.
    Date(PickedDate),
    /// Two dates for "is between": each picker stops at the other, so the two stay in order.
    DateRange(PickedDate, PickedDate),
    /// "in the last N units".
    #[non_exhaustive]
    Last {
        /// The number of units.
        n: u32,
        /// The unit, as the app names it.
        unit: String,
    },
    /// A yes or no value.
    Bool(bool),
    /// One of the field's choices, by id.
    Choice(String),
}

impl ConditionValue {
    /// "In the last `n` `unit`s".
    pub fn last(n: u32, unit: impl Into<String>) -> Self {
        ConditionValue::Last { n, unit: unit.into() }
    }
}

/// One rule.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Condition {
    /// The field's id, as `ConditionFields::fields` lists it.
    pub field: String,
    /// The operator's id, as `ConditionFields::operators` lists it.
    pub operator: String,
    /// The value the operator compares against.
    pub value: ConditionValue,
}

impl Condition {
    /// A rule of `field` and `operator` comparing against `value`.
    pub fn new(field: impl Into<String>, operator: impl Into<String>, value: ConditionValue) -> Self {
        Condition { field: field.into(), operator: operator.into(), value }
    }
}

/// A group of rules and nested groups.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ConditionGroup {
    /// How the items combine.
    pub matching: Match,
    /// The rules and nested groups, in order.
    pub items: Vec<ConditionItem>,
}

impl ConditionGroup {
    /// A group that combines `items` as `matching` says.
    pub fn new(matching: Match, items: Vec<ConditionItem>) -> Self {
        ConditionGroup { matching, items }
    }
}

/// One entry of a group: a rule or a nested group.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ConditionItem {
    /// A single rule.
    Rule(Condition),
    /// A nested group.
    Group(ConditionGroup),
}

/// The fields a builder offers and their operators: the app's.
pub trait ConditionFields {
    /// `(id, label, group)` in menu order; a change of `group` between neighbours puts a rule between them.
    fn fields(&self) -> Vec<(String, String, Option<String>)>;
    /// The kind of value editor `field` takes.
    fn kind(&self, field: &str) -> ConditionKind;
    /// `(id, label)` of the operators for `field` ("is", "contains", "is between").
    fn operators(&self, field: &str) -> Vec<(String, String)>;
    /// A sensible value for a new `field` / `operator`.
    fn default_value(&self, field: &str, operator: &str) -> ConditionValue;
    /// The choices of a `Choice` field.
    fn choices(&self, _field: &str) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Why a rule can't apply, shown under it ("Enter a number").
    fn problem(&self, _rule: &Condition) -> Option<String> {
        None
    }
    /// Why a group can't apply, shown under it. `path` is the group's place: the indices from the top group, empty
    /// for the top group itself.
    fn group_problem(&self, _path: &[usize], _group: &ConditionGroup) -> Option<String> {
        None
    }
}

/// What the builder did this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct ConditionBuilderOutput {
    /// The builder's response.
    pub response: Response,
    /// Whether the group changed this pass.
    pub changed: bool,
}

impl craft_ui::contract::Outcome for ConditionBuilderOutput {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The rules editor: "Match [all] of the following rules:", then a row per rule: field menu, operator menu, the value
/// editor for the field's kind ("and" between two values of a range), − (remove) and + (add a rule after it;
/// Alt-click: a group). Nested groups indent with their own Match row and −, up to depth 3; "+ Rule" and "+ Group"
/// at the bottom. Changing a field resets the operator and the value to the defaults. The builder reads `group` and
/// writes the edited group back as one edit a pass. Every menu, field and button is a Tab stop in reading order.
#[derive(Pattern)]
pub struct ConditionBuilder<'a> {
    label: Label,
    group: Bind<'a, ConditionGroup>,
    fields: &'a dyn ConditionFields,
}

impl<'a> ConditionBuilder<'a> {
    /// A builder named `label` that edits `group`, offering `fields`.
    pub fn new(label: impl Into<Label>, group: Bind<'a, ConditionGroup>, fields: &'a dyn ConditionFields) -> Self {
        ConditionBuilder { label: label.into(), group, fields }
    }

    /// Shows the builder across the width offered.
    pub fn show(self, ui: &mut Ui) -> ConditionBuilderOutput {
        let ConditionBuilder { label, group, fields } = self;
        let words = Env::of(ui).words::<ConditionWords>();
        let held = RefCell::new(Held::new(group));
        let changed = Cell::new(false);
        let shown = Stack::new().label(label.clone()).show(ui, |ui, cx| {
            let Some(before) = held.borrow_mut().read(cx) else { return };
            let mut edited = before.clone();
            let rows = Rows { fields, words: &words };
            rows.group(ui, &mut edited, &mut Vec::new());
            if edited != before {
                held.borrow_mut().write(cx, &before, edited, label.as_str());
                changed.set(true);
            }
        });
        ConditionBuilderOutput { response: shown.output, changed: changed.get() }
    }
}

/// How deep groups nest: a group inside a group at this depth can't be added.
const MAX_DEPTH: usize = 3;

/// What every row of a pass reads.
struct Rows<'a> {
    fields: &'a dyn ConditionFields,
    words: &'a ConditionWords,
}

impl Rows<'_> {
    /// A new rule: the first field, its first operator and their default value.
    fn new_rule(&self) -> Condition {
        let Some((field, ..)) = self.fields.fields().into_iter().next() else { return Condition::default() };
        let operator = self.fields.operators(&field).into_iter().next().map(|(id, _)| id).unwrap_or_default();
        let value = self.fields.default_value(&field, &operator);
        Condition { field, operator, value }
    }

    fn new_group(&self) -> ConditionGroup {
        ConditionGroup { matching: Match::Any, items: vec![ConditionItem::Rule(self.new_rule())] }
    }

    /// Whether `value` is one the field's kind edits.
    fn fits(&self, field: &str, value: &ConditionValue) -> bool {
        use ConditionKind as K;
        use ConditionValue as V;
        matches!(
            (self.fields.kind(field), value),
            (_, V::None)
                | (K::Text, V::Text(_))
                | (K::Number, V::Number(_) | V::Range(..))
                | (K::Date, V::Date(_) | V::DateRange(..) | V::Last { .. })
                | (K::Bool, V::Bool(_))
                | (K::Choice, V::Choice(_))
        )
    }

    /// The group at `path`: its Match row, its rules and groups, and the buttons that add more.
    fn group(&self, ui: &mut Ui, group: &mut ConditionGroup, path: &mut Vec<usize>) {
        let words = self.words;
        let depth = path.len();
        let modes =
            [Entry::new(Match::All, words.all.clone()), Entry::new(Match::Any, words.any.clone()), Entry::new(Match::None, words.none.clone())];
        Row::new().show(ui, |ui, _| {
            Text::new(words.match_word.clone()).show(ui);
            Select::new(words.match_word.clone(), Bind::bound(&mut group.matching), &modes).id_salt((path.clone(), "match")).show(ui);
            Text::new(words.of_rules.clone()).show(ui);
        });
        let mut remove = None;
        let mut insert = None;
        for (at, item) in group.items.iter_mut().enumerate() {
            path.push(at);
            match item {
                ConditionItem::Group(nested) => {
                    Row::new().show(ui, |ui, _| {
                        let button = IconButton::new(IconSpec::svg(IconId::new("minus")))
                            .label(words.remove_group.clone())
                            .tooltip(words.remove_group.as_str().to_owned())
                            .id_salt((path.clone(), "remove"))
                            .show(ui);
                        if clicked(&button) {
                            remove = Some(at);
                        }
                        Stack::new().pad(Pad::Control).show(ui, |ui, _| self.group(ui, nested, path));
                    });
                }
                ConditionItem::Rule(rule) => self.rule(ui, rule, path, depth, (at, &mut remove, &mut insert)),
            }
            path.pop();
        }
        if let Some(at) = remove {
            group.items.remove(at);
        }
        if let Some((at, item)) = insert {
            group.items.insert(at.min(group.items.len()), item);
        }
        if let Some(problem) = self.fields.group_problem(path, group) {
            Notice::new(Tone::Warn, problem).layout(NoticeLayout::Inline).show(ui);
        }
        if group.items.is_empty() || depth == 0 {
            Row::new().show(ui, |ui, _| {
                if clicked(&Button::new(words.add_rule.clone()).id_salt((path.clone(), "add-rule")).show(ui)) {
                    group.items.push(ConditionItem::Rule(self.new_rule()));
                }
                if depth < MAX_DEPTH && clicked(&Button::new(words.add_group.clone()).id_salt((path.clone(), "add-group")).show(ui)) {
                    group.items.push(ConditionItem::Group(self.new_group()));
                }
            });
        }
    }

    /// One rule: field, operator, value editor and, at the end, − and +; the problem under it.
    fn rule(&self, ui: &mut Ui, rule: &mut Condition, path: &[usize], depth: usize, (at, remove, insert): Pending<'_>) {
        let words = self.words;
        Row::new().fill_width().align(RowAlign::SpaceBetween).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                self.pick_field(ui, rule, path);
                self.pick_operator(ui, rule, path);
                self.edit_value(ui, rule, path);
            });
            Row::new().show(ui, |ui, _| {
                let minus = IconButton::new(IconSpec::svg(IconId::new("minus")))
                    .label(words.remove_rule.clone())
                    .tooltip(words.remove_rule.as_str().to_owned())
                    .id_salt((path.to_vec(), "remove"))
                    .show(ui);
                if clicked(&minus) {
                    *remove = Some(at);
                }
                let plus = IconButton::new(IconSpec::svg(IconId::new("plus")))
                    .label(words.add_rule_tip.clone())
                    .tooltip(words.add_rule_tip.as_str().to_owned())
                    .id_salt((path.to_vec(), "add"))
                    .show(ui);
                if clicked(&plus) {
                    let group = pointer::frame(ui.ctx()).mods.alt && depth < MAX_DEPTH;
                    let item = if group { ConditionItem::Group(self.new_group()) } else { ConditionItem::Rule(self.new_rule()) };
                    *insert = Some((at + 1, item));
                }
            });
        });
        if let Some(problem) = self.fields.problem(rule) {
            Notice::new(Tone::Warn, problem).layout(NoticeLayout::Inline).show(ui);
        }
    }

    /// The field menu; a new field takes its first operator and the default value.
    fn pick_field(&self, ui: &mut Ui, rule: &mut Condition, path: &[usize]) {
        let mut entries = Vec::new();
        let mut heading: Option<String> = None;
        for (at, (id, label, group)) in self.fields.fields().into_iter().enumerate() {
            if at > 0 && group != heading {
                entries.push(Entry::separator());
            }
            heading = group;
            entries.push(Entry::new(id, label));
        }
        let mut field = rule.field.clone();
        Select::new(self.words.field.clone(), Bind::bound(&mut field), &entries).id_salt((path.to_vec(), "field")).show(ui);
        if field != rule.field {
            rule.operator = self.fields.operators(&field).into_iter().next().map(|(id, _)| id).unwrap_or_default();
            rule.value = self.fields.default_value(&field, &rule.operator);
            rule.field = field;
        }
    }

    /// The operator menu; an operator that wants another kind of value takes its default.
    fn pick_operator(&self, ui: &mut Ui, rule: &mut Condition, path: &[usize]) {
        let entries: Vec<Entry<String>> = self.fields.operators(&rule.field).into_iter().map(|(id, label)| Entry::new(id, label)).collect();
        let mut operator = rule.operator.clone();
        Select::new(self.words.operator.clone(), Bind::bound(&mut operator), &entries).id_salt((path.to_vec(), "operator")).show(ui);
        if operator != rule.operator {
            let value = self.fields.default_value(&rule.field, &operator);
            if std::mem::discriminant(&value) != std::mem::discriminant(&rule.value) {
                rule.value = value;
            }
            rule.operator = operator;
        }
    }

    /// A number field over `value`; the new number when the user changed it.
    fn number(&self, ui: &mut Ui, path: &[usize], part: &str, value: f64) -> Option<f64> {
        let mut number = Some(value);
        NumberField::new(self.words.value.clone(), Bind::bound(&mut number)).id_salt((path.to_vec(), part.to_owned())).show(ui);
        number.filter(|number| *number != value)
    }

    /// The value editor the field's kind and the operator's value call for.
    fn edit_value(&self, ui: &mut Ui, rule: &mut Condition, path: &[usize]) {
        let words = self.words;
        if !self.fits(&rule.field, &rule.value) {
            rule.value = self.fields.default_value(&rule.field, &rule.operator);
        }
        let salt = |part: &str| (path.to_vec(), part.to_owned());
        match rule.value.clone() {
            ConditionValue::None => {}
            ConditionValue::Text(text) => {
                let mut edited = text.clone();
                TextField::new(words.value.clone(), Bind::bound(&mut edited)).id_salt(salt("value")).show(ui);
                if edited != text {
                    rule.value = ConditionValue::Text(edited);
                }
            }
            ConditionValue::Number(value) => {
                if let Some(number) = self.number(ui, path, "value", value) {
                    rule.value = ConditionValue::Number(number);
                }
            }
            ConditionValue::Range(low, high) => {
                let new_low = self.number(ui, path, "low", low);
                Text::new(words.and.clone()).show(ui);
                let new_high = self.number(ui, path, "high", high);
                if new_low.is_some() || new_high.is_some() {
                    rule.value = ConditionValue::Range(new_low.unwrap_or(low), new_high.unwrap_or(high));
                }
            }
            ConditionValue::Date(date) => {
                let mut picked = Some(date);
                DatePicker::new(words.value.clone(), Bind::bound(&mut picked)).id_salt(salt("value")).show(ui);
                if let Some(picked) = picked.filter(|picked| *picked != date) {
                    rule.value = ConditionValue::Date(picked);
                }
            }
            ConditionValue::DateRange(low, high) => {
                // Each picker stops at the other date.
                let (mut low_picked, mut high_picked) = (Some(low), Some(high));
                DatePicker::new(words.value.clone(), Bind::bound(&mut low_picked)).max(high).id_salt(salt("low")).show(ui);
                Text::new(words.and.clone()).show(ui);
                DatePicker::new(words.value.clone(), Bind::bound(&mut high_picked)).min(low).id_salt(salt("high")).show(ui);
                let (new_low, new_high) = (low_picked.unwrap_or(low), high_picked.unwrap_or(high));
                if (new_low, new_high) != (low, high) {
                    rule.value = ConditionValue::DateRange(new_low, new_high);
                }
            }
            ConditionValue::Last { n, unit } => {
                let new_count = self.number(ui, path, "count", f64::from(n)).map(|count| count.max(0.0).round() as u32);
                let units = self.fields.choices(&rule.field);
                let mut edited = unit.clone();
                if units.is_empty() {
                    TextField::new(words.unit.clone(), Bind::bound(&mut edited)).id_salt(salt("unit")).show(ui);
                } else {
                    let entries: Vec<Entry<String>> = units.into_iter().map(|(id, label)| Entry::new(id, label)).collect();
                    Select::new(words.unit.clone(), Bind::bound(&mut edited), &entries).id_salt(salt("unit")).show(ui);
                }
                if new_count.is_some() || edited != unit {
                    rule.value = ConditionValue::Last { n: new_count.unwrap_or(n), unit: edited };
                }
            }
            ConditionValue::Bool(on) => {
                let label = self
                    .fields
                    .fields()
                    .into_iter()
                    .find(|(id, ..)| *id == rule.field)
                    .map_or_else(|| words.bool_rule.clone(), |(_, label, _)| Label::from(label));
                let mut checked = on;
                Checkbox::new(Bind::bound(&mut checked)).label(label).id_salt(salt("value")).show(ui);
                if checked != on {
                    rule.value = ConditionValue::Bool(checked);
                }
            }
            ConditionValue::Choice(choice) => {
                let entries: Vec<Entry<String>> = self.fields.choices(&rule.field).into_iter().map(|(id, label)| Entry::new(id, label)).collect();
                let mut picked = choice.clone();
                Select::new(words.value.clone(), Bind::bound(&mut picked), &entries).id_salt(salt("value")).show(ui);
                if picked != choice {
                    rule.value = ConditionValue::Choice(picked);
                }
            }
        }
    }
}

type Pending<'a> = (usize, &'a mut Option<usize>, &'a mut Option<(usize, ConditionItem)>);

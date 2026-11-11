//! Stories of `ConditionBuilder`: a nested group, a date-range rule and a group problem.

use super::{Condition, ConditionBuilder, ConditionFields, ConditionGroup, ConditionItem, ConditionKind, ConditionValue, Match};
use craft_ui::contract::Bind;
use craft_ui::pickers::PickedDate;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// Three fields: a name, a rating and a capture date.
struct Photos;

impl ConditionFields for Photos {
    fn fields(&self) -> Vec<(String, String, Option<String>)> {
        vec![
            ("name".into(), "File name".into(), Some("File".into())),
            ("rating".into(), "Rating".into(), Some("Metadata".into())),
            ("date".into(), "Capture date".into(), Some("Metadata".into())),
        ]
    }

    fn kind(&self, field: &str) -> ConditionKind {
        match field {
            "rating" => ConditionKind::Number,
            "date" => ConditionKind::Date,
            _ => ConditionKind::Text,
        }
    }

    fn operators(&self, field: &str) -> Vec<(String, String)> {
        match self.kind(field) {
            ConditionKind::Text => vec![("contains".into(), "contains".into()), ("is".into(), "is".into())],
            _ => vec![("is".into(), "is".into()), ("between".into(), "is between".into())],
        }
    }

    fn default_value(&self, field: &str, operator: &str) -> ConditionValue {
        match (self.kind(field), operator) {
            (ConditionKind::Number, "between") => ConditionValue::Range(1.0, 5.0),
            (ConditionKind::Number, _) => ConditionValue::Number(3.0),
            (ConditionKind::Date, "between") => ConditionValue::DateRange(PickedDate::day(2026, 1, 1), PickedDate::day(2026, 12, 31)),
            (ConditionKind::Date, _) => ConditionValue::Date(PickedDate::day(2026, 6, 1)),
            _ => ConditionValue::Text(String::new()),
        }
    }

    fn problem(&self, rule: &Condition) -> Option<String> {
        matches!(&rule.value, ConditionValue::Text(text) if text.is_empty()).then(|| "Enter a value".to_owned())
    }

    fn group_problem(&self, path: &[usize], group: &ConditionGroup) -> Option<String> {
        (!path.is_empty() && group.items.len() < 2).then(|| "A group needs two rules".to_owned())
    }
}

fn rules() -> ConditionGroup {
    let rule = |field: &str, operator: &str, value| ConditionItem::Rule(Condition::new(field, operator, value));
    ConditionGroup::new(
        Match::All,
        vec![
            rule("name", "contains", ConditionValue::Text("beach".into())),
            rule("date", "between", ConditionValue::DateRange(PickedDate::day(2026, 1, 1), PickedDate::day(2026, 8, 31))),
            ConditionItem::Group(ConditionGroup::new(Match::Any, vec![rule("rating", "is", ConditionValue::Number(4.0))])),
        ],
    )
}

#[story(ConditionBuilder<'static>, covers = [Default], drive = Drive::Never)]
fn condition_builder(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let group = data.value("group", rules());
    ConditionBuilder::new("Rules", Bind::bound(group), &Photos).show(ui).into()
}

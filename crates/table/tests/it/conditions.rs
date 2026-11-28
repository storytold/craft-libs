//! `ConditionBuilder` in the joined layout: joins after the first rule, and what an edit changes.

use std::cell::RefCell;
use std::rc::Rc;

use craft_table::ui::{
    Condition, ConditionBuilder, ConditionFields, ConditionGroup, ConditionItem, ConditionKind, ConditionLayout, ConditionValue, Join, Match,
};
use craft_ui::contract::Bind;
use craft_ui::testing::{Fixture, trial};

/// One text field with one operator.
struct Names;

impl ConditionFields for Names {
    fn fields(&self) -> Vec<(String, String, Option<String>)> {
        vec![("name".into(), "File name".into(), None)]
    }

    fn kind(&self, _field: &str) -> ConditionKind {
        ConditionKind::Text
    }

    fn operators(&self, _field: &str) -> Vec<(String, String)> {
        vec![("is".into(), "is".into())]
    }

    fn default_value(&self, _field: &str, _operator: &str) -> ConditionValue {
        ConditionValue::Text("x".into())
    }
}

fn rule(text: &str) -> ConditionItem {
    ConditionItem::Rule(Condition::new("name", "is", ConditionValue::Text(text.into())))
}

fn rule_texts(group: &ConditionGroup) -> Vec<String> {
    group
        .items
        .iter()
        .filter_map(|item| match item {
            ConditionItem::Rule(Condition { value: ConditionValue::Text(text), .. }) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn joins_shown(fixture: &Fixture) -> usize {
    let tree = fixture.tree();
    tree.keys().into_iter().filter(|key| tree.name_of(*key) == Some("Join")).count()
}

/// The first row shows no join, also after the first rule is removed; picking Or sets that rule's `join`; a nested
/// group the input carries is kept.
#[trial]
fn joined_rules_show_a_join_after_the_first_and_removing_the_first_drops_it() {
    let nested = ConditionItem::Group(ConditionGroup::new(Match::Any, vec![rule("deep")]));
    let group = Rc::new(RefCell::new(ConditionGroup::new(Match::All, vec![rule("a"), rule("b"), nested.clone(), rule("c")])));
    let shared = group.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut group = shared.borrow_mut();
        ConditionBuilder::new("Rules", Bind::bound(&mut *group), &Names).layout(ConditionLayout::Joined).show(ui);
    });
    fixture.settle();
    let rules = rule_texts(&group.borrow()).len();
    assert_eq!(joins_shown(&fixture), rules - 1, "every rule after the first shows a join, the first none");
    assert!(group.borrow().items.contains(&nested), "the nested group is kept");

    let join = fixture.by_label("Join").expect("a join select");
    fixture.click(&join).settle();
    let or = fixture.by_label("Or").expect("the Or entry");
    fixture.click(&or).settle();
    let joins: Vec<Join> =
        group.borrow().items.iter().filter_map(|item| if let ConditionItem::Rule(rule) = item { Some(rule.join) } else { None }).collect();
    assert_eq!(joins.iter().filter(|join| **join == Join::Or).count(), 1, "one rule took Or");
    assert_eq!(joins[0], Join::And, "the first rule was not touched");

    let remove = fixture.by_label("Remove this rule").expect("a remove button");
    fixture.click(&remove).settle();
    assert_eq!(rule_texts(&group.borrow()), ["b", "c"], "the first rule is gone");
    assert!(group.borrow().items.contains(&nested), "the nested group is still kept");
    let rules = rule_texts(&group.borrow()).len();
    assert_eq!(joins_shown(&fixture), rules - 1, "the new first row shows no join");
}

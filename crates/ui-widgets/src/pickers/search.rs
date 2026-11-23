//! The search field at the top of a picker's popup (Select, TreePicker): it takes the key focus once the popup is
//! open, hands the list keys to the popup's list, and starts empty each time the popup opens.

use super::words::PickerWords;
use crate::contract::overlay::Popup;
use crate::contract::{Bind, BodyCx, Component, Cx, PopupHandle, Remember as _, Show as _, WordSetCx as _};
use crate::runtime::env::Label;
use crate::runtime::focus::{self, FocusTarget};
use crate::text::TextField;
use egui::{Id, Ui};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};

/// The search text of an open popup, and whether its field has taken the key focus yet.
#[derive(Clone, Default)]
struct Query {
    text: String,
    focused: bool,
}

/// Shows the search field of `popup`, named after its picker `owner`, driving the popup's list `list`, and returns
/// the query. The dropdown leaves the key focus on the picker; the field takes it once the layer has recorded the
/// picker as its opener, so typing reaches the field.
pub(super) fn show<C: Component>(ui: &mut Ui, body: &mut BodyCx<'_>, popup: PopupHandle, cx: &Cx<'_, C>, owner: &Label, list: Id) -> String {
    let words = cx.words::<PickerWords>();
    let opening = Popup::opening(popup, cx);
    let typed = body.memory::<Query>();
    if opening {
        *typed = Query::default();
    }
    let field = TextField::search(name(&words, owner), Bind::bound(&mut typed.text)).drives(list).show(ui);
    let query = typed.text.clone();
    let taken = &mut body.memory::<Query>().focused;
    if field.response.has_focus() {
        *taken = true;
    } else if !*taken && !opening {
        focus::request(ui.ctx(), FocusTarget::Id(field.response.id));
    }
    query
}

/// A stamp of the query, for a list whose rows the query decides: the lead starts over when it changes.
pub(super) fn stamp(query: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    query.trim().hash(&mut hasher);
    hasher.finish()
}

/// The field's accessible name: the picker's own name leads, so the field reads as part of it.
fn name(words: &PickerWords, owner: &Label) -> Label {
    if owner.as_str().is_empty() { words.search.clone() } else { Label::from(format!("{} {}", owner.as_str(), words.search.as_str())) }
}

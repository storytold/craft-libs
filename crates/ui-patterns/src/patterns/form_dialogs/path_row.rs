//! `PathRow`: a path typed in a text field, with a Browse button that asks the host to pick one.

#[cfg(feature = "testing")]
mod stories;

use crate::buttons::Button;
use crate::contract::{Bind, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, Sizing, clicked};
use crate::layout_components::Row;
use crate::runtime::env::{Env, Label};
use crate::text::TextField;
use craft_behavior::EditEvents;
use egui::{Response, Ui};

craft_ui::words! {
    /// The words of the path row.
    pub struct PathRowWords {
        browse: "pathRow.browse" = "Browse…";
    }
}

/// What a path row reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PathRowOutput {
    /// The row's response.
    pub response: Response,
    /// The text field's edits of the path in this pass; a given value is applied by the caller.
    pub edit: EditEvents<String>,
    /// Browse was pressed this pass: the host asks for a path by its own route and writes the answer into the value.
    pub browse: bool,
}

impl Outcome for PathRowOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A text field with the path, taking the row's room, and a Browse button after it, for a row whose enclosing `Field`
/// shows the label. Typing edits the bound path; the binding's check shows under the field (a file that is not there).
/// A press on Browse is reported, and the host files its pick request: a `PathRequest` in a form dialog, or a pick from
/// the host's file service. Tab order: the field, then Browse.
#[must_use = "show the row with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct PathRow<'a> {
    name: Label,
    value: Bind<'a, String>,
    /// The button's text; the words' "Browse…" without one.
    #[prop(into)]
    button: Option<Label>,
    /// Shown in the empty field.
    #[prop(into)]
    placeholder: Option<Label>,
}

impl<'a> PathRow<'a> {
    /// A row over the path `value`, its field named `name`.
    pub fn new(name: impl Into<Label>, value: Bind<'a, String>) -> Self {
        PathRow { name: name.into(), value, button: None, placeholder: None }
    }

    /// Shows the row.
    pub fn show(self, ui: &mut Ui) -> PathRowOutput {
        let PathRow { name, mut value, button, placeholder } = self;
        let button = button.unwrap_or_else(|| Env::of(ui).words::<PathRowWords>().browse.clone());
        let mut edit = EditEvents::new();
        let mut browse = false;
        let shown = Row::new().show(ui, |ui, _| {
            let mut field = TextField::new(name.clone(), value.reborrowed()).sizing(Sizing::FillWidth).id_salt("path");
            if let Some(placeholder) = placeholder.clone() {
                field = field.placeholder(placeholder);
            }
            edit = field.show(ui).events;
            browse = clicked(&Button::new(button.clone()).id_salt("browse").show(ui));
        });
        PathRowOutput { response: shown.output, edit, browse }
    }
}

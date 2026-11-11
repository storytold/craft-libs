//! `OutputPathRow`: where a render or an export goes, as a template list and the path in a text field with Browse.

use crate::buttons::Button;
use crate::contract::{Bind, EditResponse, Entry, NoValue, Outcome, Props as _, Show as _, ShowContainer as _, clicked};
use crate::runtime::requests::{PathKind, PathRequest};

use crate::form::Field;
use crate::layout_components::Row;
use crate::pickers::Select;
use crate::runtime::env::{Env, Label};

use crate::text::TextField;
use craft_behavior::EditEvents;
use egui::{Response, Ui};

craft_ui::words! {
    /// The words of the output path row.
    pub struct OutputPathWords {
        output_to: "outputPath.outputTo" = "Output To";
        templates: "outputPath.templates" = "Template";
        browse: "outputPath.browse" = "Choose File…";
    }
}

/// What an output path row reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct OutputPathOutput {
    /// The row's response.
    pub response: Response,
    /// The edits of the path or template in this pass, in order: a template the user picked, then the text field's
    /// edits. A given value is applied by the caller.
    pub edit: EditEvents<String>,
    /// The Browse button pressed this pass: the host opens its file dialog for the request and writes the answer into
    /// the bound value.
    pub browse: Option<PathRequest>,
}

impl Outcome for OutputPathOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// "Output To", a list of name templates (`[Comp Name].[fileExtension]`) and the chosen path in a text field with a
/// Browse button, for render queues and export dialogs.
///
/// - Picking a template sets the value; typing edits it; Browse reports a [`PathRequest`] in the output, which the
///   host answers by writing the picked path into the bound value.
/// - Whether an output may be written (duplicates, protected folders) is the app's check.
#[must_use = "show the row with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct OutputPathRow<'a> {
    value: Bind<'a, String>,
    /// The templates to pick from, as `(template, label)`.
    #[prop]
    templates: Vec<(String, Label)>,
    /// What the Browse button picks; a file to write by default.
    #[prop]
    kind: PathKind,
    /// The key of the Browse request.
    #[prop(into)]
    key: String,
    /// The row's label; the words' "Output To" without one.
    #[prop(into)]
    label: Option<Label>,
}

impl<'a> OutputPathRow<'a> {
    /// A row over the path or template `value`.
    pub fn new(value: Bind<'a, String>) -> Self {
        OutputPathRow { value, templates: Vec::new(), kind: PathKind::SaveFile, key: "output".to_owned(), label: None }
    }

    /// Shows the row.
    pub fn show(self, ui: &mut Ui) -> OutputPathOutput {
        let OutputPathRow { mut value, templates, kind, key, label } = self;
        let words = Env::of(ui).words::<OutputPathWords>();
        let name = label.unwrap_or_else(|| words.output_to.clone());
        let entries: Vec<Entry<String>> = templates.iter().map(|(template, label)| Entry::new(template.clone(), label.clone())).collect();
        let mut edit = EditEvents::new();
        let mut browse = None;
        let shown = Field::new(name.clone()).id_salt(key.as_str()).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                edit = EditEvents::new();
                if !entries.is_empty() {
                    let picked: EditResponse<String> =
                        Select::new(words.templates.clone(), value.reborrowed(), &entries).id_salt("templates").show(ui);
                    edit.extend(picked.events);
                }
                let typed = TextField::new(name.clone(), value.reborrowed()).id_salt("path").show(ui);
                edit.extend(typed.events);
                if clicked(&Button::new(words.browse.clone()).id_salt("browse").show(ui)) {
                    browse = Some(PathRequest::new(key.as_str(), kind.clone()));
                }
            });
        });
        OutputPathOutput { response: shown.output, edit, browse }
    }
}

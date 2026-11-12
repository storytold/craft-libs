//! `OutputPathRow`: where a render or an export goes, as a template list and the resulting file name as a link.

use crate::buttons::Link;
use crate::contract::{Bind, EditResponse, Entry, NoValue, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _, Sizing, clicked};
use crate::form::Field;
use crate::layout_components::Row;
use crate::pickers::Select;
use crate::primitives::Wrap;
use crate::runtime::env::{Env, Label};
use crate::runtime::focus::{self, FocusTarget};
use crate::runtime::requests::{PathKind, PathRequest};
use crate::text::TextField;
use craft_behavior::{EditEvent, EditEvents, Mixed};
use egui::{Response, Ui};
use std::sync::Arc;

craft_ui::words! {
    /// The words of the output path row.
    pub struct OutputPathWords {
        output_to: "outputPath.outputTo" = "Output To";
        templates: "outputPath.templates" = "Template";
        browse: "outputPath.browse" = "Choose File…";
        edit_template: "outputPath.editTemplate" = "Alt-click to edit the template";
    }
}

/// What an output path row reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct OutputPathOutput {
    /// The row's response.
    pub response: Response,
    /// The edits of the path or template in this pass, in order: a template the user picked, then a template typed in
    /// place. A given value is applied by the caller.
    pub edit: EditEvents<String>,
    /// The file name was clicked this pass: the host opens its file dialog for the request and writes the answer into
    /// the bound value.
    pub browse: Option<PathRequest>,
}

impl Outcome for OutputPathOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The in-place template edit: it asks for the focus, then runs until the field loses it.
#[derive(Clone, Debug, Default)]
enum Editing {
    #[default]
    Off,
    /// The field was just shown and asks for the focus.
    Pending(String),
    /// The field has had the focus; the text typed so far.
    Active(String),
}

/// "Output To", a list of name templates (`[Comp Name].[fileExtension]`) and the resulting file name as a link, for
/// render queues and export dialogs (After Effects' Render Queue).
///
/// - Picking a template sets the value. A click on the file name ("Choose File…" while it is empty) reports a
///   [`PathRequest`] in the output, which the host answers by writing the picked path into the bound value.
/// - Alt-click on the file name edits the template in place: Enter commits a text that is not blank, Escape or moving
///   the focus away leaves the value as it was.
/// - Whether an output may be written (duplicates, protected folders) is the app's check.
#[must_use = "show the row with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct OutputPathRow<'a> {
    value: Bind<'a, String>,
    /// The templates to pick from, as `(template, label)`.
    #[prop]
    templates: Vec<(String, Label)>,
    /// What the file name link picks; a file to write by default.
    #[prop]
    kind: PathKind,
    /// The key of the browse request.
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
        let shown = Field::new(name.clone()).id_salt(key.as_str()).show(ui, |ui, body| {
            edit = EditEvents::new();
            let current = value.reborrow().read(body).and_then(|held| held.same().cloned()).unwrap_or_default();
            let mut editing = body.memory::<Editing>().clone();
            let mut typed_template = None;
            Row::new().show(ui, |ui, _| {
                if !entries.is_empty() {
                    let picked: EditResponse<String> =
                        Select::new(words.templates.clone(), value.reborrowed(), &entries).id_salt("templates").sizing(Sizing::Content).show(ui);
                    edit.extend(picked.events);
                }
                editing = match std::mem::take(&mut editing) {
                    Editing::Off => {
                        let shown = if current.is_empty() { words.browse.clone() } else { Label::from(current.clone()) };
                        let link = Link::new(shown).wrap(Wrap::ElidePath).tooltip(words.edit_template.clone()).id_salt("file").show(ui);
                        if !clicked(&link) {
                            Editing::Off
                        } else if link.mods.alt {
                            Editing::Pending(current.clone())
                        } else {
                            browse = Some(PathRequest::new(key.as_str(), kind.clone()));
                            Editing::Off
                        }
                    }
                    Editing::Pending(mut text) => template_field(ui, &name, &mut text, true, &mut typed_template),
                    Editing::Active(mut text) => template_field(ui, &name, &mut text, false, &mut typed_template),
                };
            });
            if let Some(after) = typed_template {
                let event = EditEvent::set(Mixed::Same(current), after, Arc::from(name.as_str()), None);
                if value.reborrow().write(body, &event) {
                    edit.push(event);
                }
            }
            *body.memory::<Editing>() = editing;
        });
        OutputPathOutput { response: shown.output, edit, browse }
    }
}

/// The template field over `text`: it asks for the focus while `pending`, and puts a committed template that is not
/// blank in `committed`. Returns the edit's next state.
fn template_field(ui: &mut Ui, name: &Label, text: &mut String, pending: bool, committed: &mut Option<String>) -> Editing {
    let typed = TextField::new(name.clone(), Bind::bound(text)).submits().id_salt("template").show(ui);
    let focused = typed.response.has_focus();
    if pending && !focused {
        focus::request(ui.ctx(), FocusTarget::Id(typed.response.id));
    }
    if typed.submitted {
        if !text.trim().is_empty() {
            *committed = Some(text.trim().to_owned());
        }
        Editing::Off
    } else if typed.escaped || (!pending && !focused) {
        Editing::Off
    } else if focused {
        Editing::Active(std::mem::take(text))
    } else {
        Editing::Pending(std::mem::take(text))
    }
}

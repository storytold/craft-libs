//! [`PathField`]: a labelled path with a Browse button that opens a picker through `host.files()`.

use craft_behavior::{EditEvent, Mixed};
use craft_host::target::Caps;
use craft_host::{PickId, PickRequest};
use craft_ui::contract::{Bind, Held, NoValue, Outcome, Remember as _, ShowContainer as _, Validity};
use craft_ui::form::Field;
use craft_ui::patterns::form_dialogs::PathRow;
use craft_ui::runtime::env::{Env, Label, Message};
use egui::{Response, Ui};
use std::path::Path;

craft_ui::words! {
    /// The words of the path field.
    pub struct PathFieldWords {
        not_found: "pathField.notFound" = "The file was not found.";
    }
}

/// The pick the field is waiting for, kept in the field's container while it is shown.
#[derive(Clone, Copy, Default)]
struct Ticket(Option<PickId>);

/// What the path field reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct PathFieldOutput {
    /// The field's response.
    pub response: Response,
    /// The edit of this pass: the typed path (a commit when the user typed a path and left the field or pressed
    /// Enter) or the picker's answer (a set).
    pub edit: Option<EditEvent<String>>,
    /// Browse was pressed and a picker was queued; the field keeps the ticket and fills itself when the answer
    /// arrives.
    pub browsing: Option<PickId>,
}

impl Outcome for PathFieldOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A label, then the path row (a text field with the path and a Browse button) over the bound path. Typing commits on
/// Enter or focus loss; Browse queues `request` through `host.files()` and the answer fills the field on a later pass
/// (the field matches tickets, so several fields can browse in turn). Tab order: field, then Browse. With `must_exist`
/// set, a path that does not exist shows the not-found message under the field.
#[must_use = "show the field with `show`"]
#[derive(craft_ui::contract::Pattern, craft_ui::contract::Props)]
pub struct PathField<'a> {
    label: Label,
    value: Bind<'a, String>,
    request: PickRequest,
    /// Shows the not-found message for a path that does not exist (Open fields; not Save).
    #[prop]
    must_exist: bool,
}

impl<'a> PathField<'a> {
    /// A field named `label` over the bound path. `request` is what Browse asks the host to pick.
    pub fn new(label: impl Into<Label>, value: Bind<'a, String>, request: PickRequest) -> Self {
        PathField { label: label.into(), value, request, must_exist: false }
    }

    /// Shows the field. Reads the host's file service from the env: takes the answer of the ticket this field kept;
    /// other tickets' answers stay in the service.
    pub fn show(self, ui: &mut Ui) -> PathFieldOutput {
        let PathField { label, value, request, must_exist } = self;
        let words = Env::of(ui).words::<PathFieldWords>();
        let mut held = Held::new(value);
        let mut edit = None;
        let mut browsing = None;
        let shown = Field::new(label.clone()).show(ui, |ui, cx| {
            let before = held.read(cx).unwrap_or_default();
            let host = cx.env.host().cloned();
            let looks = must_exist && cx.env.shell().caps.contains(Caps::FILE_SYSTEM);
            let not_found = words.not_found.clone();
            let check = move |path: &String| {
                if looks && path_is_missing(path) { Validity::Warn(Message::new(not_found.clone())) } else { Validity::Ok }
            };
            let mut answered = None;
            if let Some(ticket) = cx.memory::<Ticket>().0
                && let Some(picked) = cx.take_pick_answer(ticket)
            {
                cx.memory::<Ticket>().0 = None;
                answered = picked.first_path().map(|path| path.to_string_lossy().into_owned());
            }
            let row = PathRow::new(label.clone(), Bind::given(before.clone()).check(check)).show(ui);
            if row.browse
                && let Some(host) = &host
            {
                let id = host.files().pick(request.clone());
                cx.memory::<Ticket>().0 = Some(id);
                browsing = Some(id);
            }
            let typed = row
                .edit
                .into_iter()
                .filter_map(|event| {
                    let after = match &event {
                        EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } => after.clone(),
                        _ => return None,
                    };
                    Some((after, event))
                })
                .next_back();
            let landed = typed.or_else(|| {
                answered.map(|path| {
                    let event = EditEvent::set(Mixed::Same(before.clone()), path.clone(), label.as_str().into(), None);
                    (path, event)
                })
            });
            if let Some((after, event)) = landed {
                held.write(cx, &before, after, label.as_str());
                edit = Some(event);
            }
        });
        PathFieldOutput { response: shown.output, edit, browsing }
    }
}

/// Whether a non-empty `path` names nothing on disk. An empty field is not flagged.
fn path_is_missing(path: &str) -> bool {
    !path.trim().is_empty() && !Path::new(path.trim()).exists()
}

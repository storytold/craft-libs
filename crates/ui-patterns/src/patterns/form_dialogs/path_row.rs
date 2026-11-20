//! `PathRow`: a path typed in a text field, with a Browse button that asks the host to pick one.

#[cfg(feature = "suite")]
pub(crate) mod stories;

use crate::buttons::Button;
use crate::contract::{Bind, NoValue, Outcome, Props as _, Remember as _, Show as _, ShowContainer as _, Sizing, TooltipSpec, clicked};
use crate::layout_components::Row;
use crate::runtime::env::{Env, Label};
use crate::text::TextField;
use craft_behavior::EditEvents;
use craft_host::{PickId, PickRequest};
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
    /// With [`PathRow::pick`] the row has asked the host's file service itself.
    pub browse: bool,
    /// With [`PathRow::pick`]: Browse filed this pick through the host's file service this pass.
    pub browsing: Option<PickId>,
    /// With [`PathRow::pick`]: the path the host's file service answered, on the pass the answer arrived. The caller
    /// writes it into the value as a set.
    pub picked: Option<String>,
}

impl Outcome for PathRowOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A text field with the path, taking the row's room, and a Browse button after it, for a row whose enclosing `Field`
/// shows the label. Typing edits the bound path; the binding's check shows under the field (a file that is not there).
/// A press on Browse is reported, and the host files its pick request: a `PathRequest` in a form dialog. With
/// [`pick`](Self::pick) the row files the pick through the host's file service itself, keeps the ticket while it is shown
/// and reports the answer in [`PathRowOutput::picked`] (rows match tickets, so several can browse in turn). Tab order:
/// the field, then Browse.
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
    /// What Browse asks the host's file service to pick; without it the caller files the request.
    #[prop(into)]
    pick: Option<PickRequest>,
    /// Browse is disabled and this tells why.
    #[prop(into)]
    browse_reason: Option<Label>,
}

/// The pick the row is waiting for, kept in the row's container while it is shown.
#[derive(Clone, Copy, Default)]
struct Ticket(Option<PickId>);

impl<'a> PathRow<'a> {
    /// A row over the path `value`, its field named `name`.
    pub fn new(name: impl Into<Label>, value: Bind<'a, String>) -> Self {
        PathRow { name: name.into(), value, button: None, placeholder: None, pick: None, browse_reason: None }
    }

    /// Shows the row.
    pub fn show(self, ui: &mut Ui) -> PathRowOutput {
        let PathRow { name, mut value, button, placeholder, pick, browse_reason } = self;
        let button = button.unwrap_or_else(|| Env::of(ui).words::<PathRowWords>().browse.clone());
        let mut edit = EditEvents::new();
        let mut browse = false;
        let mut browsing = None;
        let mut picked = None;
        let shown = Row::new().show(ui, |ui, cx| {
            if pick.is_some()
                && let Some(ticket) = cx.memory::<Ticket>().0
                && let Some(answer) = cx.take_pick_answer(ticket)
            {
                cx.memory::<Ticket>().0 = None;
                picked = answer.first_path().map(|path| path.to_string_lossy().into_owned());
            }
            let mut field = TextField::new(name.clone(), value.reborrowed()).limit(None).sizing(Sizing::FillWidth).id_salt("path");
            if let Some(placeholder) = placeholder.clone() {
                field = field.placeholder(placeholder);
            }
            edit = field.show(ui).events;
            let mut browse_button = Button::new(button.clone()).id_salt("browse");
            // A disabled button with a reason stays focusable, so the reason is reachable without a pointer.
            if let Some(reason) = &browse_reason {
                browse_button = browse_button.tooltip(TooltipSpec::new(button.clone()).disabled_text(reason.clone())).enabled(false);
            }
            browse = clicked(&browse_button.show(ui));
            if browse
                && let Some(request) = &pick
                && let Some(host) = cx.env.host().cloned()
            {
                let id = host.files().pick(request.clone());
                cx.memory::<Ticket>().0 = Some(id);
                browsing = Some(id);
            }
        });
        PathRowOutput { response: shown.output, edit, browse, browsing, picked }
    }
}

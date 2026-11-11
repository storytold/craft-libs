//! The join field: enter a session code or an invite link.

use craft_theme::variant::{Tone, Variant};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, NoValue, Outcome, Pattern, Props, Show as _, ShowContainer as _, clicked};
use craft_ui::display::{Spinner, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::notices::{Notice, NoticeAction, NoticeLayout};
use craft_ui::runtime::env::{Env, Label};
use craft_ui::text::TextField;
use egui::{Response, Ui};

use crate::collab::{CollabRequest, CollabTransport, JoinState};

#[cfg(feature = "screen-testing")]
mod stories;

craft_ui::words! {
    /// The field's words, English by default.
    #[non_exhaustive]
    pub struct JoinWords {
        hint: "join.hint" = "Code or invite link";
        name: "join.name" = "Join code or invite link";
        join: "join.join" = "Join";
        joining: "join.joining" = "Joining…";
        retry: "join.retry" = "Try Again";
        cancel: "join.cancel" = "Cancel";
    }
}

/// One field for the share popover, the home screen and the Join dialog: a masked text field (its text is not read
/// out: it may hold a link's secret) and Join, enabled when the field is not blank and no join is pending; Enter
/// joins. On join the field clears. While joining it shows Joining… with Cancel; a failure shows its reason with Try
/// Again, which reports [`CollabRequest::RetryJoin`] (the app kept what the join started with).
#[derive(Pattern, Props)]
pub struct JoinField<'a> {
    label: Label,
    transport: &'a dyn CollabTransport,
    typed: &'a mut String,
}

impl<'a> JoinField<'a> {
    /// A field that edits `typed` and joins through `transport`.
    pub fn new(transport: &'a dyn CollabTransport, typed: &'a mut String) -> Self {
        JoinField { label: Label::Empty, transport, typed }
    }

    /// Shows the field. Join, Cancel and Try Again come back as [`CollabRequest`]s; the field reads the join's
    /// progress from [`CollabTransport::join_state`].
    pub fn show(self, ui: &mut Ui) -> JoinFieldOutput {
        let JoinField { label, transport, typed } = self;
        let words = Env::of(ui).words::<JoinWords>();
        let state = transport.join_state();
        let mut requests = Vec::new();
        let shown = Stack::new().label(label).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                let field = TextField::new(words.name.clone(), Bind::bound(&mut *typed)).placeholder(words.hint.clone()).masked().show(ui);
                let filled = !typed.trim().is_empty() && !state.pending();
                let pressed = clicked(&Button::new(words.join.clone()).variant(Variant::PRIMARY).enabled(filled).show(ui));
                if filled && (pressed || field.submitted) {
                    requests.push(CollabRequest::Join(typed.trim().to_owned()));
                    typed.clear();
                }
            });
            match &state {
                JoinState::Joining | JoinState::Reconnecting { .. } => {
                    Row::new().show(ui, |ui, _| {
                        Spinner::new().show(ui);
                        Text::new(words.joining.clone()).show(ui);
                        if clicked(&Button::new(words.cancel.clone()).show(ui)) {
                            requests.push(CollabRequest::CancelJoin);
                        }
                    });
                }
                JoinState::Failed(reason) => {
                    let notice =
                        Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).action(NoticeAction::new(words.retry.clone())).show(ui);
                    if notice.action == Some(0) {
                        requests.push(CollabRequest::RetryJoin);
                    }
                }
                JoinState::Idle | JoinState::Joined => {}
            }
        });
        JoinFieldOutput { response: shown.output, requests }
    }
}

/// What the join field did this pass.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct JoinFieldOutput {
    /// The field's response.
    pub response: Response,
    /// The requests the user made: join, cancel or retry.
    pub requests: Vec<CollabRequest>,
}

impl Outcome for JoinFieldOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

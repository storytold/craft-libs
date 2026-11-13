//! `Alert` (24.4): a notice with one OK answer, a configuration of `Prompt`.

use super::host::{DialogResult, StackDialog};
use super::prompt::{Prompt, Reply, buttons};
use super::stack::{DialogId, DialogStack, dialog_id};
use crate::contract::{Bind, NoValue, Outcome};
use crate::runtime::env::{Env, Label};
use crate::runtime::overlay::CloseReason;
use crate::runtime::words::CommonWords;
use crate::sections::DialogRole;
use craft_theme::variant::Tone;
use egui::{Response, Ui};

/// What an alert reports: its response and how it closed in this pass.
#[derive(Debug)]
#[non_exhaustive]
pub struct AlertOutput {
    /// The dialog's response.
    pub response: Response,
    /// How the alert closed in this pass; `None` while it stays open. OK, Enter, Escape and the close action all
    /// close it.
    pub closed: Option<CloseReason>,
}

impl Outcome for AlertOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// A modal notice with one OK button: a prompt whose only answer is OK, which OK, Enter, Escape and the close
/// action all give.
///
/// The node has the alert-dialog role and the message as its description. The open state is the `opened` binding,
/// else the alert's own, which starts open.
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct Alert<'a> {
    title: Label,
    message: Label,
    /// The label of the OK button; "OK" in the language of the scope without it.
    #[prop(into)]
    ok_label: Option<Label>,
    tone: Option<(Tone, Label)>,
    details: Option<(Label, Label)>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
}

impl<'a> Alert<'a> {
    /// An alert titled `title` with `message` and one OK button.
    pub fn new(title: impl Into<Label>, message: impl Into<Label>) -> Self {
        Alert { title: title.into(), message: message.into(), ok_label: None, tone: None, details: None, open: None }
    }

    /// A line under the message in the colour of `tone`, with the tone's icon.
    #[must_use]
    pub fn tone(mut self, tone: Tone, text: impl Into<Label>) -> Self {
        self.tone = Some((tone, text.into()));
        self
    }

    /// A collapsible section under the message; `label` names its header ("Details").
    #[must_use]
    pub fn details(mut self, label: impl Into<Label>, text: impl Into<Label>) -> Self {
        self.details = Some((label.into(), text.into()));
        self
    }

    /// Shows the alert.
    pub fn show(self, ui: &mut Ui) -> AlertOutput {
        let (content, open) = self.split();
        let mut prompt = content.prompt(&Env::of(ui));
        if let Some(open) = open {
            prompt = prompt.opened(open);
        }
        let shown = prompt.show(ui);
        AlertOutput { response: shown.response, closed: shown.closed }
    }

    /// What the alert shows, and its open state.
    fn split(self) -> (AlertContent, Option<Bind<'a, bool>>) {
        let Alert { title, message, ok_label, tone, details, open } = self;
        (AlertContent { title, message, ok_label, tone, details }, open)
    }
}

/// What an alert shows, without its open state.
struct AlertContent {
    title: Label,
    message: Label,
    ok_label: Option<Label>,
    tone: Option<(Tone, Label)>,
    details: Option<(Label, Label)>,
}

impl AlertContent {
    /// The prompt the alert shows.
    fn prompt<'p>(&self, env: &Env) -> Prompt<'p> {
        let ok = self.ok_label.clone().unwrap_or_else(|| env.words::<CommonWords>().ok.clone());
        let mut prompt =
            Prompt::new(self.title.clone(), self.message.clone()).alert().answers(buttons(env, vec![Reply::plain(ok, DialogRole::Accept)]));
        if let Some((tone, text)) = &self.tone {
            prompt = prompt.tone(*tone, text.clone());
        }
        if let Some((label, text)) = &self.details {
            prompt = prompt.details(label.clone(), text.clone());
        }
        prompt
    }
}

/// An [`Alert`] on the [`DialogHost`](super::DialogHost): it answers `Ok(())` when it closes, however it closed. One
/// alert is open at a time: a second one opened replaces the first.
///
/// ```ignore
/// host.open(HostedAlert::new(Alert::new("Could not open", "The file is damaged.")));
/// ```
pub struct HostedAlert {
    content: AlertContent,
    open: bool,
}

impl HostedAlert {
    /// `alert` on the host; its own `opened` binding is not used.
    pub fn new(alert: Alert<'_>) -> Self {
        HostedAlert { content: alert.split().0, open: true }
    }
}

/// The key of a hosted alert.
const ALERT_KEY: &str = "alert";

impl StackDialog for HostedAlert {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id(ALERT_KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let shown = self.content.prompt(&Env::of(ui)).opened(Bind::bound(&mut self.open)).show(ui);
        shown.closed.map(|_| DialogResult::Ok(()))
    }

    fn key(&self) -> Option<&str> {
        Some(ALERT_KEY)
    }
}

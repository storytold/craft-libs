//! [`HostedWorkspace`]: a full-window workspace the [`DialogHost`](crate::dialog::DialogHost) owns.

use std::sync::Arc;

use egui::{Response, Ui};

use crate::contract::Bind;
use crate::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};

/// A full-window workspace (Liquify, Filter Gallery, Select and Mask) that a [`HostedWorkspace`] shows on the app's
/// [`DialogHost`](crate::dialog::DialogHost). The workspace keeps its session and view; the app's side of it (the
/// preview, the image) is the [`Source`](FullWindowWorkspace::Source) the host hands it each pass.
///
/// The events of the pass that finishes the workspace go with it: a workspace whose app needs them folds them into
/// its [`Result`](FullWindowWorkspace::Result).
pub trait FullWindowWorkspace: Send + 'static {
    /// What the workspace reads from the app each pass, usually a `dyn` trait of the workspace's crate.
    type Source: ?Sized + Send + Sync;
    /// What a pass reports for the app to act on: a stroke, a preview it wants, a colour to pick.
    type Event: Send + 'static;
    /// What OK answers: the session the app runs.
    type Result: Send + 'static;

    /// The key of the command the workspace answers for (`filter.liquify`); the host knows it by
    /// [`dialog_id`] of it.
    const KEY: &'static str;

    /// Shows one pass of the workspace's [`FullWindowDialog`](crate::dialog::FullWindowDialog), whose open state
    /// it binds to `open` ([`opened`](crate::dialog::FullWindowDialog::opened)).
    fn show(&mut self, ui: &mut Ui, source: &Self::Source, open: Bind<'_, bool>) -> WorkspacePass<Self::Event, Self::Result>;
}

/// What one pass of a [`FullWindowWorkspace`] reports.
#[non_exhaustive]
pub struct WorkspacePass<E, R> {
    /// The workspace dialog's response.
    pub response: Response,
    /// What happened this pass, in order.
    pub events: Vec<E>,
    /// How the workspace finished this pass: OK's answer or Cancel, as its dialog's
    /// [`DialogOutput::result`](crate::dialog::DialogOutput::result) says; `None` while it stays open.
    pub result: Option<DialogResult<R>>,
}

impl<E, R> WorkspacePass<E, R> {
    /// A pass with `events` that leaves the workspace open.
    pub fn new(response: Response, events: Vec<E>) -> Self {
        WorkspacePass { response, events, result: None }
    }

    /// How the workspace finished this pass.
    #[must_use]
    pub fn result(mut self, result: Option<DialogResult<R>>) -> Self {
        self.result = result;
        self
    }
}

/// A [`FullWindowWorkspace`] the [`DialogHost`](crate::dialog::DialogHost) owns, with the app's source. It keeps the
/// events of each pass until the app takes them, and finishes as the workspace's pass says: OK's answer, or Cancel.
///
/// ```ignore
/// dialogs.open(HostedWorkspace::new(LiquifyWorkspace::new(open), Arc::new(source)));
/// dialogs.run(ui);
/// if let Some(liquify) = dialogs.get_mut::<HostedWorkspace<LiquifyWorkspace>>() {
///     for event in liquify.take_events() { /* apply it to the engine */ }
///     liquify.set_source(Arc::new(next_source));
/// }
/// if let Some(DialogResult::Ok(steps)) = dialogs.take::<HostedWorkspace<LiquifyWorkspace>>() {
///     doc.run("filter.liquify", steps);
/// }
/// ```
pub struct HostedWorkspace<W: FullWindowWorkspace> {
    id: DialogId,
    workspace: W,
    source: Arc<W::Source>,
    events: Vec<W::Event>,
    open: bool,
    #[cfg(feature = "testing")]
    facts: crate::testing::StoryOutput,
}

impl<W: FullWindowWorkspace> HostedWorkspace<W> {
    /// `workspace`, open, reading `source`.
    pub fn new(workspace: W, source: Arc<W::Source>) -> Self {
        HostedWorkspace {
            id: dialog_id(W::KEY),
            workspace,
            source,
            events: Vec::new(),
            open: true,
            #[cfg(feature = "testing")]
            facts: crate::testing::StoryOutput::default(),
        }
    }

    /// The workspace.
    pub fn workspace(&self) -> &W {
        &self.workspace
    }

    /// The workspace, for the app's answers to what it asked (a picked colour, a backdrop).
    pub fn workspace_mut(&mut self) -> &mut W {
        &mut self.workspace
    }

    /// The source the next pass reads.
    pub fn set_source(&mut self, source: Arc<W::Source>) {
        self.source = source;
    }

    /// The events of the passes since the last call, in order.
    pub fn take_events(&mut self) -> Vec<W::Event> {
        std::mem::take(&mut self.events)
    }
}

impl<W: FullWindowWorkspace> StackDialog for HostedWorkspace<W> {
    type Result = W::Result;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<W::Result>> {
        let pass = self.workspace.show(ui, &self.source, Bind::bound(&mut self.open));
        #[cfg(feature = "testing")]
        {
            self.facts = crate::contract::Outcome::story_facts(&pass.response);
        }
        self.events.extend(pass.events);
        pass.result
    }

    fn key(&self) -> Option<&str> {
        Some(W::KEY)
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.facts.clone()
    }
}

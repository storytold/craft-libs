//! [`HostedWorkspace`]: a full-window workspace the [`DialogHost`](crate::dialog::DialogHost) owns.

use std::sync::Arc;

use egui::{Context, Response, Ui};

use crate::contract::Bind;
use crate::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use crate::runtime::env::Label;

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
    fn show(&mut self, ui: &mut Ui, cx: WorkspaceCx<'_, Self::Source>) -> WorkspacePass<Self::Event, Self::Result>;

    /// The workspace's fields as JSON for the automation surface; `None` when it has none.
    fn json_fields(&self) -> Option<serde_json::Value> {
        None
    }

    /// Sets one field of the workspace; each key applies or is refused on its own. Refuses every key by default.
    fn set_json_field(&mut self, ctx: &Context, source: &Self::Source, field: &str, value: serde_json::Value) -> Result<(), String> {
        let _ = (ctx, source, value);
        Err(FieldRefusal::NoFields.message(field))
    }

    /// Answers OK for the automation surface; `None` when the workspace has no answer to give.
    fn confirm(&mut self, ctx: &Context, source: &Self::Source) -> Option<Self::Result> {
        let _ = (ctx, source);
        None
    }

    /// The app refused the OK the workspace held: `reason` is shown. Does nothing by default.
    fn ok_failed(&mut self, reason: Label) {
        let _ = reason;
    }
}

/// What the host hands a [`FullWindowWorkspace`] each pass.
#[non_exhaustive]
pub struct WorkspaceCx<'a, S: ?Sized> {
    /// What the workspace reads from the app.
    pub source: &'a S,
    /// Counts the source changes under an open workspace; the workspace adds it to its pending key, so a document edit
    /// previews again and a late preview of the old pixels is dropped.
    pub revision: u64,
    /// The workspace dialog's open state.
    pub open: Bind<'a, bool>,
    /// OK is held: the workspace waits for the app's answer before it closes.
    pub hold_ok: bool,
    /// The app's answer to OK is pending.
    pub ok_waits: bool,
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
    /// The result OK would give, reported in the pass the Accept button is picked under `cx.hold_ok`; `result`
    /// stays `None` then.
    // P3(dialogs-w11): dl.K57 `WorkspacePass::held`
    pub held: Option<R>,
}

impl<E, R> WorkspacePass<E, R> {
    /// A pass with `events` that leaves the workspace open.
    pub fn new(response: Response, events: Vec<E>) -> Self {
        WorkspacePass { response, events, result: None, held: None } // P3(dialogs-w11): dl.K57
    }

    /// The result a held OK gives this pass.
    // P3(dialogs-w11): dl.K57 `WorkspacePass::held`
    #[must_use]
    pub fn held(self, held: Option<R>) -> Self {
        let _ = (self, held);
        todo!("dl.K57")
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
    revision: u64,
    hold_ok: bool,
    ok_waits: bool,
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
            revision: 0,
            hold_ok: false,
            ok_waits: false,
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
        self.revision += 1;
    }

    /// OK waits for the app's answer ([`answer_ok`](Self::answer_ok)) instead of closing the workspace at once.
    #[must_use]
    pub fn held_ok(mut self) -> Self {
        self.hold_ok = true;
        self
    }

    /// The OK the workspace is holding for the app to act on, once; `None` when none is held.
    pub fn take_ok(&mut self) -> Option<W::Result> {
        todo!("dl.K57")
    }

    /// The app's answer to a held OK: `Ok` closes the workspace with it, `Err(reason)` keeps it open and shows
    /// `reason`.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        let _ = answer;
        todo!("dl.K57")
    }

    /// The app's source the workspace shows.
    // P3(dialogs-w11): dl.K57 `HostedWorkspace::source`
    pub fn source(&self) -> &Arc<W::Source> {
        todo!("dl.K57")
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
        let cx = WorkspaceCx {
            source: &*self.source,
            revision: self.revision,
            open: Bind::bound(&mut self.open),
            hold_ok: self.hold_ok,
            ok_waits: self.ok_waits,
        };
        let pass = self.workspace.show(ui, cx);
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

    fn confirm(&mut self, ctx: &Context) -> Option<W::Result> {
        self.workspace.confirm(ctx, &*self.source)
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        self.workspace.json_fields()
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        self.workspace.set_json_field(ctx, &*self.source, field, value)
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.facts.clone()
    }
}

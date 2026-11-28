//! [`HostedWorkspace`]: a full-window workspace the [`DialogHost`](crate::dialog::DialogHost) owns.

use std::sync::Arc;

use egui::{Context, Response, Ui};

use crate::contract::Bind;
use crate::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use crate::runtime::env::Label;

/// A full-window workspace (Liquify, Filter Gallery, Select and Mask) that a [`HostedWorkspace`] shows on the app's
/// [`DialogHost`](crate::dialog::DialogHost). The workspace keeps its session and view; the app's side of it (the
/// preview, the image) is the [`Source`](FullWindowWorkspace::Source) the host hands it each pass.
///
/// The events of the pass that finishes the workspace go with it: a workspace whose app needs them folds them into
/// its [`Result`](FullWindowWorkspace::Result).
///
/// - Automation: [`json_fields`](FullWindowWorkspace::json_fields), [`set_json_field`](FullWindowWorkspace::set_json_field)
///   and [`confirm`](FullWindowWorkspace::confirm) are what the host's `ui.dialog` surface reaches. Each key applies or
///   is refused on its own; the events a set produces are queued by the workspace and reported by its next pass.
/// - Source revision: [`WorkspaceCx::revision`] moves with each new source; the workspace adds it to its pending key.
/// - Held OK: under [`WorkspaceCx::hold_ok`] the workspace makes its Accept button `stays_open`, disables it while
///   [`WorkspaceCx::ok_waits`], and reports the answer OK would give in [`WorkspacePass::held`]. The app takes it
///   ([`HostedWorkspace::take_ok`]) and answers ([`HostedWorkspace::answer_ok`]); a refusal reaches
///   [`ok_failed`](FullWindowWorkspace::ok_failed).
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

    /// The workspace's fields as JSON for the automation surface; `None` when it has none. `source` is what the app
    /// shows the workspace, for the fields that read it (the readout under the pointer).
    fn json_fields(&self, source: &Self::Source) -> Option<serde_json::Value> {
        let _ = source;
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

    /// The workspace closed, by any route: its own OK, Cancel or Escape, an accepted held OK, the host's
    /// [`confirm`](FullWindowWorkspace::confirm), or a close from outside (`DialogHost::cancel`, a workspace of the same
    /// kind opened over it, its parent closing). What the workspace keeps past its close (remembered settings) is
    /// written here. Does nothing by default.
    fn closed(&mut self, ctx: &Context) {
        let _ = ctx;
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

impl<'a, S: ?Sized> WorkspaceCx<'a, S> {
    /// A pass over `source` with the open state bound to `open`, at the first revision, with OK not held: what a
    /// workspace shown outside a [`HostedWorkspace`] gets. The host's stories and trials build it.
    pub fn new(source: &'a S, open: Bind<'a, bool>) -> Self {
        WorkspaceCx { source, revision: 0, open, hold_ok: false, ok_waits: false }
    }
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
    pub held: Option<R>,
    /// The canvas the workspace showed its image on this pass (the id of its `FullWindowCanvas`'s response, which
    /// keys the canvas's tool host): where the app's tool input in document points goes while the workspace is open.
    pub canvas: Option<egui::Id>,
}

impl<E, R> WorkspacePass<E, R> {
    /// A pass with `events` that leaves the workspace open.
    pub fn new(response: Response, events: Vec<E>) -> Self {
        WorkspacePass { response, events, result: None, held: None, canvas: None }
    }

    /// The result a held OK gives this pass.
    #[must_use]
    pub fn held(mut self, held: Option<R>) -> Self {
        self.held = held;
        self
    }

    /// How the workspace finished this pass.
    #[must_use]
    pub fn result(mut self, result: Option<DialogResult<R>>) -> Self {
        self.result = result;
        self
    }

    /// The canvas the workspace showed its image on this pass; `None` for a pass that showed none (a measuring run).
    #[must_use]
    pub fn canvas(mut self, canvas: Option<egui::Id>) -> Self {
        self.canvas = canvas;
        self
    }
}

/// A [`FullWindowWorkspace`] the [`DialogHost`](crate::dialog::DialogHost) owns, with the app's source. It keeps the
/// events of each pass until the app takes them, and finishes as the workspace's pass says: OK's answer, or Cancel.
/// It forwards the host's automation calls (`json_fields`, `set_json_field`, `confirm`) to the workspace with its
/// source.
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
///
/// A workspace whose OK the app checks first opens [`held_ok`](HostedWorkspace::held_ok): OK files its answer and
/// the workspace stays shown until the app answers.
///
/// ```ignore
/// dialogs.open(HostedWorkspace::new(FillWorkspace::new(open), Arc::new(source)).held_ok());
/// dialogs.run(ui);
/// if let Some(fill) = dialogs.get_mut::<HostedWorkspace<FillWorkspace>>()
///     && let Some(session) = fill.take_ok()
/// {
///     fill.answer_ok(doc.run("edit.contentAwareFill", session).map_err(|error| error.label()));
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
    /// The held OK's answer until the app answers it (the workspace disables its OK meanwhile), and why the app
    /// refused the last one, until OK files again.
    held: HeldAnswer<W::Result>,
    /// The canvas the workspace shows its image on, as its latest pass that showed one reported it.
    canvas: Option<egui::Id>,
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
            held: HeldAnswer::new(),
            canvas: None,
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
    pub fn take_ok(&mut self) -> Option<W::Result>
    where
        W::Result: Clone,
    {
        self.held.take()
    }

    /// The app's answer to a held OK: `Ok` closes the workspace with it in the next pass, `Err(reason)` keeps it
    /// open with OK enabled again and hands `reason` to [`FullWindowWorkspace::ok_failed`]. Does nothing while no OK
    /// is held.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        let refused = answer.as_ref().err().cloned();
        if self.held.answer(answer)
            && let Some(reason) = refused
        {
            self.workspace.ok_failed(reason);
        }
    }

    /// The app's source the workspace shows.
    pub fn source(&self) -> &Arc<W::Source> {
        &self.source
    }

    /// The events of the passes since the last call, in order.
    pub fn take_events(&mut self) -> Vec<W::Event> {
        std::mem::take(&mut self.events)
    }

    /// Files a held OK's `result` for the app; OK waits for the answer.
    fn file(&mut self, result: W::Result) {
        self.held.file(result);
    }
}

impl<W: FullWindowWorkspace> StackDialog for HostedWorkspace<W> {
    type Result = W::Result;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<W::Result>> {
        // The app accepted the held OK: the first pass after the answer finishes, whichever pass of a frame it is.
        if let Some(result) = self.held.finish() {
            self.workspace.closed(ui.ctx());
            return Some(DialogResult::Ok(result));
        }
        let cx = WorkspaceCx {
            source: &*self.source,
            revision: self.revision,
            open: Bind::bound(&mut self.open),
            hold_ok: self.hold_ok,
            ok_waits: self.held.is_waiting(),
        };
        let pass = self.workspace.show(ui, cx);
        if pass.canvas.is_some() {
            self.canvas = pass.canvas;
        }
        #[cfg(feature = "testing")]
        {
            self.facts = crate::contract::Outcome::story_facts(&pass.response);
        }
        self.events.extend(pass.events);
        // A pick reaches only the pass whose input held it; a later pass of the frame finds OK waiting and files
        // nothing more.
        if let Some(result) = pass.held.filter(|_| self.hold_ok && !self.held.is_waiting()) {
            self.file(result);
        }
        if pass.result.is_some() {
            self.workspace.closed(ui.ctx());
        }
        pass.result
    }

    fn key(&self) -> Option<&str> {
        Some(W::KEY)
    }

    fn canvas(&self) -> Option<egui::Id> {
        self.canvas
    }

    /// OK as the Accept button: a held workspace files the result and answers none, and answers the held result once
    /// the app accepted it.
    fn confirm(&mut self, ctx: &Context) -> Option<W::Result> {
        let result = if self.held.is_accepted() {
            self.held.finish()
        } else if self.held.is_waiting() {
            None
        } else {
            let result = self.workspace.confirm(ctx, &*self.source)?;
            if self.hold_ok {
                self.file(result);
                return None;
            }
            Some(result)
        };
        if result.is_some() {
            self.workspace.closed(ctx);
        }
        result
    }

    fn cancelled(&mut self, ctx: &Context) {
        self.workspace.closed(ctx);
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        self.workspace.json_fields(&*self.source)
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        self.workspace.set_json_field(ctx, &*self.source, field, value)
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.facts.clone()
    }
}

//! [`SuiteClient`]: the app side of the suite selection protocol over a [`SuiteWatch`], shared by
//! every app so that none re-implements it.

use std::sync::Arc;

use craft_theme::{SuiteChoice, ThemeDef, ThemeId, ThemePair};

use crate::{Accepted, Op, Problem, SelectionChange, Snapshot, StoreError, SuiteWatch, watch::with_library_theme};

/// A change sent to the watch and not answered yet.
#[derive(Clone, Debug)]
struct Pending {
    id: u64,
    change: SelectionChange,
}

/// The app's view of the suite: the accepted state, the user's changes on their way, and the
/// notices to show.
///
/// Each frame the app calls [`sync`](Self::sync) (one atomic load unless something changed),
/// shows [`shown`](Self::shown) with its own defaults, and resolves with
/// [`theme`](Self::theme) as the lookup. User picks go to [`change`](Self::change).
///
/// At most one change is in flight. Picks made meanwhile merge into one queued change (a later
/// pick of a field replaces the earlier one), which goes out with the new accepted state as
/// base when the first result arrives, so quick picks never conflict with each other.
pub struct SuiteClient {
    watch: SuiteWatch,
    generation: Option<u64>,
    snapshot: Arc<Snapshot>,
    pending: Option<Pending>,
    queued: Option<SelectionChange>,
    conflict: Option<(Accepted, SelectionChange)>,
    failure: Option<(SelectionChange, StoreError)>,
    app_theme: Option<ThemePair>,
    /// Open couldn't read `selection.json`: changes wait in the queue for the worker's first
    /// snapshot instead of merging against a state nobody read.
    hold: bool,
}

impl SuiteClient {
    /// A client for `watch`, starting from its current snapshot.
    pub fn new(watch: SuiteWatch) -> Self {
        let snapshot = watch.snapshot();
        let app_theme = watch.initial_app_theme().cloned();
        let hold = !watch.selection_read();
        SuiteClient { watch, generation: None, snapshot, pending: None, queued: None, conflict: None, failure: None, app_theme, hold }
    }

    /// An in-memory suite, for app constructors, tests and web builds.
    pub fn memory() -> Self {
        Self::new(SuiteWatch::memory())
    }

    /// Takes a new snapshot when the watch has one, handles the answer to the change in flight
    /// and sends the queued one. Returns whether anything the app shows may have changed.
    pub fn sync(&mut self) -> bool {
        let generation = self.watch.generation();
        if self.generation == Some(generation) {
            return false;
        }
        self.generation = Some(generation);
        self.snapshot = self.watch.snapshot();
        if self.hold && generation > 0 {
            self.hold = false;
            if self.pending.is_none()
                && let Some(queued) = self.queued.take()
            {
                self.send(queued);
            }
        }
        self.settle();
        true
    }

    fn settle(&mut self) {
        let Some(pending) = &self.pending else {
            return;
        };
        let result = match self.snapshot.result(pending.id) {
            Some(result) => Some(result.clone()),
            // Not run yet, unless newer results have pushed it out of the snapshot.
            None if self.snapshot.results.first().is_some_and(|(id, _)| *id > pending.id) => None,
            None => return,
        };
        let Some(pending) = self.pending.take() else {
            return;
        };
        let accepted = &self.snapshot.accepted.suite;
        match result {
            Some(Err(StoreError::Conflict { disk, mine })) => self.conflict = Some((disk, mine)),
            Some(Err(error)) => self.failure = Some((pending.change, error)),
            // More than RESULTS_KEPT other results arrived between two syncs. The accepted
            // state is the only evidence left: a change it doesn't show may have failed.
            None if pending.change.applied_to(accepted) != *accepted => self.failure = Some((pending.change, StoreError::Lost)),
            Some(Ok(_)) | None => {}
        }
        if let Some(queued) = self.queued.take() {
            self.send(queued);
        }
    }

    /// The user changed fields of the suite's selection. Notices about earlier changes are cleared.
    pub fn change(&mut self, change: SelectionChange) {
        self.dismiss();
        self.submit(change);
    }

    fn submit(&mut self, change: SelectionChange) {
        if change.is_empty() {
            return;
        }
        if self.pending.is_none() && !self.hold {
            self.send(change);
            return;
        }
        self.queued = Some(match self.queued.take() {
            None => change,
            Some(queued) => queued.then(change),
        });
    }

    fn send(&mut self, change: SelectionChange) {
        let base = self.snapshot.accepted.clone();
        let id = self.watch.request(Op::Update { base, change: change.clone() });
        self.pending = Some(Pending { id, change });
    }

    /// The suite choice to show: the accepted one with the changes on their way applied. A
    /// picked library theme the snapshot hasn't loaded yet shows in its slot only once it has,
    /// so the app never falls back to its own palette in between.
    pub fn shown(&self) -> SuiteChoice {
        let accepted = &self.snapshot.accepted.suite;
        let mut shown = accepted.clone();
        if let Some(pending) = &self.pending {
            shown = pending.change.applied_to(&shown);
        }
        if let Some(queued) = &self.queued {
            shown = queued.applied_to(&shown);
        }
        if shown.theme_light.as_ref().is_some_and(|id| self.is_loading(id)) {
            shown.theme_light = accepted.theme_light.clone();
        }
        if shown.theme_dark.as_ref().is_some_and(|id| self.is_loading(id)) {
            shown.theme_dark = accepted.theme_dark.clone();
        }
        shown
    }

    /// A built-in or a loaded library theme, for
    /// [`resolve_selection`](craft_theme::resolve_selection)'s `lookup`.
    pub fn theme(&self, id: &ThemeId) -> Option<ThemeDef> {
        self.snapshot.theme(id)
    }

    /// Any built-in or library theme, for a preview card ([`Snapshot::preview`]).
    pub fn preview(&self, id: &ThemeId) -> Option<ThemeDef> {
        self.snapshot.preview(id)
    }

    /// True while `id` is a library theme the snapshot hasn't loaded yet: one just picked for the
    /// suite or named with [`set_app_theme`](Self::set_app_theme), with no problem reported for
    /// it. An app showing its own theme keeps its current look while this is true.
    pub fn is_loading(&self, id: &ThemeId) -> bool {
        !id.is_builtin() && self.snapshot.theme(id).is_none() && self.is_requested(id) && !self.has_problem(id)
    }

    fn is_requested(&self, id: &ThemeId) -> bool {
        let picked = |c: &SelectionChange| c.picked().any(|p| p == id);
        let own = self.app_theme.as_ref().is_some_and(|p| p.light == *id || p.dark == *id);
        own || self.pending.as_ref().is_some_and(|p| picked(&p.change)) || self.queued.as_ref().is_some_and(picked)
    }

    fn has_problem(&self, id: &ThemeId) -> bool {
        self.snapshot.problems.iter().any(|p| matches!(p, Problem::Theme { stem, .. } if stem == id.as_str()))
    }

    /// The themes the app uses for its own choice ([`AppChoice::theme`](craft_theme::AppChoice)),
    /// or `None` for the suite's. Sent to the watch only when it changes; a pair of built-ins
    /// counts as `None`.
    pub fn set_app_theme(&mut self, themes: Option<ThemePair>) {
        let themes = with_library_theme(themes);
        if themes != self.app_theme {
            self.app_theme = themes.clone();
            self.watch.request(Op::AppTheme(themes));
        }
    }

    /// The state the suite last confirmed.
    pub fn accepted(&self) -> &Accepted {
        &self.snapshot.accepted
    }

    /// True while a change is on its way.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some() || self.queued.is_some()
    }

    /// The last change another app's choice blocked: the suite's state and the user's change.
    /// Show "Changed in another Craft app; your choice wasn't applied" with "Apply mine".
    pub fn conflict(&self) -> Option<&(Accepted, SelectionChange)> {
        self.conflict.as_ref()
    }

    /// The last change that failed, with the reason. Show it with "Retry". The reason is
    /// [`StoreError::Lost`] when the change's result was gone before the client saw it and the
    /// suite's state doesn't show the change.
    pub fn failure(&self) -> Option<&(SelectionChange, StoreError)> {
        self.failure.as_ref()
    }

    /// Sends the change from the last conflict again, on the state the suite has now.
    pub fn apply_mine(&mut self) {
        if let Some((_, mine)) = self.conflict.take() {
            self.submit(mine);
        }
    }

    /// Sends the last failed change again.
    pub fn retry(&mut self) {
        if let Some((change, _)) = self.failure.take() {
            self.submit(change);
        }
    }

    /// Clears the conflict and failure notices.
    pub fn dismiss(&mut self) {
        self.conflict = None;
        self.failure = None;
    }

    /// Files the suite can't use right now; each has a `Display` for the app's notice.
    pub fn problems(&self) -> &[Problem] {
        &self.snapshot.problems
    }

    /// The latest snapshot of the suite folder.
    pub fn snapshot(&self) -> &Arc<Snapshot> {
        &self.snapshot
    }

    /// For the other operations: import, delete, export, rescan.
    pub fn watch(&self) -> &SuiteWatch {
        &self.watch
    }
}

#[cfg(test)]
mod tests;

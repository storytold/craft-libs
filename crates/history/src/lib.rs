//! Snapshot-based branching history shared by Craft applications.
//!
//! No document format, renderer, filesystem, or app dependency is required. A
//! [`Policy`] supplies document-specific metadata and shared-allocation accounting;
//! [`ArchivedDocument`] supplies fallible lazy storage or compact replay.
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

/// Application adapter. Hooks must be deterministic and must not hydrate storage.
/// Keep every allocation visited by `document_bytes` alive during a deduplication pass.
/// Targets can be target, cells, slides, tracks, or an empty value.
pub trait Policy: Clone + std::fmt::Debug + Send + Sync + 'static {
    type Document: Clone + std::fmt::Debug + Send + Sync + 'static;
    type Target: Clone + Default + std::fmt::Debug + Send + Sync + 'static;
    type Id: Copy + std::fmt::Debug + Send + Sync;
    fn has_selection(document: &Self::Document) -> bool;
    fn remap_id(document: &mut Arc<Self::Document>, id: Self::Id);
    fn document_bytes(document: &Self::Document, seen: &mut HashSet<usize>) -> usize;
    fn target_bytes(_target: &Self::Target) -> usize {
        0
    }
}

#[derive(Clone, Debug)]
pub struct HistoryState<P: Policy> {
    pub label: String,
    /// The target targeted when this state was created.
    pub target: P::Target,
    pub identity: StateIdentity,
    document: StoredDocument<P>,
    has_selection: bool,
}

/// Stable navigation identity; branch colors follow `branch`, not row positions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateIdentity {
    pub id: u64,
    pub parent: Option<u64>,
    pub branch: u64,
    #[serde(default)]
    pub revision: u64,
}

#[derive(Clone, Debug)]
pub struct HistoryEntry {
    pub identity: StateIdentity,
    pub label: String,
    pub current: bool,
}

impl<P: Policy> HistoryState<P> {
    /// Build a resident recovery state without copying pixel data.
    pub fn from_document(label: impl Into<String>, document: Arc<P::Document>) -> Self {
        Self {
            label: label.into(),
            target: P::Target::default(),
            identity: StateIdentity::default(),
            has_selection: P::has_selection(&document),
            document: StoredDocument::<P>::Resident(document),
        }
    }

    /// Build a lazily restored recovery state. `has_selection` is metadata for
    /// selection menus, which must not read storage merely to draw themselves.
    pub fn from_archive(label: impl Into<String>, has_selection: bool, archive: Arc<dyn ArchivedDocument<P>>) -> Self {
        Self {
            label: label.into(),
            target: P::Target::default(),
            identity: StateIdentity::default(),
            has_selection,
            document: StoredDocument::<P>::Archived(archive),
        }
    }

    /// May read storage for a cold state; recovery writers call this on a worker.
    pub fn load_document(&self) -> Result<Arc<P::Document>, String> {
        self.document.load()
    }

    pub fn has_selection(&self) -> bool {
        self.has_selection
    }

    pub fn with_target(mut self, target: P::Target) -> Self {
        self.target = target;
        self
    }
}

/// Immutable handles and navigation metadata captured alongside the current
/// document. Undo is oldest first; redo's last entry is the next redo state.
/// Capturing this value never hydrates archived documents.
#[derive(Clone, Debug)]
pub struct HistoryCheckpoint<P: Policy> {
    pub undo: Vec<HistoryState<P>>,
    pub redo: Vec<HistoryState<P>>,
    pub branches: Vec<HistoryState<P>>,
    pub current_identity: StateIdentity,
    pub current_label: String,
    pub current_target: P::Target,
    pub max_states: usize,
    pub max_bytes: usize,
}

/// Engine-owned immutable scratch archive. Loading must preserve the complete document.
pub trait ArchivedDocument<P: Policy>: std::fmt::Debug + Send + Sync {
    fn load(&self) -> Result<Arc<P::Document>, String>;
    fn metadata_bytes(&self) -> usize {
        0
    }
    /// Resident backing for compact replay states; counted once, never mistaken for disk.
    fn resident(&self) -> Option<&Arc<P::Document>> {
        None
    }
}

#[derive(Debug)]
struct RemappedDocument<P: Policy> {
    source: Arc<dyn ArchivedDocument<P>>,
    id: P::Id,
}
impl<P: Policy> ArchivedDocument<P> for RemappedDocument<P> {
    fn load(&self) -> Result<Arc<P::Document>, String> {
        let mut doc = self.source.load()?;
        P::remap_id(&mut doc, self.id);
        Ok(doc)
    }
    fn metadata_bytes(&self) -> usize {
        std::mem::size_of::<Self>().saturating_add(self.source.metadata_bytes())
    }
    fn resident(&self) -> Option<&Arc<P::Document>> {
        self.source.resident()
    }
}

#[derive(Clone, Debug)]
enum StoredDocument<P: Policy> {
    Resident(Arc<P::Document>),
    Archived(Arc<dyn ArchivedDocument<P>>),
}

impl<P: Policy> StoredDocument<P> {
    fn load(&self) -> Result<Arc<P::Document>, String> {
        match self {
            Self::Resident(doc) => Ok(doc.clone()),
            Self::Archived(archive) => archive.load(),
        }
    }
    fn resident(&self) -> Option<&Arc<P::Document>> {
        match self {
            Self::Resident(doc) => Some(doc),
            Self::Archived(archive) => archive.resident(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct History<P: Policy> {
    /// Past states; the last one is the state *before* the current document.
    undo: VecDeque<HistoryState<P>>,
    redo: Vec<HistoryState<P>>,
    branches: Vec<HistoryState<P>>,
    current_identity: StateIdentity,
    next_id: u64,
    pub max_states: usize,
    /// Pixel memory budget in bytes for the current document plus the tiles only history holds
    /// (0 = unlimited). [`History::<P>::trim`] drops the oldest states beyond it.
    pub max_bytes: usize,
    /// Label of the step that produced the current document.
    current_label: String,
    /// The target targeted when the current document was created (opened or edited).
    current_target: P::Target,
    current_archive: Option<Arc<dyn ArchivedDocument<P>>>,
    current_archive_document: Option<std::sync::Weak<P::Document>>,
}

impl<P: Policy> Default for History<P> {
    fn default() -> Self {
        Self::new(0)
    }
}

impl<P: Policy> History<P> {
    pub fn checkpoint(&self) -> HistoryCheckpoint<P> {
        HistoryCheckpoint {
            undo: self.undo.iter().cloned().collect(),
            redo: self.redo.clone(),
            branches: self.branches.clone(),
            current_identity: self.current_identity,
            current_label: self.current_label.clone(),
            current_target: self.current_target.clone(),
            max_states: self.max_states,
            max_bytes: self.max_bytes,
        }
    }

    /// Restore the exact navigation cursor without reading document storage.
    /// Recovery metadata has independent admission limits; preference limits
    /// may subsequently be applied explicitly through `enforce_state_limit`.
    pub fn from_checkpoint(mut checkpoint: HistoryCheckpoint<P>) -> Result<Self, String> {
        const MAX_LABEL_BYTES: usize = 4096;
        if checkpoint.current_label.len() > MAX_LABEL_BYTES
            || checkpoint.undo.iter().chain(checkpoint.redo.iter()).chain(checkpoint.branches.iter()).any(|state| state.label.len() > MAX_LABEL_BYTES)
        {
            return Err("Recovery history label exceeds 4096 bytes".into());
        }
        // Legacy checkpoints have no identities. Assign their linear path once.
        if checkpoint.current_identity == StateIdentity::default()
            && checkpoint.undo.iter().chain(checkpoint.redo.iter()).all(|state| state.identity == StateIdentity::default())
            && checkpoint.branches.is_empty()
        {
            let mut id = 0u64;
            for state in &mut checkpoint.undo {
                state.identity = StateIdentity { id, parent: id.checked_sub(1), branch: 0, revision: 0 };
                id = id.saturating_add(1);
            }
            checkpoint.current_identity = StateIdentity { id, parent: id.checked_sub(1), branch: 0, revision: 0 };
            for state in checkpoint.redo.iter_mut().rev() {
                id = id.saturating_add(1);
                state.identity = StateIdentity { id, parent: id.checked_sub(1), branch: 0, revision: 0 };
            }
        }
        if checkpoint.current_identity.id > u64::MAX / 2
            || checkpoint.current_identity.revision > u64::MAX / 2
            || checkpoint.current_identity.branch > checkpoint.current_identity.id
            || checkpoint.current_identity.parent.is_some_and(|parent| parent >= checkpoint.current_identity.id)
        {
            return Err("Invalid current history identity".into());
        }
        let mut ids = HashSet::from([checkpoint.current_identity.id]);
        let mut next_id = checkpoint.current_identity.id.checked_add(1).ok_or("History identity exhausted")?;
        for state in checkpoint.undo.iter().chain(checkpoint.redo.iter()).chain(checkpoint.branches.iter()) {
            if !ids.insert(state.identity.id)
                || state.identity.id > u64::MAX / 2
                || state.identity.revision > u64::MAX / 2
                || state.identity.branch > state.identity.id
                || state.identity.parent.is_some_and(|parent| parent >= state.identity.id)
            {
                return Err("Invalid recovery history identity".into());
            }
            next_id = next_id.max(state.identity.id.checked_add(1).ok_or("History identity exhausted")?);
        }
        Ok(Self {
            undo: checkpoint.undo.into(),
            redo: checkpoint.redo,
            branches: checkpoint.branches,
            current_identity: checkpoint.current_identity,
            next_id,
            current_label: checkpoint.current_label,
            current_target: checkpoint.current_target,
            current_archive: None,
            current_archive_document: None,
            max_states: checkpoint.max_states,
            max_bytes: checkpoint.max_bytes,
        })
    }

    pub fn new(max_states: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            branches: Vec::new(),
            current_identity: StateIdentity::default(),
            next_id: 1,
            max_states,
            max_bytes: 0,
            current_label: "Open".into(),
            current_target: P::Target::default(),
            current_archive: None,
            current_archive_document: None,
        }
    }

    /// Metadata-only stamp for logical history changes (including Purge).
    pub fn checkpoint_signature(&self) -> (usize, usize, &str) {
        (self.undo.len(), self.redo.len(), &self.current_label)
    }

    /// A duplicate recovered tab can be admitted under a fresh identity.
    /// Cold states are remapped only when restored, without reading disk here.
    pub fn remap_document_id(&mut self, id: P::Id) {
        if let Some(source) = self.current_archive.take() {
            self.current_archive = Some(Arc::new(RemappedDocument { source, id }));
        }
        for state in self.undo.iter_mut().chain(self.redo.iter_mut()).chain(self.branches.iter_mut()) {
            match &mut state.document {
                StoredDocument::<P>::Resident(doc) => {
                    P::remap_id(doc, id);
                }
                StoredDocument::<P>::Archived(source) => {
                    *source = Arc::new(RemappedDocument { source: source.clone(), id });
                }
            }
        }
    }

    /// Record that `before` was replaced by a new current document via step `label`, which left
    /// `target` targeted.
    pub fn record(&mut self, label: impl Into<String>, before: Arc<P::Document>, target: P::Target) {
        self.clear_current_archive();
        let prev_label = std::mem::replace(&mut self.current_label, label.into());
        let prev_layers = std::mem::replace(&mut self.current_target, target);
        self.undo.push_back(HistoryState {
            label: prev_label,
            target: prev_layers,
            identity: self.current_identity,
            has_selection: P::has_selection(&before),
            document: StoredDocument::<P>::Resident(before),
        });
        let branch = if self.redo.is_empty() { self.current_identity.branch } else { self.next_id };
        self.current_identity = StateIdentity { id: self.next_id, parent: Some(self.current_identity.id), branch, revision: 0 };
        self.next_id = self.next_id.saturating_add(1);
        self.branches.append(&mut self.redo);
        self.enforce_state_limit();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.can_undo().then_some(self.current_label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|s| s.label.as_str())
    }
    /// Labels of the redo states, the next redo first (the History panel's greyed rows).
    pub fn redo_labels(&self) -> impl Iterator<Item = &str> {
        self.redo.iter().rev().map(|s| s.label.as_str())
    }

    /// Set the target the current document targets from its creation on: when it is first
    /// opened, or when its step coalesces or changes the selection after recording.
    pub fn set_current_target(&mut self, target: P::Target) {
        self.current_target = target;
        self.current_identity.revision = self.current_identity.revision.saturating_add(1);
    }

    pub fn current_target(&self) -> &P::Target {
        &self.current_target
    }

    /// Restore first: a failed archive read never moves either history stack.
    pub fn try_undo(&mut self, current: Arc<P::Document>) -> Result<Option<Arc<P::Document>>, String> {
        let Some(prev) = self.undo.back() else { return Ok(None) };
        let restored = prev.document.load()?;
        let Some(prev) = self.undo.pop_back() else { return Ok(None) };
        let label = std::mem::replace(&mut self.current_label, prev.label);
        let target = std::mem::replace(&mut self.current_target, prev.target.clone());
        let has_selection = P::has_selection(&current);
        let document = self.take_current_document(current);
        self.redo.push(HistoryState { label, has_selection, document, target, identity: self.current_identity });
        self.current_identity = prev.identity;
        self.current_archive = match prev.document {
            StoredDocument::<P>::Archived(archive) => Some(archive),
            _ => None,
        };
        self.current_archive_document = self.current_archive.as_ref().map(|_| Arc::downgrade(&restored));
        Ok(Some(restored))
    }

    pub fn try_redo(&mut self, current: Arc<P::Document>) -> Result<Option<Arc<P::Document>>, String> {
        let Some(next) = self.redo.last() else { return Ok(None) };
        let restored = next.document.load()?;
        let Some(next) = self.redo.pop() else { return Ok(None) };
        let label = std::mem::replace(&mut self.current_label, next.label);
        let target = std::mem::replace(&mut self.current_target, next.target.clone());
        let has_selection = P::has_selection(&current);
        let document = self.take_current_document(current);
        self.undo.push_back(HistoryState { label, has_selection, document, target, identity: self.current_identity });
        self.current_identity = next.identity;
        self.current_archive = match next.document {
            StoredDocument::<P>::Archived(archive) => Some(archive),
            _ => None,
        };
        self.current_archive_document = self.current_archive.as_ref().map(|_| Arc::downgrade(&restored));
        Ok(Some(restored))
    }

    /// Compatibility wrapper. Interactive callers should use [`Self::try_undo`] to report errors.
    pub fn undo(&mut self, current: Arc<P::Document>) -> Option<(Arc<P::Document>, P::Target)> {
        self.try_undo(current).ok().flatten().map(|doc| (doc, self.current_target.clone()))
    }
    /// Compatibility wrapper. Interactive callers should use [`Self::try_redo`] to report errors.
    pub fn redo(&mut self, current: Arc<P::Document>) -> Option<(Arc<P::Document>, P::Target)> {
        self.try_redo(current).ok().flatten().map(|doc| (doc, self.current_target.clone()))
    }

    /// Entries for a History panel: past labels oldest→newest, then the current label.
    pub fn entries(&self) -> Vec<String> {
        self.undo.iter().map(|s| s.label.clone()).chain(std::iter::once(self.current_label.clone())).collect()
    }

    /// Metadata only: drawing history must never decode a document from disk.
    pub fn timeline(&self) -> Vec<HistoryEntry> {
        let mut entries: Vec<_> = self
            .undo
            .iter()
            .chain(self.redo.iter())
            .chain(self.branches.iter())
            .map(|state| HistoryEntry { identity: state.identity, label: state.label.clone(), current: false })
            .collect();
        entries.push(HistoryEntry { identity: self.current_identity, label: self.current_label.clone(), current: true });
        entries.sort_by_key(|entry| entry.identity.id);
        entries
    }

    /// Delete one branch state and all of its descendants, preserving independent branches.
    /// The caller first moves the current document to the deleted state's predecessor.
    pub fn prune_descendants(&mut self, id: u64) {
        let timeline = self.timeline();
        let mut removed = HashSet::from([id]);
        loop {
            let before = removed.len();
            for entry in &timeline {
                if entry.identity.parent.is_some_and(|parent| removed.contains(&parent)) {
                    removed.insert(entry.identity.id);
                }
            }
            if removed.len() == before {
                break;
            }
        }
        self.undo.retain(|state| !removed.contains(&state.identity.id));
        self.redo.retain(|state| !removed.contains(&state.identity.id));
        self.branches.retain(|state| !removed.contains(&state.identity.id));
    }

    pub fn current_id(&self) -> u64 {
        self.current_identity.id
    }

    pub fn timeline_stamp(&self) -> (u64, u64, u64, usize, usize, usize) {
        (self.current_identity.id, self.current_identity.revision, self.next_id, self.undo.len(), self.redo.len(), self.branches.len())
    }

    pub fn adjacent_id(&self, redo: bool) -> Option<u64> {
        (if redo { self.redo.last() } else { self.undo.back() }).map(|state| state.identity.id)
    }

    /// Immutable destination handle, safe to restore on a worker.
    pub fn state_by_id(&self, current: Arc<P::Document>, id: u64) -> Option<HistoryState<P>> {
        if id == self.current_identity.id {
            let mut state = HistoryState::<P>::from_document(self.current_label.clone(), current).with_target(self.current_target.clone());
            state.identity = self.current_identity;
            return Some(state);
        }
        self.undo.iter().chain(self.redo.iter()).chain(self.branches.iter()).find(|state| state.identity.id == id).cloned()
    }

    /// Commit one already-loaded destination. Intervening snapshots stay cold.
    /// Build/validate the path before mutating navigation, so any error is atomic.
    pub fn jump_loaded(&mut self, current: Arc<P::Document>, id: u64, restored: Arc<P::Document>) -> Result<(), String> {
        self.jump_loaded_on_path(current, id, restored, None)
    }

    pub fn navigation_branch(&self) -> u64 {
        self.redo.first().map_or(self.current_identity.branch, |state| state.identity.branch)
    }

    pub fn jump_loaded_on_path(
        &mut self,
        current: Arc<P::Document>,
        id: u64,
        restored: Arc<P::Document>,
        preferred_branch: Option<u64>,
    ) -> Result<(), String> {
        if id == self.current_identity.id {
            return Ok(());
        }
        let mut nodes: std::collections::BTreeMap<_, _> =
            self.undo.iter().chain(self.redo.iter()).chain(self.branches.iter()).map(|state| (state.identity.id, state.clone())).collect();
        let target = nodes.remove(&id).ok_or("History state is no longer available")?;
        let mut current_state =
            HistoryState::<P>::from_document(self.current_label.clone(), current.clone()).with_target(self.current_target.clone());
        current_state.identity = self.current_identity;
        nodes.insert(self.current_identity.id, current_state);
        let mut ancestors = Vec::new();
        let mut parent = target.identity.parent;
        let mut seen = HashSet::from([id]);
        while let Some(key) = parent {
            if !seen.insert(key) {
                return Err("Cyclic history path".into());
            }
            let Some(state) = nodes.remove(&key) else { break };
            parent = state.identity.parent;
            ancestors.push(state);
        }
        ancestors.reverse();
        let branch =
            preferred_branch.unwrap_or_else(|| if target.identity.parent.is_none() { self.navigation_branch() } else { target.identity.branch });
        let mut descendants = Vec::new();
        let mut children: std::collections::BTreeMap<u64, Vec<u64>> = std::collections::BTreeMap::new();
        for state in nodes.values() {
            if let Some(parent) = state.identity.parent {
                children.entry(parent).or_default().push(state.identity.id);
            }
        }
        let mut cursor = id;
        loop {
            let next = children
                .get(&cursor)
                .into_iter()
                .flatten()
                .filter_map(|key| nodes.get(key))
                .max_by_key(|state| (state.identity.branch == branch, state.identity.id))
                .map(|state| state.identity.id);
            let Some(next) = next else { break };
            if !seen.insert(next) {
                return Err("Cyclic history path".into());
            }
            let Some(state) = nodes.remove(&next) else { break };
            cursor = next;
            descendants.push(state);
        }
        // Preserve an archived handle for the previous current document when available.
        let stored_current = self.take_current_document(current);
        for state in ancestors.iter_mut().chain(descendants.iter_mut()).chain(nodes.values_mut()) {
            if state.identity.id == self.current_identity.id {
                state.document = stored_current.clone();
            }
        }
        descendants.reverse();
        self.undo = ancestors.into();
        self.redo = descendants;
        self.branches = nodes.into_values().collect();
        self.current_identity = target.identity;
        self.current_label = target.label;
        self.current_target = target.target;
        self.current_archive = match target.document {
            StoredDocument::<P>::Archived(archive) => Some(archive),
            _ => None,
        };
        self.current_archive_document = self.current_archive.as_ref().map(|_| Arc::downgrade(&restored));
        Ok(())
    }

    /// Document of past entry `i`, indexed like [`History::<P>::entries`] (0 = oldest). The last entry is
    /// the current document, which the history does not hold, so it (and any index past it) is `None`.
    pub fn state(&self, i: usize) -> Option<Arc<P::Document>> {
        self.try_state(i).ok().flatten()
    }

    /// Metadata-only query for menus; never hydrates a cold snapshot.
    pub fn has_past_selection(&self) -> bool {
        self.undo.iter().any(|state| state.has_selection)
    }

    /// Most recent selection-bearing state, found without reading scratch storage.
    pub fn latest_selection_state(&self) -> Option<usize> {
        self.undo.iter().enumerate().rev().find_map(|(index, state)| state.has_selection.then_some(index))
    }

    pub fn resident_state(&self, i: usize) -> Option<Arc<P::Document>> {
        self.undo.get(i).and_then(|state| match &state.document {
            StoredDocument::<P>::Resident(doc) => Some(doc.clone()),
            _ => None,
        })
    }

    pub fn try_state(&self, i: usize) -> Result<Option<Arc<P::Document>>, String> {
        self.undo.get(i).map(|state| state.document.load()).transpose()
    }

    /// Resident snapshots only; this never reads scratch storage.
    pub fn resident_states(&self) -> Vec<Arc<P::Document>> {
        let mut seen = HashSet::new();
        self.undo
            .iter()
            .chain(self.redo.iter())
            .chain(self.branches.iter())
            .filter_map(|state| state.document.resident().cloned())
            .chain(self.current_archive.iter().filter_map(|archive| archive.resident().cloned()))
            .filter(|doc| seen.insert(Arc::as_ptr(doc) as usize))
            .collect()
    }

    /// Concrete snapshots eligible for scratch serialization. Compact replay backing stays resident.
    pub fn spillable_states(&self) -> Vec<Arc<P::Document>> {
        self.undo
            .iter()
            .chain(self.redo.iter())
            .chain(self.branches.iter())
            .filter_map(|state| match &state.document {
                StoredDocument::<P>::Resident(doc) => Some(doc.clone()),
                _ => None,
            })
            .collect()
    }

    /// State descriptors and compact replay records, separate from pixel payload accounting.
    pub fn metadata_bytes(&self) -> usize {
        let mut total = self.current_label.capacity().saturating_add(P::target_bytes(&self.current_target));
        for state in self.undo.iter().chain(self.redo.iter()).chain(self.branches.iter()) {
            total = total
                .saturating_add(std::mem::size_of::<HistoryState<P>>())
                .saturating_add(state.label.capacity())
                .saturating_add(P::target_bytes(&state.target));
            if let StoredDocument::<P>::Archived(archive) = &state.document {
                total = total.saturating_add(archive.metadata_bytes());
            }
        }
        if let Some(archive) = &self.current_archive {
            total = total.saturating_add(archive.metadata_bytes());
        }
        total
    }

    pub fn archived_states(&self) -> usize {
        self.undo
            .iter()
            .chain(self.redo.iter())
            .chain(self.branches.iter())
            .filter(|state| matches!(&state.document, StoredDocument::<P>::Archived(archive) if archive.resident().is_none()))
            .count()
    }

    /// Adjacent resident snapshots pinned for immediate undo/redo.
    pub fn hot_states(&self) -> Vec<Arc<P::Document>> {
        self.undo
            .back()
            .into_iter()
            .chain(self.redo.last())
            .filter_map(|state| state.document.resident().cloned())
            .chain(self.current_archive.iter().filter_map(|archive| archive.resident().cloned()))
            .collect()
    }

    /// Whether this allocation is the immediate undo or redo state.
    /// Used to revalidate asynchronous spill results after intervening navigation.
    pub fn is_hot(&self, doc: &Arc<P::Document>) -> bool {
        self.undo.back().and_then(|state| state.document.resident()).is_some_and(|hot| Arc::ptr_eq(doc, hot))
            || self.redo.last().and_then(|state| state.document.resident()).is_some_and(|hot| Arc::ptr_eq(doc, hot))
    }

    /// Oldest eligible snapshot, keeping the immediate undo and redo states hot.
    pub fn spill_candidate(&self) -> Option<Arc<P::Document>> {
        self.undo
            .iter()
            .take(self.undo.len().saturating_sub(1))
            .chain(self.redo.iter().take(self.redo.len().saturating_sub(1)))
            .filter_map(|state| state.document.resident())
            .find(|doc| {
                !self.undo.back().and_then(|state| state.document.resident()).is_some_and(|hot| Arc::ptr_eq(doc, hot))
                    && !self.redo.last().and_then(|state| state.document.resident()).is_some_and(|hot| Arc::ptr_eq(doc, hot))
            })
            .cloned()
    }

    /// Replace only the allocation actually archived; stale worker results are harmless.
    pub fn replace_resident(&mut self, doc: &Arc<P::Document>, archive: Arc<dyn ArchivedDocument<P>>) -> bool {
        let mut replaced = false;
        for state in self.undo.iter_mut().chain(self.redo.iter_mut()).chain(self.branches.iter_mut()) {
            if matches!(&state.document, StoredDocument::<P>::Resident(resident) if Arc::ptr_eq(resident, doc)) {
                state.document = StoredDocument::<P>::Archived(archive.clone());
                replaced = true;
            }
        }
        replaced
    }

    /// Budget fallback, preserving the immediate undo and redo steps.
    pub fn drop_oldest(&mut self) -> bool {
        if !self.branches.is_empty() {
            self.branches.remove(0);
            return true;
        }
        if self.undo.len() > 1 {
            self.undo.pop_front();
            true
        } else if self.redo.len() > 1 {
            self.redo.remove(0);
            true
        } else {
            false
        }
    }

    /// Explicit preference reduction may retire even the final undo/redo state.
    /// The current document is never owned by these stacks.
    pub fn discard_oldest(&mut self) -> bool {
        if !self.branches.is_empty() {
            self.branches.remove(0);
            return true;
        }
        if self.undo.pop_front().is_some() {
            true
        } else if !self.redo.is_empty() {
            self.redo.remove(0);
            true
        } else {
            false
        }
    }

    /// Enforce a lowered history-count preference without reading scratch storage.
    pub fn enforce_state_limit(&mut self) -> usize {
        if self.max_states == 0 {
            return 0;
        }
        let limit = self.max_states.max(1);
        let mut removed = 0;
        while self.undo.len().saturating_add(self.redo.len()).saturating_add(self.branches.len()) > limit {
            if !self.discard_oldest() {
                break;
            }
            removed += 1;
        }
        removed
    }

    /// Number of past states (entries before the current one).
    pub fn past_len(&self) -> usize {
        self.undo.len()
    }

    /// Rename the current step (e.g. after folding several steps into one with [`Self::purge_last`]).
    pub fn set_current_label(&mut self, label: impl Into<String>) {
        self.current_label = label.into();
    }

    /// Forget the most recent undo state and every redo state (Edit › Purge › Undo): the
    /// last step can no longer be undone and its pixels are released.
    pub fn purge_last(&mut self) -> bool {
        self.redo.clear();
        self.undo.pop_back().is_some()
    }

    /// Forget the redo states, e.g. after undoing half of a compound step that failed.
    pub fn clear_redo(&mut self) {
        self.redo.clear();
    }

    /// Invalidate replay provenance after a successful edit that changes the current document.
    pub fn clear_current_archive(&mut self) {
        self.current_archive = None;
        self.current_archive_document = None;
    }

    pub fn set_current_archive(&mut self, current: &Arc<P::Document>, archive: Arc<dyn ArchivedDocument<P>>) {
        self.current_archive_document = Some(Arc::downgrade(current));
        self.current_archive = Some(archive);
    }

    fn take_current_document(&mut self, current: Arc<P::Document>) -> StoredDocument<P> {
        let matches = self.current_archive_document.take().and_then(|bound| bound.upgrade()).is_some_and(|bound| Arc::ptr_eq(&bound, &current));
        match self.current_archive.take() {
            Some(archive) if matches => StoredDocument::<P>::Archived(archive),
            _ => StoredDocument::<P>::Resident(current),
        }
    }

    pub fn clear(&mut self) {
        self.clear_current_archive();
        self.undo.clear();
        self.redo.clear();
        self.branches.clear();
        self.current_identity.parent = None;
    }

    /// Approximate unique tile and shared binary-payload bytes held by history.
    pub fn unique_bytes(&self, current: &P::Document) -> usize {
        let mut seen = HashSet::new();
        P::document_bytes(current, &mut seen);
        self.resident_states().iter().fold(0usize, |n, doc| n.saturating_add(P::document_bytes(doc, &mut seen)))
    }

    /// Managed bytes of `current` plus the allocations only history holds (what [`History::<P>::trim`]
    /// bounds by [`History::<P>::max_bytes`]).
    pub fn pixel_bytes(&self, current: &P::Document) -> usize {
        let mut seen = HashSet::new();
        let own = P::document_bytes(current, &mut seen);
        self.resident_states().iter().fold(own, |n, doc| n.saturating_add(P::document_bytes(doc, &mut seen)))
    }

    /// Keep pixel memory within [`History::<P>::max_bytes`]: the current document's tiles plus the
    /// tiles only history holds (newest states first). The oldest undo states that don't fit are
    /// dropped; the most recent one is always kept so the last step can be undone. Returns how
    /// many states were dropped.
    pub fn trim(&mut self, current: &P::Document) -> usize {
        if self.max_bytes == 0 || self.undo.len() <= 1 {
            return 0;
        }
        let mut seen = HashSet::new();
        let mut total = P::document_bytes(current, &mut seen);
        for s in self.redo.iter().rev() {
            if let Some(doc) = s.document.resident() {
                total = total.saturating_add(P::document_bytes(doc, &mut seen));
            }
        }
        let mut keep = 0;
        for s in self.undo.iter().rev() {
            if let Some(doc) = s.document.resident() {
                total = total.saturating_add(P::document_bytes(doc, &mut seen));
            }
            if total > self.max_bytes && keep >= 1 {
                break;
            }
            keep += 1;
        }
        let drop = self.undo.len() - keep;
        self.undo.drain(..drop);
        drop
    }
}

//! The app integration contract (spec §3.6 phase A, §4.6): how any Craft app adapts its
//! document to the generic object/property model.
//!
//! An adapter produces a [`DocumentView`] (the object tree) from the app's own document and
//! applies changes back; it defines the [`DocSchema`] that validates every change against the
//! app's kinds and bounds; and it projects the app's live UI state into presence. The minimum
//! viable adapter is *three registration functions, a property table, and a simulation test*
//! (spec §18.1) — the `crafthub-adapter` template in `adapters/` shows the shape.

use crate::change::Change;
use crate::ids::NodeId;
use crate::limits::Limits;
use crate::value::Value;
use std::sync::Arc;

/// The validation context handed to [`DocSchema::validate`].
pub struct ValidationCtx<'a> {
    /// The document the change would land on.
    pub replica: &'a crate::replica::Replica,
    /// The author's client space (new ids must come from it).
    pub author_space: u32,
    /// The enforced limits.
    pub limits: &'a Limits,
}

/// The per-app document contract: what kinds exist, what parents accept, what values fit.
pub trait DocSchema {
    /// Validate one change against the app's rules (the tree rules + type/size bounds).
    ///
    /// The core already checks: ids exist, parents exist, no cycles, one page at least, id
    /// spaces, and finite numbers (spec §5.4). The schema adds the app's own knowledge:
    /// which kinds a parent accepts, per-property value ranges, per-kind property tables.
    fn validate(&self, change: &Change, ctx: &ValidationCtx<'_>) -> Result<(), String>;

    /// Run the normalization pass over derived data and return the follow-up diff (the author
    /// alone recomputes — spec §5.7 — so no two clients ever fight over a derived value).
    fn recompute(&self, touched: &[NodeId]) -> Vec<Change>;

    /// The file's limits (layer counts, sizes — from the server's configuration).
    fn limits(&self) -> &Limits;
}

/// The bridge between an app's document and the generic model (spec §4.6, verbatim shape).
pub trait HubAdapter {
    /// The app's own document type.
    type Doc;

    /// Produce the object/property view of the app's document (what syncs).
    fn view(&self, doc: &Self::Doc) -> DocumentView;

    /// Apply changes to the app's document (lenient for clients, per spec §5.5).
    fn apply(&self, doc: &mut Self::Doc, changes: &[Change]) -> ApplyReport;

    /// The cheap diff between two document states (structural sharing makes it edit-sized).
    fn diff(&self, before: &Self::Doc, after: &Self::Doc) -> Vec<Change>;

    /// Project the app's live UI state into a presence message (per-app projections, spec §6.1).
    fn presence(&self, doc: &Self::Doc, ui: &UiState) -> Option<crate::presence::PresenceMsg>;

    /// The app's document schema.
    fn schema(&self) -> &dyn DocSchema;
}

/// The object/property view of an app document (spec §4.6).
#[derive(Clone, Debug, Default)]
pub struct DocumentView {
    /// The nodes in the tree.
    pub nodes: Vec<ViewNode>,
    /// The document-level entries.
    pub entries: Vec<(String, Value)>,
    /// The document-level properties.
    pub doc: Vec<(String, Value)>,
}

/// One node in a view.
#[derive(Clone, Debug)]
pub struct ViewNode {
    /// The node id.
    pub id: NodeId,
    /// The kind.
    pub kind: Arc<str>,
    /// The parent.
    pub parent: NodeId,
    /// The position key.
    pub position: String,
    /// The properties.
    pub props: Vec<(Arc<str>, Value)>,
}

/// What applying a batch did (for the caller's bookkeeping).
#[derive(Clone, Debug, Default)]
pub struct ApplyReport {
    /// `Changes` that applied.
    pub applied: usize,
    /// `Changes` skipped (lenient mode — deleted targets and the like).
    pub skipped: usize,
    /// Follow-up recomputes the author should compose (spec §5.7).
    pub follow_up: Vec<Change>,
}

/// The live UI state an adapter projects into presence (spec §6.1's per-app rows).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiState {
    /// The cursor in page coordinates, plus the page id.
    pub cursor: Option<(f64, f64)>,
    /// The current page (canvas apps) or slide (deck apps).
    pub page: Option<u64>,
    /// The selected object ids.
    pub selection: Vec<NodeId>,
    /// The viewport rectangle (x, y, w, h).
    pub viewport: Option<(f64, f64, f64, f64)>,
    /// Editing a text object: node + caret + selection.
    pub editing_text: Option<(NodeId, usize, Option<usize>)>,
    /// The playhead position for timeline apps (`FilmCraft`/`EffectCraft`).
    pub playhead: Option<u64>,
    /// The track range being edited.
    pub track_range: Option<(u32, u64, u64)>,
    /// A generation job in flight (`ArtCraft`: job id + model).
    pub generating: Option<(String, String)>,
}

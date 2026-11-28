//! The places a dock area drew in a pass where a press starts a dock gesture: tabs, group bars, a tab strip's overflow
//! button, a floating window's grip and resize grips, and the dividers. A host names them for its own automation (an
//! element id per source) and reads which one holds key focus by the source's node id.

use crate::runtime::windows::ResizeEdge;
use egui::{Id, Rect, ViewportId};

/// A place in a dock area where a press starts a dock gesture, as the area drew it this pass.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DockSource<P> {
    /// What the gesture acts on.
    pub kind: SourceKind<P>,
    /// Where it was drawn: in the area's layer space, or in the OS window's own for a floating window in one.
    pub rect: Rect,
    /// The OS window it is in: `ViewportId::ROOT` unless it is in a floating window's own OS window.
    pub viewport: ViewportId,
    /// The id of its accessibility node, which [`focused_node`](crate::runtime::access::focused_node) reports while
    /// it holds key focus.
    pub id: Id,
}

/// A replay of the view that drew a source at another place moves its rect; what it acts on, its OS window and its node
/// stay.
impl<P: Clone> crate::runtime::Shift for DockSource<P> {
    fn shifted(&self, by: &egui::RecordShift) -> Self {
        DockSource { rect: self.rect.shifted(by), ..self.clone() }
    }

    fn snapped(&self, to: &Self, within: f32) -> Self {
        DockSource { rect: self.rect.snapped(&to.rect, within), ..self.clone() }
    }
}

impl<P> DockSource<P> {
    /// `kind` drawn at `rect` in the app window, with the node `id`.
    pub fn new(kind: SourceKind<P>, rect: Rect, id: Id) -> Self {
        DockSource { kind, rect, viewport: ViewportId::ROOT, id }
    }
}

/// What a [`DockSource`] acts on.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SourceKind<P> {
    /// The tab of a panel: a click shows it, a drag moves it.
    Tab(P),
    /// The tab of a panel's document ([`PanelSpec::documents`](super::PanelSpec::documents)), named by the panel and
    /// the document's id: a click shows the panel and picks the document. It is not dragged.
    Document(P, u64),
    /// The bar a group is dragged by, named by the group's first panel.
    GroupBar(P),
    /// The overflow button of a group's tab strip, named by the group's first panel: a click opens the menu of the tabs
    /// that do not fit. Only while some tabs do not fit.
    Overflow(P),
    /// The band a floating window is moved by, named by the window's first panel.
    WindowGrip(P),
    /// A grip a floating window is resized by, named by the window's first panel, with the edge or corner it moves.
    WindowResize(P, ResizeEdge),
    /// A divider a drag resizes the panes beside.
    Divider(DividerKind<P>),
}

/// Which divider a [`SourceKind::Divider`] is.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DividerKind<P> {
    /// The divider of the docked tree's split at this path from the root (`false` for the first child).
    Split(Vec<bool>),
    /// The divider under a group of a column, between it and the next open group, named by the group's first panel.
    Groups(P),
    /// The inner edge of a docked column, named by the column's first panel.
    ColumnEdge(P),
}

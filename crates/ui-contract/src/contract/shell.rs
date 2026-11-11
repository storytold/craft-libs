//! How a body reads the window skeleton around it.

use super::{BodyCx, PaneEdge};
use crate::runtime::env::{RevealEdge, ShellCx, ShellEntry};

impl BodyCx<'_> {
    /// The skeleton around this body, when the body runs inside an `AppShell` (or under [`ShellCx::put`]).
    pub fn shell(&self) -> Option<ShellCx> {
        self.env.get::<ShellEntry>().0
    }
}

impl PaneEdge {
    /// The edge reveal that belongs to this pane edge: `None` for the bottom edge and the centre, which never reveal.
    pub fn reveal_edge(self) -> Option<RevealEdge> {
        match self {
            PaneEdge::Top => Some(RevealEdge::Top),
            PaneEdge::Start => Some(RevealEdge::Start),
            PaneEdge::End => Some(RevealEdge::End),
            _ => None,
        }
    }
}

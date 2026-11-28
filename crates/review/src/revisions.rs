//! The changes between a document and a revised copy, to accept or reject one by one (N5.10).

/// What a change did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RevisionKind {
    /// A new element in the revised copy.
    Inserted,
    /// An element missing from the revised copy.
    Deleted,
    /// An element present in both copies that differs.
    Changed,
}

/// One change, as the app's compare found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    #[allow(missing_docs)]
    pub kind: RevisionKind,
    /// The app's heading, a link to the place ("Slide 3 changed", "Slide inserted after 2").
    pub heading: String,
    /// Up to eight detail lines are shown ("Title text changed").
    pub details: Vec<String>,
}

/// What the revisions panel asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RevisionAction {
    /// The heading was clicked: go to the change.
    Go(usize),
    /// Accept the change at this index.
    Accept(usize),
    /// Reject the change at this index.
    Reject(usize),
    /// Accept every change.
    AcceptAll,
    /// Reject every change.
    RejectAll,
    /// "End Review" once no differences are left.
    EndReview,
}

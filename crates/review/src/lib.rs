//! Review: comment threads, annotations, signatures, version history and revisions; presence,
//! voting, cursor chat and joining over the app's transport ([`collab`]). With the `ui` feature,
//! [`ui`] has the review widgets.
//!
//! Review data is the app's document data: every widget reads it and reports edits; nothing here
//! owns or stores it.

pub mod annotate;
pub mod comments;
pub mod revisions;
pub mod signature;
pub mod versions;

pub mod collab;

#[cfg(feature = "ui")]
pub mod ui;

/// The stories and word sets the integration tests register.
#[cfg(feature = "screen-testing")]
pub mod testing {
    craft_ui::testing_registered!();
}

#[cfg(test)]
mod annotate_tests;
#[cfg(test)]
mod versions_tests;

#[cfg(feature = "ui")]
pub use annotate::AnnotationEdit;
pub use annotate::{Annotation, AnnotationGroup, AnnotationHandle, AnnotationHit, AnnotationId, AnnotationKind, AnnotationTool};
pub use comments::{CommentAnchor, CommentFilter, CommentMessage, CommentSort, CommentThread, Reaction, ReviewStatus, ThreadId};
pub use revisions::{Revision, RevisionAction, RevisionKind};
pub use signature::{SavedSignature, SignatureDraft, SignatureImage, SignatureTab};
pub use versions::{Branch, BranchId, BranchState, HistoryRow, Version, VersionId};

#[cfg(all(test, feature = "screen-testing"))]
extern crate self as craft_review;
#[cfg(all(test, feature = "screen-testing"))]
#[path = "../tests/it/main.rs"]
mod integration;

#[cfg(test)]
fn main() {
    craft_ui::testing::run(module_path!(), env!("CARGO_PKG_NAME"), &craft_ui::testing::registered_in(env!("CARGO_PKG_NAME")));
}

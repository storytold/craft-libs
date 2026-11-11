//! The review widgets.
//!
//! - [`comments`] has the [`CommentsPanel`].
//! - [`thread`] has [`CommentPins`], the [`ThreadPopover`] and the [`CommentComposer`].
//! - [`annotate`] has the [`AnnotateTool`] (a canvas tool), the [`AnnotationOverlay`] and the
//!   [`AnnotationToolbar`].
//! - [`avatar`] has the [`Avatar`] every person is shown with.
//! - [`signature`] has the [`SignaturePadDialog`] and [`paint_signature`].
//! - [`versions`] has the [`VersionHistoryPanel`] and the [`VersionPreviewBanner`].
//! - [`revisions`] has the [`RevisionsPanel`].
//! - Collaboration: [`presence`] ([`PeopleBar`], [`RemoteCursors`], [`FollowFrame`]), [`vote`]
//!   ([`VotePill`]), [`chat`] ([`CursorChatBox`]) and [`join`] ([`JoinField`]).

pub mod annotate;
pub mod avatar;
pub mod chat;
pub mod comments;
pub mod join;
pub mod presence;
pub mod revisions;
pub mod signature;
pub mod thread;
pub mod versions;
pub mod vote;

pub use annotate::{AnnotateTool, AnnotateWords, AnnotationOverlay, AnnotationToolbar, AnnotationToolbarOutput, PALETTE};
pub use avatar::{Avatar, AvatarOutput, paint_avatar};
pub use chat::{ChatWords, CursorChatBox, CursorChatBoxOutput, ReviewCommand, paint_chat_bubble};
pub use comments::{CommentEdit, CommentsPanel, CommentsPanelOutput, CommentsPanelState, CommentsWords};
pub use join::{JoinField, JoinFieldOutput, JoinWords};
pub use presence::{
    FollowCommand, FollowFrame, FollowFrameOutput, PeopleBar, PeopleBarOutput, PresenceWords, RemoteCursor, RemoteCursors, paint_pointer,
};
pub use revisions::{RevisionsPanel, RevisionsPanelOutput, RevisionsWords};
pub use signature::{SignaturePadDialog, SignaturePadResult, SignatureSettings, SignatureWords, paint_signature};
pub use thread::{
    CommentComposer, CommentComposerOutput, CommentPin, CommentPins, CommentPinsOutput, PinCluster, PinMove, REACTIONS, ThreadPopover,
    ThreadPopoverOutput, ThreadWords, cluster, insert_mention, mention_matches, mention_query,
};
pub use versions::{VersionAction, VersionHistoryPanel, VersionHistoryPanelOutput, VersionPreviewBanner, VersionPreviewBannerOutput, VersionsWords};
pub use vote::{VotePill, VotePillOutput, VoteWords};

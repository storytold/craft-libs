//! Comment pins on a canvas, the thread popover and the composer.
//!
//! ```ignore
//! let pins = CommentPins::new("Comment pins", &pins, "Ana").open(open).show(ui);
//! if let Some(thread) = pins.opened { open = Some(thread) }
//! ```

mod mention;
pub(crate) mod note_popup;
pub(crate) mod pins;
pub(crate) mod popover;
#[cfg(feature = "suite")]
pub(crate) mod stories;

pub use mention::{insert_mention, mention_matches, mention_query};
pub use note_popup::{NOTE_POPUP_LIMITS, NoteDrafts, NotePopup, NotePopupMessage, NotePopupOutput, NoteReplyLook};
pub use pins::{CommentPin, CommentPins, CommentPinsOutput, PinCluster, PinMove, cluster};
pub use popover::{CommentComposer, CommentComposerOutput, ComposerClose, ComposerDraft, REACTIONS, ThreadPopover, ThreadPopoverOutput};

craft_ui::words! {
    /// The thread popover's and composer's words, English by default.
    #[non_exhaustive]
    pub struct ThreadWords {
        reply: "thread.reply" = "Reply";
        add_comment: "thread.addComment" = "Add a comment";
        post: "thread.post" = "Post";
        save: "thread.save" = "Save";
        edit: "thread.edit" = "Edit";
        delete: "thread.delete" = "Delete";
        delete_thread: "thread.deleteThread" = "Delete thread";
        more: "thread.more" = "More";
        cancel: "thread.cancel" = "Cancel";
        close: "thread.close" = "Close";
        resolve: "thread.resolve" = "Resolve";
        reopen: "thread.reopen" = "Reopen";
        copy_link: "thread.copyLink" = "Copy link";
        mark_unread: "thread.markUnread" = "Mark as unread";
        react: "thread.react" = "Add reaction";
        pin: template "thread.pin" = "Comment by {author}";
        pin_text: template "thread.pinText" = "Comment by {author}: {words}";
        cluster: template "thread.cluster" = "{n} comments";
        note_title: template "thread.noteTitle" = "{kind} – {author}";
    }
}

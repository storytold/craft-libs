# craft-review

Review for Craft apps, from PdfCraft (Acrobat), FigCraft (Figma) and DeckCraft (PowerPoint).

The core is egui-free:

- `comments`: comment threads anchored to a page point, a canvas point, a slide or a time, with messages, reactions, review statuses (Accepted, Rejected, Cancelled, Completed), checkmarks, sorting and search.
- `annotate`: Acrobat's annotation kinds and tools, their geometry, handles and hit tests, and the edits a drawing or editing gesture reports.
- `signature`: saved signatures and initials (drawn, typed, image) and the signature pad's draft.
- `versions`: saved versions and branches, with autosaves grouped.
- `revisions`: the changes of a compared copy, to accept or reject.
- `collab`: participants and their cursors, voting sessions, cursor chat and joining by code, over a `CollabTransport` the app implements. The library has no network.

With the `ui` feature, `craft_review::ui` has the `CommentsPanel`, `CommentPins`, `ThreadPopover` and `CommentComposer`, the `AnnotateTool` (a canvas tool) with its `AnnotationOverlay` and `AnnotationToolbar`, the `SignaturePadDialog`, the `VersionHistoryPanel`, the `RevisionsPanel`, and the collaboration widgets `PeopleBar`, `RemoteCursors`, `VotePill`, `CursorChatBox` and `JoinField`.

Stories are `#[story]` functions in `src/ui/*/stories.rs`; they register in `craft_ui::testing::stories()` and the `craft-ui-gallery` viewer lists them by the type they show. The collaboration widgets run over a fake `CollabTransport`.

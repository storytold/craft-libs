//! The words of the Pen (`pen.*`).

craft_ui::words! {
    /// The Pen's words, English by default. The anchor template takes `{i}` and `{n}`.
    pub struct PenWords {
        /// The undo step of a drawn path and the accessible name of the draft.
        path: "pen.path" = "Path";
        /// The accessible name of the draft's anchors as a group.
        group: "pen.group" = "Path being drawn";
        /// An anchor's accessible name: "Anchor 3 of 5".
        anchor: template "pen.anchor" = "Anchor {i} of {n}";
        /// What the draft is for assistive technology: the keys that move its anchors afterwards.
        description: "pen.description" = "Open path. Enter or Escape ends it; Direct Selection moves its anchors with the arrow keys.";
    }
}

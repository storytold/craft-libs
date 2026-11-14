//! The footer of the Paths panel.

/// A button of the Paths footer.
// P3(tools-w10): tl.T34 the seven footer buttons
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PathsFooterButton {
    /// Fill the path with the foreground colour; disabled while the document cannot be filled.
    Fill,
    /// Stroke the path.
    Stroke,
    /// Load the path as a selection; a row dropped on it is loaded.
    Load,
    /// Make a work path from the selection.
    WorkPath,
    /// Add a vector mask from the path to the layer.
    VectorMask,
    /// Create a new path; a row dropped on it is duplicated.
    New,
    /// Delete the path; a row dropped on it is deleted.
    Delete,
}

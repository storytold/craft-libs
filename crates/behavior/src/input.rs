//! What a component takes from the user, which decides its default key use.

/// The input a component takes.
// F2-MERGE: contract (`Input` is a base type no F2 track owns).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Input {
    /// A control the pointer acts on.
    Pointer,
    /// A text field.
    Text,
    /// Shows something; takes no input.
    Display,
    /// An editor with its own caret.
    Editor,
}

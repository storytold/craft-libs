//! The mock editor window: the shell, title bar, rails, dock, document tabs, canvas and dialogs of a document editor,
//! composed from library components only.

mod commands;
pub(crate) mod editor;
mod model;
mod panels;
pub(crate) mod stories;
pub(crate) mod tools;
mod words;

pub use commands::AppCommand;
pub use editor::EditorMock;
pub use model::{ColorPair, Document, Layer, LayerId, LayerMove, MarqueeDrag, MockState, Panel, Tool, ToolKey};

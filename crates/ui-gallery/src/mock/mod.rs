//! The mock editor window: the shell, title bar, rails, dock, document tabs, canvas and dialogs of a document editor,
//! composed from library components only.

mod commands;
mod editor;
mod model;
mod panels;
mod stories;
mod tools;
mod words;

pub use commands::AppCommand;
pub use editor::EditorMock;
pub use model::{ColorPair, Document, Layer, LayerId, MarqueeDrag, MockState, Panel, Tool, ToolKey};

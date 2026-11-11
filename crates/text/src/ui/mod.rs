//! The text interface on craft-ui (feature `ui`):
//!
//! - [`editor`]: [`TextEditor`](editor::TextEditor), the text editor on a canvas, over a
//!   [`TextEditorHost`](editor::TextEditorHost) the app implements for its text and layout.
//!   The marked text of an input method is written into the app's text so it lays out like typed text.
//! - [`character`]: [`CharacterControls`](character::CharacterControls) and
//!   [`ParagraphControls`](character::ParagraphControls), Photoshop's Character and Paragraph
//!   panels as controls for a panel body, a Properties section or an options bar.
//! - [`style_panel`]: [`TextStylesPanel`](style_panel::TextStylesPanel), the Character
//!   Styles and Paragraph Styles panels.
//! - [`spelling`]: [`CheckSpellingDialog`](spelling::CheckSpellingDialog), Edit › Check
//!   Spelling on the dialog stack, and the user's words as a settings section.
//!
//! The paged-text controls (6b):
//!
//! - [`ruler`] (N3.02): [`TextRuler`](ruler::TextRuler), tab stops and indents on a ruler.
//! - [`tabs_dialog`] (N3.09): [`TabStopsDialog`](tabs_dialog::TabStopsDialog).
//! - [`character`] also has the Paragraph dialog (N3.15):
//!   [`ParagraphDialog`](character::ParagraphDialog).
//! - [`style_panel`] also has multi-row highlight and the style chip (N3.04):
//!   [`StyleChip`](style_panel::StyleChip).
//! - [`notes`] (N3.18): [`NotesEditor`](notes::NotesEditor), read-only as the presenter's view.
//! - [`columns`] (N3.27): [`TextColumnsDialog`](columns::TextColumnsDialog) and the Columns menu.
//! - [`hyperlink`] (N3.28): [`HyperlinkDialog`](hyperlink::HyperlinkDialog).
//! - `find` (N3.03, with the `find` feature): `FindBar` and `FindReplaceDialog`.

pub mod character;
pub mod columns;
pub mod editor;
#[cfg(feature = "find")]
pub mod find;
pub mod hyperlink;
mod length;
pub mod notes;
pub mod ruler;
pub mod spelling;
pub mod style_panel;
pub mod tabs_dialog;

pub use length::DocUnit;

//! Sections and title rows: `TitleRow`, `FooterBar`, `Section`, `GroupBox` and `Panel`.

pub(crate) mod footer;
pub(crate) mod group_box;
#[cfg(feature = "suite")]
mod sample;
pub(crate) mod section;
pub(crate) mod stack;
pub(crate) mod title_row;
mod words;

// Panel.
pub(crate) mod panel;

pub use footer::{
    DialogButtonLook, DialogRole, DropAction, DropActionOutput, DropActionStyle, FooterAction, FooterBar, FooterButton, FooterDrop, FooterOutput,
};
pub use group_box::{GroupBox, GroupBoxStyle};
pub use section::{Section, SectionOutput, SectionStyle};
pub use stack::{SectionStack, SectionStackOutput, SectionStackStyle, StackRows, StackSlot};
#[doc(hidden)]
pub use title_row::MenuRows;
pub use title_row::{HeaderAction, HeaderCommand, HeaderStrength, QuietTitleRow, TitleOutput, TitleRow, TitleRowStyle};
pub use words::SectionWords;

pub use panel::{Panel, PanelOutput, PanelStyle};

// Reorderable sections.
mod layout;
pub(crate) mod reorderable;

pub use layout::{SectionEvent, SectionKey, SectionLayout, SectionState};
pub use reorderable::{ReorderableSections, ReorderableSectionsOutput, SectionButton, SectionCommands, SectionSpec};
pub use words::ReorderableWords;

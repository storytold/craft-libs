//! Sections and title rows: `TitleRow`, `FooterBar`, `Section`, `GroupBox` and `Panel`.

pub(crate) mod footer;
mod group_box;
#[cfg(feature = "testing")]
mod sample;
mod section;
mod stack;
mod title_row;
mod words;

// Panel.
mod panel;

pub use footer::{
    DialogRole, DropAction, DropActionOutput, DropActionStyle, FooterAction, FooterBar, FooterButton, FooterDrop, FooterOutput, accelerator_labels,
};
pub use group_box::{GroupBox, GroupBoxStyle};
pub use section::{Section, SectionOutput, SectionStyle};
pub use stack::{SectionStack, SectionStackOutput, SectionStackStyle, StackRows, StackSlot};
#[doc(hidden)]
pub use title_row::MenuRows;
pub use title_row::{HeaderAction, HeaderCommand, HeaderStrength, TitleOutput, TitleRow, TitleRowStyle};
pub use words::SectionWords;

pub use panel::{Panel, PanelOutput, PanelStyle};

// Reorderable sections.
mod layout;
mod reorderable;

pub use layout::{SectionEvent, SectionKey, SectionLayout, SectionState};
pub use reorderable::{ReorderableSections, ReorderableSectionsOutput, SectionCommands, SectionSpec};
pub use words::ReorderableWords;

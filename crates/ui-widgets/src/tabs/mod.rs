//! `TabStrip`, `Tab` and `DocumentTabs`.

mod words;

// Tab and the strip rules.
mod rules;
pub(crate) mod tab;

pub use rules::{CloseSide, DocumentTabLook, FitMode, OverflowMenu, OverflowPlace, PanelTabLook, TabCloseLook, TabKind, close_side};
pub use tab::{Tab, TabOutput, TabPartEvent, TabRole, TabStyle};
pub use words::TabWords;

// The strip and the document tabs.
pub(crate) mod document;
pub(crate) mod strip;

pub use document::{DocumentTab, DocumentTabs, DocumentTabsEvent, DocumentTabsOutput};
pub use strip::{StripEnd, StripLayout, TabRoles, TabStrip, TabStripOutput, TabStripStyle, TornOff};
pub use words::StripWords;

//! `TabStrip`, `Tab` and `DocumentTabs`.

mod words;

// Tab and the strip rules.
mod rules;
mod tab;

pub use rules::{CloseSide, FitMode, OverflowMenu, OverflowPlace, TabKind, close_side};
pub use tab::{Tab, TabOutput, TabPartEvent, TabStyle};
pub use words::TabWords;

// The strip and the document tabs.
mod document;
mod strip;

pub use document::{DocumentTab, DocumentTabs, DocumentTabsEvent, DocumentTabsOutput};
pub use strip::{StripLayout, TabStrip, TabStripOutput, TabStripStyle, TornOff};
pub use words::StripWords;

//! `List` and its rows: `ListRow`, `FindingsList`, `EventList` and `UndoList`.

mod events;
mod findings;
mod predicate;
#[doc(hidden)]
pub mod row;
#[cfg(feature = "testing")]
mod stories;
mod toggle;
mod tree;
mod undo;
mod view;
#[cfg(feature = "testing")]
mod view_stories;
mod view_words;
mod words;

pub use events::{EventLevel, EventList, EventListStyle, EventLog, EventLogState, LevelCounts, LogEvent};
pub use findings::{Finding, Findings, FindingsList, FindingsListStyle, FindingsSummary, FindingsSummaryStyle, Severity, summary_text};
pub use predicate::{DropTest, RowTest, SourceTest};
pub(crate) use row::RowThumb;
pub use row::{
    ListRow, ListRowStyle, RowEvent, RowInk, RowOutput, RowStyleCx, ThumbAspect, ThumbPaint, ThumbSize, ThumbTarget, TrailingFace, TrailingPart,
    label_color_label,
};
pub use toggle::{ToggleCell, ToggleFace, ToggleState};
pub use tree::OpenSet;
pub use undo::{UndoList, UndoListStyle, UndoMark, UndoRows, footer_text, undo_count};
pub use view::{List, ListStyle};
pub use view_words::{EventWords, FindingsWords, UndoWords};
pub use words::ListWords;

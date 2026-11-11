//! Preset patterns on collections and overlays: `PresetBrowser`, `PresetPicker`, `PresetFooter` and `NamedPresetRow`.

mod words;

// The browser.
mod browser;

// Preset data, footer, files and panel menu.
mod event;
mod files;
mod footer;
mod model;
mod panel_menu;
mod state;

// The picker and the named row.
mod named;
mod picker;

pub use browser::{PresetBrowser, PresetItemCommands, PresetReorder, ThumbPainter};
pub use event::{PresetEvent, PresetOutput};
pub use files::{PresetExport, PresetFileEvent, PresetFileFailure, PresetFileRequest, PresetFiles};
pub use footer::{PresetFooter, PresetFooterEvent, PresetFooterOptions, PresetFooterOutput};
pub use model::{
    CellShape, GroupPath, PresetChange, PresetError, PresetGroup, PresetId, PresetItem, PresetKey, PresetKind, PresetLibrary, PresetRow, Preview,
    PreviewPaint, ThumbInfo, children_end, has_captions, nest_allowed,
};
pub use named::{NameCheck, NamedPresetCommands, NamedPresetEvent, NamedPresetOutput, NamedPresetRow};
pub use panel_menu::{PresetCommands, PresetMenuAction, PresetPanelMenu};
pub use picker::{PickerField, PickerHead, PresetPicker, PresetPickerOutput, PresetPickerStyle};
pub use state::{DefaultOpen, PresetState, PresetView, default_is_open};
pub use words::PresetWords;

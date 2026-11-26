//! Preset patterns on collections and overlays: `PresetBrowser`, `PresetPicker`, `PresetFooter`, `NamedPresetRow` and
//! `PresetManager`.

mod words;

// The browser.
pub(crate) mod browser;

// Preset data, footer, files and panel menu.
mod event;
mod files;
pub(crate) mod footer;
mod model;
mod panel_menu;
mod source;
mod state;

// The picker and the named row.
pub(crate) mod manager;
pub(crate) mod named;
pub(crate) mod picker;

pub use crate::tile_grid::GroupHeads;
pub use browser::{BrowserTools, PresetBrowser, PresetDrop, PresetItemCommands, PresetReorder, RowItems};
pub use event::{PresetEvent, PresetOutput};
pub use files::{PresetExport, PresetFileEvent, PresetFileFailure, PresetFileRequest, PresetFiles};
pub use footer::{PresetFooter, PresetFooterEvent, PresetFooterOptions, PresetFooterOutput};
pub use manager::{HostedPresetManager, PresetManager, PresetManagerOutput, PresetManagerTaken};
pub use model::{
    CellShape, GroupPath, PresetChange, PresetError, PresetGroup, PresetId, PresetItem, PresetKey, PresetKind, PresetLibrary, PresetNames, PresetRow,
    PresetRowMenu, PresetTiles, Preview, ThumbInfo, children_end, has_captions, nest_allowed,
};
pub use named::{NameCheck, NameRefuses, NamedPresetCommands, NamedPresetEvent, NamedPresetOutput, NamedPresetRow, NamedPresetStyle, Naming};
pub use panel_menu::{PresetCommands, PresetMenuAction, PresetPanelMenu};
pub use picker::{PickerField, PickerHead, PresetPicker, PresetPickerOutput, PresetPickerStyle};
pub use source::{AcceptsFn, PresetFns, PresetSource, ThumbFn};
pub use state::{DefaultOpen, PresetState, PresetView, default_is_open};
pub use words::PresetWords;

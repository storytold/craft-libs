//! `AppShell` and its presets, `TitleBar`, captions, `ToastHost`, `app::run` and the `embed` hooks.

#[cfg(feature = "app")]
pub mod app;
pub mod embed;
pub mod title_bar;
mod words;

// The caption record: the exclusive window parts a title bar registers.
mod captions;

pub use captions::{Caption, CaptionRegions, WindowHit};

// Shell interface: the slot API.
mod regions;

pub use regions::{Slot, SlotSize};

// App shell, canvas panel, screen modes and reveal.
pub(crate) mod app_shell;
pub(crate) mod canvas_shell;
pub(crate) mod opening;
pub(crate) mod options_place;
mod reveal;
mod screen_mode;
pub(crate) mod screen_mode_button;

pub use app_shell::{AppShell, AppShellOutput, ChromeState, ShellDialogsCx, ShellRegion, ShellRegionOutput, WindowDialogs, chrome_state};
pub use canvas_shell::{CanvasPanel, CanvasPanelOutput, slots};
pub use opening::{OpeningCard, OpeningCardOutput};
pub use options_place::{BarGrip, OptionsBarDrag, OptionsBarPlace};
pub use screen_mode::{ScreenMode, ScreenModeCommands, ScreenModeSet};
pub use screen_mode_button::{ScreenModeButton, ScreenModeButtonOutput};
pub use words::ShellWords;

// Start screen and app windows.
mod recent;
pub(crate) mod recent_list;
pub(crate) mod start;
pub(crate) mod viewport_window;

pub use recent::{RecentFile, RecentSort, RecentView, Template, ThumbKey, sort_recent};
pub use recent_list::RecentList;
pub use start::{
    FolderListing, PINNED_FILES_SHOWN, PinnedFile, PinnedFolder, StartAction, StartLayout, StartLink, StartLinkTarget, StartOutput,
    StartPinnedCommands, StartPlace, StartRecentCommands, StartScreen,
};
pub use viewport_window::{ViewportWindow, ViewportWindowOutput};

// Caption buttons: the four families' button, its slot layout and the icon-theme glyphs.
pub(crate) mod caption_button;
mod caption_icons;

pub use caption_button::{CaptionButton, CaptionEnd, CaptionPlan, CaptionSlot};
pub use caption_icons::{CaptionIcon, CaptionIcons, CaptionMask, RenderIconFn};
pub use craft_window::policy::{CaptionFamily, Captions, Frame, MacInset};

// A document's window: a container in its own OS viewport.
pub(crate) mod document_window;

pub use document_window::{DocumentWindow, DocumentWindowOutput, DocumentWindowStyle, Wrapped};

// The title bar: the bar, its title and its output.
pub use title_bar::{Title, TitleBar, TitleBarOutput, TitleBarStyle};

//! `AppShell` and its presets, `TitleBar`, captions, `ToastHost` and `app::run`.

#[cfg(feature = "app")]
pub mod app;
pub mod title_bar;
mod words;

// The caption record: the exclusive window parts a title bar registers.
mod captions;

pub use captions::{Caption, CaptionRegions, WindowHit};

// Shell interface: the slot API.
mod regions;

pub use regions::{Slot, SlotSize};

// App shell, canvas panel, screen modes and reveal.
mod app_shell;
mod canvas_shell;
mod reveal;
mod screen_mode;

pub use app_shell::{AppShell, AppShellOutput, ShellDialogsCx, ShellRegion, ShellRegionOutput, WindowDialogs};
pub use canvas_shell::{CanvasPanel, CanvasPanelOutput, slots};
pub use screen_mode::{ScreenMode, ScreenModeCommands};
pub use words::ShellWords;

// Start screen and app windows.
mod recent;
mod start;
mod viewport_window;

pub use recent::{RecentFile, RecentSort, RecentView, Template, ThumbKey, sort_recent};
pub use start::{StartAction, StartOutput, StartPinnedCommands, StartRecentCommands, StartScreen};
pub use viewport_window::{ViewportWindow, ViewportWindowOutput};

// Caption buttons: the four families' button, its slot layout and the icon-theme glyphs.
mod caption_button;
mod caption_icons;

pub use caption_button::{CaptionButton, CaptionEnd, CaptionPlan, CaptionSlot};
pub use caption_icons::{CaptionIcon, CaptionIcons, CaptionMask, RenderIconFn};
pub use craft_window::policy::{CaptionFamily, Captions, Frame, MacInset};

// A document's window: a container in its own OS viewport.
mod document_window;

pub use document_window::{DocumentWindow, DocumentWindowOutput, DocumentWindowStyle};

// The title bar: the bar, its title and its output.
pub use title_bar::{Title, TitleBar, TitleBarOutput, TitleBarStyle};

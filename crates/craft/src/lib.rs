//! The craft libraries in one dependency. Turn on one feature per crate and `ui` for their
//! egui side; import through `craft::<crate>`.

pub mod egui_fork;

#[cfg(all(feature = "ui-system_fonts", target_arch = "wasm32"))]
compile_error!("`ui-system_fonts` is desktop-only: the system font index needs fontique, which has no web build");
#[cfg(feature = "ai-core")]
pub use craft_ai_core as ai_core;
#[cfg(feature = "atomic")]
pub use craft_atomic as atomic;
#[cfg(feature = "audio")]
pub use craft_audio as audio;
#[cfg(feature = "automation")]
pub use craft_automation as automation;
#[cfg(feature = "browse")]
pub use craft_browse as browse;
#[cfg(feature = "cms")]
pub use craft_cms as cms;
#[cfg(feature = "color")]
pub use craft_color as color;
#[cfg(feature = "crw")]
pub use craft_crw as crw;
#[cfg(feature = "dialogs")]
pub use craft_dialogs as dialogs;
#[cfg(feature = "doc")]
pub use craft_doc as doc;
#[cfg(feature = "docpath")]
pub use craft_docpath as docpath;
#[cfg(feature = "draw")]
pub use craft_draw as draw;
#[cfg(feature = "ecproj-format")]
pub use craft_ecproj_format as ecproj_format;
#[cfg(feature = "font-catalog")]
pub use craft_font_catalog as font_catalog;
#[cfg(feature = "font-manager")]
pub use craft_font_manager as font_manager;
#[cfg(feature = "font-store")]
pub use craft_font_store as font_store;
pub use craft_host as host;
pub use craft_i18n as i18n;
#[cfg(feature = "imaging")]
pub use craft_imaging as imaging;
#[cfg(feature = "jpeg")]
pub use craft_jpeg as jpeg;
#[cfg(feature = "keymap")]
pub use craft_keymap as keymap;
#[cfg(feature = "layers")]
pub use craft_layers as layers;
#[cfg(feature = "media")]
pub use craft_media as media;
#[cfg(feature = "menu")]
pub use craft_menu as menu;
#[cfg(feature = "pages")]
pub use craft_pages as pages;
#[cfg(feature = "pcraft-format")]
pub use craft_pcraft_format as pcraft_format;
#[cfg(feature = "pdf")]
pub use craft_pdf as pdf;
#[cfg(feature = "platform")]
pub use craft_platform as platform;
#[cfg(feature = "prepress")]
pub use craft_prepress as prepress;
#[cfg(feature = "print")]
pub use craft_print as print;
#[cfg(feature = "proof")]
pub use craft_proof as proof;
#[cfg(feature = "quit")]
pub use craft_quit as quit;
#[cfg(feature = "review")]
pub use craft_review as review;
#[cfg(feature = "settings")]
pub use craft_settings as settings;
#[cfg(feature = "table")]
pub use craft_table as table;
#[cfg(feature = "text")]
pub use craft_text as text;
pub use craft_theme as theme;
#[cfg(feature = "theme-store")]
pub use craft_theme_store as theme_store;
#[cfg(feature = "timeline")]
pub use craft_timeline as timeline;
#[cfg(feature = "ui")]
pub use craft_ui as ui;
#[cfg(feature = "update")]
pub use craft_update as update;
#[cfg(feature = "vectorcraft-format")]
pub use craft_vectorcraft_format as vectorcraft_format;
#[cfg(feature = "web-api")]
pub use craft_web_api as web_api;
#[cfg(feature = "web-host")]
pub use craft_web_host as web_host;
#[cfg(feature = "window")]
pub use craft_window as window;
#[cfg(feature = "ui")]
pub use egui;

/// The types an app imports first: the host handle, a dialog stack, the dialog outcome and the theme
/// tokens.
pub mod prelude {
    pub use craft_host::{AppInfo, Host, Section};
    pub use craft_theme::Tokens;
    #[cfg(feature = "ui")]
    pub use craft_ui::{contract::Outcome, dialog::DialogStack};
}

#[cfg(test)]
extern crate self as craft;

#[cfg(test)]
#[path = "../tests/main.rs"]
mod integration;

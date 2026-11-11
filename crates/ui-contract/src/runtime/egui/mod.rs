//! The one egui adapter: the only importer of egui's behavioural APIs.

pub mod access;
pub mod alloc;
pub mod areas;
pub mod child;
pub mod clipboard;
pub mod editor;
pub mod files;
#[cfg(feature = "testing")]
pub(crate) mod fixture;
pub mod focus;
pub mod fonts;
pub mod ime;
pub mod input;
pub mod memory;
pub mod output;
pub(crate) mod painted;
pub mod panels;
pub mod pass;
pub mod plugin;
pub mod repaint;
pub mod response;
pub mod scale;
pub mod sizing;
pub mod stack;
pub mod style;
#[cfg(feature = "testing")]
pub(crate) mod surface;
pub mod text_edit;
pub mod text_layout;
pub mod textures;
pub mod viewports;
pub mod widgets;

pub mod injected;
pub mod input_filter;
pub mod screenshot;

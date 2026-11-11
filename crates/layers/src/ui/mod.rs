//! The Layers panel and its parts.
//!
//! - `tree`: the `LayerTree` trait and the data it gives the panel.
//! - `panel`: `LayersPanel`, its events, footer and menus.
//! - `rows`: `BlendRows`, the blend, opacity, lock and fill rows.
//! - `filter`: `LayerFilter` and the filter bar.
//! - `options`: `LayersSettings`, `LayersOptions` and the Layers Panel Options dialog.
//! - `thumbs`: `LayerThumb`, the thumbnail key of a row.
//! - `mock`: `FakeLayers`, a `LayerTree` for stories and tests (with the `screen-testing` feature).

pub mod filter;
#[cfg(feature = "screen-testing")]
pub mod mock;
pub mod options;
pub mod panel;
pub mod rows;
pub mod thumbs;
pub mod tree;

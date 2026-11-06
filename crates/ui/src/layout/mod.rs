//! Bounded pane geometry and resize handles, independent of workspace serialization.
//!
//! Applications retain their dock trees, fixed-versus-relative sizing, commands and pane contents.
//! [`split_rect`] lays out one split, and [`ResizeHandle`] reports an interaction without mutating
//! that application state.

mod geometry;
mod resize;
mod toolbar;

pub use geometry::{SplitAxis, SplitLayout, SplitSize, split_rect};
pub(crate) use resize::drag_focus;
pub use resize::{ResizeHandle, ResizeHandleStyle, ResizeResponse};
pub use toolbar::{ToolbarFit, fit_toolbar};

#[cfg(test)]
mod tests;

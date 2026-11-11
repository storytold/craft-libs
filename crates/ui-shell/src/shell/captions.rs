//! The caption record: where an app-drawn title bar put its caption buttons and drag areas, which the Windows hit
//! test (snap layouts) answers from. A bar registers them as exclusive parts (`PartBuilder::exclusive`) and
//! [`WindowHost::caption_regions`](crate::runtime::windows::WindowHost::caption_regions) reads them back.

/// A caption button.
pub use craft_window::Button as Caption;

pub use crate::runtime::windows::{CaptionRegions, WindowHit};

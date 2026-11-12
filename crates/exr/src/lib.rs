#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! OpenEXR handling shared by the Craft apps.
//!
//! This first part is the naming layer every reader needs once it has a file's channel list and
//! header attributes, whichever decoder produced them:
//!
//! - [`channels`]: OpenEXR layer and channel names (`diffuse.R`, `ViewLayer.Combined.R`), the
//!   channels that make a colour picture, and the channels of a layer to show as red, green, blue
//!   and alpha.
//! - [`cryptomatte`]: Cryptomatte (specification 1.2) IDs, metadata keys, manifests and rank
//!   channel names.
//!
//! Nothing here reads pixels or depends on a decoder: the functions take names and attribute text.

pub mod channels;
pub mod cryptomatte;

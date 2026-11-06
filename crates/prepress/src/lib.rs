#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Prepress for the Craft apps: an ink model and ink manager shared by the writers,
//! plates rendered from a PDF, coverage and a color preview read from the plates, and process
//! color conversion through craft-cms. See the README.

pub mod cms;
pub mod composite;
pub mod coverage;
#[cfg(feature = "flatten")]
pub mod flatten;
pub mod ink;
pub mod plates;

pub use cms::{CmsConversion, ProfileError};
pub use coverage::{Above, Coverage, InkStats, MAX_REGIONS, Region, Stats};
pub use ink::{Alternate, Ink, InkColor, InkId, InkSet, Overprint, Process, Spot, normalize_name, to_pdf_color};
pub use plates::{Conversion, Error, Fallback, MAX_PLATE_PIXELS, MAX_SPOT_INKS, Options, Plates, plates};

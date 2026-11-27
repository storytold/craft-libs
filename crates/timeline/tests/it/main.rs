//! The integration tests of craft-timeline, folded into the lib's test target: the screen tests, and with
//! the `xml` and `otio` features the interchange round trips.

#[cfg(all(feature = "xml", feature = "otio"))]
pub(crate) mod fcp7;
#[cfg(all(feature = "xml", feature = "otio"))]
pub(crate) mod fcpxml;
pub(crate) mod features;
pub(crate) mod lanes;
#[cfg(all(feature = "xml", feature = "otio"))]
pub(crate) mod otio;
pub(crate) mod panel;
pub(crate) mod ruler;
#[cfg(all(feature = "xml", feature = "otio"))]
mod support;
pub(crate) mod timecode;
pub(crate) mod transport;

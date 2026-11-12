//! The integration tests of craft-timeline, folded into the lib's test target: the screen tests, and with
//! the `xml` and `otio` features the interchange round trips.

#[cfg(all(feature = "xml", feature = "otio"))]
mod fcp7;
#[cfg(all(feature = "xml", feature = "otio"))]
mod fcpxml;
mod features;
mod lanes;
#[cfg(all(feature = "xml", feature = "otio"))]
mod otio;
mod panel;
mod ruler;
#[cfg(all(feature = "xml", feature = "otio"))]
mod support;
mod timecode;
mod transport;

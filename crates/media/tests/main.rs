//! Integration tests, built into the library's own test executable.

#[cfg(any(feature = "isobmff", feature = "matroska", feature = "mpegts", feature = "mxf", feature = "ogg"))]
mod common;
#[cfg(feature = "isobmff")]
mod fuzz;
#[cfg(feature = "matroska")]
mod fuzz_matroska;
#[cfg(feature = "avi")]
mod oracle_avi;
#[cfg(feature = "isobmff")]
mod oracle_demux;
#[cfg(feature = "matroska")]
mod oracle_matroska;
#[cfg(feature = "mpegts")]
mod oracle_mpegts;
#[cfg(feature = "isobmff")]
mod oracle_mux;
#[cfg(feature = "mxf")]
mod oracle_mxf;
#[cfg(feature = "ogg")]
mod oracle_ogg;
#[cfg(feature = "isobmff")]
mod roundtrip;

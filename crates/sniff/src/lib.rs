#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![deny(clippy::indexing_slicing)]
#![forbid(unsafe_code)]

//! What a file is, from its first bytes.
//!
//! [`identify`] reads a file's leading bytes and names its [`Kind`]: the format, with its media
//! type ([`Kind::mime_type`]), usual extension ([`Kind::extension`]) and [`Family`]. It covers the
//! formats apps most often have to tell apart before they open anything: images, audio, video,
//! 3D models, archives and compressed streams, executables and databases.
//!
//! Only signatures are read. Nothing is parsed, validated or decoded, so a file that `identify`
//! names can still be damaged, and a file it doesn't name may still be readable by its own
//! reader. Formats with no signature (plain text, CSV, ASCII FBX, binary STL) are never named.
//! Which kinds an app supports, and what it does when the name and the bytes disagree, stay
//! with the app.
//!
//! [`HEAD`] bytes are enough for almost every signature; passing more changes nothing, passing fewer
//! only loses the signatures that sit further in (TAR's at byte 257, MPEG transport streams'
//! third packet). An exception is OpenRaster ([`Kind::Ora`]): the OpenRaster specification requires
//! an uncompressed `mimetype` entry as the first file in the ZIP archive, but permits variable-length
//! ZIP extra fields. If ordinary ZIP metadata pushes the `image/openraster` payload beyond [`HEAD`]
//! bytes, [`identify`] safely falls back to [`Kind::Zip`] even if the full file was supplied.
//! Downstream callers can distinguish an intentionally bounded format hint from an exhaustive
//! format guarantee. Any input is fine: nothing here allocates, panics or reads past the slice.
//!
//! Written from the formats' public specifications and registrations (IANA media types, ISO/IEC
//! 14496-12 brands, the EBML and Matroska specifications, RIFF, the ELF, PE and Mach-O headers).

mod detect;
mod kind;

pub use detect::{HEAD, identify};
pub use kind::{Family, Kind};

#[cfg(test)]
mod tests;

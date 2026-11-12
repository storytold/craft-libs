//! The `.fig` container (Figma Design and FigJam local copies), read and written from public
//! format knowledge only.
//!
//! A `.fig` file is a zip archive holding `canvas.fig` (the document), `meta.json`, a thumbnail
//! and `images/<hash>` files. `canvas.fig` (also accepted on its own: the clipboard carries a bare
//! canvas) is an 8-byte `fig-…` magic, a little-endian `u32` version and length-prefixed chunks,
//! each compressed with deflate or zstd: a binary schema in the kiwi format ([`kiwi`]), then a
//! message encoded with it. Every file carries its own schema, so the message is decoded
//! generically and read by field name: a list of node changes (each with its id, parent id and
//! position among its siblings) that make up the layer tree, plus blobs: vector outlines
//! ([`path`]), editable vector networks ([`network`]) and the bytes of images that travel inside
//! the message ([`embedded_images`]).
//!
//! This crate is the wire layer: the archive ([`Archive`], [`write_archive`]), the canvas
//! ([`decode_canvas`], [`encode_canvas`]) and the blobs. What the node changes mean is read from
//! the decoded [`kiwi::Kv`] tree by the caller.
//!
//! Everything here reads untrusted bytes: sizes, counts and totals are bounded, and malformed
//! input is an error, never a panic.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod archive;
mod canvas;
mod images;
pub mod network;
pub mod path;
pub mod rest;
#[cfg(test)]
mod tests;

pub use craft_kiwi as kiwi;

pub use archive::{Archive, Limits, MAX_ENTRIES, Package, is_fig, write_archive};
pub use canvas::{
    DESIGN_HEADER, JAM_HEADER, MAX_CANVAS, MAX_CHUNK, MAX_CHUNKS, MAX_SCHEMA, VERSION, chunks, decode_canvas, encode_canvas, inflate, value_budget,
};
pub use images::{MAX_IMAGE_HASH, embedded_aliases, embedded_copies, embedded_images};

/// Why a `.fig` file could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The bytes are not a readable `.fig` file.
    #[error(".fig file: {0}")]
    Corrupt(String),
    /// What was handed to a writer does not fit the format's limits.
    #[error("the document is too large for a .fig file")]
    TooLarge,
    /// Writing failed.
    #[error("{0}")]
    Io(String),
}

pub(crate) fn corrupt(msg: impl Into<String>) -> Error {
    Error::Corrupt(msg.into())
}

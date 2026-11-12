//! # craft-psd
//!
//! Faithful, lossless reader and writer for Adobe Photoshop PSD (version 1)
//! and PSB (version 2, "large document format") files.
//!
//! * [`PsdFile::from_bytes`] parses a file into a format-level model;
//!   [`PsdFile::to_bytes`] writes it back. Unmodified files round-trip byte
//!   for byte: channel data is kept in its original encoding, unknown tagged
//!   blocks and resources are kept raw, and padding is preserved.
//! * Channel data is decoded lazily ([`ChannelData::decode`],
//!   [`ImageData::decode`]) with all four compression methods supported for
//!   both decoding and encoding.
//! * [`PsdBuilder`] constructs new files from RGBA/gray/CMYK buffers; the
//!   caller supplies the merged composite (this crate never composites).
//! * [`Head`] reads only the header, colour mode data and image resources (size, ICC profile,
//!   EXIF, XMP) from the start of a file, without the layers or pixels.
//! * [`descriptor`] parses and writes ActionDescriptors.
//! * [`engine_data`] parses and writes the text engine document of type layers.
//! * [`text`], [`smart`] and [`adjust`] are read-only views of the blocks a viewer needs: a type
//!   layer's text and style, a smart object's placement, warp and embedded file, an adjustment
//!   layer's settings, a fill layer's colour and the layer effects.
//! * [`PsdFile::composite_f32`] and [`PsdFile::layer_f32`] decode to straight RGBA `f32` in
//!   every colour mode and depth, within a memory budget; [`smart::SmartObject::render_placed`]
//!   bakes a warped or perspective placement.
//!
//! Implemented clean-room from Adobe's public "Photoshop File Formats
//! Specification". Undocumented details that follow MIT-licensed psd-tools
//! or ag-psd behavior are noted in comments.
//!
//! The parser never panics on malformed input and enforces limits
//! (dimensions ≤ 300000, bounded allocations, bounded descriptor nesting).
//!
//! ```
//! use craft_psd::{LayerSpec, PixelData, PsdBuilder, PsdFile};
//!
//! let mut b = PsdBuilder::new(2, 1);
//! b.push_layer(LayerSpec::new("Red", 0, 0, 2, 1, PixelData::Rgba8(vec![255, 0, 0, 255, 255, 0, 0, 128])));
//! b.composite(PixelData::Rgba8(vec![255, 0, 0, 255, 255, 0, 0, 128]));
//! let bytes = b.to_bytes()?;
//!
//! let file = PsdFile::from_bytes(&bytes)?;
//! assert_eq!(file.to_bytes()?, bytes); // byte-stable
//! let layer = file.layer(0).unwrap();
//! assert_eq!(layer.name(), "Red");
//! assert_eq!(layer.rgba8()?.data[7], 128);
//! # Ok::<(), craft_psd::PsdError>(())
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod abr;
pub mod aco;
pub mod act;
pub mod adjust;
pub mod animation;
pub mod ase;
pub mod asl;
pub mod atn;
pub mod blend;
pub mod builder;
mod codepage;
pub mod compression;
pub mod csh;
pub mod descriptor;
pub mod engine_data;
pub mod error;
pub mod file;
pub mod filter_effects;
#[cfg(any(test, feature = "testgen"))]
pub mod fixtures;
pub mod float;
pub mod grd;
pub mod hdr;
pub mod head;
pub mod header;
pub mod image_data;
mod io;
pub mod layer;
mod lazy;
pub mod metadata;
pub mod path;
pub mod patterns;
pub mod pixels;
pub mod preview;
pub mod resources;
pub mod rows;
pub mod shc;
pub mod slices;
pub mod smart;
pub mod source;
pub mod tagged;
#[cfg(any(test, feature = "testgen"))]
// Test-data generator (tests and fuzz seeds only): failing loudly is the point.
#[allow(clippy::expect_used)]
pub mod testgen;
pub mod text;
pub mod tiff;
pub mod tree;

pub use blend::BlendMode;
pub use builder::{GroupSpec, LayerSpec, MaskSpec, PixelData, PsdBuilder};
pub use compression::{Compression, PlaneLayout};
pub use descriptor::{Descriptor, VersionedDescriptor};
pub use error::{PsdError, Result};
pub use file::{GlobalLayerMask, LayerInfoPlacement, PsdFile};
pub use float::FloatImage;
pub use head::Head;
pub use header::{ColorMode, Header, Version};
pub use image_data::ImageData;
pub use layer::{BlendingRanges, ChannelData, LayerFlags, LayerInfo, LayerMask, LayerRecord, MaskData, MaskParameters, RealMask, Rect};
pub use pixels::{GrayImage, Layer, RgbaImage};
pub use preview::Preview;
pub use resources::{ImageResource, ResolutionInfo, ResourceData};
pub use rows::RowDecoder;
#[cfg(any(unix, windows))]
pub use source::FileSource;
pub use source::{ReadAt, Source, Stored};
pub use tagged::{BlockData, SectionDivider, SectionType, TaggedBlock};
pub use tree::LayerNode;

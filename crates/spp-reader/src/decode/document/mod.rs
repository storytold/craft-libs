//! `/paint/document.bin`: the layer stack in the reference app's own object serialisation. The same
//! format is used by `/paint/default_material.bin` and `/editor/layersstackstate.bin` (`...2.bin` in newer projects).
//! The walker streams the object tree to a [`Visitor`]; [`ClassCensus`] counts classes and
//! [`DocumentMaterials`] reads the texture-set records the importer needs, [`DocumentSettings`] the
//! project-wide settings that decide how the reference app builds its maps.

mod census;
mod compact;
mod context;
mod header;
mod materials;
#[cfg(test)]
mod materials_tests;
mod settings;
mod tag;
#[cfg(test)]
pub(crate) mod test_writer;
#[cfg(test)]
mod tests;
mod value;
mod walker;

pub use census::ClassCensus;
pub use context::{Context, Frame};
pub use header::{DocumentHeader, Layout};
pub use materials::{BitmapRef, ChannelRecord, DocumentMaterials, MaterialRecord};
pub use settings::{DocumentSettings, NormalMapSyscoord};
pub use tag::{Payload, ValueTag};
pub use value::Scalar;
pub use walker::{Visitor, walk};

const RECORD: &str = "document";

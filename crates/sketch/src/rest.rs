//! A `.sketch` file's pages and layers mapped onto the REST API file JSON
//! ([`craft_figma_rest`]), the shape an application's importer takes.
//!
//! Every Sketch object carries its kind in `_class`. Pages become `CANVAS` nodes; artboards and
//! frames, groups, shape paths and shape groups (with boolean operations), text, bitmaps, symbol
//! masters (components) and instances (expanded from their master with the instance's overrides,
//! laid out at the master's size), stacks (auto layout), shared styles (style definitions) and
//! swatches (colour variables) map onto their REST counterparts; prototype links, hotspots,
//! export options and layout grids come along. Anything not understood is skipped with a
//! warning; malformed input never panics.
//!
//! Paths are handed over as vector networks ([`craft_figma_rest::ext::VectorNetwork`]) beside the
//! JSON, which names each by `vectorNetworkRef`. Stars and polygons whose points are still the
//! generated ones carry `pointCount` and `innerRadius` (the extensions craft-figma-rest documents).

mod path;
mod proto;
mod shared;
mod stack;
mod style;
mod symbols;
mod text;
mod tree;

#[cfg(test)]
mod fixture;
#[cfg(test)]
mod tests;

use craft_figma_rest::ext::VectorNetwork;
use serde_json::Value;

use crate::Archive;

/// Finds the installed face a PostScript name belongs to: its family and style.
pub type FontLookup = Box<dyn Fn(&str) -> Option<(String, String)>>;

/// Most layers mapped when the caller has no budget of its own.
pub const MAX_NODES: usize = 200_000;

/// How to map a file.
pub struct Options {
    /// Most layers produced, symbol expansion included.
    pub max_nodes: usize,
    /// Finds the installed faces the file's fonts (PostScript names) belong to; families of fonts
    /// it doesn't find are read from their names.
    pub fonts: Option<FontLookup>,
}

impl Default for Options {
    fn default() -> Self {
        Options { max_nodes: MAX_NODES, fonts: None }
    }
}

/// What [`to_rest`] makes of a file.
#[derive(Debug)]
pub struct Mapped {
    /// The file JSON, in the shape [`craft_figma_rest::File`] reads.
    pub json: Value,
    /// What was skipped or approximated, each once.
    pub warnings: Vec<String>,
    /// The vector networks layers name by `vectorNetworkRef`.
    pub networks: Vec<Option<VectorNetwork>>,
    /// A text layer stored the way Sketch did before version 48 was met: the file is too old
    /// ([`crate::TOO_OLD`]) even when `meta.json` didn't say so.
    pub too_old: bool,
}

/// The REST file JSON of `archive`, named `name`.
pub fn to_rest(archive: &Archive, name: &str, options: Options) -> Mapped {
    let map = tree::to_rest(archive, name, options.max_nodes, options.fonts);
    Mapped { json: map.json, warnings: map.warnings, networks: map.networks, too_old: map.too_old }
}

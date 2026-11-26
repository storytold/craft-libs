//! The crate's integration tests, linked into one test binary.

// The shared French dictionary fixture refers to the affix module through the crate root.
use craft_proof::affix;

mod affix_compatibility;
mod french_dictionary;
mod french_packs;

//! File-format data read from or matched against project files; not product names.
//!
//! The extensions, magic bytes, XML element and compiled-graph input identifiers a project and its graph packages contain, kept
//! in one place: every other module refers to these constants, never to the literals. (The dataset paths inside a
//! project are in [`crate::project::paths`], the class and field names of its document in `model/fields/table.rs`.)

/// Extension of a project file.
pub const PROJECT_EXT: &str = "spp";
/// Extension of a compiled-graph package stored in a project.
pub const PACKAGE_EXT: &str = "sbsar";
/// [`PACKAGE_EXT`] with its dot, as resource file names and URLs end.
pub const PACKAGE_SUFFIX: &str = ".sbsar";
/// Name ending of a compiled graph inside a package.
pub const COMPILED_GRAPH_SUFFIX: &str = ".sbsasm";
/// The extension of the oldest alpha packages, met in brush tip URLs.
pub const LEGACY_ALPHA_SUFFIX: &str = ".s4sbs";
/// The first four bytes of a compiled graph.
pub const COMPILED_GRAPH_MAGIC: &[u8; 4] = b"SBAM";
/// The root element of a package's XML description.
pub const DESCRIPTION_ELEMENT: &str = "sbsdescription";
/// The closing tag of [`DESCRIPTION_ELEMENT`].
pub const DESCRIPTION_END: &[u8] = b"</sbsdescription>";
/// The compiled-graph input that carries the output size (log2 per axis); read from package descriptions.
pub const OUTPUT_SIZE_INPUT: &str = "$outputsize";
/// The compiled-graph input that carries the random seed; read from package descriptions.
pub const RANDOM_SEED_INPUT: &str = "$randomseed";
/// Mesh scene root metadata: the centre the stored vertices were moved by (vec3).
pub const SCENE_NORMALIZATION_CENTER: &str = "ALG_PRE_NORMALIZATION_CENTER";
/// Mesh scene root metadata: the radius the stored vertices were divided by.
pub const SCENE_NORMALIZATION_RADIUS: &str = "ALG_PRE_NORMALIZATION_RADIUS";
/// Mesh scene root metadata: the whole transform applied before storing (a matrix).
pub const SCENE_TOTAL_TRANSFORM: &str = "ALG_PTV_TOTAL_TRANSFORM";

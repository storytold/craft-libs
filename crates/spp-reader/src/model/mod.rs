//! Stage 3 of the import pipeline: the reference app's own concepts as typed Rust, built from the `document.bin`
//! object tree. A faithful mirror (the reference app's names, numbers and ordering); mapping to our
//! paint model is `translate`'s job. Field meanings are in the comments of each type; those marked Inferred come
//! from the corpus rather than from the reference app.
//!
//! Entry point: [`ProjectDocument::from_bytes`] → ([`ProjectDocument`], [`ModelReport`]).
//! - [`baking`]: the baking settings (`/baking/baking.ini`), a separate object tree; [`cameras`]: the viewer and
//!   scene cameras (`/editor/camera.ini`), another one; [`viewer`]: the environment map of the viewer settings.
//! - [`raw`]: the generic tree ([`RawTree`]) the builder reads.
//! - [`ProjectDocument`] → [`ProjectTextureSet`]s → [`ProjectLayer`]s (bottom → top), each with
//!   [`ChannelBlending`] entries, an optional [`ProjectMask`] and a content stack of [`ProjectEffect`]s.
//! - Effects ([`FillEffect`], [`PaintEffect`], filters, levels, …) take values from [`ProjectSource`]s; paint
//!   effects hold their brush strokes ([`ProjectStroke`]).
//! - Unknown classes become [`RawEntry`]s; [`ModelReport`] counts recognised, raw and unmirrored objects.
//! - [`fields`]: what the reader does with every field; the settings it does not handle are kept, with their values, on
//!   the nearest layer, effect, set or document ([`UnhandledSetting`]).

pub mod baking;
mod blending;
mod brush;
mod builder;
pub mod cameras;
mod channel;
mod decal;
mod document;
mod effect;
pub mod fields;
mod fill;
mod fill_projection;
mod geometry_mask;
mod layer;
mod mask;
mod paint;
mod path;
pub mod preset;
mod procedural;
pub mod raw;
mod raw_entry;
mod report;
mod source;
mod stroke;
mod summary;
#[cfg(test)]
mod tests;
mod texture_set;
pub mod viewer;

pub use blending::{ChannelBlending, ProjectBlendMode, blending_for};
pub use brush::{PhysicsEmitter, PressureResponse, ProjectBrush, RibbonBrush, StampEmitter, StrokeEmitter};
pub use channel::{ChannelMask, ProjectChannel};
pub use decal::{DecalHit, HitGranularity, HitShape};
pub use document::ProjectDocument;
pub use effect::{ColorSelectionEffect, EffectGroup, EffectHeader, FilterEffect, Levels, LevelsEffect, PolygonFillEffect, ProjectEffect};
pub use fields::{FieldEntry, Rule, SettingKind, UnhandledSetting};
pub use fill::{FillEffect, UvTransform};
pub use fill_projection::{FillPlacement, GridDeformation, GridVertex, UvSizing};
pub use geometry_mask::{GeometryMaskKind, ProjectGeometryMask};
pub use layer::{LayerKind, ProjectLayer};
pub use mask::{MaskInitial, ProjectMask};
pub use paint::{PaintEffect, PaintTool};
pub use path::{PathKey, PathKeyKind, PathSegment, StrokePath3d};
pub use preset::{PRESET_DATASET, ProjectPreset};
pub use procedural::{Procedural, ProceduralInput, ProceduralOutput, Tweak, TweakValue};
pub use raw::RawTree;
pub use raw_entry::RawEntry;
pub use report::{ClassTally, ModelReport};
pub use source::{BitmapSource, ProceduralSource, ProjectSource, ReferenceSource, STROKE_WIRE_RANDOM_START, STROKE_WIRE_SEED, UniformSource};
pub use stroke::{ProjectStroke, StoredMat4, StrokeKey, StrokeView, TimelineKey};
pub use summary::{ContentKind, StackSummary};
pub use texture_set::{ChannelDecl, MeshMapRef, ProjectTextureSet};

//! [`ProjectEffect`]: one entry of an effect stack (`DataStackActions.items`). A layer's content and its
//! mask are both such stacks, listed bottom → top like the layers themselves.

use super::blending::ChannelBlending;
use super::channel::{ChannelMask, ProjectChannel};
use super::decal::DecalHit;
use super::fields::UnhandledSetting;
use super::fill::FillEffect;
use super::paint::PaintEffect;
use super::procedural::Procedural;
use super::raw_entry::RawEntry;

#[allow(clippy::large_enum_variant, reason = "a plain data model matched by value across the importer; boxing would only move the bytes")]
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectEffect {
    /// `DataActionFill`: fill or generator (sources per channel, projection).
    Fill(FillEffect),
    /// `DataActionPaint`, `…PaintProj`, `…Eraser`, `…Smudge`: brush strokes.
    Paint(PaintEffect),
    /// `DataActionGroup`: a nested effect stack. A paint layer's strokes live in one (blend mode
    /// passthrough), so do the strokes of a painted mask.
    Group(EffectGroup),
    /// `DataActionFilterProcedural`: a filter graph applied to what is below.
    Filter(FilterEffect),
    /// `DataActionFilterLevels`.
    Levels(LevelsEffect),
    /// `DataActionFilterColorSelection`: a mask from the colours of a bitmap (ID map).
    ColorSelection(ColorSelectionEffect),
    /// `DataActionAnchor`: an anchor point other effects can reference by uid.
    Anchor(EffectHeader),
    /// `DataActionDecal`: polygon fill (picked triangles/polygons/meshes/UV islands, Inferred).
    PolygonFill(PolygonFillEffect),
    /// A class the model does not know.
    Other(RawEntry),
}

/// Fields every effect class has.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EffectHeader {
    pub uid: Option<u64>,
    pub enabled: bool,
    /// `label`; empty except for anchors and a handful of effects.
    pub label: String,
    /// `channelTypes`: the channels the effect writes (0 in masks).
    pub channels: ChannelMask,
    /// `perChannelBlending` as stored (may be empty).
    pub blending: Vec<ChannelBlending>,
    /// Fields of the effect and the objects below it (sources, brush, strokes, procedural) the reader does not
    /// handle ([`super::fields`]), with their values. Nested effects keep their own.
    pub unhandled: Vec<UnhandledSetting>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectGroup {
    pub header: EffectHeader,
    pub effects: Vec<ProjectEffect>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FilterEffect {
    pub header: EffectHeader,
    pub procedural: Option<Procedural>,
    /// `inputs`: the filter's image inputs and the channel each reads (`input` for most filters).
    pub inputs: Vec<(String, ProjectChannel)>,
    /// `inputs[].identifierAlpha`: for an input of `inputs`, the graph input fed with the coverage of what it reads
    /// (`input` → `input_Alpha`). Only the non-empty ones.
    pub input_alphas: Vec<(String, String)>,
}

/// Levels (`DataLevels`), per RGB component.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Levels {
    pub input_minimum: [f32; 3],
    pub input_maximum: [f32; 3],
    pub gamma: [f32; 3],
    pub output_minimum: [f32; 3],
    pub output_maximum: [f32; 3],
    /// `clamp` enum (0 ×811, 1 ×265 in the corpus).
    pub clamp: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LevelsEffect {
    pub header: EffectHeader,
    pub levels: Option<Levels>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColorSelectionEffect {
    pub header: EffectHeader,
    pub colors: Vec<[f32; 3]>,
    pub tolerance: f32,
    pub hardness: f32,
    /// The bitmap the colours are picked from (usually the baked ID map).
    pub bitmap_url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolygonFillEffect {
    pub header: EffectHeader,
    /// The picks (`DataDecalHitRect` / `DataDecalHitPunctual`), in order: later ones paint over earlier ones.
    pub hits: Vec<DecalHit>,
    /// Hits that could not be read (unknown class or missing position).
    pub unread: usize,
}

impl ProjectEffect {
    /// The common fields, if the class is known.
    pub fn header(&self) -> Option<&EffectHeader> {
        Some(match self {
            ProjectEffect::Fill(e) => &e.header,
            ProjectEffect::Paint(e) => &e.header,
            ProjectEffect::Group(e) => &e.header,
            ProjectEffect::Filter(e) => &e.header,
            ProjectEffect::Levels(e) => &e.header,
            ProjectEffect::ColorSelection(e) => &e.header,
            ProjectEffect::Anchor(h) => h,
            ProjectEffect::PolygonFill(e) => &e.header,
            ProjectEffect::Other(_) => return None,
        })
    }

    /// [`Self::header`], to change.
    pub fn header_mut(&mut self) -> Option<&mut EffectHeader> {
        Some(match self {
            ProjectEffect::Fill(e) => &mut e.header,
            ProjectEffect::Paint(e) => &mut e.header,
            ProjectEffect::Group(e) => &mut e.header,
            ProjectEffect::Filter(e) => &mut e.header,
            ProjectEffect::Levels(e) => &mut e.header,
            ProjectEffect::ColorSelection(e) => &mut e.header,
            ProjectEffect::Anchor(h) => h,
            ProjectEffect::PolygonFill(e) => &mut e.header,
            ProjectEffect::Other(_) => return None,
        })
    }

    pub fn is_enabled(&self) -> bool {
        self.header().is_some_and(|h| h.enabled)
    }

    /// Short kind name for summaries.
    pub fn kind(&self) -> &'static str {
        match self {
            ProjectEffect::Fill(_) => "fill",
            ProjectEffect::Paint(_) => "paint",
            ProjectEffect::Group(_) => "group",
            ProjectEffect::Filter(_) => "filter",
            ProjectEffect::Levels(_) => "levels",
            ProjectEffect::ColorSelection(_) => "color selection",
            ProjectEffect::Anchor(_) => "anchor",
            ProjectEffect::PolygonFill(_) => "polygon fill",
            ProjectEffect::Other(_) => "other",
        }
    }
}

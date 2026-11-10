//! [`ProjectSource`]: where a fill, a brush or a procedural input takes its values from
//! (`DataSourceUniform`, `DataSourceBitmap`, `DataSourceProcedural`, `DataSourceReference`).

use super::channel::ChannelMask;
use super::effect::Levels;
use super::procedural::Procedural;
use super::raw_entry::RawEntry;

/// One source. `channels` says which channels it feeds (one bit per uniform value in fills; a set of
/// channels for a material graph; 0 inside masks).
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectSource {
    /// A constant value. Colours are stored **linear** (Verified); grey channels repeat the value 3 times.
    Uniform(UniformSource),
    /// An image resource (`/<name>?version=<id>` or `resource://…`).
    Bitmap(BitmapSource),
    /// A compiled-graph material, generator or brush shape (a graph package) with its parameter values.
    Procedural(ProceduralSource),
    /// The content of an anchor point elsewhere in the stack.
    Reference(ReferenceSource),
    /// A class the model does not know.
    Other(RawEntry),
}

#[derive(Debug, Clone, PartialEq)]
pub struct UniformSource {
    pub uid: Option<u64>,
    pub channels: ChannelMask,
    pub color: [f32; 3],
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BitmapSource {
    pub uid: Option<u64>,
    pub channels: ChannelMask,
    /// `DataBitmap.urlToBitmapRes`; empty if the source has no bitmap.
    pub url: String,
}

/// The `strokeWires` bit of an alpha whose stamp cycle starts at a stamp drawn from the stroke's seed (Inferred: Ink
/// Random carries it and starts 18 stamps in; Ink Dirty, the same graph without it, starts at 0).
pub const STROKE_WIRE_RANDOM_START: u32 = 32;
/// The `strokeWires` bit of an alpha rendered with a different random seed per stamp of its cycle (Inferred: the
/// charcoal brushes, whose graph has no `stampIndex`, carry it with `stampCycleCount` 16).
pub const STROKE_WIRE_SEED: u32 = 4;

#[derive(Debug, Clone, PartialEq)]
pub struct ProceduralSource {
    pub uid: Option<u64>,
    pub channels: ChannelMask,
    /// `mode` enum: 0 single output (brush shapes, grey generators), 1 material, 2 seen on generators
    /// in masks (Observed counts: 0 ×13890, 2 ×2529, 1 ×524).
    pub mode: u64,
    /// `stampCycleCount`: how many stamps a dynamic brush alpha (one with `stampIndex`) cycles through, 0 if not stored.
    /// The reference app renders that many, `stampIndex` 0 … n − 1, with the graph's own `stampCycleCount` set to n as well.
    pub stamp_cycle: u32,
    /// `strokeWires`: a bit set over what the stroke feeds into the alpha, 0 if not stored. Seen (Inferred from the
    /// presets that carry each bit): 1 and 2 on every alpha with `stampIndex`/`stampCycleCount`; 32 on Ink Random and
    /// the Wiggly brushes, where the stamp cycle starts at a drawn stamp ([`STROKE_WIRE_RANDOM_START`]).
    pub stroke_wires: u32,
    pub procedural: Option<Procedural>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceSource {
    pub uid: Option<u64>,
    pub channels: ChannelMask,
    /// `DataReference.source`: uid of the anchor effect (`DataActionAnchor`) this reads from.
    pub anchor_uid: Option<u64>,
    /// `DataReference.sourceLabel`, often empty.
    pub anchor_label: String,
    /// `levels`: applied to what is read.
    pub levels: Option<Levels>,
    /// `DataReference.bindings`: (`source` channel bit read from the anchor, `output` channel bit written; 0 = the
    /// mask's grey).
    pub bindings: Vec<(u64, u64)>,
    /// `mode` (2 ×430, 0 ×31, 1 ×6; meaning not known).
    pub mode: u64,
}

impl ProjectSource {
    pub fn channels(&self) -> ChannelMask {
        match self {
            ProjectSource::Uniform(s) => s.channels,
            ProjectSource::Bitmap(s) => s.channels,
            ProjectSource::Procedural(s) => s.channels,
            ProjectSource::Reference(s) => s.channels,
            ProjectSource::Other(_) => ChannelMask::default(),
        }
    }

    /// Short kind name for summaries ("uniform", "bitmap", …).
    pub fn kind(&self) -> &'static str {
        match self {
            ProjectSource::Uniform(_) => "uniform",
            ProjectSource::Bitmap(_) => "bitmap",
            ProjectSource::Procedural(_) => "procedural",
            ProjectSource::Reference(_) => "reference",
            ProjectSource::Other(_) => "other",
        }
    }
}

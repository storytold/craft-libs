//! What the bits of a node's mode word and flags mean, as typed fields of [`AtomicNode`], so an importer never tests
//! raw bits. The meanings were measured on compiled graphs;
//! each accessor says where its bits live.

use super::Assembly;
use super::atomic::{AtomicNode, Kind};

/// Flags bits 8 and 9: the node repeats horizontally / vertically.
const TILE_X: u16 = 0x100;
const TILE_Y: u16 = 0x200;
/// Flags bits 4–5: what the node holds per channel. Measured: "What nodes hold and how
/// they read" (a pixel processor's ramp read back at each setting: 8 bits, 16 bits, half float, single).
const DEPTH_MASK: u16 = 0x30;
const DEPTH_INT8: u16 = 0x00;
const DEPTH_INT16: u16 = 0x10;
const DEPTH_FLOAT16: u16 = 0x20;
/// Transform mode word bits 0–4: the pyramid level it reads; bit 5: bilinear. Format data.
const TRANSFORM_MIP_MASK: u32 = 0x1f;
const TRANSFORM_BILINEAR_SHIFT: u32 = 5;
// Format data: the flags' high byte of an image stored uncompressed (see [`RawLayout`]).
const RAW_CHANNELS_MASK: u16 = 0x3;
const RAW_16_BIT: u16 = 0x4;
const RAW_JPEG: u16 = 0x8;
/// The flags' high byte of a grey input node's stored image: 16 bits and 8 bits.
const RAW_GRAY_16: u16 = 5;
const RAW_GRAY_8: u16 = 1;
/// Flags bit 8 on a gradient map: each key carries a midpoint.
const KEY_MIDPOINTS: u16 = 0x100;
/// Blend mode word bit 8: the foreground also goes into the alpha.
const BLEND_FOREGROUND_ALPHA: u32 = 0x100;
/// Blend mode word bit 12, with bit 8: premultiplied (ivy_branch #377, autumn_leaf; the only graphs that set it).
const BLEND_PREMULTIPLIED: u32 = 0x1000;
/// A transform without a mode word: pyramid level 31, bilinear.
const TRANSFORM_DEFAULT: u32 = 0x3f;
/// A channel shuffle without a mode word: every channel from itself.
const SHUFFLE_IDENTITY: u32 = 0x0302_0100;

/// What a node holds per channel (flags bits 4–5; measured from a pixel processor's ramp).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleDepth {
    Int8,
    Int16,
    Float16,
    Float32,
}

/// The channels of an uncompressed image stored in a compiled graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredChannels {
    Gray,
    Rgb,
    /// R, G, B, A in that order.
    Rgba,
}

impl StoredChannels {
    pub fn count(self) -> usize {
        match self {
            StoredChannels::Gray => 1,
            StoredChannels::Rgb => 3,
            StoredChannels::Rgba => 4,
        }
    }
}

/// The layout of an uncompressed image stored in a compiled graph, from the high byte of its node's flags: bit 0 a
/// fourth channel (alpha; for greyscale the one channel), bit 1 three colour channels, bit 2 16-bit samples. Bit 3 is a
/// JPEG instead (not raw). Measured by the stored sizes on Swirls: 0x01 grey 8, 0x05 grey 16, 0x02 RGB 8, 0x07 RGBA 16
/// (the RGBA normal map's blue is its largest channel, so the order is R, G, B, A); 0x03 and 0x06 by the same rule.
/// 16-bit samples are little-endian.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawLayout {
    pub channels: StoredChannels,
    /// Bytes per sample: 1 or 2.
    pub bytes: usize,
}

impl RawLayout {
    pub fn of_flags(flags: u16) -> Option<RawLayout> {
        let h = flags >> 8;
        if h & RAW_JPEG != 0 {
            return None;
        }
        let channels = match h & RAW_CHANNELS_MASK {
            0x1 => StoredChannels::Gray,
            0x2 => StoredChannels::Rgb,
            0x3 => StoredChannels::Rgba,
            _ => return None,
        };
        Some(RawLayout { channels, bytes: if h & RAW_16_BIT != 0 { 2 } else { 1 } })
    }

    /// The bytes of a `w` × `h` image.
    pub fn len(&self, w: u32, h: u32) -> Option<u32> {
        w.checked_mul(h)?.checked_mul(u32::try_from(self.channels.count() * self.bytes).ok()?)
    }
}

/// How a blend treats alpha (mode word bits 8 and 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlendAlpha {
    /// Bit 8: the foreground also goes into the alpha (its alpha scales the opacity either way).
    pub foreground_alpha: bool,
    /// Bits 8 and 12 together: premultiplied.
    pub premultiplied: bool,
}

/// A transform's sampling (mode word bits 0–4 the pyramid level, 31 from the scale; bit 5 bilinear rather than nearest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransformSampling {
    pub mip_level: u32,
    pub bilinear: bool,
}

/// A normal node's options (mode word bit 4 keeps the height in the alpha; bits 2–3 equal to 1: y points the other way).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalOptions {
    pub height_in_alpha: bool,
    pub flip_y: bool,
}

/// An image stored in the graph itself, uncompressed: where its samples are and how they are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredImage<'a> {
    pub width: u32,
    pub height: u32,
    pub layout: RawLayout,
    pub samples: &'a [u8],
}

impl AtomicNode {
    fn mode_or(&self, default: u32) -> u32 {
        self.mode.unwrap_or(default)
    }

    /// Whether a filter repeats the image at its edges, horizontally and vertically (flags bits 8 and 9).
    pub fn tiling(&self) -> [bool; 2] {
        [self.flags & TILE_X != 0, self.flags & TILE_Y != 0]
    }

    /// What the node holds per channel (flags bits 4–5).
    pub fn sample_depth(&self) -> SampleDepth {
        match self.flags & DEPTH_MASK {
            DEPTH_INT8 => SampleDepth::Int8,
            DEPTH_INT16 => SampleDepth::Int16,
            DEPTH_FLOAT16 => SampleDepth::Float16,
            _ => SampleDepth::Float32,
        }
    }

    /// The size the compiler worked out (the second byte of the tag: log2 of the height in the high nibble, of the width
    /// in the low one), as log2 of width and height. A node without a size expression has this size, whatever the graph's.
    pub fn size_log2(&self) -> [u8; 2] {
        [self.variant & 0xf, self.variant >> 4]
    }

    /// [`Self::size_log2`] in pixels: the size of an image stored in the node.
    pub fn stored_size(&self) -> (u32, u32) {
        let [x, y] = self.size_log2();
        (1u32 << x, 1u32 << y)
    }

    /// An input node's source word: the uid of a graph input, or else the section offset of an image stored in the graph.
    pub fn image_source(&self) -> u32 {
        self.mode_or(0)
    }

    /// How an input node's image stored uncompressed in the graph is laid out (flags high byte). A colour node takes
    /// its colour layout from [`RawLayout`]; otherwise only 0x05 (grey, 16 bits) and 0x01 (grey, 8 bits) are known.
    pub fn stored_layout(&self) -> Option<RawLayout> {
        let colour = RawLayout::of_flags(self.flags).filter(|l| self.color && l.channels != StoredChannels::Gray);
        match (self.flags >> 8, colour) {
            (_, Some(layout)) => Some(layout),
            (RAW_GRAY_16, None) => Some(RawLayout { channels: StoredChannels::Gray, bytes: 2 }),
            (RAW_GRAY_8, None) => Some(RawLayout { channels: StoredChannels::Gray, bytes: 1 }),
            _ => None,
        }
    }

    /// How a blend treats alpha (mode word bits 8 and 12); `None` for other nodes.
    pub fn blend_alpha(&self) -> Option<BlendAlpha> {
        (self.kind == Kind::Blend).then(|| {
            let m = self.mode_or(0);
            BlendAlpha {
                foreground_alpha: m & BLEND_FOREGROUND_ALPHA != 0,
                premultiplied: m & (BLEND_FOREGROUND_ALPHA | BLEND_PREMULTIPLIED) == BLEND_FOREGROUND_ALPHA | BLEND_PREMULTIPLIED,
            }
        })
    }

    /// A transform's sampling; `None` for other nodes.
    pub fn transform_sampling(&self) -> Option<TransformSampling> {
        (self.kind == Kind::Transform).then(|| {
            let m = self.mode_or(TRANSFORM_DEFAULT);
            TransformSampling { mip_level: m & TRANSFORM_MIP_MASK, bilinear: (m >> TRANSFORM_BILINEAR_SHIFT) & 1 != 0 }
        })
    }

    /// A channel shuffle's source per output channel R, G, B, A: 0–3 channels of the first input, 4–7 of the second
    /// (one byte of the mode word each); `None` for other nodes.
    pub fn channel_sources(&self) -> Option<[u32; 4]> {
        (self.kind == Kind::ChannelShuffle).then(|| {
            let m = self.mode_or(SHUFFLE_IDENTITY);
            [0, 1, 2, 3].map(|k| (m >> (8 * k)) & 0xff)
        })
    }

    /// A distance node fades by distance when it has a colour source (mode word bit 1); `None` for other nodes.
    pub fn distance_fades(&self) -> Option<bool> {
        (self.kind == Kind::Distance).then(|| (self.mode_or(0) >> 1) & 1 != 0)
    }

    /// A normal node's options; `None` for other nodes.
    pub fn normal_options(&self) -> Option<NormalOptions> {
        (self.kind == Kind::Normal).then(|| {
            let m = self.mode_or(0);
            NormalOptions { height_in_alpha: (m >> 4) & 1 != 0, flip_y: (m >> 2) & 3 == 1 }
        })
    }

    /// A gradient map's keys carry a midpoint (flags bit 8).
    pub fn key_midpoints(&self) -> bool {
        self.flags & KEY_MIDPOINTS != 0
    }

    /// A dynamic gradient runs along y, so its input picks a row (flags bit 11, decoded into the mode).
    pub fn gradient_vertical(&self) -> bool {
        self.kind == Kind::GradientDynamic && self.mode_or(0) != 0
    }

    /// A curve drawn as a horizontal ramp instead of applied to its input (flags bit 9, decoded into the mode).
    pub fn curve_ramp(&self) -> bool {
        self.kind == Kind::Curve && self.mode_or(0) != 0
    }
}

impl Assembly {
    /// The uncompressed image an input node stores in the graph at its [`AtomicNode::image_source`], if its flags give a
    /// layout and the section holds that many bytes there.
    pub fn stored_image(&self, node: &AtomicNode) -> Option<StoredImage<'_>> {
        let layout = node.stored_layout()?;
        let (width, height) = node.stored_size();
        let samples = self.section().bytes(node.image_source(), layout.len(width, height)?)?;
        Some(StoredImage { width, height, layout, samples })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(kind: Kind, color: bool, mode: Option<u32>, flags: u16) -> AtomicNode {
        AtomicNode {
            index: 0,
            opcode: 0,
            kind,
            color,
            mode,
            flags,
            variant: 0x23,
            inputs: Vec::new(),
            size: None,
            seed: None,
            params: Vec::new(),
            header_end: 0,
            program: None,
            function: None,
            keys: Vec::new(),
            curve: Vec::new(),
            strips: Vec::new(),
            value_words: 0,
        }
    }

    #[test]
    fn raw_layouts_follow_the_flag_bits() {
        let l = |f: u16| RawLayout::of_flags(f).map(|l| (l.channels, l.bytes));
        assert_eq!(l(0x0108), Some((StoredChannels::Gray, 1)));
        assert_eq!(l(0x0518), Some((StoredChannels::Gray, 2)));
        assert_eq!(l(0x0208), Some((StoredChannels::Rgb, 1)));
        assert_eq!(l(0x0718), Some((StoredChannels::Rgba, 2)));
        assert_eq!(l(0x0908), None, "a JPEG");
        assert_eq!(l(0x0008), None);
        assert_eq!(RawLayout::of_flags(0x0718).and_then(|l| l.len(2048, 2048)), Some(2048 * 2048 * 8));
        assert_eq!(RawLayout::of_flags(0x0718).and_then(|l| l.len(1 << 15, 1 << 15)), None, "overflows");
    }

    #[test]
    fn stored_layouts_of_grey_and_colour_inputs() {
        let gray = |flags| node(Kind::Input, false, Some(0), flags).stored_layout();
        assert_eq!(gray(0x0500), Some(RawLayout { channels: StoredChannels::Gray, bytes: 2 }));
        assert_eq!(gray(0x0100), Some(RawLayout { channels: StoredChannels::Gray, bytes: 1 }));
        assert_eq!(gray(0x0200), None, "a grey node knows only 0x01 and 0x05");
        let colour = node(Kind::Input, true, Some(0), 0x0700);
        assert_eq!(colour.stored_layout(), Some(RawLayout { channels: StoredChannels::Rgba, bytes: 2 }));
        assert_eq!(colour.stored_size(), (8, 4));
    }

    #[test]
    fn mode_and_flag_fields() {
        let n = node(Kind::Blend, true, Some(0x1100), 0x0310);
        assert_eq!(n.blend_alpha(), Some(BlendAlpha { foreground_alpha: true, premultiplied: true }));
        assert_eq!(node(Kind::Blend, true, Some(0x1000), 0).blend_alpha(), Some(BlendAlpha { foreground_alpha: false, premultiplied: false }));
        assert_eq!(n.tiling(), [true, true]);
        assert_eq!(n.sample_depth(), SampleDepth::Int16);
        assert_eq!(node(Kind::Blend, false, None, 0x30).sample_depth(), SampleDepth::Float32);
        assert_eq!(node(Kind::Transform, false, None, 0).transform_sampling(), Some(TransformSampling { mip_level: 31, bilinear: true }));
        assert_eq!(node(Kind::ChannelShuffle, true, None, 0).channel_sources(), Some([0, 1, 2, 3]));
        assert_eq!(node(Kind::Normal, true, Some(0x14), 0).normal_options(), Some(NormalOptions { height_in_alpha: true, flip_y: true }));
        assert_eq!(node(Kind::Normal, true, Some(0x08), 0).normal_options().map(|o| o.flip_y), Some(false), "2: an expression");
        assert_eq!(node(Kind::Distance, false, Some(2), 0).distance_fades(), Some(true));
        assert!(node(Kind::GradientMap, false, None, 0x100).key_midpoints());
        assert!(node(Kind::Curve, false, Some(1), 0).curve_ramp());
        assert_eq!(node(Kind::Levels, false, Some(1), 0).blend_alpha(), None);
    }
}

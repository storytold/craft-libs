//! Typed node records: which atomic operation a node runs, its image inputs (node indices) and its parameters,
//! each a default, an inline constant, an expression or (blend opacity) a mask image.
//!
//! Every record is laid out the same way:
//! `[mode word] [inputs] [seed expression ptr if flags bit 7] [size expression ptr if flags bit 0] [parameter data]`.
//! Parameter data follows the parameter list in order: code 1 (inline) stores the value's words, code 2 a pointer
//! to an expression, code 0 nothing (default). The 2-bit codes live in the mode word or in the flags' high byte.
//! Many operations come as a grayscale and a colour opcode (blend 0x02/0x03, levels 0x1e/0x1f, …).
//!
//! What each parameter means is taken from the graph inputs that feed it across the study corpus.

use super::expr::{Expr, Instr, Operands};
use super::fxmap::Record;
use super::node::NodeRecord;
use super::section::Section;
use crate::decode::DecodeError;

const RECORD: &str = "compiled graph node";

// Format data: the opcodes of the operations this file decodes by hand (the rest are in [`SPECS`]). The compiled graph's
// own numbering, found by reading packages.
const OP_GRADIENT_MAP_GRAY: u8 = 0x00;
const OP_GRADIENT_MAP_COLOR: u8 = 0x01;
const OP_BLEND_GRAY: u8 = 0x02;
const OP_BLEND_COLOR: u8 = 0x03;
const OP_CHANNEL_SHUFFLE: u8 = 0x07;
const OP_FXMAP_GRAY: u8 = 0x08;
const OP_FXMAP_COLOR: u8 = 0x09;
const OP_VECTOR: u8 = 0x0a;
const OP_INPUT_GRAY: u8 = 0x20;
const OP_INPUT_COLOR: u8 = 0x21;
const OP_NORMAL: u8 = 0x25;
const OP_PIXEL_PROCESSOR_GRAY: u8 = 0x28;
const OP_PIXEL_PROCESSOR_COLOR: u8 = 0x29;
const OP_DISTANCE_GRAY: u8 = 0x2a;
const OP_DISTANCE_COLOR: u8 = 0x2b;
const OP_CURVE: u8 = 0x2c;

// Format data: record flag bits (the record's u16 `flags`).
/// Bit 0: a size expression pointer follows the seed.
const FLAG_SIZE_EXPR: u16 = 0x01;
/// Bit 6: the seed as an inline number (measured on a fx-map).
const FLAG_SEED_WORD: u16 = 0x40;
/// Bit 7: a seed expression pointer.
const FLAG_SEED_EXPR: u16 = 0x80;
/// Bit 8: gradient keys carry a midpoint.
const FLAG_KEY_MIDPOINT: u16 = 0x100;
/// Bit 9 of a curve: drawn as a horizontal ramp instead of applied to the input.
const FLAG_CURVE_RAMP: u16 = 0x200;
/// Bits 8 and 11 of a colour input: a stored JPEG (bit 11) with its alpha in a second image (bit 8).
const FLAG_INPUT_JPEG_ALPHA: u16 = 0x900;
/// The high byte of the flags holds the 2-bit parameter codes of [`Codes::Flags`] operations.
const FLAG_CODES_SHIFT: u16 = 8;
/// Bits 9-10 of a gradient map: float keys (either bit), plus one extra word first (both).
const FLAG_FLOAT_KEYS_SHIFT: u16 = 9;
/// Bits 10-11 of a mode-word operation: the pixel-aspect parameter's code.
const FLAG_ASPECT_SHIFT: u16 = 10;
/// Bit 11 of a dynamic gradient: the gradient runs along y.
const FLAG_ALONG_Y_SHIFT: u16 = 11;

// Format data: the blend mode word.
/// Bits 0-3: the blending mode.
const BLEND_MODE_MASK: u32 = 0xf;
/// Bits 4-5: the opacity's source (1 inline, 2 expression, 3 mask image).
const BLEND_OPACITY_SHIFT: u32 = 4;
/// Bits 6-7: the cropping's code.
const BLEND_CROP_SHIFT: u32 = 6;
/// Bit 9: with a mask, an inline opacity follows.
const BLEND_MASK_OPACITY_CONST: u32 = 0x200;
/// Bit 10: with a mask, an opacity expression follows.
const BLEND_MASK_OPACITY_EXPR: u32 = 0x400;

/// Format data: bits 10 and up of a fx-map's mode word count its image inputs.
const FXMAP_INPUT_COUNT_SHIFT: u32 = 10;
/// Format data: the low half of a pixel processor's first word counts its inputs.
const PIXEL_PROCESSOR_INPUT_MASK: u32 = 0xffff;
/// Format data: the low 3 bits of a vector shape's block head are its type (1: a triangle strip); the rest the count.
const VECTOR_TYPE_MASK: u32 = 7;
const VECTOR_TRIANGLE_STRIP: u32 = 1;

// Format data: key layouts. u16 positions and values cover 0..1 in 65535 steps, RGBA8 colours in 255.
const U16_FULL: f32 = 65535.0;
const U8_FULL: f32 = 255.0;
/// Float gradient key: position + grey value, or position + RGBA; a float midpoint adds one word.
const FLOAT_KEY_GRAY_STRIDE: u32 = 8;
const FLOAT_KEY_COLOR_STRIDE: u32 = 20;
/// A curve key: point, left handle, right handle, two floats each.
const CURVE_KEY_STRIDE: u32 = 24;

// Read guards against corrupt counts (not format limits; no package seen comes near them).
const MAX_FLOAT_GRADIENT_KEYS: u32 = 256;
const MAX_GRADIENT_KEYS: u32 = 1024;
const MAX_CURVE_KEYS: u32 = 256;
const MAX_PIXEL_PROCESSOR_INPUTS: u32 = 32;

/// Where an operation keeps its parameter codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Codes {
    /// In the first body word (the mode word), starting at `bit`.
    ModeWord,
    /// In the high byte of the flags.
    Flags,
}

/// One parameter: its name, the bit of its 2-bit code and how many words an inline value takes.
#[derive(Debug, Clone, Copy)]
struct ParamSpec {
    name: &'static str,
    bit: u8,
    words: u8,
}

const fn p(name: &'static str, bit: u8, words: u8) -> ParamSpec {
    ParamSpec { name, bit, words }
}

/// What a known operation looks like.
struct OpSpec {
    op: u8,
    kind: Kind,
    color: bool,
    codes: Codes,
    /// Whether the first body word is a mode word (it holds the codes when `codes` is `ModeWord`).
    mode_word: bool,
    /// Image inputs before the size pointer.
    inputs: u8,
    params: &'static [ParamSpec],
}

/// Atomic operations (names as in the authoring tool where the match is clear).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Blend,
    Transform,
    GrayscaleConversion,
    ChannelShuffle,
    FxMap,
    Uniform,
    Blur,
    DirectionalBlur,
    DirectionalWarp,
    GradientWarp,
    Sharpen,
    Hsl,
    Levels,
    Input,
    PixelProcessor,
    GradientMap,
    Curve,
    Vector,
    Distance,
    Normal,
    Emboss,
    GradientDynamic,
    Unknown,
}

const LEVELS: &[ParamSpec] = &[p("in_low", 0, 1), p("in_high", 2, 1), p("in_mid", 4, 1), p("out_low", 6, 1), p("out_high", 8, 1)];
const LEVELS_COLOR: &[ParamSpec] = &[p("in_low", 0, 4), p("in_high", 2, 4), p("in_mid", 4, 4), p("out_low", 6, 4), p("out_high", 8, 4)];
const HSL: &[ParamSpec] = &[p("hue", 0, 1), p("saturation", 2, 1), p("luminosity", 4, 1)];
const TRANSFORM: &[ParamSpec] = &[p("matrix", 6, 4), p("offset", 25, 2), p("background", 28, 1)];
const TRANSFORM_COLOR: &[ParamSpec] = &[p("matrix", 6, 4), p("offset", 25, 2), p("background", 28, 4)];
const UNIFORM_GRAY: &[ParamSpec] = &[p("value", 0, 1)];
const UNIFORM_COLOR: &[ParamSpec] = &[p("color", 0, 4)];
const GRAYSCALE: &[ParamSpec] = &[p("weights", 0, 4)];
const BLUR: &[ParamSpec] = &[p("aspect", 2, 2), p("intensity", 4, 1)];
const SHARPEN: &[ParamSpec] = &[p("aspect", 2, 2), p("intensity", 4, 1)];
const DIRECTIONAL_WARP: &[ParamSpec] = &[p("intensity", 1, 1), p("angle", 3, 1), p("map_range", 7, 2)];
const GRADIENT_WARP: &[ParamSpec] = &[p("aspect", 2, 2), p("intensity", 5, 1)];
const EMBOSS: &[ParamSpec] = &[p("intensity", 1, 1), p("angle", 3, 1)];
const GRADIENT_DYNAMIC: &[ParamSpec] = &[p("position", 1, 1)];
const DIRECTIONAL_BLUR: &[ParamSpec] = &[p("intensity", 0, 1), p("angle", 2, 1)];

const SPECS: &[OpSpec] = &[
    OpSpec { op: 0x10, kind: Kind::Emboss, color: false, codes: Codes::ModeWord, mode_word: true, inputs: 2, params: EMBOSS },
    OpSpec { op: 0x11, kind: Kind::Emboss, color: true, codes: Codes::ModeWord, mode_word: true, inputs: 2, params: EMBOSS },
    OpSpec { op: 0x26, kind: Kind::GradientDynamic, color: false, codes: Codes::Flags, mode_word: false, inputs: 2, params: GRADIENT_DYNAMIC },
    OpSpec { op: 0x27, kind: Kind::GradientDynamic, color: true, codes: Codes::Flags, mode_word: false, inputs: 2, params: GRADIENT_DYNAMIC },
    OpSpec { op: 0x04, kind: Kind::Transform, color: false, codes: Codes::ModeWord, mode_word: true, inputs: 1, params: TRANSFORM },
    OpSpec { op: 0x05, kind: Kind::Transform, color: true, codes: Codes::ModeWord, mode_word: true, inputs: 1, params: TRANSFORM_COLOR },
    OpSpec { op: 0x06, kind: Kind::GrayscaleConversion, color: false, codes: Codes::Flags, mode_word: false, inputs: 1, params: GRAYSCALE },
    OpSpec { op: 0x0c, kind: Kind::Uniform, color: false, codes: Codes::Flags, mode_word: false, inputs: 0, params: UNIFORM_GRAY },
    OpSpec { op: 0x0d, kind: Kind::Uniform, color: true, codes: Codes::Flags, mode_word: false, inputs: 0, params: UNIFORM_COLOR },
    OpSpec { op: 0x14, kind: Kind::Blur, color: false, codes: Codes::Flags, mode_word: false, inputs: 1, params: BLUR },
    OpSpec { op: 0x15, kind: Kind::Blur, color: true, codes: Codes::Flags, mode_word: false, inputs: 1, params: BLUR },
    OpSpec { op: 0x0e, kind: Kind::GradientWarp, color: false, codes: Codes::Flags, mode_word: false, inputs: 2, params: GRADIENT_WARP },
    OpSpec { op: 0x0f, kind: Kind::GradientWarp, color: true, codes: Codes::Flags, mode_word: false, inputs: 2, params: GRADIENT_WARP },
    OpSpec { op: 0x18, kind: Kind::DirectionalWarp, color: false, codes: Codes::ModeWord, mode_word: true, inputs: 2, params: DIRECTIONAL_WARP },
    OpSpec { op: 0x19, kind: Kind::DirectionalWarp, color: true, codes: Codes::ModeWord, mode_word: true, inputs: 2, params: DIRECTIONAL_WARP },
    OpSpec { op: 0x16, kind: Kind::DirectionalBlur, color: false, codes: Codes::ModeWord, mode_word: true, inputs: 1, params: DIRECTIONAL_BLUR },
    OpSpec { op: 0x17, kind: Kind::DirectionalBlur, color: true, codes: Codes::ModeWord, mode_word: true, inputs: 1, params: DIRECTIONAL_BLUR },
    OpSpec { op: 0x1a, kind: Kind::Sharpen, color: false, codes: Codes::Flags, mode_word: false, inputs: 1, params: SHARPEN },
    OpSpec { op: 0x1b, kind: Kind::Sharpen, color: true, codes: Codes::Flags, mode_word: false, inputs: 1, params: SHARPEN },
    OpSpec { op: 0x1d, kind: Kind::Hsl, color: true, codes: Codes::Flags, mode_word: false, inputs: 1, params: HSL },
    OpSpec { op: 0x1e, kind: Kind::Levels, color: false, codes: Codes::ModeWord, mode_word: true, inputs: 1, params: LEVELS },
    OpSpec { op: 0x1f, kind: Kind::Levels, color: true, codes: Codes::ModeWord, mode_word: true, inputs: 1, params: LEVELS_COLOR },
];

/// A parameter's value as stored.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Default,
    /// Inline words (floats or integers depending on the parameter).
    Const(Vec<u32>),
    Expr(Expr),
    /// An image (node index), e.g. the mask that scales a blend's opacity.
    Image(usize),
}

impl Value {
    /// The inline words as floats.
    pub fn floats(&self) -> Option<Vec<f32>> {
        match self {
            Value::Const(w) => Some(w.iter().map(|w| f32::from_bits(*w)).collect()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: &'static str,
    pub value: Value,
}

/// A decoded node.
#[derive(Debug, Clone, PartialEq)]
pub struct AtomicNode {
    pub index: usize,
    pub opcode: u8,
    pub kind: Kind,
    pub color: bool,
    /// The mode word, when the operation has one (blend mode, transform flags, channel map …).
    pub mode: Option<u32>,
    /// The high half of the tag: bits 8 and 9 say whether the node repeats horizontally and vertically.
    pub flags: u16,
    /// The second byte of the tag: log2 of the height (high nibble) and width (low nibble) the compiler worked out. A
    /// node without a size expression has this size, whatever the graph's.
    pub variant: u8,
    pub inputs: Vec<usize>,
    pub size: Option<Expr>,
    /// The node's random seed (usually `$randomseed`).
    pub seed: Option<Expr>,
    pub params: Vec<Param>,
    /// Section offset where the header ends (where the record's own expression data starts).
    pub header_end: u32,
    /// Fx-map: the pattern program.
    pub program: Option<Record>,
    /// Pixel processor: the per-pixel function.
    pub function: Option<Expr>,
    /// Gradient map: keys as (position, value or RGBA, midpoint), all 0..1.
    pub keys: Vec<GradientKey>,
    /// Curve: the Bézier keys.
    pub curve: Vec<CurveKey>,
    /// Vector shape: triangle strips of vertices, each a u16 x in the low half and a u16 y in the high half.
    pub strips: Vec<Vec<u32>>,
    /// A pixel processor that makes no image: its function runs once and leaves this many words in the graph's value
    /// memory (with the `store` instruction), where expressions read them with `inputat` (0 for every other node).
    pub value_words: u32,
}

/// One key of a curve (all in 0..1): its point and the tangent handles before and after it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveKey {
    pub point: [f32; 2],
    pub left: [f32; 2],
    pub right: [f32; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct GradientKey {
    pub position: f32,
    pub color: [f32; 4],
    pub midpoint: f32,
}

impl AtomicNode {
    pub fn param(&self, name: &str) -> Option<&Value> {
        self.params.iter().find(|p| p.name == name).map(|p| &p.value)
    }
}

/// Reads fields in order from a record's body.
struct Fields<'r, 's> {
    rec: &'r NodeRecord,
    s: &'r Section<'s>,
    at: usize,
}

impl Fields<'_, '_> {
    fn word(&mut self) -> Result<u32, DecodeError> {
        let w = self.rec.word(self.at).ok_or_else(|| DecodeError::invalid(RECORD, format!("node {} ends early", self.rec.index)))?;
        self.at += 1;
        Ok(w)
    }

    fn words(&mut self, n: u8) -> Result<Vec<u32>, DecodeError> {
        (0..n).map(|_| self.word()).collect()
    }

    fn index(&mut self) -> Result<usize, DecodeError> {
        Ok(self.word()? as usize)
    }

    fn expr(&mut self) -> Result<Expr, DecodeError> {
        let ptr = self.word()?;
        Expr::parse(self.s, ptr)
    }

    fn offset(&self) -> u32 {
        self.rec.word_offset(self.at)
    }
}

/// Decodes one record. Operations without a known layout come back as [`Kind::Unknown`] with no fields.
pub fn decode(rec: &NodeRecord, s: &Section<'_>) -> Result<AtomicNode, DecodeError> {
    let mut f = Fields { rec, s, at: 0 };
    let mut node = AtomicNode {
        index: rec.index,
        opcode: rec.opcode,
        kind: Kind::Unknown,
        color: false,
        mode: None,
        flags: rec.flags,
        variant: rec.variant,
        inputs: Vec::new(),
        size: None,
        seed: None,
        params: Vec::new(),
        header_end: rec.word_offset(0),
        program: None,
        function: None,
        keys: Vec::new(),
        curve: Vec::new(),
        strips: Vec::new(),
        value_words: 0,
    };
    match rec.opcode {
        OP_BLEND_GRAY | OP_BLEND_COLOR => decode_blend(&mut f, &mut node)?,
        OP_CHANNEL_SHUFFLE => decode_channel_shuffle(&mut f, &mut node)?,
        OP_FXMAP_GRAY | OP_FXMAP_COLOR => decode_fxmap(&mut f, &mut node)?,
        OP_GRADIENT_MAP_GRAY | OP_GRADIENT_MAP_COLOR => decode_gradient_map(&mut f, &mut node)?,
        OP_PIXEL_PROCESSOR_GRAY | OP_PIXEL_PROCESSOR_COLOR => decode_pixel_processor(&mut f, &mut node)?,
        OP_DISTANCE_GRAY | OP_DISTANCE_COLOR => decode_distance(&mut f, &mut node)?,
        OP_NORMAL => decode_normal(&mut f, &mut node)?,
        OP_CURVE => decode_curve(&mut f, &mut node)?,
        OP_VECTOR => decode_vector(&mut f, &mut node)?,
        OP_INPUT_GRAY | OP_INPUT_COLOR => {
            node.kind = Kind::Input;
            node.color = rec.opcode == OP_INPUT_COLOR;
            node.mode = Some(f.word()?);
            // a colour JPEG stored in the graph (flags bit 11) with bit 8 has a second word: where its alpha is stored, a
            // greyscale image of the same size (ivy_branch #162, #215: two JPEGs one after the other). Uncompressed
            // images (bit 11 clear) hold their alpha in the samples: bit 8 only counts a fourth channel there
            // (Swirls #4, flags 0x07xx: RGBA, 16 bits, one word)
            if rec.opcode == OP_INPUT_COLOR && rec.flags & FLAG_INPUT_JPEG_ALPHA == FLAG_INPUT_JPEG_ALPHA {
                node.params.push(Param { name: "alpha", value: Value::Const(vec![f.word()?]) });
            }
            frame(&mut f, &mut node)?;
        }
        op => {
            let Some(spec) = SPECS.iter().find(|s| s.op == op) else { return Ok(node) };
            node.kind = spec.kind;
            node.color = spec.color;
            let codes = if spec.mode_word {
                let m = f.word()?;
                node.mode = Some(m);
                if spec.codes == Codes::ModeWord { m } else { u32::from(rec.flags >> FLAG_CODES_SHIFT) }
            } else {
                u32::from(rec.flags >> FLAG_CODES_SHIFT)
            };
            // a gradient warp may start with a word of 0, which only shows by what follows: two more node indices
            let leading = spec.kind == Kind::GradientWarp && rec.word(0) == Some(0) && rec.word(2).is_some_and(|w| (w as usize) < rec.index);
            if leading {
                f.word()?;
            }
            for _ in 0..spec.inputs {
                node.inputs.push(f.index()?);
            }
            frame(&mut f, &mut node)?;
            if spec.codes == Codes::ModeWord {
                aspect(&mut f, &mut node)?;
            }
            for ps in spec.params {
                let value = match (codes >> ps.bit) & 3 {
                    1 => Value::Const(f.words(ps.words)?),
                    2 => Value::Expr(f.expr()?),
                    _ => Value::Default,
                };
                node.params.push(Param { name: ps.name, value });
            }
            if spec.kind == Kind::GradientDynamic {
                // flags bit 11: the gradient runs along y, so the input picks a row
                node.mode = Some(u32::from(rec.flags >> FLAG_ALONG_Y_SHIFT) & 1);
            }
        }
    }
    node.header_end = f.offset();
    Ok(node)
}

/// The format word, random-seed and size pointers that follow the inputs, in that order.
fn frame(f: &mut Fields<'_, '_>, node: &mut AtomicNode) -> Result<(), DecodeError> {
    // bit 6: the seed as a number (measured on a fx-map), for any operation
    if f.rec.flags & FLAG_SEED_WORD != 0 {
        let word = f.word()?;
        node.mode.get_or_insert(word);
        node.seed = Some(Expr { offset: 0, end: 0, instrs: vec![Instr { op: 0, components: 1, ty: 2, operands: Operands::Constant(vec![word]) }] });
    }
    if f.rec.flags & FLAG_SEED_EXPR != 0 {
        node.seed = Some(f.expr()?);
    }
    if f.rec.flags & FLAG_SIZE_EXPR != 0 {
        node.size = Some(f.expr()?);
    }
    Ok(())
}

/// Operations with a mode word keep one more code in the flags (bits 10–11): a float2 pixel-aspect factor
/// (an expression like `2^min(Δ log2 size, 0)` for non-square outputs). It follows the seed and size pointers.
fn aspect(f: &mut Fields<'_, '_>, node: &mut AtomicNode) -> Result<(), DecodeError> {
    let value = match (f.rec.flags >> FLAG_ASPECT_SHIFT) & 3 {
        1 => Value::Const(f.words(2)?),
        2 => Value::Expr(f.expr()?),
        _ => return Ok(()),
    };
    node.params.push(Param { name: "aspect", value });
    Ok(())
}

/// Blend: mode word bits 0–3 blending mode, 4–5 opacity source (1 inline, 2 expression, 3 mask image), 6–7
/// cropping (inline: two words of u16 fixed-point corners), bit 9 / 10 the opacity that goes with a mask
/// (inline / expression), bits 8 and 11 alpha handling (not decoded).
fn decode_blend(f: &mut Fields<'_, '_>, node: &mut AtomicNode) -> Result<(), DecodeError> {
    node.kind = Kind::Blend;
    node.color = f.rec.opcode == OP_BLEND_COLOR;
    let m = f.word()?;
    node.mode = Some(m);
    node.inputs.push(f.index()?);
    node.inputs.push(f.index()?);
    frame(f, node)?;
    aspect(f, node)?;
    let opacity = match (m >> BLEND_OPACITY_SHIFT) & 3 {
        1 => Value::Const(f.words(1)?),
        2 => Value::Expr(f.expr()?),
        3 => {
            node.params.push(Param { name: "mask", value: Value::Image(f.index()?) });
            if m & BLEND_MASK_OPACITY_CONST != 0 {
                Value::Const(f.words(1)?)
            } else if m & BLEND_MASK_OPACITY_EXPR != 0 {
                Value::Expr(f.expr()?)
            } else {
                Value::Default
            }
        }
        _ => Value::Default,
    };
    node.params.push(Param { name: "opacity", value: opacity });
    let crop = match (m >> BLEND_CROP_SHIFT) & 3 {
        1 => Value::Const(f.words(2)?),
        2 => Value::Expr(f.expr()?),
        _ => Value::Default,
    };
    node.params.push(Param { name: "cropping", value: crop });
    Ok(())
}

/// Channel shuffle: the mode word holds one source byte per output channel (0–3 channels of the first input,
/// 4–7 of the second).
fn decode_channel_shuffle(f: &mut Fields<'_, '_>, node: &mut AtomicNode) -> Result<(), DecodeError> {
    node.kind = Kind::ChannelShuffle;
    node.color = true;
    node.mode = Some(f.word()?);
    node.inputs.push(f.index()?);
    node.inputs.push(f.index()?);
    frame(f, node)
}

impl AtomicNode {
    /// Blending mode (bits 0–3 of the mode word): 0 copy, 1 add, 2 subtract, 3 multiply, 4 add-sub, 5 max,
    /// 6 min, 7 switch, 8 divide, 9 overlay, 10 screen, 11 soft light.
    pub fn blend_mode(&self) -> Option<u32> {
        (self.kind == Kind::Blend).then(|| self.mode.unwrap_or(0) & BLEND_MODE_MASK)
    }
}

const FXMAP: &[ParamSpec] = &[p("p0", 0, 1), p("p1", 2, 1), p("p2", 4, 1), p("frame_matrix", 6, 4), p("frame_offset", 8, 4)];

/// Fx-map: mode word (parameter codes, image input count), pointer to the pattern program, image inputs, seed, size,
/// parameters.
fn decode_fxmap(f: &mut Fields<'_, '_>, node: &mut AtomicNode) -> Result<(), DecodeError> {
    node.kind = Kind::FxMap;
    node.color = f.rec.opcode == OP_FXMAP_COLOR;
    let m = f.word()?;
    node.mode = Some(m);
    let root = f.word()?;
    // bits 10 and up of the mode word: how many image inputs follow (node indices)
    for _ in 0..m >> FXMAP_INPUT_COUNT_SHIFT {
        let input = f.index()?;
        node.inputs.push(input);
    }
    frame(f, node)?;
    for ps in FXMAP {
        let value = match (m >> ps.bit) & 3 {
            // code 3 (p0 only): the node holding the image to draw on
            3 => Value::Image(f.index()?),
            // a colour fx-map's background is a colour
            1 if ps.name == "p0" && node.color => Value::Const(f.words(4)?),
            1 => Value::Const(f.words(ps.words)?),
            2 => Value::Expr(f.expr()?),
            _ => Value::Default,
        };
        node.params.push(Param { name: ps.name, value });
    }
    node.program = Some(Record::parse(f.s, root)?);
    Ok(())
}

/// Gradient map: input, key count, pointer to the keys, seed, size. Grayscale keys are u16 position and value, colour
/// keys a u16 position and RGBA8; with flags bit 8 a midpoint follows each (u16, or f32 with float keys).
fn decode_gradient_map(f: &mut Fields<'_, '_>, node: &mut AtomicNode) -> Result<(), DecodeError> {
    node.kind = Kind::GradientMap;
    node.color = f.rec.opcode == OP_GRADIENT_MAP_COLOR;
    node.inputs.push(f.index()?);
    let n = f.word()?;
    // flags bits 9-10: either one, the keys are floats, position and RGBA; both, one more parameter comes first (not
    // understood yet). Bit 9 alone has no extra word: its keys follow at once (ivy_branch, mg_mask_editor), and the size
    // after them is $outputsize, as on the gradients around it.
    let float_keys = (f.rec.flags >> FLAG_FLOAT_KEYS_SHIFT) & 3 != 0;
    if (f.rec.flags >> FLAG_FLOAT_KEYS_SHIFT) & 3 == 3 {
        f.word()?;
    }
    let keys = f.word()?;
    frame(f, node)?;
    // flags bit 8: each key has a midpoint, a float here (-1 in every package seen); it has no effect, as the u16 one
    let mid = f.rec.flags & FLAG_KEY_MIDPOINT != 0;
    if float_keys {
        let float = |at: u32| f.s.u32(at).map(f32::from_bits);
        let stride = if node.color { FLOAT_KEY_COLOR_STRIDE } else { FLOAT_KEY_GRAY_STRIDE } + if mid { 4 } else { 0 };
        for i in 0..n.min(MAX_FLOAT_GRADIENT_KEYS) {
            let at = keys + stride * i;
            let color = if node.color {
                [float(at + 4)?, float(at + 8)?, float(at + 12)?, float(at + 16)?]
            } else {
                [float(at + 4)?, float(at + 4)?, float(at + 4)?, 1.0]
            };
            node.keys.push(GradientKey { position: float(at)?, color, midpoint: 0.5 });
        }
        return Ok(());
    }
    let unit = |v: u16| f32::from(v) / U16_FULL;
    for i in 0..n.min(MAX_GRADIENT_KEYS) {
        if node.color {
            let at = keys + if mid { 8 } else { 6 } * i;
            let byte = |k: u32| f.s.bytes(at + 2 + k, 1).and_then(|b| b.first().copied()).unwrap_or(0);
            let c = [byte(0), byte(1), byte(2), byte(3)].map(|b| f32::from(b) / U8_FULL);
            node.keys.push(GradientKey { position: unit(f.s.u16(at)?), color: c, midpoint: if mid { unit(f.s.u16(at + 6)?) } else { 0.5 } });
        } else {
            let at = keys + if mid { 6 } else { 4 } * i;
            let v = unit(f.s.u16(at + 2)?);
            node.keys.push(GradientKey { position: unit(f.s.u16(at)?), color: [v, v, v, 1.0], midpoint: if mid { unit(f.s.u16(at + 4)?) } else { 0.5 } });
        }
    }
    Ok(())
}

/// Distance: mode word (bit 0 a second input, the colour source; bit 1 fades by distance when there is a source;
/// bits 2–3 the distance's code), the mask, the source, seed, size, the pixel aspect, the distance.
fn decode_distance(f: &mut Fields<'_, '_>, node: &mut AtomicNode) -> Result<(), DecodeError> {
    node.kind = Kind::Distance;
    node.color = f.rec.opcode == OP_DISTANCE_COLOR;
    let m = f.word()?;
    node.mode = Some(m);
    node.inputs.push(f.index()?);
    if m & 1 != 0 {
        node.inputs.push(f.index()?);
    }
    frame(f, node)?;
    aspect(f, node)?;
    let distance = code_value(f, (m >> 2) & 3, 1)?;
    node.params.push(Param { name: "distance", value: distance });
    Ok(())
}

/// Normal: mode word (bits 0–1 the intensity's code; bits 2–3 whether y points the other way: 1 yes, 2 an expression;
/// bit 4 keep the height in the alpha channel), the height, seed, size, the pixel aspect, the parameters.
fn decode_normal(f: &mut Fields<'_, '_>, node: &mut AtomicNode) -> Result<(), DecodeError> {
    node.kind = Kind::Normal;
    node.color = true;
    let m = f.word()?;
    node.mode = Some(m);
    node.inputs.push(f.index()?);
    frame(f, node)?;
    aspect(f, node)?;
    let intensity = code_value(f, m & 3, 1)?;
    node.params.push(Param { name: "intensity", value: intensity });
    if (m >> 2) & 3 == 2 {
        node.params.push(Param { name: "flip_y", value: Value::Expr(f.expr()?) });
    }
    Ok(())
}

/// A parameter by its 2-bit code: 1 inline, 2 expression, else the default.
fn code_value(f: &mut Fields<'_, '_>, code: u32, words: u8) -> Result<Value, DecodeError> {
    Ok(match code {
        1 => Value::Const(f.words(words)?),
        2 => Value::Expr(f.expr()?),
        _ => Value::Default,
    })
}

/// Vector shape: a pointer to the shape, seed, size. The shape is a marker word and blocks of a u32 (bits 3 up the vertex
/// count, the low bits a type) and u16 x, y pairs (0..65535 across the image), each block ending in a zero word.
/// Type 1 is a triangle strip; type 2 isn't understood and leaves no visible trace.
fn decode_vector(f: &mut Fields<'_, '_>, node: &mut AtomicNode) -> Result<(), DecodeError> {
    node.kind = Kind::Vector;
    let shape = f.word()?;
    frame(f, node)?;
    let mut at = shape.saturating_add(4);
    let end = node.size.as_ref().map_or(f.rec.end, |s| s.offset);
    let end = if end > shape { end } else { f.rec.end };
    while at + 4 <= end {
        let head = f.s.u32(at)?;
        let n = head >> 3;
        if n == 0 || at + 4 + 4 * n + 4 > end {
            break;
        }
        if head & VECTOR_TYPE_MASK == VECTOR_TRIANGLE_STRIP {
            let mut strip = Vec::with_capacity(n as usize);
            for k in 0..n {
                strip.push(f.s.u32(at + 4 + 4 * k)?);
            }
            node.strips.push(strip);
        }
        at += 8 + 4 * n;
    }
    Ok(())
}

/// Curve: input, key count, pointer to the keys, seed, size. The pointer leads to the count again and six floats
/// per key: point, left handle, right handle. Flags bit 9 draws the curve as a horizontal ramp instead of applying
/// it to the input, which then only gives the size.
fn decode_curve(f: &mut Fields<'_, '_>, node: &mut AtomicNode) -> Result<(), DecodeError> {
    node.kind = Kind::Curve;
    node.inputs.push(f.index()?);
    let n = f.word()?;
    let keys = f.word()?;
    frame(f, node)?;
    node.mode = Some(u32::from(f.rec.flags & FLAG_CURVE_RAMP != 0));
    let float = |at: u32| f.s.u32(at).map(f32::from_bits);
    for i in 0..n.min(MAX_CURVE_KEYS) {
        let at = keys + 4 + CURVE_KEY_STRIDE * i;
        node.curve.push(CurveKey { point: [float(at)?, float(at + 4)?], left: [float(at + 8)?, float(at + 12)?], right: [float(at + 16)?, float(at + 20)?] });
    }
    Ok(())
}

/// Pixel processor: input count (low half; the high half is `value_words`), the inputs, the frame, the per-pixel function.
fn decode_pixel_processor(f: &mut Fields<'_, '_>, node: &mut AtomicNode) -> Result<(), DecodeError> {
    node.kind = Kind::PixelProcessor;
    node.color = f.rec.opcode == OP_PIXEL_PROCESSOR_COLOR;
    let word = f.word()?;
    node.value_words = word >> 16;
    for _ in 0..(word & PIXEL_PROCESSOR_INPUT_MASK).min(MAX_PIXEL_PROCESSOR_INPUTS) {
        node.inputs.push(f.index()?);
    }
    frame(f, node)?;
    node.function = Some(f.expr()?);
    Ok(())
}

//! Value expressions: the small SSA programs that compute node parameters (and the per-instance values of
//! pattern programs) from graph inputs and constants.
//!
//! An expression is a u16 count followed by that many instructions. Each instruction starts with a u16
//! token: bits 0–5 the operation, bits 6–7 the component count − 1, bits 8–15 a type byte (low two bits:
//! 0 bool, 1 float, 2 int; the other bits are not understood yet). Operands are u16 indices of earlier
//! instructions; immediates (constants, input uids) are 4-byte aligned, except boolean constants, which are
//! one u16 with a bit per lane. The expression ends 4-byte aligned.

use super::section::Section;
use crate::decode::DecodeError;

// Format data: the instruction token (see the module doc).
/// Bits 0-5: the operation.
const TOKEN_OP_MASK: u16 = 0x3f;
/// Bits 6-7: the component count - 1.
const TOKEN_COMPONENTS_SHIFT: u16 = 6;
/// Bits 8-15: the type byte.
const TOKEN_TYPE_SHIFT: u16 = 8;

const RECORD: &str = "compiled graph expression";
/// An operand the compiler left out because it is zero.
pub const NO_OPERAND: u16 = 0xffff;

/// How an operation's operands are encoded after its token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// `components` aligned 32-bit words.
    Constant,
    /// One aligned u32: a graph input uid.
    Input,
    /// One u16 that is not an instruction index (a variable slot or a built-in id).
    Slot,
    /// An instruction index, then a u16 that is not (a slot or a swizzle mask).
    RefSlot,
    /// `n` instruction indices.
    Refs(u8),
    /// An instruction index, an image input number and, when bit 2 of the type byte is set, a filter mode.
    Sample,
    /// Three instruction indices (start, stop condition, body), then an aligned u32: the number of passes when the
    /// condition is left out.
    Loop,
}

/// What is known about one operation: its encoding and, when understood, a name.
struct OpInfo {
    op: u8,
    shape: Shape,
    name: &'static str,
}

/// Operations whose encoding is known. The numbering follows the public list of function nodes (add, sub,
/// mul, div, mod, neg, dot, …); names ending in `?` are placed by that order but not yet seen in a clear context.
const OPS: &[OpInfo] = &[
    OpInfo { op: 0x00, shape: Shape::Constant, name: "const" },
    OpInfo { op: 0x01, shape: Shape::Slot, name: "builtin" },
    OpInfo { op: 0x02, shape: Shape::Input, name: "input" },
    OpInfo { op: 0x03, shape: Shape::Slot, name: "inputat" },
    OpInfo { op: 0x04, shape: Shape::Slot, name: "get" },
    OpInfo { op: 0x06, shape: Shape::RefSlot, name: "store" },
    OpInfo { op: 0x07, shape: Shape::RefSlot, name: "set" },
    OpInfo { op: 0x09, shape: Shape::Refs(3), name: "ifelse" },
    OpInfo { op: 0x0b, shape: Shape::Loop, name: "loop" },
    OpInfo { op: 0x0c, shape: Shape::Refs(2), name: "sequence" },
    OpInfo { op: 0x0d, shape: Shape::Refs(2), name: "vector" },
    OpInfo { op: 0x0e, shape: Shape::Refs(3), name: "vector3?" },
    OpInfo { op: 0x0f, shape: Shape::Refs(4), name: "vector4" },
    OpInfo { op: 0x10, shape: Shape::RefSlot, name: "swizzle" },
    OpInfo { op: 0x11, shape: Shape::Refs(1), name: "cast" },
    OpInfo { op: 0x12, shape: Shape::Refs(2), name: "add" },
    OpInfo { op: 0x13, shape: Shape::Refs(2), name: "sub" },
    OpInfo { op: 0x14, shape: Shape::Refs(2), name: "mul" },
    OpInfo { op: 0x15, shape: Shape::Refs(2), name: "div" },
    OpInfo { op: 0x16, shape: Shape::Refs(2), name: "mod" },
    OpInfo { op: 0x17, shape: Shape::Refs(1), name: "neg" },
    OpInfo { op: 0x18, shape: Shape::Refs(2), name: "dot" },
    OpInfo { op: 0x19, shape: Shape::Refs(2), name: "cross" },
    OpInfo { op: 0x1a, shape: Shape::Refs(2), name: "and" },
    OpInfo { op: 0x1b, shape: Shape::Refs(2), name: "or" },
    OpInfo { op: 0x1c, shape: Shape::Refs(1), name: "not" },
    OpInfo { op: 0x1d, shape: Shape::Refs(2), name: "eq" },
    OpInfo { op: 0x1e, shape: Shape::Refs(2), name: "noteq" },
    OpInfo { op: 0x1f, shape: Shape::Refs(2), name: "gt" },
    OpInfo { op: 0x20, shape: Shape::Refs(2), name: "gteq" },
    OpInfo { op: 0x21, shape: Shape::Refs(2), name: "lt" },
    OpInfo { op: 0x22, shape: Shape::Refs(2), name: "lteq" },
    OpInfo { op: 0x23, shape: Shape::Refs(1), name: "abs" },
    OpInfo { op: 0x24, shape: Shape::Refs(1), name: "floor?" },
    OpInfo { op: 0x25, shape: Shape::Refs(1), name: "ceil?" },
    OpInfo { op: 0x26, shape: Shape::Refs(1), name: "cos" },
    OpInfo { op: 0x27, shape: Shape::Refs(1), name: "sin" },
    OpInfo { op: 0x28, shape: Shape::Refs(1), name: "sqrt?" },
    OpInfo { op: 0x29, shape: Shape::Refs(1), name: "log?" },
    OpInfo { op: 0x2a, shape: Shape::Refs(1), name: "exp?" },
    OpInfo { op: 0x2b, shape: Shape::Refs(1), name: "pow2" },
    OpInfo { op: 0x2c, shape: Shape::Refs(1), name: "tan" },
    OpInfo { op: 0x2d, shape: Shape::Refs(1), name: "atan2" },
    OpInfo { op: 0x2e, shape: Shape::Refs(2), name: "cartesian" },
    OpInfo { op: 0x2f, shape: Shape::Refs(3), name: "lerp" },
    OpInfo { op: 0x30, shape: Shape::Refs(2), name: "min" },
    OpInfo { op: 0x31, shape: Shape::Refs(2), name: "max" },
    OpInfo { op: 0x32, shape: Shape::Refs(1), name: "rand" },
    OpInfo { op: 0x33, shape: Shape::Sample, name: "samplelum" },
    OpInfo { op: 0x34, shape: Shape::Sample, name: "samplecol" },
    OpInfo { op: 0x35, shape: Shape::Refs(1), name: "log2" },
    // placed by context only (it follows log2 and takes two operands), not verified against the reference app: without an entry
    // here an expression using the operation fails to parse
    OpInfo { op: 0x36, shape: Shape::Refs(2), name: "pow" },
];

fn info(op: u8) -> Option<&'static OpInfo> {
    OPS.iter().find(|i| i.op == op)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operands {
    Constant(Vec<u32>),
    Input(u32),
    Slot(u16),
    RefSlot(u16, u16),
    Refs(Vec<u16>),
    /// position, image input number, filter mode (0 when absent)
    Sample(u16, u16, u16),
    /// start, stop condition, body; passes when the condition is left out
    Loop(Vec<u16>, u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instr {
    pub op: u8,
    pub components: u8,
    pub ty: u8,
    pub operands: Operands,
}

/// The low two bits of an instruction's type byte: the kind of value it makes.
pub const VALUE_TYPE_BITS: u8 = 0x03;

/// The kind of value an instruction makes ([`VALUE_TYPE_BITS`] of its type byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    /// 0: a boolean, one value whatever its lane count (a constant is a u16 with a bit per lane).
    Bool,
    /// 1.
    Float,
    /// 2.
    Int,
    /// 3: not seen.
    Other,
}

impl ValueType {
    pub fn of_type_byte(ty: u8) -> ValueType {
        match ty & VALUE_TYPE_BITS {
            0 => ValueType::Bool,
            1 => ValueType::Float,
            2 => ValueType::Int,
            _ => ValueType::Other,
        }
    }
}

/// The input lane a swizzle reads for its output lane `lane`: two bits per lane of its mask.
pub fn swizzle_source(mask: u16, lane: u16) -> u8 {
    (mask.checked_shr(2 * u32::from(lane)).unwrap_or(0) & 3) as u8
}

impl Instr {
    pub fn name(&self) -> &'static str {
        info(self.op).map_or("?", |i| i.name)
    }

    /// The kind of value the instruction makes.
    pub fn value_type(&self) -> ValueType {
        ValueType::of_type_byte(self.ty)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr {
    /// Section offset of the count.
    pub offset: u32,
    /// Section offset just past the expression (4-byte aligned).
    pub end: u32,
    pub instrs: Vec<Instr>,
}

impl Expr {
    /// Decodes the expression at section offset `off`. Fails on an operation whose encoding is not known
    /// or an operand that does not name an earlier instruction.
    pub fn parse(s: &Section<'_>, off: u32) -> Result<Expr, DecodeError> {
        let n = s.u16(off)?;
        let mut p = off + 2;
        let mut instrs = Vec::with_capacity(usize::from(n));
        for i in 0..n {
            let tok = s.u16(p)?;
            p += 2;
            let op = (tok & TOKEN_OP_MASK) as u8;
            let components = ((tok >> TOKEN_COMPONENTS_SHIFT) & 3) as u8 + 1;
            let ty = (tok >> TOKEN_TYPE_SHIFT) as u8;
            let Some(info) = info(op) else {
                return Err(DecodeError::invalid(RECORD, format!("unknown operation {op:#04x} at {:#x}", p - 2)));
            };
            let check = |r: u16| -> Result<u16, DecodeError> {
                // 0xffff: left out, read as zero
                if r < i || r == NO_OPERAND {
                    Ok(r)
                } else {
                    Err(DecodeError::invalid(RECORD, format!("operand {r} of instruction {i} (operation {op:#04x}) is not earlier")))
                }
            };
            let operands = match info.shape {
                // a boolean constant is one u16: a bit per lane
                Shape::Constant if ValueType::of_type_byte(ty) == ValueType::Bool => {
                    let bits = s.u16(p)?;
                    p += 2;
                    Operands::Constant(vec![u32::from(bits)])
                }
                Shape::Constant => {
                    p = align4(p);
                    let words = s.words(p, u32::from(components))?;
                    p += 4 * u32::from(components);
                    Operands::Constant(words)
                }
                Shape::Input => {
                    p = align4(p);
                    let uid = s.u32(p)?;
                    p += 4;
                    Operands::Input(uid)
                }
                Shape::Slot => {
                    let v = s.u16(p)?;
                    p += 2;
                    Operands::Slot(v)
                }
                Shape::RefSlot => {
                    let r = check(s.u16(p)?)?;
                    let v = s.u16(p + 2)?;
                    p += 4;
                    Operands::RefSlot(r, v)
                }
                Shape::Sample => {
                    let r = check(s.u16(p)?)?;
                    let image = s.u16(p + 2)?;
                    p += 4;
                    // bit 2 of the type byte: a filter mode follows (token 0xd33, 0x933 has none)
                    let filter = if ty & 4 != 0 {
                        p += 2;
                        s.u16(p - 2)?
                    } else {
                        0
                    };
                    Operands::Sample(r, image, filter)
                }
                Shape::Loop => {
                    let mut refs = Vec::with_capacity(3);
                    for _ in 0..3 {
                        refs.push(check(s.u16(p)?)?);
                        p += 2;
                    }
                    p = align4(p);
                    let passes = s.u32(p)?;
                    p += 4;
                    Operands::Loop(refs, passes)
                }
                Shape::Refs(k) => {
                    let mut refs = Vec::with_capacity(usize::from(k));
                    for _ in 0..k {
                        refs.push(check(s.u16(p)?)?);
                        p += 2;
                    }
                    Operands::Refs(refs)
                }
            };
            instrs.push(Instr { op, components, ty, operands });
        }
        Ok(Expr { offset: off, end: align4(p), instrs })
    }

    /// A graph input the whole expression is, if it is just one read.
    pub fn as_input(&self) -> Option<u32> {
        match self.instrs.as_slice() {
            [Instr { operands: Operands::Input(uid), .. }] => Some(*uid),
            _ => None,
        }
    }

    /// The constant the whole expression is, if it is just one constant.
    pub fn as_constant(&self) -> Option<&[u32]> {
        match self.instrs.as_slice() {
            [Instr { operands: Operands::Constant(v), .. }] => Some(v),
            _ => None,
        }
    }

    /// One line per instruction, `%i = name(args)`, for dumps.
    pub fn listing(&self) -> Vec<String> {
        self.instrs
            .iter()
            .enumerate()
            .map(|(i, ins)| {
                let args = match &ins.operands {
                    Operands::Constant(v) => {
                        if ins.value_type() == ValueType::Float {
                            v.iter().map(|w| format!("{}", f32::from_bits(*w))).collect::<Vec<_>>().join(", ")
                        } else {
                            v.iter().map(|w| format!("{}", *w as i32)).collect::<Vec<_>>().join(", ")
                        }
                    }
                    Operands::Input(uid) => format!("{uid:#010x}"),
                    Operands::Slot(v) => format!("#{v}"),
                    Operands::RefSlot(r, v) => format!("%{r}, #{v}"),
                    Operands::Sample(r, i, f) => format!("%{r}, image {i}, filter {f}"),
                    Operands::Refs(r) => r.iter().map(|x| format!("%{x}")).collect::<Vec<_>>().join(", "),
                    Operands::Loop(r, n) => format!("{}, {n} passes", r.iter().map(|x| format!("%{x}")).collect::<Vec<_>>().join(", ")),
                };
                format!("%{i} = {}{} [{:02x}] {}({args})", ins.name(), ins.components, ins.ty, if ins.op == 0x10 { "swz" } else { "" })
            })
            .collect()
    }
}

fn align4(p: u32) -> u32 {
    (p + 3) & !3
}

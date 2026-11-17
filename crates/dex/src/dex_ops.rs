//! Dalvik bytecode: the opcode table (all 256 slots and three payload pseudo-instructions) and a
//! decoder for every instruction format. Pure decoding; index operands are resolved by the caller.

use serde::Serialize;

/// What an index operand refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexKind {
    None,
    String,
    Type,
    Field,
    Method,
    CallSite,
    MethodHandle,
    Proto,
}

/// Instruction formats from the Dalvik specification (`10x`, `35c`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Fmt {
    F10x,
    F12x,
    F11n,
    F11x,
    F10t,
    F20t,
    F22x,
    F21t,
    F21s,
    F21h,
    F21c,
    F23x,
    F22b,
    F22t,
    F22s,
    F22c,
    F30t,
    F32x,
    F31i,
    F31t,
    F31c,
    F35c,
    F3rc,
    F45cc,
    F4rcc,
    F51l,
}

impl Fmt {
    /// Length in 16-bit code units.
    pub fn units(self) -> usize {
        use Fmt::*;
        match self {
            F10x | F12x | F11n | F11x | F10t => 1,
            F20t | F22x | F21t | F21s | F21h | F21c | F23x | F22b | F22t | F22s | F22c => 2,
            F30t | F32x | F31i | F31t | F31c | F35c | F3rc => 3,
            F45cc | F4rcc => 4,
            F51l => 5,
        }
    }
}

/// One opcode slot.
#[derive(Debug, Clone, Copy)]
pub struct Op {
    pub name: &'static str,
    pub fmt: Fmt,
    pub index: IndexKind,
}

const fn op(name: &'static str, fmt: Fmt, index: IndexKind) -> Op {
    Op { name, fmt, index }
}

const UNUSED: Op = op("unused", Fmt::F10x, IndexKind::None);

const UNOPS: [&str; 21] = [
    "neg-int",
    "not-int",
    "neg-long",
    "not-long",
    "neg-float",
    "neg-double",
    "int-to-long",
    "int-to-float",
    "int-to-double",
    "long-to-int",
    "long-to-float",
    "long-to-double",
    "float-to-int",
    "float-to-long",
    "float-to-double",
    "double-to-int",
    "double-to-long",
    "double-to-float",
    "int-to-byte",
    "int-to-char",
    "int-to-short",
];

const BINOPS: [&str; 32] = [
    "add-int",
    "sub-int",
    "mul-int",
    "div-int",
    "rem-int",
    "and-int",
    "or-int",
    "xor-int",
    "shl-int",
    "shr-int",
    "ushr-int",
    "add-long",
    "sub-long",
    "mul-long",
    "div-long",
    "rem-long",
    "and-long",
    "or-long",
    "xor-long",
    "shl-long",
    "shr-long",
    "ushr-long",
    "add-float",
    "sub-float",
    "mul-float",
    "div-float",
    "rem-float",
    "add-double",
    "sub-double",
    "mul-double",
    "div-double",
    "rem-double",
];

const BINOPS_2ADDR: [&str; 32] = [
    "add-int/2addr",
    "sub-int/2addr",
    "mul-int/2addr",
    "div-int/2addr",
    "rem-int/2addr",
    "and-int/2addr",
    "or-int/2addr",
    "xor-int/2addr",
    "shl-int/2addr",
    "shr-int/2addr",
    "ushr-int/2addr",
    "add-long/2addr",
    "sub-long/2addr",
    "mul-long/2addr",
    "div-long/2addr",
    "rem-long/2addr",
    "and-long/2addr",
    "or-long/2addr",
    "xor-long/2addr",
    "shl-long/2addr",
    "shr-long/2addr",
    "ushr-long/2addr",
    "add-float/2addr",
    "sub-float/2addr",
    "mul-float/2addr",
    "div-float/2addr",
    "rem-float/2addr",
    "add-double/2addr",
    "sub-double/2addr",
    "mul-double/2addr",
    "div-double/2addr",
    "rem-double/2addr",
];

const LIT16: [&str; 8] = ["add-int/lit16", "rsub-int", "mul-int/lit16", "div-int/lit16", "rem-int/lit16", "and-int/lit16", "or-int/lit16", "xor-int/lit16"];
const LIT8: [&str; 11] = [
    "add-int/lit8",
    "rsub-int/lit8",
    "mul-int/lit8",
    "div-int/lit8",
    "rem-int/lit8",
    "and-int/lit8",
    "or-int/lit8",
    "xor-int/lit8",
    "shl-int/lit8",
    "shr-int/lit8",
    "ushr-int/lit8",
];
const CMP: [&str; 5] = ["cmpl-float", "cmpg-float", "cmpl-double", "cmpg-double", "cmp-long"];
const IF2: [&str; 6] = ["if-eq", "if-ne", "if-lt", "if-ge", "if-gt", "if-le"];
const IFZ: [&str; 6] = ["if-eqz", "if-nez", "if-ltz", "if-gez", "if-gtz", "if-lez"];
const ARR: [&str; 14] = [
    "aget",
    "aget-wide",
    "aget-object",
    "aget-boolean",
    "aget-byte",
    "aget-char",
    "aget-short",
    "aput",
    "aput-wide",
    "aput-object",
    "aput-boolean",
    "aput-byte",
    "aput-char",
    "aput-short",
];
const IFIELD: [&str; 14] = [
    "iget",
    "iget-wide",
    "iget-object",
    "iget-boolean",
    "iget-byte",
    "iget-char",
    "iget-short",
    "iput",
    "iput-wide",
    "iput-object",
    "iput-boolean",
    "iput-byte",
    "iput-char",
    "iput-short",
];
const SFIELD: [&str; 14] = [
    "sget",
    "sget-wide",
    "sget-object",
    "sget-boolean",
    "sget-byte",
    "sget-char",
    "sget-short",
    "sput",
    "sput-wide",
    "sput-object",
    "sput-boolean",
    "sput-byte",
    "sput-char",
    "sput-short",
];
const INVOKE: [&str; 5] = ["invoke-virtual", "invoke-super", "invoke-direct", "invoke-static", "invoke-interface"];
const INVOKE_RANGE: [&str; 5] = ["invoke-virtual/range", "invoke-super/range", "invoke-direct/range", "invoke-static/range", "invoke-interface/range"];

fn pick(list: &[&'static str], i: u8, base: u8) -> &'static str {
    list.get(usize::from(i.wrapping_sub(base))).copied().unwrap_or("unused")
}

/// The opcode table entry for `b`.
pub fn opcode(b: u8) -> Op {
    use Fmt::*;
    use IndexKind as K;
    match b {
        0x00 => op("nop", F10x, K::None),
        0x01 => op("move", F12x, K::None),
        0x02 => op("move/from16", F22x, K::None),
        0x03 => op("move/16", F32x, K::None),
        0x04 => op("move-wide", F12x, K::None),
        0x05 => op("move-wide/from16", F22x, K::None),
        0x06 => op("move-wide/16", F32x, K::None),
        0x07 => op("move-object", F12x, K::None),
        0x08 => op("move-object/from16", F22x, K::None),
        0x09 => op("move-object/16", F32x, K::None),
        0x0a => op("move-result", F11x, K::None),
        0x0b => op("move-result-wide", F11x, K::None),
        0x0c => op("move-result-object", F11x, K::None),
        0x0d => op("move-exception", F11x, K::None),
        0x0e => op("return-void", F10x, K::None),
        0x0f => op("return", F11x, K::None),
        0x10 => op("return-wide", F11x, K::None),
        0x11 => op("return-object", F11x, K::None),
        0x12 => op("const/4", F11n, K::None),
        0x13 => op("const/16", F21s, K::None),
        0x14 => op("const", F31i, K::None),
        0x15 => op("const/high16", F21h, K::None),
        0x16 => op("const-wide/16", F21s, K::None),
        0x17 => op("const-wide/32", F31i, K::None),
        0x18 => op("const-wide", F51l, K::None),
        0x19 => op("const-wide/high16", F21h, K::None),
        0x1a => op("const-string", F21c, K::String),
        0x1b => op("const-string/jumbo", F31c, K::String),
        0x1c => op("const-class", F21c, K::Type),
        0x1d => op("monitor-enter", F11x, K::None),
        0x1e => op("monitor-exit", F11x, K::None),
        0x1f => op("check-cast", F21c, K::Type),
        0x20 => op("instance-of", F22c, K::Type),
        0x21 => op("array-length", F12x, K::None),
        0x22 => op("new-instance", F21c, K::Type),
        0x23 => op("new-array", F22c, K::Type),
        0x24 => op("filled-new-array", F35c, K::Type),
        0x25 => op("filled-new-array/range", F3rc, K::Type),
        0x26 => op("fill-array-data", F31t, K::None),
        0x27 => op("throw", F11x, K::None),
        0x28 => op("goto", F10t, K::None),
        0x29 => op("goto/16", F20t, K::None),
        0x2a => op("goto/32", F30t, K::None),
        0x2b => op("packed-switch", F31t, K::None),
        0x2c => op("sparse-switch", F31t, K::None),
        0x2d..=0x31 => op(pick(&CMP, b, 0x2d), F23x, K::None),
        0x32..=0x37 => op(pick(&IF2, b, 0x32), F22t, K::None),
        0x38..=0x3d => op(pick(&IFZ, b, 0x38), F21t, K::None),
        0x44..=0x51 => op(pick(&ARR, b, 0x44), F23x, K::None),
        0x52..=0x5f => op(pick(&IFIELD, b, 0x52), F22c, K::Field),
        0x60..=0x6d => op(pick(&SFIELD, b, 0x60), F21c, K::Field),
        0x6e..=0x72 => op(pick(&INVOKE, b, 0x6e), F35c, K::Method),
        0x74..=0x78 => op(pick(&INVOKE_RANGE, b, 0x74), F3rc, K::Method),
        0x7b..=0x8f => op(pick(&UNOPS, b, 0x7b), F12x, K::None),
        0x90..=0xaf => op(pick(&BINOPS, b, 0x90), F23x, K::None),
        0xb0..=0xcf => op(pick(&BINOPS_2ADDR, b, 0xb0), F12x, K::None),
        0xd0..=0xd7 => op(pick(&LIT16, b, 0xd0), F22s, K::None),
        0xd8..=0xe2 => op(pick(&LIT8, b, 0xd8), F22b, K::None),
        0xfa => op("invoke-polymorphic", F45cc, K::Method),
        0xfb => op("invoke-polymorphic/range", F4rcc, K::Method),
        0xfc => op("invoke-custom", F35c, K::CallSite),
        0xfd => op("invoke-custom/range", F3rc, K::CallSite),
        0xfe => op("const-method-handle", F21c, K::MethodHandle),
        0xff => op("const-method-type", F21c, K::Proto),
        _ => UNUSED,
    }
}

/// Switch and array payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Payload {
    PackedSwitch { first_key: i32, targets: Vec<i32> },
    SparseSwitch { keys: Vec<i32>, targets: Vec<i32> },
    FillArrayData { element_width: u16, count: u32, data: Vec<u8> },
}

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Insn {
    /// Offset in 16-bit code units from the start of the method's code.
    pub offset: u32,
    pub opcode: u8,
    pub name: &'static str,
    /// Length in code units.
    pub units: u32,
    /// Registers in operand order (for ranges: every register in the range).
    pub regs: Vec<u32>,
    /// True for `/range` forms.
    pub range: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub literal: Option<i64>,
    /// Absolute branch or payload target, in code units.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<i64>,
    pub index_kind: IndexKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    /// Second index (the proto of `invoke-polymorphic`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index2: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Payload>,
}

/// Most entries decoded from one payload.
const MAX_PAYLOAD: usize = 65_536;

fn u32_at(code: &[u16], i: usize) -> Option<u32> {
    Some(u32::from(*code.get(i)?) | (u32::from(*code.get(i + 1)?) << 16))
}

/// Decode a payload pseudo-instruction at `pc`; returns it and its length in units.
fn payload(code: &[u16], pc: usize) -> Option<(Payload, usize)> {
    let ident = *code.get(pc)?;
    match ident {
        0x0100 => {
            let size = usize::from(*code.get(pc + 1)?);
            let first_key = u32_at(code, pc + 2)? as i32;
            let mut targets = Vec::with_capacity(size.min(MAX_PAYLOAD));
            for i in 0..size.min(MAX_PAYLOAD) {
                targets.push(u32_at(code, pc + 4 + i * 2)? as i32);
            }
            Some((Payload::PackedSwitch { first_key, targets }, size * 2 + 4))
        }
        0x0200 => {
            let size = usize::from(*code.get(pc + 1)?);
            let mut keys = Vec::with_capacity(size.min(MAX_PAYLOAD));
            let mut targets = Vec::with_capacity(size.min(MAX_PAYLOAD));
            for i in 0..size.min(MAX_PAYLOAD) {
                keys.push(u32_at(code, pc + 2 + i * 2)? as i32);
                targets.push(u32_at(code, pc + 2 + size * 2 + i * 2)? as i32);
            }
            Some((Payload::SparseSwitch { keys, targets }, size * 4 + 2))
        }
        0x0300 => {
            let width = *code.get(pc + 1)?;
            let count = u32_at(code, pc + 2)?;
            let bytes = usize::from(width).checked_mul(count as usize)?;
            let units = bytes.checked_add(1)? / 2 + 4;
            if pc.checked_add(units)? > code.len() {
                return None;
            }
            let mut data = Vec::with_capacity(bytes.min(MAX_PAYLOAD));
            for i in 0..bytes.min(MAX_PAYLOAD) {
                let u = *code.get(pc + 4 + i / 2)?;
                data.push(if i % 2 == 0 { u as u8 } else { (u >> 8) as u8 });
            }
            Some((Payload::FillArrayData { element_width: width, count, data }, units))
        }
        _ => None,
    }
}

/// Decode one instruction at `pc`, or `None` when it runs past the end.
pub fn decode_one(code: &[u16], pc: usize) -> Option<Insn> {
    use Fmt::*;
    let u0 = *code.get(pc)?;
    let b = (u0 & 0xff) as u8;
    let hi = (u0 >> 8) as u8;
    if b == 0 && hi != 0 {
        let (p, units) = payload(code, pc)?;
        let name = match p {
            Payload::PackedSwitch { .. } => "packed-switch-payload",
            Payload::SparseSwitch { .. } => "sparse-switch-payload",
            Payload::FillArrayData { .. } => "array-data-payload",
        };
        return Some(Insn {
            offset: pc as u32,
            opcode: 0,
            name,
            units: units as u32,
            regs: vec![],
            range: false,
            literal: None,
            target: None,
            index_kind: IndexKind::None,
            index: None,
            index2: None,
            payload: Some(p),
        });
    }
    let o = opcode(b);
    let n = o.fmt.units();
    if pc.checked_add(n)? > code.len() {
        return None;
    }
    let u = |i: usize| code.get(pc + i).copied().unwrap_or(0);
    let a4 = u32::from(hi & 0x0f);
    let b4 = u32::from(hi >> 4);
    let aa = u32::from(hi);
    let rel = |d: i64| pc as i64 + d;
    let mut insn = Insn {
        offset: pc as u32,
        opcode: b,
        name: o.name,
        units: n as u32,
        regs: vec![],
        range: false,
        literal: None,
        target: None,
        index_kind: o.index,
        index: None,
        index2: None,
        payload: None,
    };
    match o.fmt {
        F10x => {}
        F12x => insn.regs = vec![a4, b4],
        F11n => {
            insn.regs = vec![a4];
            insn.literal = Some(i64::from((hi as i8) >> 4));
        }
        F11x => insn.regs = vec![aa],
        F10t => insn.target = Some(rel(i64::from(hi as i8))),
        F20t => insn.target = Some(rel(i64::from(u(1) as i16))),
        F22x => insn.regs = vec![aa, u32::from(u(1))],
        F21t => {
            insn.regs = vec![aa];
            insn.target = Some(rel(i64::from(u(1) as i16)));
        }
        F21s => {
            insn.regs = vec![aa];
            insn.literal = Some(i64::from(u(1) as i16));
        }
        F21h => {
            insn.regs = vec![aa];
            let v = i64::from(u(1) as i16);
            insn.literal = Some(if b == 0x19 { v << 48 } else { v << 16 });
        }
        F21c => {
            insn.regs = vec![aa];
            insn.index = Some(u32::from(u(1)));
        }
        F23x => insn.regs = vec![aa, u32::from(u(1) & 0xff), u32::from(u(1) >> 8)],
        F22b => {
            insn.regs = vec![aa, u32::from(u(1) & 0xff)];
            insn.literal = Some(i64::from((u(1) >> 8) as u8 as i8));
        }
        F22t => {
            insn.regs = vec![a4, b4];
            insn.target = Some(rel(i64::from(u(1) as i16)));
        }
        F22s => {
            insn.regs = vec![a4, b4];
            insn.literal = Some(i64::from(u(1) as i16));
        }
        F22c => {
            insn.regs = vec![a4, b4];
            insn.index = Some(u32::from(u(1)));
        }
        F30t => insn.target = Some(rel(i64::from((u32::from(u(1)) | (u32::from(u(2)) << 16)) as i32))),
        F32x => insn.regs = vec![u32::from(u(1)), u32::from(u(2))],
        F31i => {
            insn.regs = vec![aa];
            insn.literal = Some(i64::from((u32::from(u(1)) | (u32::from(u(2)) << 16)) as i32));
        }
        F31t => {
            insn.regs = vec![aa];
            insn.target = Some(rel(i64::from((u32::from(u(1)) | (u32::from(u(2)) << 16)) as i32)));
        }
        F31c => {
            insn.regs = vec![aa];
            insn.index = Some(u32::from(u(1)) | (u32::from(u(2)) << 16));
        }
        F35c | F45cc => {
            let count = b4.min(5) as usize;
            let g = a4;
            let w = u(2);
            let all = [u32::from(w & 0xf), u32::from((w >> 4) & 0xf), u32::from((w >> 8) & 0xf), u32::from(w >> 12), g];
            insn.regs = all.iter().take(count).copied().collect();
            insn.index = Some(u32::from(u(1)));
            if o.fmt == F45cc {
                insn.index2 = Some(u32::from(u(3)));
            }
        }
        F3rc | F4rcc => {
            let first = u32::from(u(2));
            insn.regs = (0..aa).map(|i| first.saturating_add(i)).collect();
            insn.range = true;
            insn.index = Some(u32::from(u(1)));
            if o.fmt == F4rcc {
                insn.index2 = Some(u32::from(u(3)));
            }
        }
        F51l => {
            insn.regs = vec![aa];
            let v = u64::from(u(1)) | (u64::from(u(2)) << 16) | (u64::from(u(3)) << 32) | (u64::from(u(4)) << 48);
            insn.literal = Some(v as i64);
        }
    }
    Some(insn)
}

/// Decode a whole method body, attaching each switch/array payload to the instruction that
/// references it. Stops at `max` instructions.
pub fn decode(code: &[u16], max: usize) -> (Vec<Insn>, bool) {
    let mut out = Vec::new();
    let mut pc = 0usize;
    let mut complete = true;
    while pc < code.len() {
        if out.len() >= max {
            complete = false;
            break;
        }
        match decode_one(code, pc) {
            Some(i) => {
                pc += (i.units as usize).max(1);
                out.push(i);
            }
            None => {
                complete = false;
                break;
            }
        }
    }
    // Attach payloads to their users (31t formats).
    let payloads: Vec<(i64, Payload)> = out.iter().filter_map(|i| i.payload.clone().map(|p| (i64::from(i.offset), p))).collect();
    for i in out.iter_mut().filter(|i| matches!(i.opcode, 0x26 | 0x2b | 0x2c)) {
        if let Some(t) = i.target
            && let Some((_, p)) = payloads.iter().find(|(o, _)| *o == t)
        {
            i.payload = Some(p.clone());
        }
    }
    (out, complete)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_complete() {
        let mut named = 0;
        for b in 0..=255u8 {
            let o = opcode(b);
            if o.name != "unused" {
                named += 1;
            }
        }
        // 256 slots minus the 0x3e-0x43, 0x73, 0x79-0x7a and 0xe3-0xf9 holes.
        assert_eq!(named, 256 - 6 - 1 - 2 - 23);
        assert_eq!(opcode(0x6e).name, "invoke-virtual");
        assert_eq!(opcode(0x8f).name, "int-to-short");
        assert_eq!(opcode(0xaf).name, "rem-double");
        assert_eq!(opcode(0xcf).name, "rem-double/2addr");
        assert_eq!(opcode(0xe2).name, "ushr-int/lit8");
    }

    #[test]
    fn decodes_formats() {
        // const/4 v1, #-1 ; invoke-static {v0, v1}, meth@5 ; packed-switch v0, +4 ; return-void
        // payload: packed, size 2, first 10, targets 1, 2
        let code: Vec<u16> = vec![0xf112, 0x2071, 5, 0x0010, 0x002b, 4, 0, 0x000e, 0x0100, 2, 10, 0, 1, 0, 2, 0];
        let (ins, complete) = decode(&code, 100);
        assert!(complete);
        assert_eq!(ins[0].name, "const/4");
        assert_eq!(ins[0].literal, Some(-1));
        assert_eq!(ins[0].regs, vec![1]);
        assert_eq!(ins[1].name, "invoke-static");
        assert_eq!(ins[1].regs, vec![0, 1]);
        assert_eq!(ins[1].index, Some(5));
        assert_eq!(ins[2].name, "packed-switch");
        assert_eq!(ins[2].target, Some(8));
        assert_eq!(ins[2].payload, Some(Payload::PackedSwitch { first_key: 10, targets: vec![1, 2] }));
        assert_eq!(ins[4].name, "packed-switch-payload");
        // const-wide v0, 0x1122334455667788
        let (w, _) = decode(&[0x0018, 0x7788, 0x5566, 0x3344, 0x1122], 10);
        assert_eq!(w[0].literal, Some(0x1122_3344_5566_7788));
        // fill-array-data payload width 1 count 3
        let (f, _) = decode(&[0x0300, 1, 3, 0, 0x0201, 0x0003], 10);
        assert_eq!(f[0].payload, Some(Payload::FillArrayData { element_width: 1, count: 3, data: vec![1, 2, 3] }));
        // invoke-virtual/range {v4..v6}
        let (r, _) = decode(&[0x0374, 9, 4], 10);
        assert_eq!(r[0].regs, vec![4, 5, 6]);
        assert!(r[0].range);
    }

    #[test]
    fn hostile() {
        for seed in 0..200 {
            let n = craft_anything_testkit::noise(seed, 256);
            let code: Vec<u16> = n.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
            let _ = decode(&code, 1000);
        }
        let _ = decode(&[0x0300, 0xffff, 0xffff, 0xffff], 10);
        let _ = decode(&[0x0200, 0xffff], 10);
        let _ = decode(&[0x0018], 10);
    }
}

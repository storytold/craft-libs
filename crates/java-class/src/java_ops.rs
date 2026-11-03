//! JVM bytecode: the opcode table (every defined opcode plus the `wide`, `tableswitch` and
//! `lookupswitch` variable-length forms) and a decoder for each instruction. Pure decoding;
//! constant-pool index operands are resolved by the caller (see [`crate::java_class`]).

use serde::Serialize;

/// The shape of an opcode's operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Operands {
    /// No operands.
    None,
    /// One signed byte immediate (`bipush`).
    Byte,
    /// One signed short immediate (`sipush`).
    Short,
    /// A one-byte local-variable index.
    Local,
    /// `iinc`: a one-byte local index and a signed byte increment.
    LocalConst,
    /// A two-byte signed branch offset.
    Branch2,
    /// A four-byte signed branch offset (`goto_w`, `jsr_w`).
    Branch4,
    /// A one-byte constant-pool index (`ldc`).
    Cp1,
    /// A two-byte constant-pool index.
    Cp2,
    /// `newarray`: a one-byte primitive array type.
    NewArray,
    /// `invokeinterface`: a two-byte index, a count byte and a zero byte.
    InvokeInterface,
    /// `invokedynamic`: a two-byte index and two zero bytes.
    InvokeDynamic,
    /// `multianewarray`: a two-byte class index and a dimension byte.
    MultiANewArray,
    /// `tableswitch` (variable length, 4-byte aligned).
    TableSwitch,
    /// `lookupswitch` (variable length, 4-byte aligned).
    LookupSwitch,
    /// `wide`: a prefix that widens the following load/store/ret/iinc.
    Wide,
    /// A reserved or unknown opcode.
    Reserved,
}

/// One opcode slot.
#[derive(Debug, Clone, Copy)]
pub struct Op {
    pub name: &'static str,
    pub operands: Operands,
}

const fn op(name: &'static str, operands: Operands) -> Op {
    Op { name, operands }
}

/// The opcode table entry for `b`.
pub fn opcode(b: u8) -> Op {
    use Operands as O;
    match b {
        0x00 => op("nop", O::None),
        0x01 => op("aconst_null", O::None),
        0x02 => op("iconst_m1", O::None),
        0x03 => op("iconst_0", O::None),
        0x04 => op("iconst_1", O::None),
        0x05 => op("iconst_2", O::None),
        0x06 => op("iconst_3", O::None),
        0x07 => op("iconst_4", O::None),
        0x08 => op("iconst_5", O::None),
        0x09 => op("lconst_0", O::None),
        0x0a => op("lconst_1", O::None),
        0x0b => op("fconst_0", O::None),
        0x0c => op("fconst_1", O::None),
        0x0d => op("fconst_2", O::None),
        0x0e => op("dconst_0", O::None),
        0x0f => op("dconst_1", O::None),
        0x10 => op("bipush", O::Byte),
        0x11 => op("sipush", O::Short),
        0x12 => op("ldc", O::Cp1),
        0x13 => op("ldc_w", O::Cp2),
        0x14 => op("ldc2_w", O::Cp2),
        0x15 => op("iload", O::Local),
        0x16 => op("lload", O::Local),
        0x17 => op("fload", O::Local),
        0x18 => op("dload", O::Local),
        0x19 => op("aload", O::Local),
        0x1a => op("iload_0", O::None),
        0x1b => op("iload_1", O::None),
        0x1c => op("iload_2", O::None),
        0x1d => op("iload_3", O::None),
        0x1e => op("lload_0", O::None),
        0x1f => op("lload_1", O::None),
        0x20 => op("lload_2", O::None),
        0x21 => op("lload_3", O::None),
        0x22 => op("fload_0", O::None),
        0x23 => op("fload_1", O::None),
        0x24 => op("fload_2", O::None),
        0x25 => op("fload_3", O::None),
        0x26 => op("dload_0", O::None),
        0x27 => op("dload_1", O::None),
        0x28 => op("dload_2", O::None),
        0x29 => op("dload_3", O::None),
        0x2a => op("aload_0", O::None),
        0x2b => op("aload_1", O::None),
        0x2c => op("aload_2", O::None),
        0x2d => op("aload_3", O::None),
        0x2e => op("iaload", O::None),
        0x2f => op("laload", O::None),
        0x30 => op("faload", O::None),
        0x31 => op("daload", O::None),
        0x32 => op("aaload", O::None),
        0x33 => op("baload", O::None),
        0x34 => op("caload", O::None),
        0x35 => op("saload", O::None),
        0x36 => op("istore", O::Local),
        0x37 => op("lstore", O::Local),
        0x38 => op("fstore", O::Local),
        0x39 => op("dstore", O::Local),
        0x3a => op("astore", O::Local),
        0x3b => op("istore_0", O::None),
        0x3c => op("istore_1", O::None),
        0x3d => op("istore_2", O::None),
        0x3e => op("istore_3", O::None),
        0x3f => op("lstore_0", O::None),
        0x40 => op("lstore_1", O::None),
        0x41 => op("lstore_2", O::None),
        0x42 => op("lstore_3", O::None),
        0x43 => op("fstore_0", O::None),
        0x44 => op("fstore_1", O::None),
        0x45 => op("fstore_2", O::None),
        0x46 => op("fstore_3", O::None),
        0x47 => op("dstore_0", O::None),
        0x48 => op("dstore_1", O::None),
        0x49 => op("dstore_2", O::None),
        0x4a => op("dstore_3", O::None),
        0x4b => op("astore_0", O::None),
        0x4c => op("astore_1", O::None),
        0x4d => op("astore_2", O::None),
        0x4e => op("astore_3", O::None),
        0x4f => op("iastore", O::None),
        0x50 => op("lastore", O::None),
        0x51 => op("fastore", O::None),
        0x52 => op("dastore", O::None),
        0x53 => op("aastore", O::None),
        0x54 => op("bastore", O::None),
        0x55 => op("castore", O::None),
        0x56 => op("sastore", O::None),
        0x57 => op("pop", O::None),
        0x58 => op("pop2", O::None),
        0x59 => op("dup", O::None),
        0x5a => op("dup_x1", O::None),
        0x5b => op("dup_x2", O::None),
        0x5c => op("dup2", O::None),
        0x5d => op("dup2_x1", O::None),
        0x5e => op("dup2_x2", O::None),
        0x5f => op("swap", O::None),
        0x60 => op("iadd", O::None),
        0x61 => op("ladd", O::None),
        0x62 => op("fadd", O::None),
        0x63 => op("dadd", O::None),
        0x64 => op("isub", O::None),
        0x65 => op("lsub", O::None),
        0x66 => op("fsub", O::None),
        0x67 => op("dsub", O::None),
        0x68 => op("imul", O::None),
        0x69 => op("lmul", O::None),
        0x6a => op("fmul", O::None),
        0x6b => op("dmul", O::None),
        0x6c => op("idiv", O::None),
        0x6d => op("ldiv", O::None),
        0x6e => op("fdiv", O::None),
        0x6f => op("ddiv", O::None),
        0x70 => op("irem", O::None),
        0x71 => op("lrem", O::None),
        0x72 => op("frem", O::None),
        0x73 => op("drem", O::None),
        0x74 => op("ineg", O::None),
        0x75 => op("lneg", O::None),
        0x76 => op("fneg", O::None),
        0x77 => op("dneg", O::None),
        0x78 => op("ishl", O::None),
        0x79 => op("lshl", O::None),
        0x7a => op("ishr", O::None),
        0x7b => op("lshr", O::None),
        0x7c => op("iushr", O::None),
        0x7d => op("lushr", O::None),
        0x7e => op("iand", O::None),
        0x7f => op("land", O::None),
        0x80 => op("ior", O::None),
        0x81 => op("lor", O::None),
        0x82 => op("ixor", O::None),
        0x83 => op("lxor", O::None),
        0x84 => op("iinc", O::LocalConst),
        0x85 => op("i2l", O::None),
        0x86 => op("i2f", O::None),
        0x87 => op("i2d", O::None),
        0x88 => op("l2i", O::None),
        0x89 => op("l2f", O::None),
        0x8a => op("l2d", O::None),
        0x8b => op("f2i", O::None),
        0x8c => op("f2l", O::None),
        0x8d => op("f2d", O::None),
        0x8e => op("d2i", O::None),
        0x8f => op("d2l", O::None),
        0x90 => op("d2f", O::None),
        0x91 => op("i2b", O::None),
        0x92 => op("i2c", O::None),
        0x93 => op("i2s", O::None),
        0x94 => op("lcmp", O::None),
        0x95 => op("fcmpl", O::None),
        0x96 => op("fcmpg", O::None),
        0x97 => op("dcmpl", O::None),
        0x98 => op("dcmpg", O::None),
        0x99 => op("ifeq", O::Branch2),
        0x9a => op("ifne", O::Branch2),
        0x9b => op("iflt", O::Branch2),
        0x9c => op("ifge", O::Branch2),
        0x9d => op("ifgt", O::Branch2),
        0x9e => op("ifle", O::Branch2),
        0x9f => op("if_icmpeq", O::Branch2),
        0xa0 => op("if_icmpne", O::Branch2),
        0xa1 => op("if_icmplt", O::Branch2),
        0xa2 => op("if_icmpge", O::Branch2),
        0xa3 => op("if_icmpgt", O::Branch2),
        0xa4 => op("if_icmple", O::Branch2),
        0xa5 => op("if_acmpeq", O::Branch2),
        0xa6 => op("if_acmpne", O::Branch2),
        0xa7 => op("goto", O::Branch2),
        0xa8 => op("jsr", O::Branch2),
        0xa9 => op("ret", O::Local),
        0xaa => op("tableswitch", O::TableSwitch),
        0xab => op("lookupswitch", O::LookupSwitch),
        0xac => op("ireturn", O::None),
        0xad => op("lreturn", O::None),
        0xae => op("freturn", O::None),
        0xaf => op("dreturn", O::None),
        0xb0 => op("areturn", O::None),
        0xb1 => op("return", O::None),
        0xb2 => op("getstatic", O::Cp2),
        0xb3 => op("putstatic", O::Cp2),
        0xb4 => op("getfield", O::Cp2),
        0xb5 => op("putfield", O::Cp2),
        0xb6 => op("invokevirtual", O::Cp2),
        0xb7 => op("invokespecial", O::Cp2),
        0xb8 => op("invokestatic", O::Cp2),
        0xb9 => op("invokeinterface", O::InvokeInterface),
        0xba => op("invokedynamic", O::InvokeDynamic),
        0xbb => op("new", O::Cp2),
        0xbc => op("newarray", O::NewArray),
        0xbd => op("anewarray", O::Cp2),
        0xbe => op("arraylength", O::None),
        0xbf => op("athrow", O::None),
        0xc0 => op("checkcast", O::Cp2),
        0xc1 => op("instanceof", O::Cp2),
        0xc2 => op("monitorenter", O::None),
        0xc3 => op("monitorexit", O::None),
        0xc4 => op("wide", O::Wide),
        0xc5 => op("multianewarray", O::MultiANewArray),
        0xc6 => op("ifnull", O::Branch2),
        0xc7 => op("ifnonnull", O::Branch2),
        0xc8 => op("goto_w", O::Branch4),
        0xc9 => op("jsr_w", O::Branch4),
        0xca => op("breakpoint", O::None),
        0xfe => op("impdep1", O::None),
        0xff => op("impdep2", O::None),
        _ => op("unknown", O::Reserved),
    }
}

/// The primitive array type for a `newarray` atype operand.
pub fn array_type(atype: i32) -> &'static str {
    match atype {
        4 => "boolean",
        5 => "char",
        6 => "float",
        7 => "double",
        8 => "byte",
        9 => "short",
        10 => "int",
        11 => "long",
        _ => "?",
    }
}

/// A decoded `tableswitch`/`lookupswitch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Switch {
    /// Absolute byte offset of the default branch.
    pub default: i64,
    /// `(key, absolute target)` pairs in listing order.
    pub pairs: Vec<(i32, i64)>,
}

/// One decoded instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Insn {
    /// Byte offset from the start of the method's code.
    pub pc: u32,
    pub opcode: u8,
    pub name: &'static str,
    /// Total length in bytes, including operands.
    pub len: u32,
    /// Local-variable index operand (load/store/ret/iinc).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local: Option<u16>,
    /// Integer immediate (`bipush`/`sipush`/`iinc` increment/`newarray` type/`multianewarray` dims).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub int: Option<i32>,
    /// Constant-pool index operand (resolved by the caller).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cp_index: Option<u16>,
    /// Absolute byte offset of a branch target.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<i64>,
    /// A switch table when the opcode is `tableswitch`/`lookupswitch`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch: Option<Switch>,
    /// True when this instruction was prefixed by `wide`.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub wide: bool,
}

/// Most entries decoded from one switch table.
const MAX_SWITCH: usize = 1 << 20;

fn s8(code: &[u8], at: usize) -> Option<i32> {
    code.get(at).map(|&b| i32::from(b as i8))
}
fn u16b(code: &[u8], at: usize) -> Option<u16> {
    Some(u16::from(*code.get(at)?) << 8 | u16::from(*code.get(at + 1)?))
}
fn s16(code: &[u8], at: usize) -> Option<i32> {
    u16b(code, at).map(|v| i32::from(v as i16))
}
fn s32(code: &[u8], at: usize) -> Option<i32> {
    Some(i32::from(*code.get(at)?) << 24 | i32::from(*code.get(at + 1)?) << 16 | i32::from(*code.get(at + 2)?) << 8 | i32::from(*code.get(at + 3)?))
}

fn base(pc: usize, opcode: u8, name: &'static str) -> Insn {
    Insn { pc: pc as u32, opcode, name, len: 1, local: None, int: None, cp_index: None, branch: None, switch: None, wide: false }
}

/// Decode one instruction at `pc`, or `None` when it (or its operands) runs past the end.
pub fn decode_one(code: &[u8], pc: usize) -> Option<Insn> {
    let b = *code.get(pc)?;
    let o = opcode(b);
    let mut insn = base(pc, b, o.name);
    let rel = |d: i32| pc as i64 + i64::from(d);
    match o.operands {
        Operands::None | Operands::Reserved => {}
        Operands::Byte => {
            insn.int = Some(s8(code, pc + 1)?);
            insn.len = 2;
        }
        Operands::Short => {
            insn.int = Some(s16(code, pc + 1)?);
            insn.len = 3;
        }
        Operands::Local => {
            insn.local = Some(u16::from(*code.get(pc + 1)?));
            insn.len = 2;
        }
        Operands::LocalConst => {
            insn.local = Some(u16::from(*code.get(pc + 1)?));
            insn.int = Some(s8(code, pc + 2)?);
            insn.len = 3;
        }
        Operands::Branch2 => {
            insn.branch = Some(rel(s16(code, pc + 1)?));
            insn.len = 3;
        }
        Operands::Branch4 => {
            insn.branch = Some(rel(s32(code, pc + 1)?));
            insn.len = 5;
        }
        Operands::Cp1 => {
            insn.cp_index = Some(u16::from(*code.get(pc + 1)?));
            insn.len = 2;
        }
        Operands::Cp2 => {
            insn.cp_index = Some(u16b(code, pc + 1)?);
            insn.len = 3;
        }
        Operands::NewArray => {
            insn.int = Some(i32::from(*code.get(pc + 1)?));
            insn.len = 2;
        }
        Operands::InvokeInterface => {
            insn.cp_index = Some(u16b(code, pc + 1)?);
            if *code.get(pc + 3)? == 0 || *code.get(pc + 4)? != 0 {
                return None;
            }
            insn.len = 5;
        }
        Operands::InvokeDynamic => {
            insn.cp_index = Some(u16b(code, pc + 1)?);
            if *code.get(pc + 3)? != 0 || *code.get(pc + 4)? != 0 {
                return None;
            }
            insn.len = 5;
        }
        Operands::MultiANewArray => {
            insn.cp_index = Some(u16b(code, pc + 1)?);
            insn.int = Some(i32::from(*code.get(pc + 3)?));
            insn.len = 4;
        }
        Operands::Wide => return decode_wide(code, pc),
        Operands::TableSwitch => return decode_tableswitch(code, pc),
        Operands::LookupSwitch => return decode_lookupswitch(code, pc),
    }
    if pc.checked_add(insn.len as usize)? > code.len() {
        return None;
    }
    Some(insn)
}

fn decode_wide(code: &[u8], pc: usize) -> Option<Insn> {
    let real = *code.get(pc + 1)?;
    if !matches!(real, 0x15..=0x19 | 0x36..=0x3a | 0x84 | 0xa9) {
        return None;
    }
    let o = opcode(real);
    let mut insn = base(pc, real, o.name);
    insn.wide = true;
    if real == 0x84 {
        // wide iinc: 2-byte local, 2-byte signed const
        insn.local = Some(u16b(code, pc + 2)?);
        insn.int = Some(s16(code, pc + 4)?);
        insn.len = 6;
    } else {
        // wide load/store/ret: 2-byte local
        insn.local = Some(u16b(code, pc + 2)?);
        insn.len = 4;
    }
    if pc.checked_add(insn.len as usize)? > code.len() {
        return None;
    }
    Some(insn)
}

/// Padding so the operand word after a switch opcode is 4-byte aligned from the code start.
fn switch_pad(pc: usize) -> usize {
    (4 - ((pc + 1) % 4)) % 4
}

fn decode_tableswitch(code: &[u8], pc: usize) -> Option<Insn> {
    let mut at = pc + 1 + switch_pad(pc);
    let default = rel_i64(pc, s32(code, at)?);
    let low = s32(code, at + 4)?;
    let high = s32(code, at + 8)?;
    at += 12;
    if high < low {
        return None;
    }
    let count = (i64::from(high) - i64::from(low) + 1).max(0) as usize;
    if count > MAX_SWITCH {
        return None;
    }
    let mut pairs = Vec::with_capacity(count.min(MAX_SWITCH));
    for i in 0..count {
        let off = s32(code, at + i * 4)?;
        pairs.push((low.wrapping_add(i as i32), rel_i64(pc, off)));
    }
    let end = at + count * 4;
    let mut insn = base(pc, 0xaa, "tableswitch");
    insn.switch = Some(Switch { default, pairs });
    insn.len = (end - pc) as u32;
    Some(insn)
}

fn decode_lookupswitch(code: &[u8], pc: usize) -> Option<Insn> {
    let mut at = pc + 1 + switch_pad(pc);
    let default = rel_i64(pc, s32(code, at)?);
    let npairs = s32(code, at + 4)?;
    at += 8;
    if npairs < 0 {
        return None;
    }
    let count = npairs as usize;
    if count > MAX_SWITCH {
        return None;
    }
    let mut pairs = Vec::with_capacity(count.min(MAX_SWITCH));
    for i in 0..count {
        let key = s32(code, at + i * 8)?;
        let off = s32(code, at + i * 8 + 4)?;
        pairs.push((key, rel_i64(pc, off)));
    }
    let end = at + count * 8;
    let mut insn = base(pc, 0xab, "lookupswitch");
    insn.switch = Some(Switch { default, pairs });
    insn.len = (end - pc) as u32;
    Some(insn)
}

fn rel_i64(pc: usize, off: i32) -> i64 {
    pc as i64 + i64::from(off)
}

/// Decode a whole method body, stopping at `max` instructions. `complete` is false when a
/// trailing instruction runs past the end or the cap was hit.
pub fn decode(code: &[u8], max: usize) -> (Vec<Insn>, bool) {
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
                pc += (i.len as usize).max(1);
                out.push(i);
            }
            None => {
                complete = false;
                break;
            }
        }
    }
    (out, complete)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_covers_defined_opcodes() {
        let mut named = 0;
        for b in 0..=255u8 {
            if opcode(b).name != "unknown" {
                named += 1;
            }
        }
        // 0x00..=0xca is 203 opcodes, plus impdep1/impdep2.
        assert_eq!(named, 203 + 2);
        assert_eq!(opcode(0xb6).name, "invokevirtual");
        assert_eq!(opcode(0x10).name, "bipush");
        assert_eq!(opcode(0xc4).name, "wide");
    }

    #[test]
    fn decodes_core_formats() {
        // iconst_1; bipush 7; istore_1; goto +3 ; (pad to reach len) return
        let code = [0x04, 0x10, 0x07, 0x3c, 0xa7, 0x00, 0x03, 0xb1];
        let (ins, complete) = decode(&code, 100);
        assert!(complete, "{ins:?}");
        assert_eq!(ins[0].name, "iconst_1");
        assert_eq!(ins[1].name, "bipush");
        assert_eq!(ins[1].int, Some(7));
        assert_eq!(ins[2].name, "istore_1");
        assert_eq!(ins[3].name, "goto");
        assert_eq!(ins[3].branch, Some(7)); // pc 4 + 3
        assert_eq!(ins[4].name, "return");
    }

    #[test]
    fn decodes_invoke_and_wide() {
        // invokevirtual #5 ; invokeinterface #6 count 2 ; wide iload 300
        let code = [0xb6, 0x00, 0x05, 0xb9, 0x00, 0x06, 0x02, 0x00, 0xc4, 0x15, 0x01, 0x2c];
        let (ins, complete) = decode(&code, 100);
        assert!(complete);
        assert_eq!(ins[0].name, "invokevirtual");
        assert_eq!(ins[0].cp_index, Some(5));
        assert_eq!(ins[0].len, 3);
        assert_eq!(ins[1].name, "invokeinterface");
        assert_eq!(ins[1].cp_index, Some(6));
        assert_eq!(ins[1].len, 5);
        assert_eq!(ins[2].name, "iload");
        assert!(ins[2].wide);
        assert_eq!(ins[2].local, Some(300));
        assert_eq!(ins[2].len, 4);
    }

    #[test]
    fn decodes_switches() {
        // tableswitch at pc 0: pad 3, default +40, low 1, high 2, offsets +10, +20
        let mut code = vec![0xaa, 0, 0, 0];
        code.extend_from_slice(&40i32.to_be_bytes());
        code.extend_from_slice(&1i32.to_be_bytes());
        code.extend_from_slice(&2i32.to_be_bytes());
        code.extend_from_slice(&10i32.to_be_bytes());
        code.extend_from_slice(&20i32.to_be_bytes());
        let i = decode_one(&code, 0).unwrap();
        assert_eq!(i.name, "tableswitch");
        let sw = i.switch.unwrap();
        assert_eq!(sw.default, 40);
        assert_eq!(sw.pairs, vec![(1, 10), (2, 20)]);
        assert_eq!(i.len as usize, code.len());

        // lookupswitch at pc 0: pad 3, default +4, npairs 1, (key 99 -> +8)
        let mut lk = vec![0xab, 0, 0, 0];
        lk.extend_from_slice(&4i32.to_be_bytes());
        lk.extend_from_slice(&1i32.to_be_bytes());
        lk.extend_from_slice(&99i32.to_be_bytes());
        lk.extend_from_slice(&8i32.to_be_bytes());
        let l = decode_one(&lk, 0).unwrap();
        let sw = l.switch.unwrap();
        assert_eq!(sw.default, 4);
        assert_eq!(sw.pairs, vec![(99, 8)]);
    }

    #[test]
    fn hostile() {
        for seed in 0..256 {
            let n = craft_anything_testkit::noise(seed, 512);
            let _ = decode(&n, 10_000);
        }
        // truncated operands never panic
        let _ = decode(&[0xb6, 0x00], 10);
        let _ = decode(&[0xaa, 0, 0, 0, 0, 0, 0, 0], 10);
        let _ = decode(&[0xab, 0, 0, 0, 0, 0, 0, 0, 0x7f, 0xff, 0xff, 0xff], 10);
        let _ = decode(&[0xc4], 10);
    }
}

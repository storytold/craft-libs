//! Dalvik executables (DEX): header, string/type/proto/field/method ids, class definitions,
//! class data, code items with try/catch handlers, class annotations, a smali-like listing of
//! method bodies and a reference finder. Counts are checked against the file size, strings are
//! decoded from MUTF-8 under a total budget, and every read is bounds-checked.

use crate::FormatError;
use crate::bytes::Reader;
use crate::dex_ops::{IndexKind, Insn, Payload};
use serde::Serialize;

pub const NO_INDEX: u32 = 0xffff_ffff;
/// Longest string decoded (UTF-16 units).
const MAX_STRING_UNITS: usize = 16 * 1024;
/// Total UTF-16 units decoded across all strings of one file.
const STRING_BUDGET: usize = 64 * 1024 * 1024;
/// Most members per class list.
const MAX_MEMBERS: usize = 65_536;
/// Most try blocks / handlers per method.
const MAX_TRIES: usize = 4096;
const MAX_HANDLERS: usize = 256;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Header {
    pub version: String,
    pub checksum: String,
    /// Adler-32 recomputed over the file.
    pub checksum_ok: bool,
    /// SHA-1 field as stored (not recomputed).
    pub signature: String,
    pub file_size: u32,
    pub header_size: u32,
    pub endian_tag: String,
    pub string_ids: u32,
    pub type_ids: u32,
    pub proto_ids: u32,
    pub field_ids: u32,
    pub method_ids: u32,
    pub class_defs: u32,
    pub data_size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProtoId {
    shorty: u32,
    ret: u32,
    params_off: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberId {
    pub class_idx: u16,
    /// Type (fields) or proto (methods).
    pub type_or_proto: u16,
    pub name_idx: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassDef {
    pub class_idx: u32,
    pub access: u32,
    pub superclass_idx: u32,
    pub interfaces_off: u32,
    pub source_file_idx: u32,
    pub annotations_off: u32,
    pub class_data_off: u32,
    pub static_values_off: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EncodedField {
    pub field_idx: u32,
    pub access: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct EncodedMethod {
    pub method_idx: u32,
    pub access: u32,
    pub code_off: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassData {
    pub static_fields: Vec<EncodedField>,
    pub instance_fields: Vec<EncodedField>,
    pub direct_methods: Vec<EncodedMethod>,
    pub virtual_methods: Vec<EncodedMethod>,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Handler {
    /// Exception type descriptor; `None` for catch-all.
    pub exception: Option<String>,
    pub address: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Try {
    pub start: u32,
    pub end: u32,
    pub handlers: Vec<Handler>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Code {
    pub registers: u16,
    pub ins: u16,
    pub outs: u16,
    pub insns: Vec<u16>,
    pub tries: Vec<Try>,
    pub debug_info_off: u32,
}

/// A parsed DEX file.
#[derive(Debug, Clone)]
pub struct Dex<'a> {
    data: &'a [u8],
    pub header: Header,
    pub strings: Vec<String>,
    types: Vec<u32>,
    protos: Vec<ProtoId>,
    pub fields: Vec<MemberId>,
    pub methods: Vec<MemberId>,
    pub classes: Vec<ClassDef>,
    pub warnings: Vec<String>,
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

/// Decode modified UTF-8 (as used by DEX and class files) to a String.
pub fn mutf8(r: &Reader<'_>, mut at: usize, max_units: usize) -> (String, usize) {
    let mut units: Vec<u16> = Vec::new();
    let mut consumed = 0usize;
    while units.len() < max_units {
        let Some(b0) = r.u8(at) else { break };
        if b0 == 0 {
            break;
        }
        let (u, n) = crate::bytes::mutf8_unit(r.data().get(at..).unwrap_or_default());
        units.push(u);
        at = at.saturating_add(n);
        consumed += n;
    }
    (String::from_utf16_lossy(&units), consumed)
}

/// Signed LEB128 at `off`: (value, bytes).
pub fn sleb128(r: &Reader<'_>, off: usize) -> Option<(i64, usize)> {
    let mut result = 0i64;
    let mut shift = 0u32;
    for i in 0..10usize {
        let b = r.u8(off.checked_add(i)?)?;
        // Only one value bit remains in the tenth byte; the rest must extend its sign.
        if i == 9 && !matches!(b, 0x00 | 0x7f) {
            return None;
        }
        result |= i64::from(b & 0x7f).checked_shl(shift)?;
        shift += 7;
        if b & 0x80 == 0 {
            if shift < 64 && b & 0x40 != 0 {
                result |= -1i64 << shift;
            }
            return Some((result, i + 1));
        }
    }
    None
}

/// `Lcom/x/Y;` → `com.x.Y`; `[I` → `int[]`.
pub fn java_name(desc: &str) -> String {
    let dims = desc.chars().take_while(|&c| c == '[').count();
    let base = desc.get(dims..).unwrap_or("");
    let name = match base {
        "V" => "void".to_string(),
        "Z" => "boolean".to_string(),
        "B" => "byte".to_string(),
        "S" => "short".to_string(),
        "C" => "char".to_string(),
        "I" => "int".to_string(),
        "J" => "long".to_string(),
        "F" => "float".to_string(),
        "D" => "double".to_string(),
        b if b.starts_with('L') && b.ends_with(';') => b.get(1..b.len() - 1).unwrap_or("").replace('/', "."),
        b => b.to_string(),
    };
    format!("{name}{}", "[]".repeat(dims.min(255)))
}

/// `com.x.Y` or `com/x/Y` or `Lcom/x/Y;` → `Lcom/x/Y;`.
pub fn to_descriptor(name: &str) -> String {
    let n = name.trim();
    if n.starts_with('L') && n.ends_with(';') { n.to_string() } else { format!("L{};", n.replace('.', "/")) }
}

/// Access flag names for a class, field or method.
pub fn access_flags(flags: u32, kind: &str) -> Vec<&'static str> {
    let mut v = Vec::new();
    let table: &[(u32, &str)] = &[
        (0x1, "public"),
        (0x2, "private"),
        (0x4, "protected"),
        (0x8, "static"),
        (0x10, "final"),
        (0x100, "native"),
        (0x200, "interface"),
        (0x400, "abstract"),
        (0x800, "strictfp"),
        (0x1000, "synthetic"),
        (0x2000, "annotation"),
        (0x4000, "enum"),
        (0x10000, "constructor"),
        (0x20000, "declared-synchronized"),
    ];
    for (bit, name) in table {
        if flags & bit != 0 {
            v.push(*name);
        }
    }
    match kind {
        "method" => {
            if flags & 0x20 != 0 {
                v.push("synchronized");
            }
            if flags & 0x40 != 0 {
                v.push("bridge");
            }
            if flags & 0x80 != 0 {
                v.push("varargs");
            }
        }
        "field" => {
            if flags & 0x40 != 0 {
                v.push("volatile");
            }
            if flags & 0x80 != 0 {
                v.push("transient");
            }
        }
        _ => {}
    }
    v
}

fn table(r: &Reader<'_>, count_off: usize, what: &str, entry: usize) -> Result<(usize, usize), FormatError> {
    let n = r.u32_le(count_off).ok_or_else(|| FormatError::Truncated("header".into()))? as usize;
    let off = r.u32_le(count_off + 4).ok_or_else(|| FormatError::Truncated("header".into()))? as usize;
    if n == 0 {
        return Ok((0, 0));
    }
    let bytes = n.checked_mul(entry).ok_or_else(|| FormatError::Malformed(format!("{what} count overflows")))?;
    if off.checked_add(bytes).is_none_or(|e| e > r.len()) {
        return Err(FormatError::Malformed(format!("{n} {what} at offset {off} run past the end of the file")));
    }
    Ok((n, off))
}

/// True when bytes start with a DEX magic.
pub fn is_dex(data: &[u8]) -> bool {
    data.starts_with(b"dex\n") && data.len() >= 0x70
}

/// Parse a DEX file.
pub fn parse(data: &[u8]) -> Result<Dex<'_>, FormatError> {
    let r = Reader::new(data);
    if !data.starts_with(b"dex\n") {
        return Err(FormatError::Malformed("not a DEX file (no dex magic)".into()));
    }
    if data.len() < 0x70 {
        return Err(FormatError::Truncated("DEX header".into()));
    }
    let mut warnings = Vec::new();
    let version = r.cstr(4, 4).unwrap_or_default();
    let checksum = r.u32_le(8).unwrap_or(0);
    let file_size = r.u32_le(32).unwrap_or(0);
    let endian = r.u32_le(40).unwrap_or(0);
    if endian != 0x1234_5678 {
        return Err(FormatError::Unsupported(format!("byte-swapped DEX (endian tag {endian:#x})")));
    }
    if file_size as usize != data.len() {
        warnings.push(format!("header says {file_size} bytes, the file has {}", data.len()));
    }
    let sum_end = (file_size as usize).min(data.len());
    let checksum_ok = data.get(12..sum_end).is_some_and(|d| adler32(d) == checksum);
    if !checksum_ok {
        warnings.push("Adler-32 checksum does not match the content".into());
    }
    let (ns, so) = table(&r, 56, "strings", 4)?;
    let (nt, to) = table(&r, 64, "types", 4)?;
    let (np, po) = table(&r, 72, "protos", 12)?;
    let (nf, fo) = table(&r, 80, "fields", 8)?;
    let (nm, mo) = table(&r, 88, "methods", 8)?;
    let (nc, co) = table(&r, 96, "classes", 32)?;
    let header = Header {
        version,
        checksum: format!("{checksum:08x}"),
        checksum_ok,
        signature: r.slice(12, 20).map(|b| b.iter().map(|x| format!("{x:02x}")).collect()).unwrap_or_default(),
        file_size,
        header_size: r.u32_le(36).unwrap_or(0),
        endian_tag: format!("{endian:#x}"),
        string_ids: ns as u32,
        type_ids: nt as u32,
        proto_ids: np as u32,
        field_ids: nf as u32,
        method_ids: nm as u32,
        class_defs: nc as u32,
        data_size: r.u32_le(104).unwrap_or(0),
    };
    let mut strings = Vec::with_capacity(ns);
    let mut budget = STRING_BUDGET;
    let mut over_budget = false;
    for i in 0..ns {
        let off = r.u32_le(so + i * 4).unwrap_or(0) as usize;
        let s = match r.uleb128(off) {
            Some((len, n)) if budget > 0 => {
                let want = (len as usize).min(MAX_STRING_UNITS).min(budget);
                let (s, _) = mutf8(&r, off.saturating_add(n), want);
                budget = budget.saturating_sub(want.max(1));
                s
            }
            Some(_) => {
                over_budget = true;
                format!("<string#{i}>")
            }
            None => format!("<string#{i}>"),
        };
        strings.push(s);
    }
    if over_budget {
        warnings.push("string decoding budget reached; later strings are shown as <string#n>".into());
    }
    let types = (0..nt).map(|i| r.u32_le(to + i * 4).unwrap_or(NO_INDEX)).collect();
    let protos = (0..np)
        .map(|i| {
            let o = po + i * 12;
            ProtoId { shorty: r.u32_le(o).unwrap_or(0), ret: r.u32_le(o + 4).unwrap_or(NO_INDEX), params_off: r.u32_le(o + 8).unwrap_or(0) }
        })
        .collect();
    let member =
        |o: usize| MemberId { class_idx: r.u16_le(o).unwrap_or(0), type_or_proto: r.u16_le(o + 2).unwrap_or(0), name_idx: r.u32_le(o + 4).unwrap_or(NO_INDEX) };
    let fields = (0..nf).map(|i| member(fo + i * 8)).collect();
    let methods = (0..nm).map(|i| member(mo + i * 8)).collect();
    let classes = (0..nc)
        .map(|i| {
            let o = co + i * 32;
            let g = |k: usize| r.u32_le(o + k * 4).unwrap_or(0);
            ClassDef {
                class_idx: g(0),
                access: g(1),
                superclass_idx: g(2),
                interfaces_off: g(3),
                source_file_idx: g(4),
                annotations_off: g(5),
                class_data_off: g(6),
                static_values_off: g(7),
            }
        })
        .collect();
    Ok(Dex { data, header, strings, types, protos, fields, methods, classes, warnings })
}

impl<'a> Dex<'a> {
    fn r(&self) -> Reader<'a> {
        Reader::new(self.data)
    }

    pub fn string(&self, i: u32) -> &str {
        self.strings.get(i as usize).map(String::as_str).unwrap_or("<bad-string>")
    }

    /// The descriptor of type `i` (`Lcom/x/Y;`).
    pub fn type_desc(&self, i: u32) -> String {
        match self.types.get(i as usize) {
            Some(&s) => self.string(s).to_string(),
            None => format!("<type#{i}>"),
        }
    }

    fn type_list(&self, off: u32) -> Vec<u32> {
        if off == 0 {
            return Vec::new();
        }
        let r = self.r();
        let n = r.u32_le(off as usize).unwrap_or(0) as usize;
        (0..n.min(256)).filter_map(|i| r.u16_le(off as usize + 4 + i * 2).map(u32::from)).collect()
    }

    /// `(Ljava/lang/String;I)V`
    pub fn proto_desc(&self, i: u32) -> String {
        match self.protos.get(i as usize) {
            Some(p) => {
                let params: String = self.type_list(p.params_off).into_iter().map(|t| self.type_desc(t)).collect();
                format!("({params}){}", self.type_desc(p.ret))
            }
            None => format!("<proto#{i}>"),
        }
    }

    /// Parameter type descriptors and return type of proto `i`.
    pub fn proto_parts(&self, i: u32) -> (Vec<String>, String) {
        match self.protos.get(i as usize) {
            Some(p) => (self.type_list(p.params_off).into_iter().map(|t| self.type_desc(t)).collect(), self.type_desc(p.ret)),
            None => (Vec::new(), "?".into()),
        }
    }

    pub fn shorty(&self, i: u32) -> &str {
        self.protos.get(i as usize).map(|p| self.string(p.shorty)).unwrap_or("")
    }

    /// `Lcls;->name:Ltype;`
    pub fn field_ref(&self, i: u32) -> String {
        match self.fields.get(i as usize) {
            Some(f) => format!("{}->{}:{}", self.type_desc(u32::from(f.class_idx)), self.string(f.name_idx), self.type_desc(u32::from(f.type_or_proto))),
            None => format!("<field#{i}>"),
        }
    }

    /// `Lcls;->name(…)R`
    pub fn method_ref(&self, i: u32) -> String {
        match self.methods.get(i as usize) {
            Some(m) => format!("{}->{}{}", self.type_desc(u32::from(m.class_idx)), self.string(m.name_idx), self.proto_desc(u32::from(m.type_or_proto))),
            None => format!("<method#{i}>"),
        }
    }

    pub fn class_name(&self, c: &ClassDef) -> String {
        self.type_desc(c.class_idx)
    }

    pub fn find_class(&self, name: &str) -> Option<&ClassDef> {
        let want = to_descriptor(name);
        self.classes.iter().find(|c| self.class_name(c) == want)
    }

    pub fn interfaces(&self, c: &ClassDef) -> Vec<String> {
        self.type_list(c.interfaces_off).into_iter().map(|t| self.type_desc(t)).collect()
    }

    /// Type descriptors of the class-level annotations.
    pub fn annotations(&self, c: &ClassDef) -> Vec<String> {
        let r = self.r();
        let mut out = Vec::new();
        if c.annotations_off == 0 {
            return out;
        }
        let Some(set_off) = r.u32_le(c.annotations_off as usize) else { return out };
        if set_off == 0 {
            return out;
        }
        let n = r.u32_le(set_off as usize).unwrap_or(0) as usize;
        for i in 0..n.min(256) {
            let Some(item) = r.u32_le(set_off as usize + 4 + i * 4) else { break };
            // visibility u8, then encoded_annotation: uleb type_idx …
            let vis = r.u8(item as usize).unwrap_or(0);
            if let Some((t, _)) = r.uleb128(item as usize + 1) {
                let v = match vis {
                    0 => "build",
                    1 => "runtime",
                    2 => "system",
                    _ => "?",
                };
                out.push(format!("{} ({v})", self.type_desc(t as u32)));
            }
        }
        out
    }

    pub fn class_data(&self, c: &ClassDef) -> ClassData {
        let mut cd = ClassData::default();
        if c.class_data_off == 0 {
            cd.complete = true;
            return cd;
        }
        let r = self.r();
        let mut pos = c.class_data_off as usize;
        let next = |pos: &mut usize| -> Option<u32> {
            let (v, n) = r.uleb128(*pos)?;
            *pos += n;
            u32::try_from(v).ok()
        };
        let (Some(sf), Some(inf), Some(dm), Some(vm)) = (next(&mut pos), next(&mut pos), next(&mut pos), next(&mut pos)) else { return cd };
        let remaining = self.data.len().saturating_sub(pos);
        let total = [sf, inf, dm, vm].iter().map(|&x| x as usize).sum::<usize>();
        if total > remaining || total > MAX_MEMBERS * 4 {
            return cd;
        }
        for (count, is_method, slot) in [(sf, false, 0), (inf, false, 1), (dm, true, 2), (vm, true, 3)] {
            let mut idx: u32 = 0;
            for _ in 0..count.min(MAX_MEMBERS as u32) {
                let Some(diff) = next(&mut pos) else { return cd };
                let Some(access) = next(&mut pos) else { return cd };
                idx = match idx.checked_add(diff) {
                    Some(v) => v,
                    None => return cd,
                };
                if is_method {
                    let Some(code_off) = next(&mut pos) else { return cd };
                    let m = EncodedMethod { method_idx: idx, access, code_off };
                    if slot == 2 {
                        cd.direct_methods.push(m);
                    } else {
                        cd.virtual_methods.push(m);
                    }
                } else {
                    let f = EncodedField { field_idx: idx, access };
                    if slot == 0 {
                        cd.static_fields.push(f);
                    } else {
                        cd.instance_fields.push(f);
                    }
                }
            }
        }
        cd.complete = true;
        cd
    }

    /// The code item at `off`.
    pub fn code(&self, off: u32) -> Option<Code> {
        if off == 0 {
            return None;
        }
        let r = self.r();
        let o = off as usize;
        let registers = r.u16_le(o)?;
        let ins = r.u16_le(o + 2)?;
        let outs = r.u16_le(o + 4)?;
        let tries_size = usize::from(r.u16_le(o + 6)?);
        let debug_info_off = r.u32_le(o + 8)?;
        let n = r.u32_le(o + 12)? as usize;
        let bytes = r.slice(o + 16, n.checked_mul(2)?)?;
        let insns: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        let mut tries = Vec::new();
        let mut t_off = o + 16 + n * 2;
        if tries_size > 0 && n % 2 == 1 {
            t_off += 2;
        }
        let handlers_base = t_off + tries_size * 8;
        for i in 0..tries_size.min(MAX_TRIES) {
            let at = t_off + i * 8;
            let (Some(start), Some(count), Some(h_off)) = (r.u32_le(at), r.u16_le(at + 4), r.u16_le(at + 6)) else { break };
            let mut handlers = Vec::new();
            let mut p = handlers_base + usize::from(h_off);
            if let Some((size, k)) = sleb128(&r, p) {
                p += k;
                for _ in 0..size.unsigned_abs().min(MAX_HANDLERS as u64) {
                    let Some((t, k1)) = r.uleb128(p) else { break };
                    let Some((addr, k2)) = r.uleb128(p + k1) else { break };
                    p += k1 + k2;
                    handlers.push(Handler { exception: Some(self.type_desc(t as u32)), address: addr as u32 });
                }
                if size <= 0
                    && let Some((addr, _)) = r.uleb128(p)
                {
                    handlers.push(Handler { exception: None, address: addr as u32 });
                }
            }
            tries.push(Try { start, end: start.saturating_add(u32::from(count)), handlers });
        }
        Some(Code { registers, ins, outs, insns, tries, debug_info_off })
    }

    /// Name of method `i`.
    pub fn method_name(&self, i: u32) -> &str {
        self.methods.get(i as usize).map(|m| self.string(m.name_idx)).unwrap_or("")
    }

    /// Java-like signature: `public static void log(java.lang.String)`.
    pub fn method_signature(&self, m: &EncodedMethod) -> String {
        let Some(id) = self.methods.get(m.method_idx as usize) else { return format!("<method#{}>", m.method_idx) };
        let (params, ret) = self.proto_parts(u32::from(id.type_or_proto));
        let flags = access_flags(m.access, "method").into_iter().filter(|f| *f != "constructor").collect::<Vec<_>>().join(" ");
        let params = params.iter().map(|p| java_name(p)).collect::<Vec<_>>().join(", ");
        format!("{}{}{} {}({params})", flags, if flags.is_empty() { "" } else { " " }, java_name(&ret), self.string(id.name_idx)).trim().to_string()
    }

    /// Render one instruction smali-style.
    pub fn format_insn(&self, i: &Insn) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !i.regs.is_empty() {
            let is_list = i.name.starts_with("invoke") || i.name.starts_with("filled-new-array");
            if i.range {
                match (i.regs.first(), i.regs.last()) {
                    (Some(a), Some(b)) => parts.push(format!("{{v{a} .. v{b}}}")),
                    _ => parts.push("{}".into()),
                }
            } else if is_list {
                parts.push(format!("{{{}}}", i.regs.iter().map(|r| format!("v{r}")).collect::<Vec<_>>().join(", ")));
            } else {
                parts.extend(i.regs.iter().map(|r| format!("v{r}")));
            }
        } else if i.range || i.name.starts_with("invoke") {
            parts.push("{}".into());
        }
        if let Some(l) = i.literal {
            parts.push(if (-9..=9).contains(&l) { format!("{l}") } else { format!("{l:#x}") });
        }
        if let Some(idx) = i.index {
            parts.push(match i.index_kind {
                IndexKind::String => format!("{:?}", self.string(idx)),
                IndexKind::Type => self.type_desc(idx),
                IndexKind::Field => self.field_ref(idx),
                IndexKind::Method => self.method_ref(idx),
                IndexKind::CallSite => format!("call_site@{idx}"),
                IndexKind::MethodHandle => format!("method_handle@{idx}"),
                IndexKind::Proto => self.proto_desc(idx),
                IndexKind::None => format!("@{idx}"),
            });
        }
        if let Some(p) = i.index2 {
            parts.push(self.proto_desc(p));
        }
        if let Some(t) = i.target {
            parts.push(format!(":{t:04x}"));
        }
        let mut s = if parts.is_empty() { i.name.to_string() } else { format!("{} {}", i.name, parts.join(", ")) };
        match (&i.payload, i.opcode) {
            (Some(Payload::PackedSwitch { first_key, targets }), 0x2b) => {
                let cases: Vec<String> = targets
                    .iter()
                    .enumerate()
                    .take(32)
                    .map(|(k, t)| format!("{} -> :{:04x}", i64::from(*first_key) + k as i64, i64::from(i.offset) + i64::from(*t)))
                    .collect();
                s.push_str(&format!("  # {}", cases.join(", ")));
            }
            (Some(Payload::SparseSwitch { keys, targets }), 0x2c) => {
                let cases: Vec<String> =
                    keys.iter().zip(targets).take(32).map(|(k, t)| format!("{k} -> :{:04x}", i64::from(i.offset) + i64::from(*t))).collect();
                s.push_str(&format!("  # {}", cases.join(", ")));
            }
            (Some(Payload::FillArrayData { element_width, count, .. }), _) => s.push_str(&format!("  # {count} x {element_width}-byte elements")),
            (Some(Payload::PackedSwitch { targets, .. }), 0) => s.push_str(&format!(" ({} cases)", targets.len())),
            (Some(Payload::SparseSwitch { targets, .. }), 0) => s.push_str(&format!(" ({} cases)", targets.len())),
            _ => {}
        }
        s
    }
}

/// One place code refers to a class, method or field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reference {
    /// Caller method, `Lcls;->name(…)R`.
    pub caller: String,
    /// Code unit offset in the caller.
    pub offset: u32,
    pub instruction: &'static str,
    /// method, field or type
    pub kind: &'static str,
    pub target: String,
}

/// Every instruction that refers to class `desc` (optionally only its methods named `method`).
pub fn references(dex: &Dex<'_>, desc: &str, method: Option<&str>, max: usize) -> (Vec<Reference>, bool) {
    let mut out = Vec::new();
    for c in &dex.classes {
        let cd = dex.class_data(c);
        for m in cd.direct_methods.iter().chain(&cd.virtual_methods) {
            let Some(code) = dex.code(m.code_off) else { continue };
            let (insns, _) = crate::dex_ops::decode(&code.insns, 200_000);
            for i in &insns {
                let Some(idx) = i.index else { continue };
                let hit = match i.index_kind {
                    IndexKind::Method => dex.methods.get(idx as usize).and_then(|id| {
                        let cls = dex.type_desc(u32::from(id.class_idx));
                        (cls == desc && method.is_none_or(|n| dex.string(id.name_idx) == n)).then(|| ("method", dex.method_ref(idx)))
                    }),
                    IndexKind::Field if method.is_none() => {
                        dex.fields.get(idx as usize).and_then(|id| (dex.type_desc(u32::from(id.class_idx)) == desc).then(|| ("field", dex.field_ref(idx))))
                    }
                    IndexKind::Type if method.is_none() => {
                        let t = dex.type_desc(idx);
                        (t.trim_start_matches('[') == desc).then_some(("type", t))
                    }
                    _ => None,
                };
                if let Some((kind, target)) = hit {
                    if out.len() >= max {
                        return (out, true);
                    }
                    out.push(Reference { caller: dex.method_ref(m.method_idx), offset: i.offset, instruction: i.name, kind, target });
                }
            }
        }
    }
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_anything_testkit::android as fx;

    #[test]
    fn parses_fixture() {
        let data = fx::notes_dex();
        let d = parse(&data).unwrap();
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        assert!(d.header.checksum_ok);
        assert_eq!(d.header.version, "035");
        assert_eq!(d.classes.len(), 2);
        let main = d.find_class("com.example.notes.MainActivity").unwrap();
        assert_eq!(d.type_desc(main.superclass_idx), "Landroid/app/Activity;");
        assert_eq!(d.interfaces(main), vec!["Ljava/lang/Runnable;"]);
        assert_eq!(d.string(main.source_file_idx), "MainActivity.java");
        assert_eq!(d.annotations(main), vec!["Ljava/lang/Deprecated; (runtime)"]);
        let cd = d.class_data(main);
        assert!(cd.complete);
        assert_eq!(cd.static_fields.len(), 1);
        assert_eq!(cd.instance_fields.len(), 1);
        assert_eq!(cd.direct_methods.len(), 1);
        assert_eq!(cd.virtual_methods.len(), 3);
        let on_create = cd.virtual_methods.iter().find(|m| d.method_name(m.method_idx) == "onCreate").unwrap();
        assert_eq!(d.method_signature(on_create), "public void onCreate(android.os.Bundle)");
        let code = d.code(on_create.code_off).unwrap();
        let (ins, complete) = crate::dex_ops::decode(&code.insns, 1000);
        assert!(complete);
        let text: Vec<String> = ins.iter().map(|i| d.format_insn(i)).collect();
        assert_eq!(text[0], "const-string v0, \"Hello, Craft!\"");
        assert_eq!(text[1], "invoke-static {v0}, Lcom/example/notes/Util;->log(Ljava/lang/String;)V");
        let pick = cd.virtual_methods.iter().find(|m| d.method_name(m.method_idx) == "pick").unwrap();
        let pc = d.code(pick.code_off).unwrap();
        assert_eq!(pc.tries.len(), 1);
        assert_eq!(pc.tries[0].handlers[0].exception.as_deref(), Some("Ljava/lang/RuntimeException;"));
        assert_eq!(pc.tries[0].handlers[1].exception, None);
        let (pins, _) = crate::dex_ops::decode(&pc.insns, 1000);
        let sw = pins.iter().find(|i| i.name == "packed-switch").unwrap();
        assert!(d.format_insn(sw).contains("# 1 -> :"), "{}", d.format_insn(sw));
        let (refs, more) = references(&d, "Lcom/example/notes/Util;", Some("log"), 100);
        assert!(!more);
        assert_eq!(refs.len(), 2, "{refs:?}");
        assert_eq!(refs[0].caller, "Lcom/example/notes/MainActivity;->onCreate(Landroid/os/Bundle;)V");
        let (all, _) = references(&d, "Ljava/lang/System;", None, 100);
        assert_eq!(all[0].kind, "field");
    }

    #[test]
    fn helpers() {
        assert_eq!(java_name("[[Ljava/lang/String;"), "java.lang.String[][]");
        assert_eq!(java_name("J"), "long");
        assert_eq!(to_descriptor("a.b.C"), "La/b/C;");
        assert_eq!(to_descriptor("La/b/C;"), "La/b/C;");
        assert_eq!(access_flags(0x10009, "method"), vec!["public", "static", "constructor"]);
        let r = Reader::new(&[0x7f]);
        assert_eq!(sleb128(&r, 0), Some((-1, 1)));
        let s = [0xc0, 0x80, 0xe2, 0x82, 0xac, 0x41, 0];
        assert_eq!(mutf8(&Reader::new(&s), 0, 100).0, "\0\u{20ac}A");
    }

    #[test]
    fn hostile() {
        let data = fx::notes_dex();
        let walk = |d: &Dex<'_>| {
            for c in &d.classes {
                let cd = d.class_data(c);
                let _ = d.annotations(c);
                let _ = d.interfaces(c);
                for m in cd.direct_methods.iter().chain(&cd.virtual_methods) {
                    let _ = d.method_signature(m);
                    if let Some(code) = d.code(m.code_off) {
                        let (ins, _) = crate::dex_ops::decode(&code.insns, 10_000);
                        for i in &ins {
                            let _ = d.format_insn(i);
                        }
                    }
                }
            }
            let _ = references(d, "Lcom/example/notes/Util;", None, 100);
        };
        for m in craft_anything_testkit::mutations(&data) {
            if let Ok(d) = parse(&m) {
                walk(&d);
            }
        }
        for len in 0..data.len() {
            if let Ok(d) = parse(&data[..len]) {
                walk(&d);
            }
        }
        for seed in 0..64 {
            let mut n = craft_anything_testkit::noise(seed, 2048);
            n[..8].copy_from_slice(b"dex\n035\0");
            n[40..44].copy_from_slice(&0x1234_5678u32.to_le_bytes());
            if let Ok(d) = parse(&n) {
                walk(&d);
            }
        }
    }
}

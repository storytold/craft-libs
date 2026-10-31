//! Java `.class` files (JVMS): the magic and version, the constant pool (every tag including
//! `MethodHandle`/`MethodType`/`Dynamic`/`InvokeDynamic`/`Module`/`Package`), access flags, the
//! this/super class and interfaces, fields and methods with their attributes (`Code` with its
//! exception table and line numbers, `ConstantValue`, `Exceptions`, `Signature`), and the
//! class-level `SourceFile`, `InnerClasses` and annotation attributes (type names, best-effort).
//!
//! Counts and lengths are checked against the file size, strings are decoded from modified UTF-8,
//! and every read is bounds-checked, so hostile input yields warnings, not panics.

use crate::bytes::Reader;
use crate::java_ops::{self, Insn};
use crate::{FormatError, dex};
use serde::Serialize;

/// Most constant-pool entries, fields, methods, attributes or interfaces we read from one file.
const MAX_ITEMS: usize = 1 << 20;
/// Most attributes per element.
const MAX_ATTRS: usize = 64 * 1024;
/// Deepest annotation/element-value nesting we walk.
const MAX_ANNO_DEPTH: usize = 32;

/// One constant-pool entry. Index 0 and the slot after a `Long`/`Double` are `Unusable`.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Unusable,
    Utf8(String),
    Integer(i32),
    Float(f32),
    Long(i64),
    Double(f64),
    /// `name_index` → a Utf8 internal name.
    Class(u16),
    /// `string_index` → a Utf8.
    String(u16),
    /// `class_index`, `name_and_type_index`.
    FieldRef(u16, u16),
    MethodRef(u16, u16),
    InterfaceMethodRef(u16, u16),
    /// `name_index`, `descriptor_index`.
    NameAndType(u16, u16),
    /// `reference_kind`, `reference_index`.
    MethodHandle(u8, u16),
    /// `descriptor_index`.
    MethodType(u16),
    /// `bootstrap_method_attr_index`, `name_and_type_index`.
    Dynamic(u16, u16),
    InvokeDynamic(u16, u16),
    Module(u16),
    Package(u16),
}

/// The class-file version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
}

impl Version {
    /// A friendly Java platform label (`Java 8`, `Java 17`, `Java 1.3`).
    pub fn java_label(self) -> String {
        match self.major {
            45..=48 => format!("Java 1.{}", self.major - 44),
            m if m >= 49 => format!("Java {}", m - 44),
            m => format!("pre-standard (major {m})"),
        }
    }
}

/// An exception-table row of a `Code` attribute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExceptionEntry {
    pub start_pc: u16,
    pub end_pc: u16,
    pub handler_pc: u16,
    /// Dotted caught type; `None` is catch-all (a `finally` or `catch (Throwable)` with index 0).
    pub catch_type: Option<String>,
}

/// A parsed `Code` attribute.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Code {
    pub max_stack: u16,
    pub max_locals: u16,
    pub bytecode: Vec<u8>,
    pub exceptions: Vec<ExceptionEntry>,
    /// `(start_pc, line)` from the `LineNumberTable`.
    pub line_numbers: Vec<(u16, u16)>,
}

/// A field or method.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Member {
    pub access: u16,
    pub name: String,
    pub descriptor: String,
    /// The generic `Signature` attribute, if present.
    pub signature: Option<String>,
    /// A field's `ConstantValue`, rendered to text.
    pub constant_value: Option<String>,
    /// A method's declared `throws` types (dotted).
    pub exceptions: Vec<String>,
    /// A method's code.
    pub code: Option<Code>,
    pub deprecated: bool,
    /// The `Synthetic` attribute (separate from the access flag).
    pub synthetic_attr: bool,
}

/// An `InnerClasses` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InnerClass {
    pub inner: Option<String>,
    pub outer: Option<String>,
    /// Simple name; `None` for an anonymous class.
    pub name: Option<String>,
    pub access: u16,
}

/// A parsed class file.
#[derive(Debug, Clone)]
pub struct ClassFile {
    pub version: Version,
    pub access: u16,
    /// Internal name of this class (`com/example/Main`).
    pub this_class: String,
    /// Internal name of the superclass; `None` only for `java/lang/Object` and modules.
    pub super_class: Option<String>,
    /// Internal names of the implemented interfaces.
    pub interfaces: Vec<String>,
    pub fields: Vec<Member>,
    pub methods: Vec<Member>,
    pub source_file: Option<String>,
    pub signature: Option<String>,
    pub inner_classes: Vec<InnerClass>,
    /// Class-level annotation types (dotted), with `(runtime)`/`(class)` visibility.
    pub annotations: Vec<String>,
    pub pool: Vec<Const>,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Names, descriptors and access flags

/// `java/lang/String` → `java.lang.String`; `[Ljava/lang/String;` → `java.lang.String[]`.
pub fn internal_to_dotted(name: &str) -> String {
    if name.starts_with('[') { dex::java_name(name) } else { name.replace('/', ".") }
}

/// A field descriptor (`Ljava/lang/String;`, `[I`, `I`) as a Java type.
pub fn field_type(desc: &str) -> String {
    dex::java_name(desc)
}

/// Split a run of field-type descriptors into individual descriptors (safe, bounded).
fn split_types(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() && out.len() < MAX_ITEMS {
        let start = i;
        while b.get(i) == Some(&b'[') {
            i += 1;
        }
        match b.get(i) {
            Some(b'L') => match s.get(i..).and_then(|r| r.find(';')) {
                Some(semi) => i = i + semi + 1,
                None => i = b.len(),
            },
            Some(_) => i += 1,
            None => break,
        }
        match s.get(start..i) {
            Some(t) if !t.is_empty() => out.push(t.to_string()),
            _ => break,
        }
    }
    out
}

/// Parameter types and return type of a method descriptor (`(Ljava/lang/String;I)V`).
pub fn method_params(desc: &str) -> (Vec<String>, String) {
    let Some(rest) = desc.strip_prefix('(') else { return (Vec::new(), "?".into()) };
    let Some((params, ret)) = rest.split_once(')') else { return (Vec::new(), "?".into()) };
    (split_types(params).iter().map(|t| field_type(t)).collect(), field_type(ret))
}

/// What a set of access flags belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Class,
    Field,
    Method,
    InnerClass,
}

/// Human names of the set access flags.
pub fn access_flags(flags: u16, kind: AccessKind) -> Vec<&'static str> {
    let mut v = Vec::new();
    let common: &[(u16, &str)] = &[(0x0001, "public"), (0x0002, "private"), (0x0004, "protected"), (0x0008, "static"), (0x0010, "final")];
    for (bit, name) in common {
        if flags & bit != 0 {
            v.push(*name);
        }
    }
    match kind {
        AccessKind::Method => {
            for (bit, name) in [
                (0x0020u16, "synchronized"),
                (0x0040, "bridge"),
                (0x0080, "varargs"),
                (0x0100, "native"),
                (0x0400, "abstract"),
                (0x0800, "strictfp"),
                (0x1000, "synthetic"),
            ] {
                if flags & bit != 0 {
                    v.push(name);
                }
            }
        }
        AccessKind::Field => {
            for (bit, name) in [(0x0040u16, "volatile"), (0x0080, "transient"), (0x1000, "synthetic"), (0x4000, "enum")] {
                if flags & bit != 0 {
                    v.push(name);
                }
            }
        }
        AccessKind::Class | AccessKind::InnerClass => {
            for (bit, name) in [
                (0x0020u16, "super"),
                (0x0200, "interface"),
                (0x0400, "abstract"),
                (0x1000, "synthetic"),
                (0x2000, "annotation"),
                (0x4000, "enum"),
                (0x8000, "module"),
            ] {
                if flags & bit != 0 {
                    // "super" is an internal marker on classes; hide it unless it is the only flag of note.
                    if name == "super" && kind == AccessKind::Class {
                        continue;
                    }
                    v.push(name);
                }
            }
        }
    }
    v
}

/// A resolved reference to a field or method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ref {
    /// Internal owner name (`java/lang/System`).
    pub owner: String,
    pub name: String,
    pub descriptor: String,
}

impl Ref {
    /// `java.lang.System.out:java.io.PrintStream` style display. A field descriptor is shown as a
    /// Java type; a method descriptor is left in its raw `(args)ret` form.
    pub fn display(&self) -> String {
        let ty = if self.descriptor.starts_with('(') { self.descriptor.clone() } else { field_type(&self.descriptor) };
        format!("{}.{}:{}", internal_to_dotted(&self.owner), self.name, ty)
    }
}

// ---------------------------------------------------------------------------------------------
// Modified UTF-8

/// Decode a modified-UTF-8 byte run (class-file `Utf8` content) to a String.
fn decode_mutf8(bytes: &[u8]) -> String {
    let mut units: Vec<u16> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let Some(&b0) = bytes.get(i) else { break };
        if b0 < 0x80 {
            units.push(u16::from(b0));
            i += 1;
        } else if b0 & 0xe0 == 0xc0 {
            let b1 = bytes.get(i + 1).copied().unwrap_or(0);
            units.push((u16::from(b0 & 0x1f) << 6) | u16::from(b1 & 0x3f));
            i += 2;
        } else if b0 & 0xf0 == 0xe0 {
            let b1 = bytes.get(i + 1).copied().unwrap_or(0);
            let b2 = bytes.get(i + 2).copied().unwrap_or(0);
            units.push((u16::from(b0 & 0x0f) << 12) | (u16::from(b1 & 0x3f) << 6) | u16::from(b2 & 0x3f));
            i += 3;
        } else {
            units.push(0xfffd);
            i += 1;
        }
    }
    String::from_utf16_lossy(&units)
}

// ---------------------------------------------------------------------------------------------
// Cursor

struct Cursor<'a> {
    r: Reader<'a>,
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Cursor { r: Reader::new(data), pos: 0 }
    }
    fn u8(&mut self) -> Option<u8> {
        let v = self.r.u8(self.pos)?;
        self.pos += 1;
        Some(v)
    }
    fn u16(&mut self) -> Option<u16> {
        let v = self.r.u16_be(self.pos)?;
        self.pos += 2;
        Some(v)
    }
    fn u32(&mut self) -> Option<u32> {
        let v = self.r.u32_be(self.pos)?;
        self.pos += 4;
        Some(v)
    }
    fn u64(&mut self) -> Option<u64> {
        let v = self.r.u64_be(self.pos)?;
        self.pos += 8;
        Some(v)
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.r.slice(self.pos, n)?;
        self.pos += n;
        Some(s)
    }
}

// ---------------------------------------------------------------------------------------------
// Parsing

/// True when the bytes begin a Java class file (0xCAFEBABE and a plausible major version).
/// A big-endian Mach-O fat binary shares the magic but has a small arch count where the version
/// is, so it fails the major-version check.
pub fn is_class(data: &[u8]) -> bool {
    if !data.starts_with(&[0xca, 0xfe, 0xba, 0xbe]) {
        return false;
    }
    let r = Reader::new(data);
    matches!(r.u16_be(6), Some(major) if (45..=255).contains(&major))
}

fn utf8_of(pool: &[Const], idx: u16) -> &str {
    match pool.get(idx as usize) {
        Some(Const::Utf8(s)) => s.as_str(),
        _ => "",
    }
}

/// Parse a class file.
pub fn parse(data: &[u8]) -> Result<ClassFile, FormatError> {
    if !data.starts_with(&[0xca, 0xfe, 0xba, 0xbe]) {
        return Err(FormatError::Malformed("not a Java class file (no 0xCAFEBABE magic)".into()));
    }
    let mut c = Cursor::new(data);
    let mut warnings = Vec::new();
    let _magic = c.u32();
    let minor = c.u16().ok_or_else(|| FormatError::Truncated("version".into()))?;
    let major = c.u16().ok_or_else(|| FormatError::Truncated("version".into()))?;
    if major < 45 {
        return Err(FormatError::Unsupported(format!("class major version {major} is below 45 (not a standard class file)")));
    }
    let version = Version { major, minor };
    let cp_count = c.u16().ok_or_else(|| FormatError::Truncated("constant pool count".into()))?;
    if cp_count == 0 {
        return Err(FormatError::Malformed("constant pool count is zero".into()));
    }
    let pool = parse_pool(&mut c, cp_count, &mut warnings)?;
    let access = c.u16().ok_or_else(|| FormatError::Truncated("access flags".into()))?;
    let this_idx = c.u16().ok_or_else(|| FormatError::Truncated("this_class".into()))?;
    let super_idx = c.u16().ok_or_else(|| FormatError::Truncated("super_class".into()))?;
    let this_class = class_name_at(&pool, this_idx);
    let super_class = if super_idx == 0 { None } else { Some(class_name_at(&pool, super_idx)) };
    // Interfaces
    let n_iface = c.u16().ok_or_else(|| FormatError::Truncated("interfaces count".into()))? as usize;
    let mut interfaces = Vec::new();
    for _ in 0..n_iface.min(MAX_ITEMS) {
        let Some(i) = c.u16() else {
            warnings.push("interface list ended early".into());
            break;
        };
        interfaces.push(class_name_at(&pool, i));
    }
    // Fields
    let fields = parse_members(&mut c, &pool, &mut warnings, "field")?;
    // Methods
    let methods = parse_members(&mut c, &pool, &mut warnings, "method")?;
    // Class attributes
    let attrs = read_attrs(&mut c, &pool);
    let mut source_file = None;
    let mut signature = None;
    let mut inner_classes = Vec::new();
    let mut annotations = Vec::new();
    for a in &attrs {
        match a.name.as_str() {
            "SourceFile" => {
                if let Some(i) = Reader::new(a.info).u16_be(0) {
                    source_file = Some(utf8_of(&pool, i).to_string());
                }
            }
            "Signature" => {
                if let Some(i) = Reader::new(a.info).u16_be(0) {
                    signature = Some(utf8_of(&pool, i).to_string());
                }
            }
            "InnerClasses" => inner_classes = parse_inner_classes(a.info, &pool),
            "RuntimeVisibleAnnotations" => annotations.extend(parse_annotation_types(a.info, &pool, "runtime")),
            "RuntimeInvisibleAnnotations" => annotations.extend(parse_annotation_types(a.info, &pool, "class")),
            _ => {}
        }
    }
    Ok(ClassFile { version, access, this_class, super_class, interfaces, fields, methods, source_file, signature, inner_classes, annotations, pool, warnings })
}

fn parse_pool(c: &mut Cursor<'_>, cp_count: u16, warnings: &mut Vec<String>) -> Result<Vec<Const>, FormatError> {
    let mut pool = vec![Const::Unusable]; // index 0
    let mut i = 1u16;
    while i < cp_count {
        let tag = c.u8().ok_or_else(|| FormatError::Truncated("constant pool entry".into()))?;
        let entry = match tag {
            1 => {
                let len = c.u16().ok_or_else(|| FormatError::Truncated("Utf8 length".into()))? as usize;
                let bytes = c.take(len).ok_or_else(|| FormatError::Truncated("Utf8 bytes".into()))?;
                Const::Utf8(decode_mutf8(bytes))
            }
            3 => Const::Integer(c.u32().ok_or_else(|| trunc("Integer"))? as i32),
            4 => Const::Float(f32::from_bits(c.u32().ok_or_else(|| trunc("Float"))?)),
            5 => Const::Long(c.u64().ok_or_else(|| trunc("Long"))? as i64),
            6 => Const::Double(f64::from_bits(c.u64().ok_or_else(|| trunc("Double"))?)),
            7 => Const::Class(c.u16().ok_or_else(|| trunc("Class"))?),
            8 => Const::String(c.u16().ok_or_else(|| trunc("String"))?),
            9 => Const::FieldRef(c.u16().ok_or_else(|| trunc("Fieldref"))?, c.u16().ok_or_else(|| trunc("Fieldref"))?),
            10 => Const::MethodRef(c.u16().ok_or_else(|| trunc("Methodref"))?, c.u16().ok_or_else(|| trunc("Methodref"))?),
            11 => Const::InterfaceMethodRef(c.u16().ok_or_else(|| trunc("InterfaceMethodref"))?, c.u16().ok_or_else(|| trunc("InterfaceMethodref"))?),
            12 => Const::NameAndType(c.u16().ok_or_else(|| trunc("NameAndType"))?, c.u16().ok_or_else(|| trunc("NameAndType"))?),
            15 => Const::MethodHandle(c.u8().ok_or_else(|| trunc("MethodHandle"))?, c.u16().ok_or_else(|| trunc("MethodHandle"))?),
            16 => Const::MethodType(c.u16().ok_or_else(|| trunc("MethodType"))?),
            17 => Const::Dynamic(c.u16().ok_or_else(|| trunc("Dynamic"))?, c.u16().ok_or_else(|| trunc("Dynamic"))?),
            18 => Const::InvokeDynamic(c.u16().ok_or_else(|| trunc("InvokeDynamic"))?, c.u16().ok_or_else(|| trunc("InvokeDynamic"))?),
            19 => Const::Module(c.u16().ok_or_else(|| trunc("Module"))?),
            20 => Const::Package(c.u16().ok_or_else(|| trunc("Package"))?),
            other => {
                return Err(FormatError::Malformed(format!("unknown constant pool tag {other} at index {i}")));
            }
        };
        let wide = matches!(entry, Const::Long(_) | Const::Double(_));
        pool.push(entry);
        if wide {
            pool.push(Const::Unusable);
            i += 2;
        } else {
            i += 1;
        }
    }
    if pool.len() > MAX_ITEMS {
        warnings.push("constant pool is unusually large".into());
    }
    Ok(pool)
}

fn trunc(what: &str) -> FormatError {
    FormatError::Truncated(format!("constant pool {what}"))
}

struct RawAttr<'a> {
    name: String,
    info: &'a [u8],
}

fn read_attrs<'a>(c: &mut Cursor<'a>, pool: &[Const]) -> Vec<RawAttr<'a>> {
    let n = c.u16().unwrap_or(0) as usize;
    let mut out = Vec::new();
    for _ in 0..n.min(MAX_ATTRS) {
        let (Some(ni), Some(len)) = (c.u16(), c.u32()) else { break };
        let Some(info) = c.take(len as usize) else { break };
        out.push(RawAttr { name: utf8_of(pool, ni).to_string(), info });
    }
    out
}

fn parse_members(c: &mut Cursor<'_>, pool: &[Const], warnings: &mut Vec<String>, what: &str) -> Result<Vec<Member>, FormatError> {
    let n = c.u16().ok_or_else(|| FormatError::Truncated(format!("{what} count")))? as usize;
    let mut out = Vec::new();
    for _ in 0..n.min(MAX_ITEMS) {
        let (Some(access), Some(name_idx), Some(desc_idx)) = (c.u16(), c.u16(), c.u16()) else {
            warnings.push(format!("{what} table ended early"));
            break;
        };
        let mut m = Member { access, name: utf8_of(pool, name_idx).to_string(), descriptor: utf8_of(pool, desc_idx).to_string(), ..Default::default() };
        for a in read_attrs(c, pool) {
            match a.name.as_str() {
                "Code" => m.code = parse_code(a.info, pool),
                "ConstantValue" => {
                    if let Some(i) = Reader::new(a.info).u16_be(0) {
                        m.constant_value = Some(const_display(pool, i));
                    }
                }
                "Exceptions" => m.exceptions = parse_exceptions_attr(a.info, pool),
                "Signature" => {
                    if let Some(i) = Reader::new(a.info).u16_be(0) {
                        m.signature = Some(utf8_of(pool, i).to_string());
                    }
                }
                "Deprecated" => m.deprecated = true,
                "Synthetic" => m.synthetic_attr = true,
                _ => {}
            }
        }
        out.push(m);
    }
    Ok(out)
}

fn parse_code(info: &[u8], pool: &[Const]) -> Option<Code> {
    let mut c = Cursor::new(info);
    let max_stack = c.u16()?;
    let max_locals = c.u16()?;
    let code_len = c.u32()? as usize;
    let bytecode = c.take(code_len)?.to_vec();
    let n_exc = c.u16()? as usize;
    let mut exceptions = Vec::new();
    for _ in 0..n_exc.min(MAX_ITEMS) {
        let (Some(start_pc), Some(end_pc), Some(handler_pc), Some(catch)) = (c.u16(), c.u16(), c.u16(), c.u16()) else { break };
        exceptions.push(ExceptionEntry { start_pc, end_pc, handler_pc, catch_type: (catch != 0).then(|| class_name_at(pool, catch)) });
    }
    let mut line_numbers = Vec::new();
    for a in read_attrs(&mut c, pool) {
        if a.name == "LineNumberTable" {
            let mut lc = Cursor::new(a.info);
            let n = lc.u16().unwrap_or(0) as usize;
            for _ in 0..n.min(MAX_ITEMS) {
                let (Some(pc), Some(line)) = (lc.u16(), lc.u16()) else { break };
                line_numbers.push((pc, line));
            }
        }
    }
    Some(Code { max_stack, max_locals, bytecode, exceptions, line_numbers })
}

fn parse_exceptions_attr(info: &[u8], pool: &[Const]) -> Vec<String> {
    let mut c = Cursor::new(info);
    let n = c.u16().unwrap_or(0) as usize;
    let mut out = Vec::new();
    for _ in 0..n.min(MAX_ITEMS) {
        let Some(i) = c.u16() else { break };
        out.push(class_name_at(pool, i));
    }
    out
}

fn parse_inner_classes(info: &[u8], pool: &[Const]) -> Vec<InnerClass> {
    let mut c = Cursor::new(info);
    let n = c.u16().unwrap_or(0) as usize;
    let mut out = Vec::new();
    for _ in 0..n.min(MAX_ITEMS) {
        let (Some(inner), Some(outer), Some(name), Some(access)) = (c.u16(), c.u16(), c.u16(), c.u16()) else { break };
        out.push(InnerClass {
            inner: (inner != 0).then(|| class_name_at(pool, inner)),
            outer: (outer != 0).then(|| class_name_at(pool, outer)),
            name: (name != 0).then(|| utf8_of(pool, name).to_string()),
            access,
        });
    }
    out
}

/// Extract annotation type names (dotted) from a `Runtime*Annotations` attribute, best-effort.
fn parse_annotation_types(info: &[u8], pool: &[Const], visibility: &str) -> Vec<String> {
    let mut c = Cursor::new(info);
    let n = c.u16().unwrap_or(0) as usize;
    let mut out = Vec::new();
    for _ in 0..n.min(MAX_ITEMS) {
        match read_annotation(&mut c, pool, 0) {
            Some(t) => out.push(format!("{t} ({visibility})")),
            None => break,
        }
    }
    out
}

/// Read one `annotation` structure, returning its (dotted) type name; advances past its values.
fn read_annotation(c: &mut Cursor<'_>, pool: &[Const], depth: usize) -> Option<String> {
    if depth > MAX_ANNO_DEPTH {
        return None;
    }
    let type_idx = c.u16()?;
    let n_pairs = c.u16()? as usize;
    let type_name = field_type(utf8_of(pool, type_idx));
    for _ in 0..n_pairs.min(MAX_ITEMS) {
        let _name_idx = c.u16()?;
        skip_element_value(c, pool, depth + 1)?;
    }
    Some(type_name)
}

fn skip_element_value(c: &mut Cursor<'_>, pool: &[Const], depth: usize) -> Option<()> {
    if depth > MAX_ANNO_DEPTH {
        return None;
    }
    let tag = c.u8()?;
    match tag {
        b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' | b's' | b'c' => {
            c.u16()?;
        }
        b'e' => {
            c.u16()?;
            c.u16()?;
        }
        b'@' => {
            read_annotation(c, pool, depth + 1)?;
        }
        b'[' => {
            let n = c.u16()? as usize;
            for _ in 0..n.min(MAX_ITEMS) {
                skip_element_value(c, pool, depth + 1)?;
            }
        }
        _ => return None,
    }
    Some(())
}

/// Internal class name at a `Class` constant index (`java/lang/String` or `[I`).
fn class_name_at(pool: &[Const], idx: u16) -> String {
    match pool.get(idx as usize) {
        Some(Const::Class(name_idx)) => utf8_of(pool, *name_idx).to_string(),
        _ => format!("<class#{idx}>"),
    }
}

/// Render a loadable constant (for `ldc`/`ConstantValue`) to text.
fn const_display(pool: &[Const], idx: u16) -> String {
    match pool.get(idx as usize) {
        Some(Const::Integer(v)) => v.to_string(),
        Some(Const::Float(v)) => format!("{v}f"),
        Some(Const::Long(v)) => format!("{v}L"),
        Some(Const::Double(v)) => v.to_string(),
        Some(Const::String(s)) => format!("{:?}", utf8_of(pool, *s)),
        Some(Const::Class(n)) => format!("{}.class", internal_to_dotted(utf8_of(pool, *n))),
        Some(Const::MethodHandle(k, _)) => format!("MethodHandle(kind {k})"),
        Some(Const::MethodType(d)) => format!("MethodType({})", utf8_of(pool, *d)),
        _ => format!("#{idx}"),
    }
}

// ---------------------------------------------------------------------------------------------
// Resolution and formatting on a parsed class

impl ClassFile {
    /// The Utf8 at `idx`, or an empty string.
    pub fn utf8(&self, idx: u16) -> &str {
        utf8_of(&self.pool, idx)
    }

    /// The dotted name of this class.
    pub fn name_dotted(&self) -> String {
        internal_to_dotted(&self.this_class)
    }

    /// Resolve a `Fieldref`/`Methodref`/`InterfaceMethodref` index to owner, name and descriptor.
    pub fn resolve_ref(&self, idx: u16) -> Option<Ref> {
        let (class_idx, nat_idx) = match self.pool.get(idx as usize)? {
            Const::FieldRef(c, n) | Const::MethodRef(c, n) | Const::InterfaceMethodRef(c, n) => (*c, *n),
            _ => return None,
        };
        let owner = class_name_at(&self.pool, class_idx);
        let (name, descriptor) = self.name_and_type(nat_idx);
        Some(Ref { owner, name, descriptor })
    }

    /// Resolve a `NameAndType` index to (name, descriptor).
    pub fn name_and_type(&self, idx: u16) -> (String, String) {
        match self.pool.get(idx as usize) {
            Some(Const::NameAndType(n, d)) => (self.utf8(*n).to_string(), self.utf8(*d).to_string()),
            _ => (format!("<nat#{idx}>"), String::new()),
        }
    }

    /// The (name, descriptor) of an `InvokeDynamic` (or `Dynamic`) constant's `NameAndType`.
    pub fn name_and_type_of_indy(&self, idx: u16) -> Option<(String, String)> {
        match self.pool.get(idx as usize)? {
            Const::InvokeDynamic(_, nat) | Const::Dynamic(_, nat) => Some(self.name_and_type(*nat)),
            _ => None,
        }
    }

    /// Dotted class name at a `Class` constant index.
    pub fn class_dotted(&self, idx: u16) -> String {
        internal_to_dotted(&class_name_at(&self.pool, idx))
    }

    /// A Java-style method signature (`public static void log(java.lang.String)`).
    pub fn method_signature(&self, m: &Member) -> String {
        let (params, ret) = method_params(&m.descriptor);
        let flags = access_flags(m.access, AccessKind::Method).join(" ");
        let params = params.join(", ");
        if m.name == "<init>" || m.name == "<clinit>" {
            format!("{}{}{}({params})", flags, if flags.is_empty() { "" } else { " " }, m.name).trim().to_string()
        } else {
            format!("{}{}{} {}({params})", flags, if flags.is_empty() { "" } else { " " }, ret, m.name).trim().to_string()
        }
    }

    /// Render one decoded instruction with its constant-pool reference resolved.
    pub fn format_insn(&self, i: &Insn) -> String {
        let mut operand = String::new();
        if let Some(cp) = i.cp_index {
            operand = match i.opcode {
                0x12..=0x14 => const_display(&self.pool, cp),
                0xb2..=0xb5 => self.resolve_ref(cp).map(|r| r.display()).unwrap_or_else(|| format!("#{cp}")),
                0xb6..=0xb9 => {
                    self.resolve_ref(cp).map(|r| format!("{}.{}{}", internal_to_dotted(&r.owner), r.name, r.descriptor)).unwrap_or_else(|| format!("#{cp}"))
                }
                0xba => {
                    // invokedynamic: show the bootstrap name:descriptor
                    match self.pool.get(cp as usize) {
                        Some(Const::InvokeDynamic(_, nat)) => {
                            let (n, d) = self.name_and_type(*nat);
                            format!("{n}{d}")
                        }
                        _ => format!("#{cp}"),
                    }
                }
                0xbb | 0xbd | 0xc0 | 0xc1 | 0xc5 => self.class_dotted(cp),
                _ => format!("#{cp}"),
            };
        } else if i.opcode == 0xbc {
            // newarray
            operand = java_ops::array_type(i.int.unwrap_or(0)).to_string();
        } else if i.opcode == 0x84 {
            // iinc local, const
            operand = format!("{}, {}", i.local.unwrap_or(0), i.int.unwrap_or(0));
        } else if let Some(l) = i.local {
            operand = l.to_string();
        } else if let Some(v) = i.int {
            operand = v.to_string();
        } else if let Some(b) = i.branch {
            operand = format!("{b}");
        }
        if i.opcode == 0xc5 {
            // multianewarray class, dims
            operand = format!("{}, {}", operand, i.int.unwrap_or(0));
        }
        let mut s = if operand.is_empty() { i.name.to_string() } else { format!("{} {}", i.name, operand) };
        if let Some(sw) = &i.switch {
            let cases: Vec<String> = sw.pairs.iter().take(32).map(|(k, t)| format!("{k} -> {t}")).collect();
            s.push_str(&format!("  # default -> {}; {}", sw.default, cases.join(", ")));
        }
        s
    }
}

// ---------------------------------------------------------------------------------------------
// References

/// One place a method's bytecode refers to a class, method or field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reference {
    /// The calling method, `owner.name:descriptor`.
    pub caller: String,
    /// Byte offset in the caller's code.
    pub pc: u32,
    pub instruction: &'static str,
    /// `method`, `field` or `type`.
    pub kind: &'static str,
    pub target: String,
}

/// Every instruction in `cf` that refers to the internal class `owner` (optionally only calls to
/// its methods named `method`). Stops after `max` hits; the bool is true when it stopped early.
pub fn references(cf: &ClassFile, owner: &str, method: Option<&str>, max: usize) -> (Vec<Reference>, bool) {
    let mut out = Vec::new();
    for m in &cf.methods {
        let Some(code) = &m.code else { continue };
        let caller = Ref { owner: cf.this_class.clone(), name: m.name.clone(), descriptor: m.descriptor.clone() }.display();
        let (insns, _) = java_ops::decode(&code.bytecode, 200_000);
        for i in &insns {
            let Some(cp) = i.cp_index else { continue };
            let hit = match i.opcode {
                0xb6..=0xb9 => cf.resolve_ref(cp).and_then(|r| (r.owner == owner && method.is_none_or(|n| r.name == n)).then(|| ("method", r.display()))),
                0xb2..=0xb5 if method.is_none() => cf.resolve_ref(cp).and_then(|r| (r.owner == owner).then(|| ("field", r.display()))),
                0xbb | 0xbd | 0xc0 | 0xc1 | 0xc5 if method.is_none() => {
                    let name = class_name_at(&cf.pool, cp);
                    (name.trim_start_matches('[') == owner).then_some(("type", internal_to_dotted(&name)))
                }
                _ => None,
            };
            if let Some((kind, target)) = hit {
                if out.len() >= max {
                    return (out, true);
                }
                out.push(Reference { caller: caller.clone(), pc: i.pc, instruction: i.name, kind, target });
            }
        }
    }
    (out, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_anything_testkit::java as fx;

    #[test]
    fn parses_fixture() {
        let data = fx::hello_class();
        let cf = parse(&data).unwrap();
        assert!(cf.warnings.is_empty(), "{:?}", cf.warnings);
        assert_eq!(cf.this_class, "com/example/Hello");
        assert_eq!(cf.super_class.as_deref(), Some("java/lang/Object"));
        assert_eq!(cf.interfaces, vec!["java/lang/Runnable"]);
        assert_eq!(cf.source_file.as_deref(), Some("Hello.java"));
        assert!(cf.version.major >= 45);
        // field
        let f = cf.fields.iter().find(|f| f.name == "count").unwrap();
        assert_eq!(field_type(&f.descriptor), "int");
        // run() loads a string and calls println
        let run = cf.methods.iter().find(|m| m.name == "run").unwrap();
        assert_eq!(cf.method_signature(run), "public void run()");
        let code = run.code.as_ref().unwrap();
        let (ins, complete) = java_ops::decode(&code.bytecode, 1000);
        assert!(complete, "{ins:?}");
        let text: Vec<String> = ins.iter().map(|i| cf.format_insn(i)).collect();
        assert!(text.iter().any(|t| t == "getstatic java.lang.System.out:java.io.PrintStream"), "{text:?}");
        assert!(text.iter().any(|t| t.starts_with("ldc \"Hello")), "{text:?}");
        assert!(text.iter().any(|t| t == "invokevirtual java.io.PrintStream.println(Ljava/lang/String;)V"), "{text:?}");
        // references to System from this class
        let (refs, more) = references(&cf, "java/lang/System", None, 100);
        assert!(!more);
        assert!(refs.iter().any(|r| r.kind == "field" && r.target.contains("out")), "{refs:?}");
    }

    #[test]
    fn helpers() {
        assert_eq!(internal_to_dotted("java/lang/String"), "java.lang.String");
        assert_eq!(internal_to_dotted("[Ljava/lang/String;"), "java.lang.String[]");
        assert_eq!(method_params("(Ljava/lang/String;I)V"), (vec!["java.lang.String".to_string(), "int".to_string()], "void".to_string()));
        assert_eq!(method_params("()I"), (Vec::new(), "int".to_string()));
        assert_eq!(Version { major: 52, minor: 0 }.java_label(), "Java 8");
        assert_eq!(Version { major: 65, minor: 0 }.java_label(), "Java 21");
        assert_eq!(decode_mutf8(&[0xc0, 0x80, 0x41]), "\0A");
    }

    #[test]
    fn hostile() {
        let data = fx::hello_class();
        let walk = |cf: &ClassFile| {
            let _ = cf.name_dotted();
            for m in cf.methods.iter().chain(&cf.fields) {
                let _ = cf.method_signature(m);
                if let Some(code) = &m.code {
                    let (ins, _) = java_ops::decode(&code.bytecode, 10_000);
                    for i in &ins {
                        let _ = cf.format_insn(i);
                    }
                }
            }
            let _ = references(cf, "java/lang/System", None, 100);
        };
        for m in craft_anything_testkit::mutations(&data) {
            if let Ok(cf) = parse(&m) {
                walk(&cf);
            }
        }
        for len in 0..data.len() {
            if let Ok(cf) = parse(&data[..len]) {
                walk(&cf);
            }
        }
        for seed in 0..64 {
            let mut n = craft_anything_testkit::noise(seed, 2048);
            n[..4].copy_from_slice(&[0xca, 0xfe, 0xba, 0xbe]);
            n[6] = 0;
            n[7] = 52;
            if let Ok(cf) = parse(&n) {
                walk(&cf);
            }
        }
    }
}

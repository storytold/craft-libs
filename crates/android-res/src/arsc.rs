//! Android compiled resource tables (`resources.arsc`): the global string pool, packages, type
//! and key string pools, type specs, and typed entries per configuration (locale, density,
//! orientation, SDK level…). Simple and complex (bag) entries, sparse and 16-bit offset tables.

use crate::Error;
use crate::axml::{MAX_RETAINED_VALUE_BYTES, ResValue, StringPool, ValueBudget, chunk, report_partial_string_pool};
use crate::bytes::Reader;
use serde::Serialize;

const RES_TABLE: u16 = 0x0002;
const RES_PACKAGE: u16 = 0x0200;
const RES_TYPE: u16 = 0x0201;
const RES_TYPE_SPEC: u16 = 0x0202;
const RES_LIBRARY: u16 = 0x0203;
const NO_ENTRY: u32 = 0xffff_ffff;
const MAX_PACKAGES: usize = 64;
const MAX_BAG: usize = 4096;

/// Limits for one table.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Most entries decoded across all types and configurations.
    pub max_entries: usize,
    /// Most bytes retained in formatted simple-entry and bag values.
    pub max_value_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_entries: 2_000_000, max_value_bytes: MAX_RETAINED_VALUE_BYTES }
    }
}

const VALUE_BUDGET_WARNING: &str = "resource value budget reached; remaining values were omitted";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TypeInfo {
    pub id: u8,
    pub name: String,
    pub entry_count: u32,
    /// Configuration qualifiers that have values for this type ("default" for none).
    pub configs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Package {
    pub id: u32,
    pub name: String,
    pub types: Vec<TypeInfo>,
    pub keys: usize,
    /// Shared-library package references (id → name).
    pub libraries: Vec<(u32, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BagItem {
    pub name: String,
    pub value: String,
    pub value_type: &'static str,
}

/// One value of one resource in one configuration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Entry {
    /// `0xPPTTEEEE`
    pub id: String,
    pub package: String,
    #[serde(rename = "type")]
    pub type_name: String,
    pub key: String,
    /// Qualifier string, e.g. `fr`, `es-rMX`, `night-xhdpi`, or `default`.
    pub config: String,
    /// Android locale qualifier, empty for the default locale.
    pub locale: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bag: Vec<BagItem>,
    #[serde(skip)]
    pub raw: Option<ResValue>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Table {
    pub packages: Vec<Package>,
    pub global_strings: usize,
    pub entries: Vec<Entry>,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

fn utf16_fixed(r: &Reader<'_>, off: usize, chars: usize) -> String {
    let units: Vec<u16> = (0..chars).map_while(|i| r.u16_le(off + i * 2)).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units)
}

fn ascii2(b: &[u8]) -> String {
    b.iter().take_while(|&&c| c != 0).map(|&c| if c.is_ascii_graphic() { c as char } else { '?' }).collect()
}

/// A packed 2-byte language or region code (3-letter codes are packed into 15 bits).
fn unpack_code(b: &[u8], base: u8) -> String {
    let (b0, b1) = (b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0));
    if b0 & 0x80 != 0 {
        let first = b1 & 0x1f;
        let second = ((b1 & 0xe0) >> 5) | ((b0 & 0x03) << 3);
        let third = (b0 & 0x7c) >> 2;
        [first, second, third].iter().map(|c| (c.wrapping_add(base)) as char).collect()
    } else {
        ascii2(&[b0, b1])
    }
}

fn locale_qualifier(r: &Reader<'_>, off: usize, size: usize, get8: impl Fn(usize) -> Option<u8>) -> String {
    let lang = if size >= 12 { r.slice(off + 8, 2).map(|b| unpack_code(b, b'a')).unwrap_or_default() } else { String::new() };
    let region = if size >= 12 { r.slice(off + 10, 2).map(|b| unpack_code(b, b'0')).unwrap_or_default() } else { String::new() };
    // Extended locale fields were appended to ResTable_config. Older
    // configurations end before one or more of these fields.
    let subtag = |start: usize, len: usize| {
        if start + len <= size { r.slice(off + start, len).map(ascii2).unwrap_or_default() } else { String::new() }
    };
    let script = if get8(52).unwrap_or(0) == 0 { subtag(36, 4) } else { String::new() };
    let variant = subtag(40, 8);
    let numbering = subtag(53, 8);
    let mut locale = lang;
    if !locale.is_empty() && (!script.is_empty() || !variant.is_empty() || !numbering.is_empty()) {
        locale.insert_str(0, "b+");
        for part in [&script, &region, &variant].into_iter().filter(|p| !p.is_empty()) {
            locale.push('+');
            locale.push_str(part);
        }
        if !numbering.is_empty() {
            locale.push_str("+u+nu+");
            locale.push_str(&numbering);
        }
    } else if !locale.is_empty() && !region.is_empty() {
        locale.push_str(&format!("-r{region}"));
    }
    locale
}

/// Qualifier string and locale for a `ResTable_config` at `off`.
fn config_qualifiers(r: &Reader<'_>, off: usize) -> (String, String) {
    let size = r.u32_le(off).unwrap_or(0) as usize;
    let get8 = |o: usize| if o < size { r.u8(off + o) } else { None };
    let get16 = |o: usize| if o + 1 < size { r.u16_le(off + o) } else { None };
    let mut q: Vec<String> = Vec::new();
    if let Some(mcc) = get16(4).filter(|&v| v != 0) {
        q.push(format!("mcc{mcc}"));
    }
    if let Some(mnc) = get16(6).filter(|&v| v != 0) {
        q.push(format!("mnc{}", if mnc == u16::MAX { 0 } else { mnc }));
    }
    let locale = locale_qualifier(r, off, size, get8);
    if !locale.is_empty() {
        q.push(locale.clone());
    }
    let axis = |q: &mut Vec<String>, v: Option<u8>, mask, names: &[(u8, &str)]| {
        if let Some(key) = v.map(|n| n & mask)
            && let Some((_, name)) = names.iter().find(|(value, _)| *value == key)
        {
            q.push((*name).to_owned());
        }
    };
    axis(&mut q, get8(19), 0x03, &[(1, "neuter"), (2, "feminine"), (3, "masculine")]);
    if let Some(sl) = get8(28) {
        match sl & 0xc0 {
            0x40 => q.push("ldltr".into()),
            0x80 => q.push("ldrtl".into()),
            _ => {}
        }
    }
    if let Some(sw) = get16(30).filter(|&v| v != 0) {
        q.push(format!("sw{sw}dp"));
    }
    if let Some(w) = get16(32).filter(|&v| v != 0) {
        q.push(format!("w{w}dp"));
    }
    if let Some(h) = get16(34).filter(|&v| v != 0) {
        q.push(format!("h{h}dp"));
    }
    if let Some(sl) = get8(28) {
        match sl & 0x0f {
            1 => q.push("small".into()),
            2 => q.push("normal".into()),
            3 => q.push("large".into()),
            4 => q.push("xlarge".into()),
            _ => {}
        }
        match sl & 0x30 {
            0x10 => q.push("notlong".into()),
            0x20 => q.push("long".into()),
            _ => {}
        }
    }
    axis(&mut q, get8(48), 0x03, &[(1, "notround"), (2, "round")]);
    axis(&mut q, get8(49), 0x03, &[(1, "nowidecg"), (2, "widecg")]);
    axis(&mut q, get8(49), 0x0c, &[(4, "lowdr"), (8, "highdr")]);
    match get8(12) {
        Some(1) => q.push("port".into()),
        Some(2) => q.push("land".into()),
        Some(3) => q.push("square".into()),
        _ => {}
    }
    if let Some(ui) = get8(29) {
        match ui & 0x0f {
            2 => q.push("desk".into()),
            3 => q.push("car".into()),
            4 => q.push("television".into()),
            5 => q.push("appliance".into()),
            6 => q.push("watch".into()),
            7 => q.push("vrheadset".into()),
            _ => {}
        }
        match ui & 0x30 {
            0x10 => q.push("notnight".into()),
            0x20 => q.push("night".into()),
            _ => {}
        }
    }
    if let Some(d) = get16(14).filter(|&v| v != 0) {
        q.push(match d {
            120 => "ldpi".into(),
            160 => "mdpi".into(),
            213 => "tvdpi".into(),
            240 => "hdpi".into(),
            320 => "xhdpi".into(),
            480 => "xxhdpi".into(),
            640 => "xxxhdpi".into(),
            0xfffe => "anydpi".into(),
            0xffff => "nodpi".into(),
            d => format!("{d}dpi"),
        });
    }
    match get8(13) {
        Some(1) => q.push("notouch".into()),
        Some(3) => q.push("finger".into()),
        _ => {}
    }
    axis(&mut q, get8(18), 0x03, &[(1, "keysexposed"), (2, "keyshidden"), (3, "keyssoft")]);
    match get8(16) {
        Some(1) => q.push("nokeys".into()),
        Some(2) => q.push("qwerty".into()),
        Some(3) => q.push("12key".into()),
        _ => {}
    }
    axis(&mut q, get8(18), 0x0c, &[(4, "navexposed"), (8, "navhidden")]);
    match get8(17) {
        Some(1) => q.push("nonav".into()),
        Some(2) => q.push("dpad".into()),
        Some(3) => q.push("trackball".into()),
        Some(4) => q.push("wheel".into()),
        _ => {}
    }
    if let (Some(w), Some(h)) = (get16(20), get16(22))
        && w != 0
        && h != 0
    {
        q.push(format!("{w}x{h}"));
    }
    if let Some(v) = get16(24).filter(|&v| v != 0) {
        q.push(format!("v{v}"));
    }
    let s = if q.is_empty() { "default".to_string() } else { q.join("-") };
    (s, locale)
}

/// A type's configuration is part of its header, not the entry offsets or payload.
fn type_config(r: &Reader<'_>, at: usize, header_size: usize) -> Option<(String, String)> {
    let config = Reader::new(r.slice(at.checked_add(20)?, header_size.checked_sub(20)?)?);
    let size = config.u32_le(0)? as usize;
    if size < 4 || config.slice(0, size).is_none() {
        return None;
    }
    Some(config_qualifiers(&config, 0))
}

struct Pkg<'a> {
    name: &'a str,
    id: u32,
    types: &'a StringPool,
    keys: &'a StringPool,
}

fn type_name(p: &Pkg<'_>, id: u8) -> String {
    p.types.get(u32::from(id).wrapping_sub(1)).map(str::to_string).unwrap_or_else(|| format!("type{id}"))
}

/// Parse a resource table.
pub fn parse(data: &[u8], limits: Limits) -> Result<Table, Error> {
    let r = Reader::new(data);
    let (t, h, size) = chunk(&r, 0).ok_or_else(|| Error::Truncated("resource table header".into()))?;
    if t != RES_TABLE {
        return Err(Error::Malformed(format!("not a resource table (chunk type {t:#x})")));
    }
    if h < 12 {
        return Err(Error::Truncated("resource table header".into()));
    }
    let data = data.get(..size).ok_or_else(|| Error::Truncated("resource table payload".into()))?;
    let r = Reader::new(data);
    let mut table = Table::default();
    let mut global = StringPool::default();
    let mut value_budget = ValueBudget::new(limits.max_value_bytes);
    let mut pos = h;
    while pos < size {
        let Some((ct, _ch, cs)) = chunk(&r, pos) else {
            table.truncated = true;
            table.warnings.push(format!("unreadable chunk at offset {pos}"));
            break;
        };
        match ct {
            crate::axml::RES_STRING_POOL => match StringPool::parse(data, pos) {
                Ok(p) => {
                    report_partial_string_pool(&p, "global string pool", &mut table.truncated, &mut table.warnings);
                    table.global_strings = p.declared;
                    global = p;
                }
                Err(e) => {
                    table.truncated = true;
                    table.warnings.push(format!("global string pool: {e}"));
                }
            },
            RES_PACKAGE => {
                if table.packages.len() >= MAX_PACKAGES {
                    table.truncated = true;
                    table.warnings.push("too many packages; the rest were skipped".into());
                } else {
                    parse_package(data, pos, cs, &global, &mut table, limits, &mut value_budget);
                }
            }
            _ => {}
        }
        pos = match pos.checked_add(cs) {
            Some(p) => p,
            None => break,
        };
    }
    Ok(table)
}

fn parse_package(data: &[u8], pos: usize, cs: usize, global: &StringPool, table: &mut Table, limits: Limits, value_budget: &mut ValueBudget) {
    let Some(data) = pos.checked_add(cs).and_then(|end| data.get(..end)) else {
        table.truncated = true;
        table.warnings.push("package runs past its resource table".into());
        return;
    };
    let r = Reader::new(data);
    let hsize = usize::from(r.u16_le(pos + 2).unwrap_or(0));
    if hsize < 284 {
        table.truncated = true;
        table.warnings.push(format!("package at offset {pos}: incomplete header"));
        return;
    }
    let id = r.u32_le(pos + 8).unwrap_or(0);
    let name = utf16_fixed(&r, pos + 12, 128);
    // Older 284-byte packages predate this field. Type names still use the
    // unshifted chunk ID, while public resource identities include the offset.
    let type_id_offset = if hsize >= 288 { r.u32_le(pos + 284).unwrap_or(0) } else { 0 };
    let Ok(type_id_offset) = u8::try_from(type_id_offset) else {
        table.truncated = true;
        table.warnings.push(format!("package {name}: type ID offset is outside the one-byte type field"));
        return;
    };
    let type_strings = r.u32_le(pos + 268).unwrap_or(0) as usize;
    let key_strings = r.u32_le(pos + 276).unwrap_or(0) as usize;
    let end = data.len();
    let mut read_pool = |offset: usize, label: &str| {
        if offset == 0 {
            return StringPool::default();
        }
        let Some(start) = pos.checked_add(offset).filter(|&start| offset >= hsize && start < end) else {
            table.truncated = true;
            table.warnings.push(format!("package {name}: {label} string pool is outside its payload"));
            return StringPool::default();
        };
        match StringPool::parse(data, start) {
            Ok(pool) => {
                report_partial_string_pool(&pool, &format!("package {name}: {label} string pool"), &mut table.truncated, &mut table.warnings);
                pool
            }
            Err(error) => {
                table.truncated = true;
                table.warnings.push(format!("package {name}: {label} string pool: {error}"));
                StringPool::default()
            }
        }
    };
    let types = read_pool(type_strings, "type");
    let keys = read_pool(key_strings, "key");
    let mut package = Package { id, name: name.clone(), types: Vec::new(), keys: keys.declared, libraries: Vec::new() };
    let p = Pkg { name: &name, id, types: &types, keys: &keys };
    let mut c = pos.saturating_add(hsize.max(8));
    while c < end {
        let Some((ct, ch, sz)) = chunk(&r, c) else {
            table.truncated = true;
            table.warnings.push(format!("package {name}: unreadable chunk at offset {c}"));
            break;
        };
        // Each child was checked against its package; reads within it must also stop before
        // the next sibling, including fixed-width library records and bag headers.
        let Some(child) = data.get(..c.saturating_add(sz)) else {
            table.truncated = true;
            table.warnings.push(format!("package {name}: child chunk runs past its package"));
            break;
        };
        let r = Reader::new(child);
        let local_tid = r.u8(c + 8).unwrap_or(0);
        let type_id = local_tid.checked_add(type_id_offset).filter(|_| local_tid != 0);
        match (ct, type_id) {
            (RES_TYPE_SPEC, Some(tid)) if ch >= 16 => {
                let count = r.u32_le(c + 12).unwrap_or(0);
                if !package.types.iter().any(|t| t.id == tid) && package.types.len() < 256 {
                    package.types.push(TypeInfo { id: tid, name: type_name(&p, local_tid), entry_count: count, configs: Vec::new() });
                }
            }
            (RES_TYPE, Some(tid)) if ch >= 24 => {
                let Some((cfg, locale)) = type_config(&r, c, ch) else {
                    table.truncated = true;
                    table.warnings.push(format!("package {name}: incomplete type configuration at offset {c}"));
                    // The child extent was already validated by chunk().
                    c += sz;
                    continue;
                };
                match package.types.iter_mut().find(|t| t.id == tid) {
                    Some(t) => {
                        if !t.configs.contains(&cfg) && t.configs.len() < 4096 {
                            t.configs.push(cfg.clone());
                        }
                    }
                    None => {
                        if package.types.len() < 256 {
                            package.types.push(TypeInfo {
                                id: tid,
                                name: type_name(&p, local_tid),
                                entry_count: r.u32_le(c + 12).unwrap_or(0),
                                configs: vec![cfg.clone()],
                            })
                        }
                    }
                }
                parse_type(&r, c, (ch, sz), (cfg, locale), &p, tid, global, table, limits, value_budget);
            }
            (RES_TYPE_SPEC | RES_TYPE, None) if ch >= 16 => {
                table.truncated = true;
                table.warnings.push(format!("package {name}: type ID {local_tid} with offset {type_id_offset} is outside the one-byte type field"));
            }
            (RES_LIBRARY, _) if ch >= 12 => {
                let n = r.u32_le(c + 8).unwrap_or(0) as usize;
                let available = (sz - ch) / 260;
                if n > available {
                    table.warnings.push(format!("package {name}: library records run past their chunk"));
                }
                if n > available.min(256) {
                    table.truncated = true;
                }
                for i in 0..n.min(256).min(available) {
                    let e = c + ch + i * 260;
                    if let Some(lid) = r.u32_le(e) {
                        package.libraries.push((lid, utf16_fixed(&r, e + 4, 128)));
                    }
                }
            }
            (RES_TYPE_SPEC | RES_TYPE | RES_LIBRARY, _) => {
                table.truncated = true;
                table.warnings.push(format!("package {name}: incomplete chunk header at offset {c}"));
            }
            _ => {}
        }
        c = match c.checked_add(sz) {
            Some(x) => x,
            None => break,
        };
    }
    table.packages.push(package);
}

#[allow(clippy::too_many_arguments)]
fn parse_type(
    r: &Reader<'_>,
    c: usize,
    (ch, sz): (usize, usize),
    (cfg, locale): (String, String),
    p: &Pkg<'_>,
    type_id: u8,
    global: &StringPool,
    table: &mut Table,
    limits: Limits,
    value_budget: &mut ValueBudget,
) {
    let tid = r.u8(c + 8).unwrap_or(0);
    let flags = r.u8(c + 9).unwrap_or(0);
    let count = r.u32_le(c + 12).unwrap_or(0) as usize;
    let entries_start = r.u32_le(c + 16).unwrap_or(0) as usize;
    let end = c + sz;
    let tname = type_name(p, tid);
    let sparse = flags & 0x01 != 0;
    let off16 = flags & 0x02 != 0;
    // (entry index, offset from entries_start)
    let mut slots: Vec<(u32, usize)> = Vec::new();
    let table_bytes = if sparse || off16 { count.saturating_mul(if sparse { 4 } else { 2 }) } else { count.saturating_mul(4) };
    if c.saturating_add(ch).saturating_add(table_bytes) > end {
        table.truncated = true;
        table.warnings.push(format!("{tname} ({cfg}): entry table runs past its chunk"));
        return;
    }
    if count != 0 && (entries_start < ch.saturating_add(table_bytes) || entries_start > sz) {
        table.truncated = true;
        table.warnings.push(format!("{tname} ({cfg}): invalid entry data start"));
        return;
    }
    for i in 0..count {
        let at = c + ch;
        if sparse {
            let (Some(idx), Some(o)) = (r.u16_le(at + i * 4), r.u16_le(at + i * 4 + 2)) else { break };
            slots.push((u32::from(idx), usize::from(o) * 4));
        } else if off16 {
            let Some(o) = r.u16_le(at + i * 2) else { break };
            if o != 0xffff {
                slots.push((i as u32, usize::from(o) * 4));
            }
        } else {
            let Some(o) = r.u32_le(at + i * 4) else { break };
            if o != NO_ENTRY {
                slots.push((i as u32, o as usize));
            }
        }
    }
    let mut skipped = 0usize;
    for (idx, o) in slots {
        if table.entries.len() >= limits.max_entries {
            table.truncated = true;
            break;
        }
        let Some(e) = c.checked_add(entries_start).and_then(|x| x.checked_add(o)).filter(|e| e.saturating_add(8) <= end) else {
            skipped += 1;
            continue;
        };
        let esize = usize::from(r.u16_le(e).unwrap_or(0));
        let eflags = r.u16_le(e + 2).unwrap_or(0);
        let id = (p.id << 24) | (u32::from(type_id) << 16) | (idx & 0xffff);
        let mut entry = Entry {
            id: format!("{id:#010x}"),
            package: p.name.to_string(),
            type_name: tname.clone(),
            key: String::new(),
            config: cfg.clone(),
            locale: locale.clone(),
            value: None,
            value_type: None,
            parent: None,
            bag: Vec::new(),
            raw: None,
        };
        if eflags & 0x08 != 0 {
            // Compact entry: key index in the size field, type in the high byte of flags.
            entry.key = p.keys.get(u32::from(r.u16_le(e).unwrap_or(0))).unwrap_or("").to_string();
            let v = ResValue { data_type: (eflags >> 8) as u8, data: r.u32_le(e + 4).unwrap_or(0) };
            entry.value = value_budget.take(v.format(global), &mut table.truncated, &mut table.warnings, VALUE_BUDGET_WARNING);
            entry.value_type = Some(v.type_name());
            entry.raw = Some(v);
        } else {
            entry.key = r.u32_le(e + 4).and_then(|k| p.keys.get(k)).unwrap_or("").to_string();
            if eflags & 0x01 != 0 {
                if esize < 16 || e.saturating_add(esize) > end {
                    table.truncated = true;
                    table.warnings.push(format!("{tname} ({cfg}): incomplete bag header"));
                    table.entries.push(entry);
                    continue;
                }
                let n = r.u32_le(e + 12).unwrap_or(0) as usize;
                let mut m = e + esize;
                if n > (end - m) / 12 {
                    table.truncated = true;
                    table.warnings.push(format!("{tname} ({cfg}): incomplete bag data"));
                    table.entries.push(entry);
                    continue;
                }
                let parent = r.u32_le(e + 8).unwrap_or(0);
                if parent != 0 {
                    entry.parent = Some(format!("@{parent:#010x}"));
                }
                if n > MAX_BAG {
                    table.truncated = true;
                }
                for _ in 0..n.min(MAX_BAG) {
                    if m.saturating_add(12) > end {
                        break;
                    }
                    let name = r.u32_le(m).unwrap_or(0);
                    let v = ResValue { data_type: r.u8(m + 7).unwrap_or(0), data: r.u32_le(m + 8).unwrap_or(0) };
                    let value =
                        value_budget.take(v.format(global), &mut table.truncated, &mut table.warnings, VALUE_BUDGET_WARNING).unwrap_or_default();
                    entry.bag.push(BagItem { name: format!("@{name:#010x}"), value, value_type: v.type_name() });
                    m += 12;
                }
            } else {
                let v_at = e.saturating_add(esize.max(8));
                if v_at.saturating_add(8) <= end {
                    let v = ResValue { data_type: r.u8(v_at + 3).unwrap_or(0), data: r.u32_le(v_at + 4).unwrap_or(0) };
                    entry.value = value_budget.take(v.format(global), &mut table.truncated, &mut table.warnings, VALUE_BUDGET_WARNING);
                    entry.value_type = Some(v.type_name());
                    entry.raw = Some(v);
                } else {
                    table.truncated = true;
                    table.warnings.push(format!("{tname} ({cfg}): incomplete value"));
                }
            }
        }
        table.entries.push(entry);
    }
    if skipped > 0 {
        table.truncated = true;
        table.warnings.push(format!("{tname} ({cfg}): {skipped} entries run past their chunk"));
    }
}

impl Table {
    /// String resources grouped by locale (`""` = default): (locale, key, value).
    pub fn strings_by_locale(&self) -> std::collections::BTreeMap<String, Vec<(String, String)>> {
        let mut out: std::collections::BTreeMap<String, Vec<(String, String)>> = Default::default();
        for e in self.entries.iter().filter(|e| e.type_name == "string") {
            if let Some(v) = &e.value {
                out.entry(e.locale.clone()).or_default().push((e.key.clone(), v.clone()));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures as fx;

    #[test]
    fn incomplete_table_prefix_is_rejected() {
        let bytes = fx::resources_arsc();
        assert!(matches!(parse(&bytes[..7], Limits::default()), Err(Error::Truncated(_))));
    }

    #[test]
    fn decodes_table() {
        let t = parse(&fx::resources_arsc(), Limits::default()).unwrap();
        assert!(t.warnings.is_empty(), "{:?}", t.warnings);
        assert_eq!(t.packages.len(), 1);
        let p = &t.packages[0];
        assert_eq!(p.id, 0x7f);
        assert_eq!(p.name, fx::PACKAGE);
        assert_eq!(p.types[0].name, "string");
        assert_eq!(p.types[0].configs, vec!["default", "fr"]);
        let by = t.strings_by_locale();
        assert_eq!(by[""], vec![("app_name".to_string(), "Notes".to_string()), ("greeting".to_string(), "Hello".to_string())]);
        assert_eq!(by["fr"], vec![("app_name".to_string(), "Notes FR".to_string()), ("greeting".to_string(), "Bonjour".to_string())]);
        let app = t.entries.iter().find(|e| e.key == "app_name" && e.locale.is_empty()).unwrap();
        assert_eq!(app.id, "0x7f010000");
        let style = t.entries.iter().find(|e| e.type_name == "style").unwrap();
        assert_eq!(style.key, "AppTheme");
        assert_eq!(style.parent.as_deref(), Some("@0x01030005"));
        assert_eq!(style.bag[0].value, "#ff336699");
        assert_eq!(style.config, "night-xhdpi-v21");
    }

    #[test]
    fn skipped_type_chunks_and_entries_mark_the_table_partial() {
        let bytes = fx::resources_arsc();
        // The French type chunk: RES_TYPE, an 84-byte header, and "fr" in its configuration.
        let fr = (0..bytes.len() - 30).find(|&i| bytes[i..i + 4] == [0x01, 0x02, 84, 0] && &bytes[i + 28..i + 30] == b"fr").unwrap();
        let corrupt = |at: usize, value: u32| {
            let mut b = bytes.clone();
            b[at..at + 4].copy_from_slice(&value.to_le_bytes());
            parse(&b, Limits::default()).unwrap()
        };
        for (what, t) in [
            ("entry table past its chunk", corrupt(fr + 12, 1000)),
            ("invalid entry data start", corrupt(fr + 16, 0xffff)),
            ("an entry past its chunk", corrupt(fr + 84, 0x1000)),
            // The second entry's header fits, but its value runs past the chunk.
            ("a value past its chunk", corrupt(fr + 88, 24)),
        ] {
            let french = t.entries.iter().filter(|e| e.locale == "fr" && e.value.is_some()).count();
            assert!(french < 2, "{what}: the corruption should lose a French value");
            assert!(t.truncated, "{what}: lost content must mark the table partial");
            assert!(!t.warnings.is_empty(), "{what}: lost content must be reported");
        }
    }

    #[test]
    fn distinct_locale_scripts_and_variants_keep_separate_keys() {
        let mut cfg = [0u8; 64];
        cfg[0] = 64;
        cfg[8..12].copy_from_slice(b"zhTW");
        cfg[36..40].copy_from_slice(b"Hant");
        assert_eq!(config_qualifiers(&Reader::new(&cfg), 0).1, "b+zh+Hant+TW");
        cfg[36..40].copy_from_slice(b"Hans");
        assert_eq!(config_qualifiers(&Reader::new(&cfg), 0).1, "b+zh+Hans+TW");
        cfg[52] = 1; // The inferred script is not part of the requested locale.
        assert_eq!(config_qualifiers(&Reader::new(&cfg), 0).1, "zh-rTW");
        cfg[40..45].copy_from_slice(b"POSIX");
        assert_eq!(config_qualifiers(&Reader::new(&cfg), 0).1, "b+zh+TW+POSIX");
        cfg[53..57].copy_from_slice(b"latn");
        assert_eq!(config_qualifiers(&Reader::new(&cfg), 0).1, "b+zh+TW+POSIX+u+nu+latn");
        cfg[0] = 36; // Bytes beyond the declared configuration are unrelated.
        assert_eq!(config_qualifiers(&Reader::new(&cfg), 0).1, "zh-rTW");
    }

    #[test]
    fn qualifiers() {
        let mut cfg = vec![0u8; 64];
        cfg[0] = 64;
        cfg[8] = b'e';
        cfg[9] = b's';
        cfg[10] = b'M';
        cfg[11] = b'X';
        cfg[12] = 2;
        let (q, l) = config_qualifiers(&Reader::new(&cfg), 0);
        assert_eq!(q, "es-rMX-land");
        assert_eq!(l, "es-rMX");
    }

    fn le_u16(v: &mut Vec<u8>, x: u16) {
        v.extend_from_slice(&x.to_le_bytes());
    }

    fn le_u32(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_le_bytes());
    }

    fn type_spec(id: u8, entries: u32) -> Vec<u8> {
        let mut c = Vec::new();
        le_u16(&mut c, 0x0202);
        le_u16(&mut c, 16);
        le_u32(&mut c, 16 + 4 * entries);
        c.push(id);
        c.push(0);
        le_u16(&mut c, 0);
        le_u32(&mut c, entries);
        for _ in 0..entries {
            le_u32(&mut c, 0);
        }
        c
    }

    fn type_chunk(id: u8, entries: &[Vec<u8>]) -> Vec<u8> {
        let mut cfg = vec![0u8; 64];
        cfg[0] = 64;
        let header = 20 + cfg.len();
        let start = header + 4 * entries.len();
        let mut body = Vec::new();
        let mut offs = Vec::new();
        for e in entries {
            offs.push(body.len() as u32);
            body.extend_from_slice(e);
        }
        let mut c = Vec::new();
        le_u16(&mut c, 0x0201);
        le_u16(&mut c, header as u16);
        le_u32(&mut c, (start + body.len()) as u32);
        c.push(id);
        c.push(0);
        le_u16(&mut c, 0);
        le_u32(&mut c, entries.len() as u32);
        le_u32(&mut c, start as u32);
        c.extend_from_slice(&cfg);
        for o in offs {
            le_u32(&mut c, o);
        }
        c.extend(body);
        c
    }

    fn table_with_entries(pool: &[&str], type_name: &str, keys: &[&str], entries: &[Vec<u8>]) -> Vec<u8> {
        let global = fx::string_pool(pool, true);
        let types = fx::string_pool(&[type_name], false);
        let key_pool = fx::string_pool(keys, false);
        let mut chunks = type_spec(1, entries.len() as u32);
        chunks.extend(type_chunk(1, entries));
        let header = 288usize;
        let mut pkg = Vec::new();
        le_u16(&mut pkg, 0x0200);
        le_u16(&mut pkg, header as u16);
        le_u32(&mut pkg, (header + types.len() + key_pool.len() + chunks.len()) as u32);
        le_u32(&mut pkg, 0x7f);
        pkg.extend(vec![0u8; 256]);
        le_u32(&mut pkg, header as u32);
        le_u32(&mut pkg, 1);
        le_u32(&mut pkg, (header + types.len()) as u32);
        le_u32(&mut pkg, keys.len() as u32);
        le_u32(&mut pkg, 0);
        pkg.extend(types);
        pkg.extend(key_pool);
        pkg.extend(chunks);
        let mut out = Vec::new();
        le_u16(&mut out, 0x0002);
        le_u16(&mut out, 12);
        le_u32(&mut out, (12 + global.len() + pkg.len()) as u32);
        le_u32(&mut out, 1);
        out.extend(global);
        out.extend(pkg);
        out
    }

    fn simple_string_entry(key: u32, pool_index: u32) -> Vec<u8> {
        let mut e = Vec::new();
        le_u16(&mut e, 8);
        le_u16(&mut e, 0);
        le_u32(&mut e, key);
        le_u16(&mut e, 8);
        e.push(0);
        e.push(0x03);
        le_u32(&mut e, pool_index);
        e
    }

    fn compact_string_entry(key: u16, pool_index: u32) -> Vec<u8> {
        let mut e = Vec::new();
        le_u16(&mut e, key);
        le_u16(&mut e, 0x0308);
        le_u32(&mut e, pool_index);
        e
    }

    fn bag_string_entry(key: u32, pool_indices: &[u32]) -> Vec<u8> {
        let mut e = Vec::new();
        le_u16(&mut e, 16);
        le_u16(&mut e, 1);
        le_u32(&mut e, key);
        le_u32(&mut e, 0);
        le_u32(&mut e, pool_indices.len() as u32);
        for (i, idx) in pool_indices.iter().enumerate() {
            le_u32(&mut e, 0x0101_0000 + i as u32);
            le_u16(&mut e, 8);
            e.push(0);
            e.push(0x03);
            le_u32(&mut e, *idx);
        }
        e
    }

    #[test]
    fn value_budget_omits_long_copies_and_keeps_a_later_short_value() {
        let long = "x".repeat(16);
        let tight = Limits { max_value_bytes: 20, ..Limits::default() };
        let keys = ["a", "b", "c"];

        let simple =
            table_with_entries(&[&long, "ok"], "string", &keys, &[simple_string_entry(0, 0), simple_string_entry(1, 0), simple_string_entry(2, 1)]);
        let complete = parse(&simple, Limits::default()).unwrap();
        assert!(!complete.truncated, "{:?}", complete.warnings);
        assert_eq!(complete.entries.iter().map(|e| e.value.as_deref()).collect::<Vec<_>>(), [Some(long.as_str()), Some(long.as_str()), Some("ok")]);

        let partial = parse(&simple, tight).unwrap();
        assert!(partial.truncated);
        assert!(partial.warnings.iter().any(|w| w.contains("value budget")), "{:?}", partial.warnings);
        assert_eq!(partial.entries.iter().map(|e| e.value.as_deref()).collect::<Vec<_>>(), [Some(long.as_str()), None, Some("ok")]);

        let compact = table_with_entries(
            &[&long, "ok"],
            "string",
            &keys,
            &[compact_string_entry(0, 0), compact_string_entry(1, 0), compact_string_entry(2, 1)],
        );
        let partial = parse(&compact, tight).unwrap();
        assert!(partial.truncated);
        assert!(partial.warnings.iter().any(|w| w.contains("value budget")), "{:?}", partial.warnings);
        assert_eq!(partial.entries.iter().map(|e| e.value.as_deref()).collect::<Vec<_>>(), [Some(long.as_str()), None, Some("ok")]);

        let bag = table_with_entries(&[&long, "ok"], "style", &["theme"], &[bag_string_entry(0, &[0, 0, 1])]);
        let complete = parse(&bag, Limits::default()).unwrap();
        assert!(!complete.truncated, "{:?}", complete.warnings);
        assert_eq!(complete.entries[0].bag.iter().map(|i| i.value.as_str()).collect::<Vec<_>>(), [long.as_str(), long.as_str(), "ok"]);

        let partial = parse(&bag, tight).unwrap();
        assert!(partial.truncated);
        assert!(partial.warnings.iter().any(|w| w.contains("value budget")), "{:?}", partial.warnings);
        assert_eq!(partial.entries[0].bag.iter().map(|i| i.value.as_str()).collect::<Vec<_>>(), [long.as_str(), "", "ok"]);
    }
}

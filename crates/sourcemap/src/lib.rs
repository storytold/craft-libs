#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Source maps (revision 3), including indexed maps with sections.
//!
//! Lookups follow the "original positions for a generated position" rule: on the generated line,
//! take the greatest segment column not after the requested column and return every segment that
//! starts there. Lines are 0-based here (callers add one for display); columns are 0-based UTF-16
//! code units as the format defines.

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::cell::Cell;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Biggest map JSON accepted.
    pub max_bytes: usize,
    /// Most decoded segments across the whole map.
    pub max_segments: usize,
    /// Most generated lines across the whole map, including empty lines and inline sections.
    pub max_lines: usize,
    /// Deepest section nesting (the format allows one level; we tolerate a few).
    pub max_depth: usize,
    /// Most `sources`, `sourcesContent`, `names` and ignore-list entries and index-map sections
    /// across the whole map, counted while the JSON is read, so a map of tiny entries is refused
    /// before it is kept. An entry a repeated key replaces gives its slot back.
    pub max_entries: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_bytes: 64 * 1024 * 1024, max_segments: 4_000_000, max_lines: 1_000_000, max_depth: 8, max_entries: 4_000_000 }
    }
}

/// Limits on retained source names after applying each map's root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventoryLimits {
    pub max_sources: usize,
    /// Total UTF-8 bytes in the expanded source paths, across all sections.
    pub max_bytes: usize,
}

impl Default for InventoryLimits {
    fn default() -> Self {
        Self { max_sources: 1_000_000, max_bytes: 64 * 1024 * 1024 }
    }
}

/// An ordered prefix of the map's source names, with their complete identities.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceInventory {
    pub sources: Vec<String>,
    /// More names existed than the limits or available allocations could retain.
    pub truncated: bool,
    /// Index-map sections that pointed at an external map nobody read, so the sources they
    /// would add are unknown (not absent).
    pub external_sections: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceMapError {
    #[error("source map is {0} bytes; the limit is {1}")]
    TooLarge(usize, usize),
    #[error("source map is not valid JSON: {0}")]
    BadJson(String),
    #[error("not a version 3 source map")]
    BadVersion,
    #[error("source map mappings are malformed at segment {0}")]
    BadMappings(usize),
    #[error("source map index has invalid sections")]
    BadSections,
    #[error("source map has more than {0} segments")]
    TooManySegments(usize),
    #[error("source map has more than {0} generated lines")]
    TooManyLines(usize),
    #[error("could not reserve memory for generated source-map lines")]
    LineAllocationFailed,
    #[error("source map sections nest deeper than {0}")]
    TooDeep(usize),
    #[error("source map has more than {0} source, content, name, ignore-list and section entries")]
    TooManyEntries(usize),
}

/// One decoded mapping segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub generated_column: u32,
    /// (source index, original line, original column, name index)
    pub original: Option<(u32, u32, u32, Option<u32>)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourceMap {
    pub file: Option<String>,
    pub source_root: Option<String>,
    pub sources: Vec<Option<String>>,
    pub sources_content: Vec<Option<String>>,
    pub names: Vec<String>,
    pub ignore_list: Vec<u32>,
    /// Segments per generated line, sorted by column.
    pub lines: Vec<Vec<Segment>>,
    pub segments: usize,
    /// Indexed maps: (line offset, column offset, map).
    pub sections: Vec<(u32, u32, SourceMap)>,
    /// Index-map sections that point at an external map (`url`), which isn't fetched here: the
    /// sources that map would add are unknown, not absent.
    pub external_sections: usize,
}

/// A position in an original source.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct OriginalPosition {
    pub source_index: u32,
    pub source: Option<String>,
    /// 0-based.
    pub line: u32,
    /// 0-based.
    pub column: u32,
    pub name: Option<String>,
    /// 0-based generated column of the matched segment.
    pub generated_column: u32,
}

impl SourceMap {
    /// The source path with `sourceRoot` prefixed before URL interpretation.
    pub fn source_path(&self, index: u32) -> Option<String> {
        let s = self.sources.get(index as usize)?.as_ref()?;
        // Generators commonly emit an empty root to mean no root. Keep those source identities
        // unchanged even though the literal ECMA-426 prefix algorithm would prepend a slash.
        Some(match self.source_root.as_deref().filter(|root| !root.is_empty()) {
            Some(root) => {
                let join = if root.ends_with('/') { "" } else { "/" };
                format!("{root}{join}{s}")
            }
            None => s.clone(),
        })
    }

    pub fn source_content(&self, index: u32) -> Option<&str> {
        self.sources_content.get(index as usize)?.as_deref()
    }

    /// Every original position for a generated (line, column), both 0-based.
    pub fn original_positions(&self, line: u32, column: u32) -> Vec<OriginalPosition> {
        if !self.sections.is_empty() {
            // Last section that starts at or before the position.
            let end = self.sections.partition_point(|(l, c, _)| (*l, *c) <= (line, column));
            let Some((l, c, map)) = end.checked_sub(1).and_then(|i| self.sections.get(i)) else { return Vec::new() };
            let rel_line = line.saturating_sub(*l);
            let rel_col = if rel_line == 0 { column.saturating_sub(*c) } else { column };
            return map
                .original_positions(rel_line, rel_col)
                .into_iter()
                .filter_map(|mut position| {
                    if rel_line == 0 {
                        position.generated_column = position.generated_column.checked_add(*c)?;
                    }
                    Some(position)
                })
                .collect();
        }
        let Some(segs) = self.lines.get(line as usize) else { return Vec::new() };
        // Greatest generated column <= column.
        let idx = segs.partition_point(|s| s.generated_column <= column);
        let Some(best) = idx.checked_sub(1).and_then(|i| segs.get(i)) else { return Vec::new() };
        let col = best.generated_column;
        let first = segs.partition_point(|s| s.generated_column < col);
        segs.get(first..idx)
            .unwrap_or_default()
            .iter()
            .filter_map(|s| {
                let (src, l, c, n) = s.original?;
                Some(OriginalPosition {
                    source_index: src,
                    source: self.source_path(src),
                    line: l,
                    column: c,
                    name: n.and_then(|n| self.names.get(n as usize).cloned()),
                    generated_column: col,
                })
            })
            .collect()
    }

    /// Sources flattened across sections, within the default inventory limits.
    /// Use `source_inventory` to choose limits and observe truncation.
    pub fn all_sources(&self) -> Vec<String> {
        self.source_inventory(InventoryLimits::default()).sources
    }

    /// Retain a bounded prefix, checking expanded lengths before constructing paths.
    pub fn source_inventory(&self, limits: InventoryLimits) -> SourceInventory {
        let mut sources = Vec::new();
        let mut bytes_left = limits.max_bytes;
        let truncated = !self.collect_sources(limits.max_sources, &mut bytes_left, &mut sources);
        SourceInventory { sources, truncated, external_sections: self.external_sections_total() }
    }

    /// This map's external sections plus those of every map it holds.
    fn external_sections_total(&self) -> usize {
        self.sections.iter().fold(self.external_sections, |n, (_, _, map)| n.saturating_add(map.external_sections_total()))
    }

    /// Append this map and its sections using one shared entry and byte budget.
    fn collect_sources(&self, max_sources: usize, bytes_left: &mut usize, out: &mut Vec<String>) -> bool {
        let root = self.source_root.as_deref().unwrap_or_default();
        let join = if root.is_empty() || root.ends_with('/') { "" } else { "/" };
        for source in self.sources.iter().flatten() {
            let Some(bytes) = root.len().checked_add(join.len()).and_then(|n| n.checked_add(source.len())) else { return false };
            if out.len() >= max_sources || bytes > *bytes_left {
                return false;
            }
            let mut path = String::new();
            if out.try_reserve(1).is_err() || path.try_reserve_exact(bytes).is_err() {
                return false;
            }
            path.push_str(root);
            path.push_str(join);
            path.push_str(source);
            out.push(path);
            *bytes_left = bytes_left.saturating_sub(bytes);
        }
        for (_, _, map) in &self.sections {
            if !map.collect_sources(max_sources, bytes_left, out) {
                return false;
            }
        }
        true
    }
}

/// Parse a source map.
pub fn parse(json: &[u8], limits: Limits) -> Result<SourceMap, SourceMapError> {
    read(json, limits, true)
}

/// Read a map's sources, their content, names and ignore list without decoding its mappings or
/// checking its section offsets, so a map [`parse`] refuses for those
/// ([`SourceMapError::BadMappings`] or [`SourceMapError::BadSections`]) can still list its sources.
/// The map has no lines or segments. A map whose mappings, as far as they decode, name a source
/// past the end of its `sources` list is still refused, since the list is known to be short, and so are a section
/// that has neither a map nor a URL, or both, and a `sections` value that isn't an array.
pub fn parse_sources(json: &[u8], limits: Limits) -> Result<SourceMap, SourceMapError> {
    read(json, limits, false)
}

fn read(json: &[u8], limits: Limits, mappings: bool) -> Result<SourceMap, SourceMapError> {
    if json.len() > limits.max_bytes {
        return Err(SourceMapError::TooLarge(json.len(), limits.max_bytes));
    }
    // Some servers prefix maps with `)]}'` to defeat XSSI; the format allows skipping that line.
    let body = if json.starts_with(b")]}") { json.iter().position(|b| *b == b'\n').and_then(|p| json.get(p + 1..)).unwrap_or(&[]) } else { json };
    let entries = Entries { left: Cell::new(limits.max_entries), exhausted: Cell::new(false) };
    let mut de = serde_json::Deserializer::from_slice(body);
    let raw = RawMapSeed(&entries).deserialize(&mut de).and_then(|raw| de.end().map(|()| raw));
    let raw = match raw {
        Ok(raw) => raw,
        Err(_) if entries.exhausted.get() => return Err(SourceMapError::TooManyEntries(limits.max_entries)),
        Err(e) => return Err(SourceMapError::BadJson(e.to_string())),
    };
    let mut budget = limits.max_segments;
    let mut line_budget = limits.max_lines;
    from_raw(raw, 0, limits, &mut budget, &mut line_budget, mappings)
}

/// `mappings`: decode the mappings and check section offsets, as [`parse`] does.
fn from_raw(
    raw: Option<RawMap>,
    depth: usize,
    limits: Limits,
    budget: &mut usize,
    line_budget: &mut usize,
    mappings: bool,
) -> Result<SourceMap, SourceMapError> {
    if depth > limits.max_depth {
        return Err(SourceMapError::TooDeep(limits.max_depth));
    }
    let Some(raw) = raw.filter(|raw| raw.version == Some(3)) else {
        return Err(SourceMapError::BadVersion);
    };
    if let Some(sections) = raw.sections {
        // The index has its own file name. Only its section maps describe original sources.
        let mut map = SourceMap { file: raw.file, ..Default::default() };
        let sections = sections.ok_or(SourceMapError::BadSections)?;
        for section in sections {
            let offset = |n: Option<u64>| n.and_then(|n| u32::try_from(n).ok()).ok_or(SourceMapError::BadSections);
            let (line, col) = section.offset.unwrap_or_default();
            let (line, col) = if mappings { (offset(line)?, offset(col)?) } else { (offset(line).unwrap_or(0), offset(col).unwrap_or(0)) };
            if mappings && map.sections.last().is_some_and(|(l, c, _)| (*l, *c) > (line, col)) {
                return Err(SourceMapError::BadSections);
            }
            let (sub, external) = match (section.map, section.url) {
                (Some(m), None) => (from_raw(m.map(|m| *m), depth + 1, limits, budget, line_budget, mappings)?, false),
                // External maps are never fetched here. Retain their start so a lookup
                // cannot leak through to the previous inline map, and count it so the
                // sources it would add are reported unknown rather than absent.
                (None, Some(true)) => (SourceMap::default(), true),
                _ => return Err(SourceMapError::BadSections),
            };
            if external {
                map.external_sections = map.external_sections.saturating_add(1);
            }
            map.segments = map.segments.saturating_add(sub.segments);
            map.sections.push((line, col, sub));
        }
        return Ok(map);
    }
    let mut map = SourceMap {
        file: raw.file,
        source_root: raw.source_root,
        sources: raw.sources,
        sources_content: raw.sources_content,
        names: raw.names.into_iter().map(Option::unwrap_or_default).collect(),
        ignore_list: raw
            .ignore_list
            .or(raw.google_ignore_list)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|n| n.and_then(|n| u32::try_from(n).ok()))
            .collect(),
        ..Default::default()
    };
    let m = raw.mappings.as_deref().unwrap_or_default();
    if mappings {
        decode_mappings(m, &mut map, budget, line_budget, limits.max_lines)?;
    } else if let Some(count) = past_sources(m, map.sources.len()) {
        return Err(SourceMapError::BadMappings(count));
    }
    Ok(map)
}

/// The entry budget shared by every list in a map and its sections while the JSON is read.
struct Entries {
    left: Cell<usize>,
    exhausted: Cell<bool>,
}

impl Entries {
    fn take<E: de::Error>(&self) -> Result<(), E> {
        let Some(left) = self.left.get().checked_sub(1) else {
            self.exhausted.set(true);
            return Err(E::custom("too many source-map entries"));
        };
        self.left.set(left);
        Ok(())
    }

    /// Return the slots of entries that won't be kept after all. Callers drop those entries
    /// first, so a replaced list and its replacement are never held together.
    fn give_back(&self, n: usize) {
        self.left.set(self.left.get().saturating_add(n));
    }
}

/// The fields a map is built from, read without keeping anything else. Any JSON value is
/// accepted where the format expects another kind; it reads as absent, as before.
#[derive(Default)]
struct RawMap {
    version: Option<u64>,
    file: Option<String>,
    source_root: Option<String>,
    sources: Vec<Option<String>>,
    sources_content: Vec<Option<String>>,
    names: Vec<Option<String>>,
    /// Present even when it isn't an array, so `x_google_ignoreList` stays unused then.
    ignore_list: Option<Vec<Option<u64>>>,
    google_ignore_list: Option<Vec<Option<u64>>>,
    mappings: Option<String>,
    /// `Some(None)`: present but not an array.
    sections: Option<Option<Vec<RawSection>>>,
}

impl RawMap {
    /// The entry slots this map and its sections hold.
    fn held(&self) -> usize {
        let lists = |l: &Option<Vec<Option<u64>>>| l.as_ref().map_or(0, Vec::len);
        self.sources.len()
            + self.sources_content.len()
            + self.names.len()
            + lists(&self.ignore_list)
            + lists(&self.google_ignore_list)
            + held_by(&self.sections)
    }
}

fn held_by(sections: &Option<Option<Vec<RawSection>>>) -> usize {
    sections.iter().flatten().flatten().map(|s| 1 + s.map.iter().flatten().map(|m| m.held()).sum::<usize>()).sum()
}

#[derive(Default)]
struct RawSection {
    offset: Option<(Option<u64>, Option<u64>)>,
    /// `Some(None)`: present but not an object. Boxed, so a section that isn't a map stays small.
    map: Option<Option<Box<RawMap>>>,
    /// Whether the URL is a string.
    url: Option<bool>,
}

/// Visitor methods that read any other kind of value, without keeping it, as the default.
macro_rules! skip {
    ($de:lifetime; bool) => { fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> { Ok(Default::default()) } };
    ($de:lifetime; i64) => { fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> { Ok(Default::default()) } };
    ($de:lifetime; u64) => { fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> { Ok(Default::default()) } };
    ($de:lifetime; f64) => { fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> { Ok(Default::default()) } };
    ($de:lifetime; str) => { fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> { Ok(Default::default()) } };
    ($de:lifetime; unit) => { fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> { Ok(Default::default()) } };
    ($de:lifetime; seq) => {
        fn visit_seq<A: SeqAccess<$de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            while seq.next_element_seed(Skip)?.is_some() {}
            Ok(Default::default())
        }
    };
    ($de:lifetime; map) => {
        fn visit_map<A: MapAccess<$de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            while map.next_key_seed(Skip)?.is_some() {
                map.next_value_seed(Skip)?;
            }
            Ok(Default::default())
        }
    };
    ($de:lifetime; $($kind:ident),+) => { $(skip!($de; $kind);)+ };
}

/// Read a value the map doesn't keep, with the checks a full read makes (string escapes and
/// UTF-8, number range, nesting depth), so skipping it can't let a broken map through.
#[derive(Clone, Copy)]
struct Skip;

impl<'de> DeserializeSeed<'de> for Skip {
    type Value = ();
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Skip {
    type Value = ();
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }
    skip!('de; bool, i64, u64, f64, str, unit, seq, map);
}

/// A string, or `None` for any other value.
#[derive(Clone, Copy)]
struct MaybeStr;

impl<'de> Visitor<'de> for MaybeStr {
    type Value = Option<String>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
        Ok(Some(s.to_owned()))
    }
    fn visit_string<E: de::Error>(self, s: String) -> Result<Self::Value, E> {
        Ok(Some(s))
    }
    skip!('de; bool, i64, u64, f64, unit, seq, map);
}

/// A non-negative integer, or `None` for any other value.
#[derive(Clone, Copy)]
struct MaybeU64;

impl<'de> Visitor<'de> for MaybeU64 {
    type Value = Option<u64>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_u64<E: de::Error>(self, n: u64) -> Result<Self::Value, E> {
        Ok(Some(n))
    }
    skip!('de; bool, i64, f64, str, unit, seq, map);
}

/// One value read through `V`, after taking its slot from the entry budget.
struct Entry<'a, V>(&'a Entries, V);

impl<'de, V: Visitor<'de>> DeserializeSeed<'de> for Entry<'_, V> {
    type Value = V::Value;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        self.0.take()?;
        d.deserialize_any(self.1)
    }
}

/// A list whose entries count against the budget; any other value reads as an empty list.
struct EntryList<'a, V>(&'a Entries, V);

impl<'de, V: Visitor<'de> + Copy> Visitor<'de> for EntryList<'_, V> {
    type Value = Vec<V::Value>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut out = Vec::new();
        while let Some(value) = seq.next_element_seed(Entry(self.0, self.1))? {
            out.push(value);
        }
        Ok(out)
    }
    skip!('de; bool, i64, u64, f64, str, unit, map);
}

impl<'de, V: Visitor<'de> + Copy> DeserializeSeed<'de> for EntryList<'_, V> {
    type Value = Vec<V::Value>;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}

/// A map object, or `None` for any other value.
#[derive(Clone, Copy)]
struct RawMapSeed<'a>(&'a Entries);

impl<'de> Visitor<'de> for RawMapSeed<'_> {
    type Value = Option<RawMap>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut fields: A) -> Result<Self::Value, A::Error> {
        let mut raw = RawMap::default();
        // A repeated key replaces the earlier value, as it did when the whole JSON was read.
        while let Some(key) = fields.next_key::<Field>()? {
            match key {
                Field::Version => raw.version = fields.next_value_seed(Any(MaybeU64))?,
                Field::File => raw.file = fields.next_value_seed(Any(MaybeStr))?,
                Field::SourceRoot => raw.source_root = fields.next_value_seed(Any(MaybeStr))?,
                Field::Sources => {
                    self.0.give_back(std::mem::take(&mut raw.sources).len());
                    raw.sources = fields.next_value_seed(EntryList(self.0, MaybeStr))?;
                }
                Field::SourcesContent => {
                    self.0.give_back(std::mem::take(&mut raw.sources_content).len());
                    raw.sources_content = fields.next_value_seed(EntryList(self.0, MaybeStr))?;
                }
                Field::Names => {
                    self.0.give_back(std::mem::take(&mut raw.names).len());
                    raw.names = fields.next_value_seed(EntryList(self.0, MaybeStr))?;
                }
                // `ignoreList` wins over `x_google_ignoreList` wherever either appears.
                Field::IgnoreList => {
                    let unused = raw.ignore_list.take().into_iter().chain(raw.google_ignore_list.take());
                    self.0.give_back(unused.map(|l| l.len()).sum());
                    raw.ignore_list = Some(fields.next_value_seed(EntryList(self.0, MaybeU64))?);
                }
                Field::GoogleIgnoreList if raw.ignore_list.is_some() => fields.next_value_seed(Skip)?,
                Field::GoogleIgnoreList => {
                    self.0.give_back(raw.google_ignore_list.take().map_or(0, |l| l.len()));
                    raw.google_ignore_list = Some(fields.next_value_seed(EntryList(self.0, MaybeU64))?);
                }
                Field::Mappings => raw.mappings = fields.next_value_seed(Any(MaybeStr))?,
                Field::Sections => {
                    self.0.give_back(held_by(&raw.sections.take()));
                    raw.sections = Some(fields.next_value_seed(Any(Sections(self.0)))?);
                }
                Field::Other => fields.next_value_seed(Skip)?,
            }
        }
        Ok(Some(raw))
    }
    skip!('de; bool, i64, u64, f64, str, unit, seq);
}

impl<'de> DeserializeSeed<'de> for RawMapSeed<'_> {
    type Value = Option<RawMap>;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}

/// Read any JSON value through `V`.
struct Any<V>(V);

impl<'de, V: Visitor<'de>> DeserializeSeed<'de> for Any<V> {
    type Value = V::Value;
    fn deserialize<D: de::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self.0)
    }
}

#[derive(serde::Deserialize)]
#[serde(field_identifier, rename_all = "camelCase")]
enum Field {
    Version,
    File,
    SourceRoot,
    Sources,
    SourcesContent,
    Names,
    IgnoreList,
    #[serde(rename = "x_google_ignoreList")]
    GoogleIgnoreList,
    Mappings,
    Sections,
    #[serde(other)]
    Other,
}

/// The sections array, or `None` for any other value.
struct Sections<'a>(&'a Entries);

impl<'de> Visitor<'de> for Sections<'_> {
    type Value = Option<Vec<RawSection>>;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut out = Vec::new();
        while let Some(section) = seq.next_element_seed(Entry(self.0, Section(self.0)))? {
            out.push(section);
        }
        Ok(Some(out))
    }
    skip!('de; bool, i64, u64, f64, str, unit, map);
}

/// One section; any value that isn't an object reads as a section without an offset.
struct Section<'a>(&'a Entries);

impl<'de> Visitor<'de> for Section<'_> {
    type Value = RawSection;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut fields: A) -> Result<Self::Value, A::Error> {
        let mut section = RawSection::default();
        while let Some(key) = fields.next_key::<SectionField>()? {
            match key {
                SectionField::Offset => section.offset = Some(fields.next_value_seed(Any(Offset))?),
                SectionField::Map => {
                    self.0.give_back(section.map.take().flatten().map_or(0, |m| m.held()));
                    section.map = Some(fields.next_value_seed(RawMapSeed(self.0))?.map(Box::new));
                }
                SectionField::Url => section.url = Some(fields.next_value_seed(Any(MaybeStr))?.is_some()),
                SectionField::Other => fields.next_value_seed(Skip)?,
            }
        }
        Ok(section)
    }
    skip!('de; bool, i64, u64, f64, str, unit, seq);
}

#[derive(serde::Deserialize)]
#[serde(field_identifier, rename_all = "lowercase")]
enum SectionField {
    Offset,
    Map,
    Url,
    #[serde(other)]
    Other,
}

/// A section offset's line and column; any value that isn't an object has neither.
struct Offset;

impl<'de> Visitor<'de> for Offset {
    type Value = (Option<u64>, Option<u64>);
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any JSON value")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut fields: A) -> Result<Self::Value, A::Error> {
        let (mut line, mut column) = (None, None);
        while let Some(key) = fields.next_key::<OffsetField>()? {
            match key {
                OffsetField::Line => line = fields.next_value_seed(Any(MaybeU64))?,
                OffsetField::Column => column = fields.next_value_seed(Any(MaybeU64))?,
                OffsetField::Other => fields.next_value_seed(Skip)?,
            }
        }
        Ok((line, column))
    }
    skip!('de; bool, i64, u64, f64, str, unit, seq);
}

#[derive(serde::Deserialize)]
#[serde(field_identifier, rename_all = "lowercase")]
enum OffsetField {
    Line,
    Column,
    #[serde(other)]
    Other,
}

fn b64(c: u8) -> Option<i64> {
    Some(match c {
        b'A'..=b'Z' => i64::from(c - b'A'),
        b'a'..=b'z' => i64::from(c - b'a') + 26,
        b'0'..=b'9' => i64::from(c - b'0') + 52,
        b'+' => 62,
        b'/' => 63,
        _ => return None,
    })
}

/// Decode one VLQ field from `bytes` at `*pos`.
fn vlq(bytes: &[u8], pos: &mut usize) -> Option<i64> {
    let mut result = 0u32;
    let mut shift = 0u32;
    loop {
        let d = b64(*bytes.get(*pos)?)? as u32;
        *pos += 1;
        let bits = d & 31;
        if bits != 0 {
            if bits > u32::MAX.checked_shr(shift)? {
                return None;
            }
            result |= bits << shift;
        }
        if d & 32 == 0 {
            break;
        }
        // Redundant zero groups do not change the value.
        shift = shift.saturating_add(5);
    }
    let magnitude = i64::from(result >> 1);
    Some(match (result & 1, magnitude) {
        (1, 0) => i64::from(i32::MIN),
        (1, n) => -n,
        _ => magnitude,
    })
}

/// Decode mapping deltas, rejecting fields that cannot name a declared source or symbol.
fn decode_mappings(m: &str, map: &mut SourceMap, budget: &mut usize, line_budget: &mut usize, line_limit: usize) -> Result<(), SourceMapError> {
    let bytes = m.as_bytes();
    let (mut src, mut ol, mut oc, mut name) = (0i64, 0i64, 0i64, 0i64);
    let mut pos = 0usize;
    let mut line: Vec<Segment> = Vec::new();
    let mut col = 0i64;
    let mut count = 0usize;
    let to_u32 = |v: i64, count: usize| u32::try_from(v).map_err(|_| SourceMapError::BadMappings(count));
    while pos <= bytes.len() {
        match bytes.get(pos) {
            None | Some(b';') => {
                if *line_budget == 0 {
                    return Err(SourceMapError::TooManyLines(line_limit));
                }
                *line_budget -= 1;
                line.sort_by_key(|s| s.generated_column);
                map.lines.try_reserve(1).map_err(|_| SourceMapError::LineAllocationFailed)?;
                map.lines.push(std::mem::take(&mut line));
                col = 0;
                pos += 1;
                continue;
            }
            Some(b',') => {
                pos += 1;
                continue;
            }
            _ => {}
        }
        if *budget == 0 {
            return Err(SourceMapError::TooManySegments(count));
        }
        *budget -= 1;
        count += 1;
        col += vlq(bytes, &mut pos).ok_or(SourceMapError::BadMappings(count))?;
        let mut seg = Segment { generated_column: to_u32(col, count)?, original: None };
        if !matches!(bytes.get(pos), None | Some(b',') | Some(b';')) {
            src += vlq(bytes, &mut pos).ok_or(SourceMapError::BadMappings(count))?;
            ol += vlq(bytes, &mut pos).ok_or(SourceMapError::BadMappings(count))?;
            oc += vlq(bytes, &mut pos).ok_or(SourceMapError::BadMappings(count))?;
            let mut n = None;
            if !matches!(bytes.get(pos), None | Some(b',') | Some(b';')) {
                name += vlq(bytes, &mut pos).ok_or(SourceMapError::BadMappings(count))?;
                n = Some(to_u32(name, count)?);
            }
            let source = to_u32(src, count)?;
            if source as usize >= map.sources.len() || n.is_some_and(|index| index as usize >= map.names.len()) {
                return Err(SourceMapError::BadMappings(count));
            }
            seg.original = Some((source, to_u32(ol, count)?, to_u32(oc, count)?, n));
        }
        if !matches!(bytes.get(pos), None | Some(b',') | Some(b';')) {
            return Err(SourceMapError::BadMappings(count));
        }
        line.push(seg);
    }
    map.segments = count;
    Ok(())
}

/// The segment, counted as [`decode_mappings`] does, that names a source past the end of a list
/// of `sources`. Mappings that can't be decoded end the search: past them nothing is known.
fn past_sources(m: &str, sources: usize) -> Option<usize> {
    let bytes = m.as_bytes();
    let (mut pos, mut src, mut count) = (0usize, 0i64, 0usize);
    let at_end = |pos: usize| matches!(bytes.get(pos), None | Some(b',') | Some(b';'));
    while pos < bytes.len() {
        if matches!(bytes[pos], b',' | b';') {
            pos += 1;
            continue;
        }
        count += 1;
        vlq(bytes, &mut pos)?;
        if at_end(pos) {
            continue;
        }
        src += vlq(bytes, &mut pos)?;
        // A negative index is wrong too, but the deltas after it still decode.
        if usize::try_from(src).is_ok_and(|src| src >= sources) {
            return Some(count);
        }
        while !at_end(pos) {
            vlq(bytes, &mut pos)?;
        }
    }
    None
}

#[cfg(test)]
mod tests;

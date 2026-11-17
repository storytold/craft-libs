//! Source maps (revision 3), including indexed maps with sections.
//!
//! Lookups follow the "original positions for a generated position" rule: on the generated line,
//! take the greatest segment column not after the requested column and return every segment that
//! starts there. Lines are 0-based here (callers add one for display); columns are 0-based UTF-16
//! code units as the format defines.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Biggest map JSON accepted.
    pub max_bytes: usize,
    /// Most decoded segments across the whole map.
    pub max_segments: usize,
    /// Deepest section nesting (the format allows one level; we tolerate a few).
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_bytes: 64 * 1024 * 1024, max_segments: 4_000_000, max_depth: 8 }
    }
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
    #[error("source map has more than {0} segments")]
    TooManySegments(usize),
    #[error("source map sections nest deeper than {0}")]
    TooDeep(usize),
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
    /// The source path with `sourceRoot` applied.
    pub fn source_path(&self, index: u32) -> Option<String> {
        let s = self.sources.get(index as usize)?.as_ref()?;
        Some(match self.source_root.as_deref().filter(|r| !r.is_empty()) {
            Some(root) if !s.contains("://") && !s.starts_with('/') => format!("{}/{}", root.trim_end_matches('/'), s),
            _ => s.clone(),
        })
    }

    pub fn source_content(&self, index: u32) -> Option<&str> {
        self.sources_content.get(index as usize)?.as_deref()
    }

    /// Every original position for a generated (line, column), both 0-based.
    pub fn original_positions(&self, line: u32, column: u32) -> Vec<OriginalPosition> {
        if !self.sections.is_empty() {
            // Last section that starts at or before the position.
            let found = self.sections.iter().rev().find(|(l, c, _)| (*l, *c) <= (line, column));
            let Some((l, c, map)) = found else { return Vec::new() };
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
        segs.iter()
            .filter(|s| s.generated_column == col)
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

    /// All sources, flattened across sections.
    pub fn all_sources(&self) -> Vec<String> {
        let mut out: Vec<String> = (0..self.sources.len() as u32).filter_map(|i| self.source_path(i)).collect();
        for (_, _, m) in &self.sections {
            out.extend(m.all_sources());
        }
        out
    }
}

/// Parse a source map.
pub fn parse(json: &[u8], limits: Limits) -> Result<SourceMap, SourceMapError> {
    if json.len() > limits.max_bytes {
        return Err(SourceMapError::TooLarge(json.len(), limits.max_bytes));
    }
    // Some servers prefix maps with `)]}'` to defeat XSSI; the format allows skipping that line.
    let body = if json.starts_with(b")]}") { json.iter().position(|b| *b == b'\n').and_then(|p| json.get(p + 1..)).unwrap_or(&[]) } else { json };
    let v: Value = serde_json::from_slice(body).map_err(|e| SourceMapError::BadJson(e.to_string()))?;
    let mut budget = limits.max_segments;
    from_value(&v, 0, limits, &mut budget)
}

fn from_value(v: &Value, depth: usize, limits: Limits, budget: &mut usize) -> Result<SourceMap, SourceMapError> {
    if depth > limits.max_depth {
        return Err(SourceMapError::TooDeep(limits.max_depth));
    }
    if v.get("version").and_then(Value::as_u64) != Some(3) {
        return Err(SourceMapError::BadVersion);
    }
    let strs = |k: &str| -> Vec<Option<String>> {
        v.get(k).and_then(Value::as_array).map(|a| a.iter().map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default()
    };
    let mut map = SourceMap {
        file: v.get("file").and_then(Value::as_str).map(String::from),
        source_root: v.get("sourceRoot").and_then(Value::as_str).map(String::from),
        sources: strs("sources"),
        sources_content: strs("sourcesContent"),
        names: strs("names").into_iter().map(Option::unwrap_or_default).collect(),
        ignore_list: v
            .get("ignoreList")
            .or_else(|| v.get("x_google_ignoreList"))
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|n| n.as_u64().and_then(|n| u32::try_from(n).ok())).collect())
            .unwrap_or_default(),
        ..Default::default()
    };
    if let Some(sections) = v.get("sections").and_then(Value::as_array) {
        for s in sections {
            let line = s.pointer("/offset/line").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
            let col = s.pointer("/offset/column").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
            // Only inline maps; a `url` section would need a fetch, which a parser never does.
            if let Some(m) = s.get("map") {
                let sub = from_value(m, depth + 1, limits, budget)?;
                map.segments = map.segments.saturating_add(sub.segments);
                map.sections.push((line, col, sub));
            }
        }
        map.sections.sort_by_key(|(l, c, _)| (*l, *c));
        return Ok(map);
    }
    let mappings = v.get("mappings").and_then(Value::as_str).unwrap_or_default();
    decode_mappings(mappings, &mut map, budget)?;
    Ok(map)
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
    let mut result: i64 = 0;
    let mut shift = 0u32;
    loop {
        let d = b64(*bytes.get(*pos)?)?;
        *pos += 1;
        if shift > 35 {
            return None;
        }
        result |= (d & 31) << shift;
        if d & 32 == 0 {
            break;
        }
        shift += 5;
    }
    let neg = result & 1 == 1;
    let v = result >> 1;
    Some(if neg { -v } else { v })
}

fn decode_mappings(m: &str, map: &mut SourceMap, budget: &mut usize) -> Result<(), SourceMapError> {
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
                line.sort_by_key(|s| s.generated_column);
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
            seg.original = Some((to_u32(src, count)?, to_u32(ol, count)?, to_u32(oc, count)?, n));
        }
        if !matches!(bytes.get(pos), None | Some(b',') | Some(b';')) {
            return Err(SourceMapError::BadMappings(count));
        }
        line.push(seg);
    }
    map.segments = count;
    Ok(())
}


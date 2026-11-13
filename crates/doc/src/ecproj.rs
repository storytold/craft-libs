//! EffectCraft `.ecproj` projects: one UTF-8 JSON object, uncompressed:
//!
//! ```text
//! { "savedBy": "0.12.0", "schema": 2, "settings": {...},
//!   "items": { "1": { "id": 1, "name": "Comp 1", "kind": { "type": "Comp", "width": 1920, ... } } },
//!   ... }
//! ```
//!
//! Projects saved before `savedBy` existed start with `schema`. A project holds no rendered
//! frames: footage is referenced by path. `savedBy` and `schema` are read with
//! [`craft_ecproj_format::Head`].

use std::collections::BTreeMap;

use craft_ecproj_format::{Head, ReadError};
use serde::Deserialize;

use crate::{Budget, Error, Kind, Layer, Outline, Page, Rendition, Timing, check_depth, counted_map, counted_option, malformed, parse, too_new};

/// EffectCraft's time unit.
const TICKS_PER_SECOND: f64 = 254_016_000_000.0;

/// Calls `stop` with each member name of the JSON object `text` starts with, in order, as far as
/// `text` goes, until it returns `true`. Members of nested objects are skipped. A name is given as
/// written, escapes included.
fn top_level_keys<'a>(text: &'a [u8], control: Option<&crate::cancel::Control<'_>>, mut stop: impl FnMut(&'a [u8]) -> bool) -> Result<(), Error> {
    let mut scan = crate::cancel::Scan::new(control);
    let mut start = 0;
    while text.get(start).is_some_and(u8::is_ascii_whitespace) {
        scan.check(start)?;
        start += 1;
    }
    let Some(body) = text.get(start..).and_then(|s| s.strip_prefix(b"{")) else { return Ok(()) };
    // Start a new offset domain after leading whitespace.
    let mut scan = crate::cancel::Scan::new(control);
    let (mut depth, mut want_key, mut i) = (0u32, true, 0);
    while let Some(&b) = body.get(i) {
        scan.check(i)?;
        match b {
            b'"' => {
                let start = i + 1;
                let mut end = start;
                loop {
                    scan.check(end)?;
                    match body.get(end) {
                        None => return Ok(()),
                        Some(b'\\') => end += 2,
                        Some(b'"') => break,
                        Some(_) => end += 1,
                    }
                }
                if depth == 0
                    && want_key
                    && let Some(key) = body.get(start..end)
                    && stop(key)
                {
                    return Ok(());
                }
                want_key = false;
                i = end;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' if depth == 0 => return Ok(()),
            b'}' | b']' => depth -= 1,
            b',' if depth == 0 => want_key = true,
            _ => {}
        }
        i += 1;
    }
    Ok(())
}

/// A project starts with `savedBy` followed by `schema`, or, saved before `savedBy` existed, with
/// `schema` followed by `items` or `savedBy`, all top-level members.
pub(crate) fn sniff(head: &[u8]) -> bool {
    sniff_checked(head, None).unwrap_or(false)
}

pub(crate) fn sniff_checked(head: &[u8], control: Option<&crate::cancel::Control<'_>>) -> Result<bool, Error> {
    let (mut first, mut found) = (None::<&[u8]>, false);
    top_level_keys(head, control, |key| match first {
        None => {
            first = Some(key);
            !matches!(key, b"savedBy" | b"schema")
        }
        Some(b"savedBy") => {
            found = key == b"schema";
            found
        }
        Some(_) => {
            found = key == b"items" || key == b"savedBy";
            found
        }
    })?;
    Ok(found)
}

fn json<'a>(bytes: &'a [u8], budget: &Budget) -> Result<&'a [u8], Error> {
    budget.check_json(bytes.len() as u64, "the project")?;
    Ok(bytes)
}

/// The project's [`Head`] read with [`Head::read`]; [`ReadError::TooNew`] when its schema is
/// newer than this crate reads.
fn read_head(json: &[u8], budget: &Budget) -> Result<Result<Head, ReadError>, Error> {
    check_depth(json, budget.limits.max_depth, budget.control)?;
    let text = std::str::from_utf8(json).map_err(|e| malformed(format!("the project is not UTF-8: {e}")))?;
    let head = match budget.control {
        Some(control) => Head::read_from(crate::cancel::Reader::new(json, Some(control))),
        None => Head::read(text),
    };
    budget.check_cancelled()?;
    Ok(head)
}

/// The project's [`Head`], refused when its schema is newer than this crate reads.
fn head(json: &[u8], budget: &Budget) -> Result<Head, Error> {
    read_head(json, budget)?.map_err(|e| match e {
        ReadError::TooNew { too_new: e, .. } => too_new(Kind::EffectCraft, e.schema),
        e => malformed(e),
    })
}

pub(crate) fn info(bytes: &[u8], budget: &mut Budget) -> Result<(u32, Option<String>), Error> {
    match read_head(json(bytes, budget)?, budget)? {
        Ok(head) => Ok((head.schema, head.saved_by)),
        Err(ReadError::TooNew { too_new, saved_by, .. }) => Ok((too_new.schema, saved_by)),
        Err(e) => Err(malformed(e)),
    }
}

pub(crate) fn renditions(bytes: &[u8], budget: &mut Budget) -> Result<Vec<Rendition>, Error> {
    head(json(bytes, budget)?, budget)?;
    Ok(vec![])
}

#[derive(Deserialize)]
struct Project {
    #[serde(default, deserialize_with = "counted_map")]
    items: BTreeMap<String, ItemM>,
}

#[derive(Deserialize)]
struct ItemM {
    id: u64,
    #[serde(default)]
    name: String,
    kind: ItemKindM,
}

/// The members of an item's kind that a composition has. Footage and solids share some of them
/// with the same types; folders have none.
#[derive(Deserialize)]
struct ItemKindM {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    frame_rate: Option<FrameRateM>,
    /// In ticks.
    #[serde(default)]
    duration: Option<i64>,
    /// A composition's layers, the top one first.
    #[serde(default, deserialize_with = "counted_option")]
    layers: Option<Vec<LayerM>>,
}

#[derive(Deserialize)]
struct FrameRateM {
    num: i64,
    den: i64,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize)]
struct LayerM {
    #[serde(default)]
    name: String,
    #[serde(default)]
    source: Option<SourceM>,
    #[serde(default)]
    switches: Option<SwitchesM>,
    #[serde(default)]
    blend_mode: Option<String>,
}

#[derive(Deserialize)]
struct SourceM {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct SwitchesM {
    #[serde(default = "yes")]
    video: bool,
}

pub(crate) fn outline(bytes: &[u8], budget: &mut Budget) -> Result<Outline, Error> {
    let json = json(bytes, budget)?;
    head(json, budget)?;
    let p: Project = parse(json, budget)?;
    project_outline(p, budget)
}

fn project_outline(p: Project, budget: &Budget) -> Result<Outline, Error> {
    let mut comps = Vec::new();
    for i in p.items.into_values() {
        budget.check_cancelled()?;
        if i.kind.kind == "Comp" {
            comps.push(i);
        }
    }
    comps.sort_by_key(|i| i.id);
    let pages = comps
        .into_iter()
        .map(|i| {
            budget.check_cancelled()?;
            let k = i.kind;
            let (Some(width), Some(height)) = (k.width, k.height) else {
                return Err(malformed(format!("composition `{}` has no size", i.name)));
            };
            let timing = k.frame_rate.zip(k.duration).map(|(r, duration)| Timing {
                frame_rate_num: r.num,
                frame_rate_den: r.den,
                duration_seconds: duration as f64 / TICKS_PER_SECOND,
            });
            let layers = k
                .layers
                .unwrap_or_default()
                .into_iter()
                .rev()
                .map(|l| {
                    budget.check_cancelled()?;
                    Ok(Layer {
                        name: l.name,
                        kind: l.source.map(|s| s.kind).unwrap_or_default(),
                        visible: l.switches.is_none_or(|s| s.video),
                        opacity: None,
                        blend: l.blend_mode,
                        children: vec![],
                    })
                })
                .collect::<Result<_, Error>>()?;
            Ok(Page { name: i.name, x: 0.0, y: 0.0, width: f64::from(width), height: f64::from(height), timing, layers })
        })
        .collect::<Result<_, Error>>()?;
    Ok(Outline { canvas: None, pages, layers: vec![] })
}

#[cfg(test)]
mod cancellation_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::cell::Cell;

    #[test]
    fn conversion_stops_inside_composition_layers() {
        let project: Project =
            serde_json::from_str(r#"{"items":{"1":{"id":1,"kind":{"type":"Comp","width":1,"height":1,"layers":[{},{},{},{},{},{}]}}}}"#).unwrap();
        let polls = Cell::new(0);
        let cancel = || {
            polls.set(polls.get() + 1);
            polls.get() == 6
        };
        let result = crate::cancel::run(&cancel, |control| project_outline(project, &Budget::controlled(&crate::Limits::default(), control)));
        assert_eq!(result, Err(Error::Cancelled));
        assert_eq!(polls.get(), 6);
    }
}

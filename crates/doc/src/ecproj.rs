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

use crate::{Budget, Error, Kind, Layer, Outline, Page, Rendition, Timing, check_depth, malformed, parse, too_new};

/// EffectCraft's time unit.
const TICKS_PER_SECOND: f64 = 254_016_000_000.0;

/// Calls `stop` with each member name of the JSON object `text` starts with, in order, as far as
/// `text` goes, until it returns `true`. Members of nested objects are skipped. A name is given as
/// written, escapes included.
fn top_level_keys<'a>(text: &'a [u8], mut stop: impl FnMut(&'a [u8]) -> bool) {
    let Some(body) = text.trim_ascii_start().strip_prefix(b"{") else { return };
    let (mut depth, mut want_key, mut i) = (0u32, true, 0);
    while let Some(&b) = body.get(i) {
        match b {
            b'"' => {
                let start = i + 1;
                let mut end = start;
                loop {
                    match body.get(end) {
                        None => return,
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
                    return;
                }
                want_key = false;
                i = end;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' if depth == 0 => return,
            b'}' | b']' => depth -= 1,
            b',' if depth == 0 => want_key = true,
            _ => {}
        }
        i += 1;
    }
}

/// A project starts with `savedBy` followed by `schema`, or, saved before `savedBy` existed, with
/// `schema` followed by `items` or `savedBy`, all top-level members.
pub(crate) fn sniff(head: &[u8]) -> bool {
    let (mut first, mut found) = (None::<&[u8]>, false);
    top_level_keys(head, |key| match first {
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
    });
    found
}

fn json<'a>(bytes: &'a [u8], budget: &Budget) -> Result<&'a [u8], Error> {
    budget.check_json(bytes.len() as u64, "the project")?;
    Ok(bytes)
}

/// The project's [`Head`] read with [`Head::read`]; [`ReadError::TooNew`] when its schema is
/// newer than this crate reads.
fn read_head(json: &[u8], budget: &Budget) -> Result<Result<Head, ReadError>, Error> {
    check_depth(json, budget.limits.max_depth)?;
    let text = std::str::from_utf8(json).map_err(|e| malformed(format!("the project is not UTF-8: {e}")))?;
    Ok(Head::read(text))
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
    #[serde(default)]
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
    #[serde(default)]
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
    let p: Project = parse(json, budget.limits)?;
    let mut comps: Vec<ItemM> = p.items.into_values().filter(|i| i.kind.kind == "Comp").collect();
    comps.sort_by_key(|i| i.id);
    let pages = comps
        .into_iter()
        .map(|i| {
            let k = i.kind;
            let (Some(width), Some(height)) = (k.width, k.height) else {
                return Err(malformed(format!("composition `{}` has no size", i.name)));
            };
            let timing = k.frame_rate.map(|r| Timing {
                frame_rate_num: r.num,
                frame_rate_den: r.den,
                duration_seconds: k.duration.unwrap_or(0) as f64 / TICKS_PER_SECOND,
            });
            let layers = k
                .layers
                .unwrap_or_default()
                .into_iter()
                .rev()
                .map(|l| Layer {
                    name: l.name,
                    kind: l.source.map(|s| s.kind).unwrap_or_default(),
                    visible: l.switches.is_none_or(|s| s.video),
                    opacity: None,
                    blend: l.blend_mode,
                    children: vec![],
                })
                .collect();
            Ok(Page { name: i.name, x: 0.0, y: 0.0, width: f64::from(width), height: f64::from(height), timing, layers })
        })
        .collect::<Result<_, Error>>()?;
    Ok(Outline { canvas: None, pages, layers: vec![] })
}

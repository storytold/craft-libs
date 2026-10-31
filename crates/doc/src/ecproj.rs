//! EffectCraft `.ecproj` projects: one UTF-8 JSON object, uncompressed:
//!
//! ```text
//! { "savedBy": "0.12.0", "schema": 2, "settings": {...},
//!   "items": { "1": { "id": 1, "name": "Comp 1", "kind": { "type": "Comp", "width": 1920, ... } } },
//!   ... }
//! ```
//!
//! Projects saved before `savedBy` existed start with `schema`. A project holds no rendered
//! frames: footage is referenced by path.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{Budget, Error, Kind, Layer, Outline, Page, Rendition, Timing, check_version, malformed, parse, parse_versioned};

/// The newest `schema` read.
pub(crate) const SCHEMA_VERSION: u32 = 2;
/// EffectCraft's time unit.
const TICKS_PER_SECOND: f64 = 254_016_000_000.0;

/// The first member name of the JSON object `text` starts with.
fn first_key(text: &[u8]) -> Option<&[u8]> {
    let s = text.trim_ascii_start().strip_prefix(b"{")?.trim_ascii_start().strip_prefix(b"\"")?;
    s.get(..s.iter().position(|&b| b == b'"')?)
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

pub(crate) fn sniff(head: &[u8]) -> bool {
    let has = |k: &[u8]| contains(head, k);
    match first_key(head) {
        Some(b"savedBy") => has(b"\"schema\""),
        Some(b"schema") => has(b"\"items\"") || has(b"\"savedBy\""),
        _ => false,
    }
}

#[derive(Deserialize)]
struct Head {
    #[serde(rename = "savedBy", default)]
    saved_by: Option<String>,
    schema: u32,
}

fn json<'a>(bytes: &'a [u8], budget: &Budget) -> Result<&'a [u8], Error> {
    budget.check_json(bytes.len() as u64, "the project")?;
    Ok(bytes)
}

pub(crate) fn info(bytes: &[u8], budget: &mut Budget) -> Result<(u32, Option<String>), Error> {
    let head: Head = parse(json(bytes, budget)?, budget.limits)?;
    Ok((head.schema, head.saved_by))
}

pub(crate) fn renditions(bytes: &[u8], budget: &mut Budget) -> Result<Vec<Rendition>, Error> {
    let head: Head = parse_versioned(json(bytes, budget)?, budget.limits, Kind::EffectCraft)?;
    check_version(Kind::EffectCraft, head.schema)?;
    Ok(vec![])
}

#[derive(Deserialize)]
struct Project {
    schema: u32,
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
    let p: Project = parse_versioned(json(bytes, budget)?, budget.limits, Kind::EffectCraft)?;
    check_version(Kind::EffectCraft, p.schema)?;
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

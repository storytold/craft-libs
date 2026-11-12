//! A decoded `.fig` message mapped onto the REST API file JSON ([`craft_figma_rest`]), the shape
//! an application's importer takes.
//!
//! The message lists node changes; each names its id (`guid`), its parent and its position among
//! the siblings (`parentIndex`). Nodes are read by field name, so files written with newer
//! schemas still map what is understood. Instances carry only their overrides: their layers are
//! expanded here from the main component (overrides and derived layout matched by id path,
//! component property values applied) so the result looks as saved. Layer kinds the REST shape
//! doesn't have (sticky notes, connectors, tables, …) come out as the closest frames, text and
//! vectors. Anything not understood is skipped with a warning; malformed input never panics.
//!
//! Besides the public REST fields, the result carries the extensions [`craft_figma_rest::ext`]
//! documents (`vectorNetwork`, `vectorNetworkRef`, `pointCount`, `innerRadius`,
//! `_letterSpacingPercent`) and private `_fig*` handoffs for an importer that wants the file's
//! provenance ([`FIG_SOURCE_FIELD`], [`FIG_LIBRARY_CONTEXTS_FIELD`], [`FIG_CAPPED_MAINS_FIELD`],
//! `_figRegionFills`, `_figLineStarts`, `_figTextPath`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use craft_figma_rest::ext::VectorNetwork;
use kurbo::{Affine, Point, Rect};
use serde_json::{Map, Value, json};

use crate::kiwi::Kv;
use crate::network;

mod component;
mod connector;
mod document;
mod geometry;
mod layer;
mod layout;
mod paint;
mod proto;
mod swap;
mod text;
mod text_path;
mod variables;

pub use geometry::{Geometry, MAX_RUNS, RouteEnd, Routing, SavedRuns, Side, Straight};

use component::OverrideScope;
#[cfg(test)]
use paint::paint;

type Obj = Map<String, Value>;

/// Most layers mapped from a file when the caller has no budget of its own
/// ([`layer_budget`] gives one that follows the file's size).
pub const MAX_NODES: usize = 200_000;
/// The provenance of a layer mapped from the file (its guid, whether it was expanded from a
/// main, …), on each layer.
pub const FIG_SOURCE_FIELD: &str = "_figSource";
/// Component-bearing branches of the file's internal canvases (cached library definitions), on
/// the file.
pub const FIG_LIBRARY_CONTEXTS_FIELD: &str = "_figLibraryContexts";
/// Mains in the file that the layer cap kept out, on the file: their instances are told apart
/// from those of components the file doesn't have.
pub const FIG_CAPPED_MAINS_FIELD: &str = "_figCappedMains";

/// Deepest nesting followed.
const MAX_DEPTH: usize = 256;
/// Deepest nesting of conditional prototype actions followed.
const MAX_ACTION_DEPTH: usize = 16;
/// Deepest nesting of variable expressions followed.
const MAX_EXPR_DEPTH: usize = 32;
const MAX_GRID_TRACKS: usize = 1_000;
const MAX_GRID_TRACK_VALUE: f64 = 1e6;
/// Smallest and largest scale factor of a scaled instance (`symbolData.uniformScaleFactor`, times
/// those of the instances around it) that is followed: a junk factor changes nothing.
const MIN_SCALE_FACTOR: f64 = 1e-3;
const MAX_SCALE_FACTOR: f64 = 1e3;
/// Id of "no style".
const NO_STYLE: &str = "4294967295:4294967295";
/// Longest asset `key` or `version` kept for resolving references given as assets.
const MAX_ASSET_PART: usize = 256;
/// Compressed canvas bytes per layer of [`layer_budget`].
const PACKED_BYTES_PER_LAYER: usize = 8;
/// Most layers [`layer_budget`] gives.
pub const MAX_FIG_NODES: usize = 2_000_000;

/// Most layers to map from a file whose compressed canvas is `packed` bytes: one per 8 bytes, at
/// least [`MAX_NODES`] and at most [`MAX_FIG_NODES`]. Instances expand into a layer per layer of
/// their component, and chunks inflate up to [`crate::MAX_CHUNK`], so the budget follows what the
/// file takes, not what it unpacks to: a small file can't claim a large count, however it's
/// built; a large one gets room. Design systems are mostly instances, which pack tightly: a 7.8 MB
/// canvas stopped at 244 248 layers at 32 bytes a layer. At 8, files under about 1.6 MB keep the
/// floor and the ceiling comes at a 16 MB canvas.
pub fn layer_budget(packed: usize) -> usize {
    (packed / PACKED_BYTES_PER_LAYER).clamp(MAX_NODES, MAX_FIG_NODES)
}

/// How to map a message.
#[derive(Clone, Copy)]
pub struct Options<'g> {
    /// Most layers produced or scanned, instance expansion included ([`layer_budget`]).
    pub max_nodes: usize,
    /// Connector routing and text path checks ([`Straight`] when the caller has none).
    pub geometry: &'g dyn Geometry,
    /// The plugin data (plugin id, key) under which a writer kept what a layer written as the
    /// outline of its stroke was: read back as `outlinedStroke` (an extension of the file JSON).
    pub outlined_stroke: Option<(&'g str, &'g str)>,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Options { max_nodes: MAX_NODES, geometry: &Straight, outlined_stroke: None }
    }
}

/// What [`to_rest`] makes of a message.
#[derive(Debug)]
pub struct Mapped {
    /// The file JSON, in the shape [`craft_figma_rest::File`] reads.
    pub json: Value,
    /// What was skipped or approximated, each once (`(×n)` after one met n times).
    pub warnings: Vec<String>,
    /// The vector networks layers name by `vectorNetworkRef` (each built once, however many
    /// instances show it). `None` where one couldn't be built.
    pub networks: Vec<Option<VectorNetwork>>,
}

/// The REST API file JSON for message `msg` (decoded with its file's schema), named `name`.
pub fn to_rest<'a>(msg: &'a Kv<'a>, name: &str, options: &Options<'a>) -> Mapped {
    let mut m = Mapper::new(msg, options);
    let _canon = VarCanon::install(&m);
    let doc = m.document();
    // Large values are moved in, never passed through `json!`, which serialises its arguments:
    // a deep copy of the whole tree, briefly holding it twice.
    let mut out = json!({"name": name});
    out["document"] = doc;
    if !m.library_contexts.is_empty() {
        out[FIG_LIBRARY_CONTEXTS_FIELD] = Value::Array(std::mem::take(&mut m.library_contexts));
    }
    // Mains in the file that the layer cap kept out: their instances are told apart from those of
    // components the file doesn't have.
    if m.count >= m.max_nodes {
        let capped: Vec<Value> = m.referenced_mains.difference(&m.mapped_mains).map(|id| json!(id)).collect();
        if !capped.is_empty() {
            out[FIG_CAPPED_MAINS_FIELD] = Value::Array(capped);
        }
    }
    if !m.styles.is_empty() {
        out["styles"] = Value::Object(std::mem::take(&mut m.styles));
    }
    if !m.components.is_empty() {
        out["components"] = Value::Object(std::mem::take(&mut m.components));
    }
    if !m.sets.is_empty() {
        out["componentSets"] = Value::Object(std::mem::take(&mut m.sets));
    }
    if let Some(vars) = m.variables() {
        out["meta"] = vars;
    }
    let styles = m.style_definitions();
    if !styles.is_empty() {
        out["styleDefinitions"] = Value::Array(styles);
    }
    let networks = std::mem::take(&mut m.networks);
    Mapped { json: out, warnings: std::mem::take(&mut m.warnings).finish(), networks }
}

/// The axis-aligned bounds of `rect` after `xf`.
fn bbox(xf: Affine, rect: Rect) -> Rect {
    let corners = [Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y0), Point::new(rect.x1, rect.y1), Point::new(rect.x0, rect.y1)];
    let mut it = corners.into_iter().map(|p| xf * p);
    let Some(first) = it.next() else { return Rect::ZERO };
    it.fold(Rect::from_points(first, first), |r, p| r.union_pt(p))
}

/// `s` cut at 200 characters, for quoting names from the file in warnings.
fn clip_str(s: &str) -> &str {
    s.char_indices().nth(200).and_then(|(i, _)| s.get(..i)).unwrap_or(s)
}

#[cfg(test)]
mod image_filter_tests;
#[cfg(test)]
mod swap_tests;
#[cfg(test)]
mod tests;

// ---- small readers ----

/// A generated child is identified by one exact template GUID, not its array position.
fn generated_record<'a>(v: &View<'a>, field: &str, local_id: u32) -> Option<&'a Kv<'a>> {
    v.get(field)?.get("overrides")?.list().iter().find(|row| {
        row.get("guidPath").and_then(|path| path.get("guids")).map(Kv::list).is_some_and(|guids| {
            guids.len() == 1 && num(&guids[0], "sessionID") == Some(40_000_000.0) && num(&guids[0], "localID") == Some(f64::from(local_id))
        })
    })
}

fn num(n: &Kv, k: &str) -> Option<f64> {
    n.get(k).and_then(Kv::f64)
}

fn guid(v: &Kv) -> Option<String> {
    Some(format!("{}:{}", num(v, "sessionID")? as u64, num(v, "localID")? as u64))
}

/// The main an `overriddenSymbolID` swaps to: none for the empty GUID written there for "no
/// swap", which names no node.
fn swap_guid(v: &Kv) -> Option<String> {
    guid(v).filter(|g| g != NO_STYLE)
}

#[cfg(test)]
mod connector_snapshot_budget_tests;

/// A `GUID`-holding reference (`StyleId`, `VariableID`, …): its `guid`.
fn ref_guid(v: &Kv) -> Option<String> {
    v.get("guid").and_then(guid).filter(|g| g != NO_STYLE)
}

fn vec2(v: &Kv) -> Option<Value> {
    Some(json!({"x": num(v, "x")?, "y": num(v, "y")?}))
}

fn size(v: &Kv) -> Option<(f64, f64)> {
    Some((num(v, "x")?, num(v, "y")?))
}

fn matrix(m: &Kv) -> Option<Affine> {
    let c = |k: &str| num(m, k);
    let a = Affine::new([c("m00")?, c("m10")?, c("m01")?, c("m11")?, c("m02")?, c("m12")?]);
    a.as_coeffs().iter().all(|x| x.is_finite()).then_some(a)
}

fn rows(a: Affine) -> Value {
    let [a, b, c, d, e, f] = a.as_coeffs();
    json!([[a, c, e], [b, d, f]])
}

fn copy_num(o: &mut Obj, n: &View, from: &str, to: &str) {
    if let Some(x) = n.num(from) {
        o.insert(to.into(), json!(x));
    }
}

/// [`copy_num`] for a length: the layers of a scaled instance give it at the instance's scale.
fn copy_len(o: &mut Obj, n: &View, from: &str, to: &str) {
    if let Some(x) = n.len(from) {
        o.insert(to.into(), json!(x));
    }
}

/// `x` at scale `k`; `None` when the result is not finite.
fn scaled(x: f64, k: f64) -> Option<f64> {
    if k == 1.0 {
        return Some(x);
    }
    let r = x * k;
    r.is_finite().then_some(r)
}

fn copy_str(o: &mut Obj, n: &View, from: &str, to: &str) {
    if let Some(s) = n.str(from) {
        o.insert(to.into(), json!(s));
    }
}

/// A node seen through override layers: the last layer with a field wins.
#[derive(Clone)]
struct View<'a> {
    layers: Vec<&'a Kv<'a>>,
    /// The factor each layer's lengths take (`scales[i]` is for `layers[i]`): the layers of a main
    /// inside a scaled instance have the instance's, the override and derived rows of the instance
    /// 1, being at the instance's scale already (#2091).
    scales: Vec<f64>,
    /// Size and transform worked out from the constraints, over the layers' own.
    fitted: Option<((f64, f64), Affine)>,
}

impl<'a> View<'a> {
    fn of(n: &'a Kv<'a>) -> Self {
        Self::of_scaled(n, 1.0)
    }

    /// A node of a main inside an instance scaled by `k`: its lengths read `k` times as long.
    fn of_scaled(n: &'a Kv<'a>, k: f64) -> Self {
        Self { layers: vec![n], scales: vec![k], fitted: None }
    }

    /// This view with the layers of `upper` over it (`upper`'s fitted size and place).
    fn over(&self, upper: &Self) -> Self {
        let mut v = self.clone();
        v.layers.extend(upper.layers.iter().copied());
        v.scales.extend(upper.scales.iter().copied());
        v.fitted = upper.fitted;
        v
    }

    /// The factor of the lowest layer (the layer's own main), for values a style gives.
    fn base_scale(&self) -> f64 {
        self.scales.first().copied().unwrap_or(1.0)
    }

    fn size(&self) -> Option<(f64, f64)> {
        self.fitted.map(|f| f.0).or_else(|| {
            let (v, k) = self.find("size")?;
            let (x, y) = size(v)?;
            Some((scaled(x, k)?, scaled(y, k)?))
        })
    }

    fn transform(&self) -> Option<Affine> {
        self.fitted.map(|f| f.1).or_else(|| {
            let (v, k) = self.find("transform")?;
            let m = matrix(v)?;
            if k == 1.0 {
                return Some(m);
            }
            let [a, b, c, d, e, f] = m.as_coeffs();
            let m = Affine::new([a, b, c, d, scaled(e, k)?, scaled(f, k)?]);
            Some(m)
        })
    }

    /// This view with `top` over it, each layer with the factor its lengths take.
    fn with(&self, top: impl IntoIterator<Item = (&'a Kv<'a>, f64)>) -> Self {
        let mut v = self.clone();
        for (layer, scale) in top {
            v.layers.push(layer);
            v.scales.push(scale);
        }
        v
    }

    /// The factor of the layer that links a style in `key` (or `legacy`), the lowest layer's when
    /// none does: what the linked style's lengths take.
    fn link_scale(&self, key: &str, legacy: &str) -> f64 {
        self.find(key).or_else(|| self.find(legacy)).map_or_else(|| self.base_scale(), |f| f.1)
    }

    /// This view with `style` just above the layer that links it in `key` (or `legacy`): the style
    /// beats that layer's and lower layers' copies of its fields (stale when the style changed
    /// since, or written for the style linked before), and override rows above the link beat it.
    fn styled(&self, style: Option<&'a Kv<'a>>, key: &str, legacy: &str) -> Self {
        let mut v = self.clone();
        if let Some(s) = style {
            let linked_in = |k: &str| v.layers.iter().rposition(|l| l.get(k).is_some());
            let at = linked_in(key).or_else(|| linked_in(legacy)).map_or(1, |i| i + 1);
            let at = at.min(v.layers.len());
            // The style is at the scale of the layer that links it.
            let k = at.checked_sub(1).and_then(|i| v.scales.get(i)).copied().unwrap_or(1.0);
            v.layers.insert(at, s);
            v.scales.insert(at, k);
        }
        v
    }

    /// An effect/grid list follows the same precedence as text styles: an override above the
    /// style link wins, while the style beats stale copies on and below its linking row. A style
    /// without the list is empty; it must not expose a stale list from a lower row.
    fn styled_list(&self, style: &'a Kv<'a>, list: &str, key: &str, legacy: &str) -> (&'a [Kv<'a>], f64) {
        let linked_in = |k: &str| self.layers.iter().rposition(|l| l.get(k).is_some());
        let above = linked_in(key).or_else(|| linked_in(legacy)).map_or(1, |i| i + 1);
        self.layers
            .iter()
            .zip(&self.scales)
            .skip(above)
            .rev()
            .find_map(|(row, scale)| row.get(list).map(|v| (v.list(), *scale)))
            .unwrap_or_else(|| (style.get(list).map(Kv::list).unwrap_or_default(), self.link_scale(key, legacy)))
    }

    fn get(&self, k: &str) -> Option<&'a Kv<'a>> {
        self.layers.iter().rev().find_map(|l| l.get(k))
    }

    /// [`Self::get`], and the factor of the layer that has it.
    fn find(&self, k: &str) -> Option<(&'a Kv<'a>, f64)> {
        self.layers.iter().zip(&self.scales).rev().find_map(|(l, s)| l.get(k).map(|v| (v, *s)))
    }

    /// A length: the number `k` holds at the scale of the layer that has it.
    fn len(&self, k: &str) -> Option<f64> {
        let (v, s) = self.find(k)?;
        scaled(v.f64()?, s)
    }

    /// [`Self::list`], and the factor of the layer that has it.
    fn list_scaled(&self, k: &str) -> (&'a [Kv<'a>], f64) {
        self.find(k).map_or((&[], 1.0), |(v, s)| (v.list(), s))
    }

    fn num(&self, k: &str) -> Option<f64> {
        self.get(k).and_then(Kv::f64)
    }

    fn str(&self, k: &str) -> Option<&'a str> {
        self.get(k).and_then(Kv::str)
    }

    fn flag(&self, k: &str) -> Option<bool> {
        self.get(k).and_then(Kv::bool)
    }

    fn list(&self, k: &str) -> &'a [Kv<'a>] {
        self.get(k).map(Kv::list).unwrap_or_default()
    }
}

struct Mapper<'a> {
    nodes: BTreeMap<String, &'a Kv<'a>>,
    kids: BTreeMap<String, Vec<(&'a str, String)>>,
    blobs: Vec<&'a [u8]>,
    /// Component property definitions: id → (REST name "Name#id", type).
    prop_defs: BTreeMap<String, (String, &'a str)>,
    /// The copies of a set's definitions each variant keeps (no name or type of their own; the
    /// variant's layers and its instances' assignments use them): copy id → the definition's id.
    prop_parents: BTreeMap<String, String>,
    /// File nodes by asset `key` and `version`: references can name a collection, variable or
    /// style as the asset instead of the guid, and the file carries the resources to resolve
    /// them (the collection node itself says the same `key`).
    assets: BTreeMap<String, BTreeMap<String, String>>,
    styles: Obj,
    /// Descriptions of components and component sets (REST `components` / `componentSets`).
    components: Obj,
    sets: Obj,
    /// Layers and keys of each main, built when an instance of it first needs them.
    override_scopes: BTreeMap<String, Rc<OverrideScope>>,
    /// Entries held by `override_scopes` (bounded by `max_nodes` together).
    override_scope_entries: usize,
    /// Component-bearing branches of the file's internal canvases (cached library definitions).
    library_contexts: Vec<Value>,
    /// Layers visited while scanning internal canvases, across all of them.
    library_scan_count: usize,
    warnings: craft_figma_rest::Warnings,
    geometry: &'a dyn Geometry,
    outlined_stroke: Option<(&'a str, &'a str)>,
    count: usize,
    /// The mains in the file that instances or instance swap defaults name, and the mains mapped
    /// as layers: the difference is what the layer cap kept out.
    referenced_mains: BTreeSet<String>,
    mapped_mains: BTreeSet<String>,
    /// Most layers produced or scanned, instance expansion included; the one place the cap comes
    /// from (see [`layer_budget`]).
    max_nodes: usize,
    /// Layers kept back for the visible pages' top level while library copies are mapped, so the
    /// copies can't take the whole budget and leave the pages empty.
    reserved: usize,
    /// Vector networks, each built once: the layers that show one (every instance of a main
    /// holding it) name it by `vectorNetworkRef`. Kept out of the JSON, where a large file's
    /// networks took gigabytes.
    networks: Vec<Option<VectorNetwork>>,
    /// Index in `networks` by its blob, position scale, style table, point-radius scale and the
    /// layer's corner radius (which points without one of their own take).
    network_ids: HashMap<(usize, u64, u64, usize, u64, u64), usize>,
    /// Region paint elements (paints and gradient stops) read so far: one budget for the whole
    /// file, since layers with region paints don't share their network.
    region_paint_elements: usize,
}

impl<'a> Mapper<'a> {
    fn new(msg: &'a Kv<'a>, options: &Options<'a>) -> Self {
        let max_nodes = options.max_nodes;
        let mut nodes = BTreeMap::new();
        let mut kids: BTreeMap<String, Vec<(&'a str, String)>> = BTreeMap::new();
        let mut prop_defs = BTreeMap::new();
        let mut prop_parents = BTreeMap::new();
        let changes = msg.get("nodeChanges").map(Kv::list).unwrap_or_default();
        // The last change of a GUID wins; a REMOVED change deletes what came before it.
        for n in changes {
            let Some(id) = n.get("guid").and_then(guid) else { continue };
            if n.get("phase").and_then(Kv::str) == Some("REMOVED") {
                nodes.remove(&id);
            } else {
                nodes.insert(id, n);
            }
        }
        for n in changes {
            let Some(id) = n.get("guid").and_then(guid) else { continue };
            if !nodes.get(&id).is_some_and(|latest| std::ptr::eq(*latest, n)) {
                continue;
            }
            // Variables and their collections are read from the node list (`of_type`), never
            // as layers: out of the layer tree, an instance's expansion doesn't walk them again for
            // every instance without counting them (#1077).
            if let Some(p) = n.get("parentIndex")
                && let Some(parent) = p.get("guid").and_then(guid)
                && !matches!(n.get("type").and_then(Kv::str), Some("VARIABLE" | "VARIABLE_SET"))
            {
                kids.entry(parent).or_default().push((p.get("position").and_then(Kv::str).unwrap_or(""), id.clone()));
            }
            for d in n.get("componentPropDefs").map(Kv::list).unwrap_or_default() {
                let Some(did) = d.get("id").and_then(guid) else { continue };
                if let (Some(name), Some(ty)) = (d.get("name").and_then(Kv::str), d.get("type").and_then(Kv::str)) {
                    prop_defs.insert(did.clone(), (format!("{name}#{did}"), ty));
                } else if let Some(parent) = d.get("parentPropDefId").and_then(guid) {
                    prop_parents.insert(did, parent);
                }
            }
        }
        for list in kids.values_mut() {
            list.sort();
            list.dedup_by(|a, b| a.1 == b.1);
        }
        let blobs =
            msg.get("blobs").map(Kv::list).unwrap_or_default().iter().map(|b| b.get("bytes").and_then(Kv::bytes).unwrap_or_default()).collect();
        let mut assets: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for n in msg.get("nodeChanges").map(Kv::list).unwrap_or_default() {
            let (Some(key), Some(id)) = (n.get("key").and_then(Kv::str), n.get("guid").and_then(guid)) else { continue };
            if key.is_empty() || key.len() > MAX_ASSET_PART {
                continue;
            }
            let version = n.get("version").and_then(Kv::str).filter(|v| !v.is_empty() && v.len() <= MAX_ASSET_PART).unwrap_or("");
            assets.entry(key.to_string()).or_default().entry(version.to_string()).or_insert(id);
        }
        Self {
            nodes,
            kids,
            blobs,
            prop_defs,
            prop_parents,
            assets,
            styles: Obj::new(),
            components: Obj::new(),
            sets: Obj::new(),
            override_scopes: BTreeMap::new(),
            override_scope_entries: 0,
            library_contexts: Vec::new(),
            library_scan_count: 0,
            warnings: craft_figma_rest::Warnings::default(),
            geometry: options.geometry,
            outlined_stroke: options.outlined_stroke,
            count: 0,
            referenced_mains: BTreeSet::new(),
            mapped_mains: BTreeSet::new(),
            max_nodes,
            reserved: 0,
            networks: Vec::new(),
            network_ids: HashMap::new(),
            region_paint_elements: 0,
        }
    }

    /// The id of the resource a `VariableSetID` (or `VariableID`, `StyleId`) names: its `guid`,
    /// else the file node its asset `key` and `version` names (a collection from a library is
    /// referenced as the asset, and the file's own node for it says the same `key`). `None` when
    /// the reference has neither or the asset names nothing (or ambiguously) in the file.
    /// Original code for this importer: the field names are those of the schema each file embeds.
    fn resource_id(&self, v: &Kv) -> Option<String> {
        // A guid names the resource when this file holds a node by it. One from another file (the
        // library a style or collection came from, as a paste carries) names nothing here: the
        // asset it also names finds the file's copy (#1638).
        if let Some(g) = v.get("guid").and_then(guid).filter(|g| *g != NO_STYLE && self.nodes.contains_key(g)) {
            return Some(g);
        }
        let asset = v.get("assetRef")?;
        let key = asset.get("key").and_then(Kv::str)?;
        let versions = self.assets.get(key)?;
        let version = asset.get("version").and_then(Kv::str);
        match version {
            Some(v) => versions.get(v),
            // Without a version: an unversioned record, else the file's only one.
            None => versions.get("").or_else(|| (versions.len() == 1).then(|| versions.values().next()).flatten()),
        }
        .cloned()
    }

    /// The collection a `VariableSetID` names, as one collection per library asset: a file that
    /// took several updates of a library holds a copy of the collection for each version it met,
    /// all with the same asset `key`. Variables and explicit modes may name any of those copies,
    /// while the mode ids stay the same across them, so every copy maps to the newest one (the
    /// one [`Self::variables`] keeps, with the modes of the others added). Without this, a page set
    /// to a mode an older copy didn't have yet lost its mode, and the layers fell back to the
    /// default mode of the newest copy. A reference by asset that names no copy by itself (no
    /// version, or a version the file doesn't hold, while it holds several) still names the asset:
    /// its newest live copy.
    pub(super) fn collection_id(&self, v: &Kv) -> Option<String> {
        let by_asset = || v.get("assetRef")?.get("key").and_then(Kv::str).and_then(VarCanon::newest_set);
        match self.resource_id(v) {
            Some(id) if self.nodes.contains_key(&id) => Some(self.canonical_collection(&id).unwrap_or(id)),
            // The asset index can name a node a later change removed: the asset's live copy instead.
            Some(id) => by_asset().or(Some(id)),
            None => by_asset(),
        }
    }

    /// The newest live copy of collection node `id`'s library asset, when it has a key.
    pub(super) fn canonical_collection(&self, id: &str) -> Option<String> {
        let key = self.nodes.get(id)?.get("key").and_then(Kv::str).filter(|k| !k.is_empty())?;
        VarCanon::newest_set(key)
    }

    /// A note about what this file's mapping left out or changed. Counted, as the REST importer
    /// counts its own: these notes are met once per layer, so the kinds were named once each with
    /// no sign of how much of the file they covered -- a file whose every text layer lost a
    /// setting read the same as one where a single layer did (#467). The collector keeps the
    /// first-seen order and the same cap, since some notes name fields of the file (image
    /// filters).
    fn warn(&mut self, w: impl Into<String>) {
        self.warnings.add(w.into());
    }

    fn children(&self, id: &str) -> Vec<String> {
        self.kids.get(id).map(|k| k.iter().map(|(_, c)| c.clone()).collect()).unwrap_or_default()
    }

    /// How many layers may be produced now: the cap, less what is kept for the visible pages.
    fn budget(&self) -> usize {
        self.max_nodes.saturating_sub(self.reserved)
    }
}

/// A library version (`"40727:1183"`) as numbers to order by; unparsable parts rank lowest.
fn version_rank(v: &str) -> (u64, u64) {
    let mut parts = v.split(':').map(|p| p.trim().parse::<u64>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

/// Library variables, one per asset. A file that took several updates of a library keeps a copy
/// of a variable for each version it met (same `key`, another `version`), and layers and other
/// variables name them by that asset (`assetRef {key, version}`) rather than by guid. Each
/// reference goes to the newest copy: it has the values of every mode the library has now, where
/// an older one lacks the modes added since (a layer under a page set to such a mode fell back to
/// the default one). Kept per thread while a file is mapped: the paint and variable readers that
/// meet references are free functions.
#[derive(Default)]
pub(super) struct VarCanon {
    by_key: HashMap<String, String>,
    by_id: HashMap<String, String>,
    /// Every live variable node of the file, with a key or not.
    local: BTreeSet<String>,
    /// The newest live copy of each library collection, by asset key. Read from the file's live
    /// nodes (the last change of each guid, REMOVED ones gone), not from the asset index, which
    /// keeps the first node it met for a key and version even when a later change removed it.
    sets_by_key: HashMap<String, String>,
}

thread_local! {
    static VAR_CANON: std::cell::RefCell<Option<VarCanon>> = const { std::cell::RefCell::new(None) };
}

/// Removes the thread's table when the mapping is done.
pub(super) struct VarCanonGuard;

impl Drop for VarCanonGuard {
    /// Clears the thread's table, so a later file on this thread starts without it.
    fn drop(&mut self) {
        VAR_CANON.with(|c| c.borrow_mut().take());
    }
}

impl VarCanon {
    /// Builds the table from mapper `m`'s live nodes (newest copy per asset key, for variables and
    /// for collections) and installs it for this thread until the returned guard is dropped.
    fn install(m: &Mapper) -> VarCanonGuard {
        let mut newest: HashMap<String, ((u64, u64), String)> = HashMap::new();
        let mut copies: Vec<(String, String)> = Vec::new();
        let mut local = BTreeSet::new();
        let mut sets: HashMap<String, ((u64, u64), String)> = HashMap::new();
        for (id, n) in &m.nodes {
            let ty = n.get("type").and_then(Kv::str);
            if !matches!(ty, Some("VARIABLE" | "VARIABLE_SET")) || n.get("isSoftDeleted").and_then(Kv::bool) == Some(true) {
                continue;
            }
            if ty == Some("VARIABLE_SET") {
                if let Some(key) = n.get("key").and_then(Kv::str).filter(|k| !k.is_empty()) {
                    let rank = version_rank(n.get("version").and_then(Kv::str).unwrap_or(""));
                    let e = sets.entry(key.to_string()).or_insert_with(|| (rank, id.clone()));
                    if rank > e.0 || (rank == e.0 && *id > e.1) {
                        *e = (rank, id.clone());
                    }
                }
                continue;
            }
            local.insert(id.clone());
            let Some(key) = n.get("key").and_then(Kv::str).filter(|k| !k.is_empty()) else { continue };
            let rank = version_rank(n.get("version").and_then(Kv::str).unwrap_or(""));
            copies.push((id.clone(), key.to_string()));
            let e = newest.entry(key.to_string()).or_insert_with(|| (rank, id.clone()));
            if rank > e.0 || (rank == e.0 && *id > e.1) {
                *e = (rank, id.clone());
            }
        }
        let mut t = VarCanon::default();
        for (id, key) in copies {
            if let Some((_, canon)) = newest.get(&key) {
                t.by_id.insert(id, canon.clone());
            }
        }
        t.by_key = newest.into_iter().map(|(k, (_, id))| (k, id)).collect();
        t.local = local;
        t.sets_by_key = sets.into_iter().map(|(k, (_, id))| (k, id)).collect();
        VAR_CANON.with(|c| *c.borrow_mut() = Some(t));
        VarCanonGuard
    }

    /// The variable a reference (`{guid}` or `{assetRef}`) names, as its newest copy. A guid
    /// counts when this file holds a variable by it; one from another file (a paste carries the
    /// library's own guids) names nothing here, so the asset it also names is tried before it is
    /// kept as it is.
    pub(super) fn resolve(r: &Kv) -> Option<String> {
        let by_guid = r.get("guid").and_then(guid).filter(|g| g != NO_STYLE);
        VAR_CANON.with(|c| {
            let c = c.borrow();
            let Some(t) = c.as_ref() else { return by_guid.clone() };
            if let Some(g) = by_guid.as_ref().filter(|g| t.local.contains(*g)) {
                return Some(t.by_id.get(g).cloned().unwrap_or_else(|| g.clone()));
            }
            let by_asset = r.get("assetRef").and_then(|a| a.get("key")).and_then(Kv::str).and_then(|k| t.by_key.get(k)).cloned();
            by_asset.or(by_guid)
        })
    }

    /// The newest live copy of the library collection with asset key `key`.
    pub(super) fn newest_set(key: &str) -> Option<String> {
        VAR_CANON.with(|c| c.borrow().as_ref().and_then(|t| t.sets_by_key.get(key).cloned()))
    }

    /// Whether a table is installed on this thread (for tests: none outlives the mapping).
    #[cfg(test)]
    fn installed() -> bool {
        VAR_CANON.with(|c| c.borrow().is_some())
    }

    /// Whether variable node `id` is an older copy another one replaces.
    pub(super) fn superseded(id: &str) -> bool {
        VAR_CANON.with(|c| c.borrow().as_ref().and_then(|t| t.by_id.get(id)).is_some_and(|canon| canon != id))
    }
}

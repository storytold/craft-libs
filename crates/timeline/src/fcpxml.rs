//! FCPXML (1.9–1.11) import/export.
//!
//! Mapping between the track model of [`crate::interchange`] and FCPXML's magnetic model:
//! - V1 is the primary storyline (`spine`): clips, `gap`s and `transition`s.
//! - Other video tracks become connected clips with `lane = track index` (V2 → lane 1); audio-only
//!   items on audio track n become connected clips on lane `-n`. A V1/V2… clip whose linked audio
//!   sits on the same-numbered audio track with identical timing is written as one clip carrying
//!   both (`srcEnable="all"`), otherwise `srcEnable="video"`/`"audio"`.
//! - Rational times (`1001/24000s`) map exactly onto ticks.
//! - Standalone `video` elements reference the picture of an asset (including still images),
//!   without importing that asset's audio. Effect-backed generators are reported as unsupported.
//! - `adjust-transform` (position in percent of frame height, FCP y-up, counter-clockwise
//!   rotation), `adjust-blend` (opacity), `adjust-volume` (dB) and their keyframes are mapped;
//!   constant speed, reverse and freeze frames use a two-point `timeMap`.
//! - Markers on primary-storyline elements become sequence markers; markers on connected clips stay
//!   clip markers. Transitions are supported on the primary storyline (and read from secondary
//!   storylines); transitions elsewhere are reported.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::build::{
    Builder, Imported, MAX_IMPORTED_TRACKS, MediaSpec, base_item, db_to_gain, empty_sequence, gain_to_db, generator_of, item_media, item_path,
    kind_from_ext, param, param_modified, scaled, set_param, settings_for, standard_effects, transition_effect, transition_name,
};
use crate::interchange::{
    BinId, Catalog, ClipId, Interpolation, ItemId, ItemKind, Keyframe, Label, Marker, MarkerId, MarkerKind, MediaKind, Param, ParamValue, Project,
    Sequence, TrackItem, TrackKind, Transition, TransitionAlign, TransitionId, Vec2,
};
use crate::path::{file_name, path_to_file_url, resolve_path, resolve_url};
use crate::rates::FrameRate;
use crate::report::Report;
use crate::time::{TICKS_PER_SECOND, Tick};
use crate::xml::{Node, child, children, elements, escape_attr, num};
use crate::{Error, ExportOptions, Format, ImportOptions, Result};

// ---------------------------------------------------------------------------------------------
// Rational time
// ---------------------------------------------------------------------------------------------

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// Parse an FCPXML time value (`"1001/24000s"`, `"5s"`, `"0s"`, `"3.5s"`) into ticks.
pub(crate) fn parse_time(s: &str) -> Option<Tick> {
    let s = s.trim().strip_suffix('s').unwrap_or(s.trim());
    if s.is_empty() {
        return None;
    }
    if let Some((a, b)) = s.split_once('/') {
        let a: i128 = a.trim().parse().ok()?;
        let b: i128 = b.trim().parse().ok()?;
        if b == 0 {
            return None;
        }
        let n = a * TICKS_PER_SECOND as i128;
        // Exact for every broadcast rate; round to the nearest tick otherwise.
        let q = n.div_euclid(b);
        let r = n.rem_euclid(b);
        return Some(Tick((if 2 * r >= b.abs() { q + 1 } else { q }) as i64));
    }
    if let Ok(i) = s.parse::<i64>() {
        return Some(Tick(i * TICKS_PER_SECOND));
    }
    s.parse::<f64>().ok().map(Tick::from_seconds_f64)
}

/// Format ticks as an FCPXML rational time, frame-based at `rate` when the value is on a frame.
pub(crate) fn fmt_time(t: Tick, rate: FrameRate) -> String {
    if t == Tick::ZERO {
        return "0s".into();
    }
    let f = rate.frame_at(t);
    if rate.tick_of(f) == t {
        let num = f as i128 * rate.den as i128;
        let den = rate.num as i128;
        if num % den == 0 {
            return format!("{}s", num / den);
        }
        return format!("{num}/{den}s");
    }
    let g = gcd(t.0 as i128, TICKS_PER_SECOND as i128);
    let (a, b) = (t.0 as i128 / g, TICKS_PER_SECOND as i128 / g);
    if b == 1 { format!("{a}s") } else { format!("{a}/{b}s") }
}

fn attr_time(n: Node, name: &str) -> Option<Tick> {
    n.attribute(name).and_then(parse_time)
}

fn rate_from_frame_duration(s: &str) -> Option<FrameRate> {
    let s = s.trim().strip_suffix('s')?;
    let (a, b) = s.split_once('/').unwrap_or((s, "1"));
    let (a, b): (i64, i64) = (a.parse().ok()?, b.parse().ok()?);
    // both parts must be positive: a zero or negative numerator of the rate divides by zero later
    (a > 0 && b > 0).then(|| FrameRate::new(b, a))
}

fn audio_rate_str(sr: u32) -> String {
    if sr.is_multiple_of(1000) { format!("{}k", sr / 1000) } else { format!("{}k", sr as f64 / 1000.0) }
}

fn parse_audio_rate(s: &str) -> Option<u32> {
    let s = s.trim();
    let r = match s.strip_suffix('k') {
        Some(k) => k.parse::<f64>().ok().map(|v| (v * 1000.0).round() as u32),
        None => s.parse().ok(),
    };
    r.filter(|&sr| sr > 0 && sr <= 384_000)
}

// ---------------------------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------------------------

#[derive(Clone)]
struct Asset {
    item: ItemId,
    start: Tick,
    has_video: bool,
    has_audio: bool,
}

// Bound semantic resource chains separately from XML syntax: flat media resources can
// reference one another without adding any XML nesting.
const MAX_MEDIA_DEPTH: usize = 64;
const MAX_XML_DEPTH: usize = 128;
// Retrying truncated resources is useful at shallower depths, but must remain bounded.
const MAX_REIMPORTS: usize = 4096;
// A hostile document may reference the same large cached graph repeatedly. Bound total
// reachability work, not just one walk, and refuse reuse when safety cannot be established.
const MAX_CACHE_EDGES: usize = 1_000_000;

struct Imp<'a, 'c, 'r> {
    b: Builder<'c>,
    opts: &'a ImportOptions<'a>,
    report: &'r mut Report,
    report_limit: Option<usize>,
    formats: HashMap<String, (FrameRate, u32, u32)>,
    asset_nodes: HashMap<String, Node<'a>>,
    assets: HashMap<String, Asset>,
    media_nodes: HashMap<String, Node<'a>>,
    media_items: HashMap<String, ItemId>,
    media_active: HashSet<String>,
    pending_audio: HashMap<ItemId, Vec<PendingAudio<'a>>>,
    audio_replay: HashMap<ItemId, AudioReplay<'a>>,
    depth_hits: usize,
    // Depths at which a cached resource was truncated. A retry may improve neither limit
    // at the expense of the other, and reuses the same item so earlier references stay valid.
    media_truncated_at: HashMap<String, (usize, usize)>,
    reimports_left: usize,
    cache_edges_left: usize,
    depth: usize,
    effects: HashMap<String, String>,
}

/// Where an element's keyframe times start, the media time it starts at and its playback speed.
#[derive(Clone, Copy)]
struct LocalTiming {
    local_start: Tick,
    src_in: Tick,
    speed: f64,
}

/// Where a written clip element sits: offset in its parent's local time, connected lane, and
/// whether it is on the primary storyline.
#[derive(Clone, Copy)]
struct ClipPlacement {
    offset: Tick,
    lane: Option<i32>,
    primary: bool,
}

/// An audio reference whose nested sequence did not yet expose sound.
struct PendingAudio<'a> {
    parent: ItemId,
    child: ItemId,
    audio: TrackItem,
    video: Option<ClipId>,
    diagnostics: Option<Node<'a>>,
}

/// Original audio preferences and owned failures survive until final placement is known.
#[derive(Default)]
struct AudioReplay<'a> {
    order: Vec<(ClipId, usize)>,
    unplaced: Vec<TrackItem>,
    paired: HashMap<ClipId, (ClipId, Option<u64>)>,
    video_index: Option<HashMap<ClipId, (usize, usize)>>,
    final_cap_skips: usize,
    diagnostics: HashMap<ClipId, Node<'a>>,
}

/// Small immutable input for exact encounter-order replay planning.
#[derive(Clone, Copy)]
struct AudioInterval {
    id: ClipId,
    preferred: usize,
    start: Tick,
    end: Tick,
}

fn spend_audio_work(left: &mut usize, work: usize) -> bool {
    let Some(rest) = left.checked_sub(work) else { return false };
    *left = rest;
    true
}

/// Logarithmic index-work units, not measured instructions or wall time.
fn audio_index_work(items: usize) -> usize {
    (usize::BITS - items.leading_zeros()) as usize + 1
}

fn audio_sort_work(items: usize) -> usize {
    if items < 2 { 0 } else { items.saturating_mul(audio_index_work(items)) }
}

/// Plan with predecessor/successor queries; no stored sequence or replay state is mutated.
fn indexed_audio_plan(audio: &[AudioInterval], transitions: &[usize], work: &mut usize) -> Option<Vec<(ClipId, Option<usize>)>> {
    let mut tracks: Vec<BTreeSet<(Tick, Tick, ClipId)>> = Vec::new();
    let mut plan = Vec::with_capacity(audio.len());
    for clip in audio {
        let mut placed = None;
        for track in clip.preferred..MAX_IMPORTED_TRACKS {
            if !spend_audio_work(work, 1) {
                return None;
            }
            if track >= tracks.len() {
                if !spend_audio_work(work, track + 1 - tracks.len()) {
                    return None;
                }
                tracks.resize_with(track + 1, BTreeSet::new);
            }
            let intervals = &mut tracks[track];
            let index_work = audio_index_work(intervals.len());
            if !spend_audio_work(work, index_work.saturating_mul(2)) {
                return None;
            }
            let at = (clip.start, Tick(i64::MAX), ClipId(u64::MAX));
            let before = intervals.range(..=at).next_back().is_some_and(|&(start, end, _)| start < clip.end && clip.start < end);
            let after = intervals
                .range((std::ops::Bound::Excluded(at), std::ops::Bound::Unbounded))
                .next()
                .is_some_and(|&(start, end, _)| start < clip.end && clip.start < end);
            if before || after {
                continue;
            }
            if !spend_audio_work(work, index_work) {
                return None;
            }
            intervals.insert((clip.start, clip.end, clip.id));
            placed = Some(track);
            break;
        }
        plan.push((clip.id, placed));
    }
    for track in 0..tracks.len().max(transitions.len()) {
        let items = tracks.get(track).map_or(0, BTreeSet::len);
        let transitions = transitions.get(track).copied().unwrap_or(0);
        if !spend_audio_work(work, audio_sort_work(items).saturating_add(audio_sort_work(transitions))) {
            return None;
        }
    }
    Some(plan)
}

/// Placement context of a storyline element.
struct Ctx {
    sequence: ItemId,
    seq_rate: FrameRate,
    frame: (u32, u32),
}

/// Read an FCPXML document. Media paths are resolved against `opts.base_dir`; filters are looked
/// up in `opts.catalog` by name. What cannot be represented is in `report`.
pub fn import(text: &str, opts: &ImportOptions, report: &mut Report) -> Result<Imported> {
    import_with_report_limit(text, opts, report, None)
}
/// Read FCPXML with an optional distinct-diagnostic limit.
pub fn import_with_report_limit(text: &str, opts: &ImportOptions, report: &mut Report, report_limit: Option<usize>) -> Result<Imported> {
    let doc = crate::xml::parse(text).map_err(|e| Error::parse(Format::Fcpxml, e))?;
    let root = Node(&doc);
    if root.name() != "fcpxml" {
        return Err(Error::parse(Format::Fcpxml, "root element is not <fcpxml>"));
    }
    if let Some(v) = root.attribute("version") {
        let minor = v.split('.').nth(1).and_then(|m| m.parse::<u32>().ok()).unwrap_or(0);
        if !v.starts_with("1.") || minor > 11 {
            report.info_limited(report_limit, format!("FCPXML version {v} is newer than 1.11; unknown elements were ignored"));
        }
    }
    let lib_name = child(root, "library").and_then(|l| child(l, "event")).and_then(|e| e.attribute("name")).unwrap_or("Imported FCPXML");
    let mut imp = Imp {
        b: Builder::new(lib_name, opts.catalog),
        opts,
        report,
        report_limit,
        formats: HashMap::new(),
        asset_nodes: HashMap::new(),
        assets: HashMap::new(),
        media_nodes: HashMap::new(),
        media_items: HashMap::new(),
        media_active: HashSet::new(),
        pending_audio: HashMap::new(),
        audio_replay: HashMap::new(),
        depth_hits: 0,
        media_truncated_at: HashMap::new(),
        reimports_left: MAX_REIMPORTS,
        cache_edges_left: MAX_CACHE_EDGES,
        depth: 0,
        effects: HashMap::new(),
    };
    if let Some(res) = child(root, "resources") {
        for r in elements(res) {
            let Some(id) = r.attribute("id") else { continue };
            match r.name() {
                "format" => {
                    let rate = match r.attribute("frameDuration") {
                        Some(fd) => rate_from_frame_duration(fd).unwrap_or_else(|| {
                            imp.report.warn_limited(
                                imp.report_limit,
                                format!("format \"{id}\" has an invalid frameDuration \"{fd}\"; the default frame rate was used"),
                            );
                            FrameRate::default()
                        }),
                        None => FrameRate::default(),
                    };
                    let w = r.attribute("width").and_then(|v| v.parse().ok()).unwrap_or(1920);
                    let h = r.attribute("height").and_then(|v| v.parse().ok()).unwrap_or(1080);
                    imp.formats.insert(id.into(), (rate, w, h));
                }
                "asset" => {
                    imp.asset_nodes.insert(id.into(), r);
                }
                "media" => {
                    imp.media_nodes.insert(id.into(), r);
                }
                "effect" => {
                    imp.effects.insert(id.into(), r.attribute("name").unwrap_or("").to_string());
                }
                _ => {}
            }
        }
    }
    let mut events: Vec<Node> = Vec::new();
    if let Some(lib) = child(root, "library") {
        events.extend(children(lib, "event"));
    }
    events.extend(children(root, "event"));
    for ev in events {
        let bin = imp.b.bin(ev.attribute("name").unwrap_or("Event"), None);
        for c in elements(ev) {
            match c.name() {
                "project" => {
                    if let Some(s) = child(c, "sequence") {
                        let name = c.attribute("name").unwrap_or("Project");
                        let id = imp.sequence(s, name, Some(bin), None);
                        imp.b.top.push(id);
                    }
                }
                "asset-clip" | "clip" => {
                    if let Some(r) = c.attribute("ref") {
                        imp.asset(r, Some(bin));
                    }
                }
                _ => {}
            }
        }
    }
    for p in children(root, "project") {
        if let Some(s) = child(p, "sequence") {
            let id = imp.sequence(s, p.attribute("name").unwrap_or("Project"), None, None);
            imp.b.top.push(id);
        }
    }
    if imp.b.p.items.is_empty() {
        return Err(Error::Empty);
    }
    imp.restore_deferred_audio();
    let mut imported = imp.b.finish();
    imported.bound_with_report_limit(report, report_limit);
    Ok(imported)
}

fn flag(n: Node, name: &str) -> Option<bool> {
    n.attribute(name).map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

impl<'a> Imp<'a, '_, '_> {
    fn asset(&mut self, id: &str, bin: Option<BinId>) -> Option<Asset> {
        if let Some(a) = self.assets.get(id) {
            return Some(a.clone());
        }
        let n = *self.asset_nodes.get(id)?;
        let name = n.attribute("name").unwrap_or("Asset").to_string();
        let src = child(n, "media-rep").and_then(|m| m.attribute("src")).or_else(|| n.attribute("src"));
        let fmt = n.attribute("format").and_then(|f| self.formats.get(f)).copied();
        let mut has_video = flag(n, "hasVideo").unwrap_or(false);
        let has_audio = flag(n, "hasAudio").unwrap_or(false);
        if !has_video && !has_audio {
            has_video = kind_from_ext(src.unwrap_or(&name)) != MediaKind::AudioOnly;
        }
        let start = attr_time(n, "start").unwrap_or(Tick::ZERO);
        let rate = fmt.map(|f| f.0).unwrap_or_default();
        let spec = MediaSpec {
            duration: attr_time(n, "duration"),
            video: has_video.then(|| fmt.map(|(r, w, h)| (w, h, r)).unwrap_or((1920, 1080, rate))),
            audio: has_audio.then(|| {
                (
                    n.attribute("audioRate").and_then(parse_audio_rate).unwrap_or(48_000),
                    n.attribute("audioChannels").and_then(|c| c.parse().ok()).unwrap_or(2),
                )
            }),
            start_tc: (start != Tick::ZERO).then(|| rate.frame_at(start)),
            kind: None,
        };
        let item = match src {
            Some(url) => {
                let p = resolve_url(url, self.opts.base_dir);
                self.b.file_media(&format!("file:{p}"), &name, &p, &spec, bin)
            }
            None => {
                self.report.warn_limited(self.report_limit, format!("asset \"{name}\" has no media; imported offline"));
                let p = resolve_path(&name, self.opts.base_dir);
                let it = self.b.file_media(&format!("asset:{id}"), &name, &p, &spec, bin);
                if let Some(m) = self.b.p.item_mut(it).and_then(|i| i.as_media_mut()) {
                    m.offline = true;
                }
                it
            }
        };
        let a = Asset { item, start, has_video, has_audio };
        self.assets.insert(id.into(), a.clone());
        Some(a)
    }

    /// A rebuilt item retains its ID. A cached ancestor that already points to it must
    /// not be inserted underneath it, even when the resource names are not directly active.
    fn cache_reaches_active(&mut self, root: ItemId) -> bool {
        let active: HashSet<ItemId> = self.media_active.iter().filter_map(|id| self.media_items.get(id).copied()).collect();
        self.cache_reaches_any(root, &active)
    }

    /// Refuse an edge when it could reach its parent, or when the shared work budget is exhausted.
    fn cache_reaches_any(&mut self, root: ItemId, targets: &HashSet<ItemId>) -> bool {
        if targets.is_empty() {
            return false;
        }
        let mut seen = HashSet::from([root]);
        let mut pending = vec![root];
        while let Some(id) = pending.pop() {
            if targets.contains(&id) {
                return true;
            }
            let Some(seq) = self.b.p.sequence(id) else { continue };
            for item in seq.all_tracks().flat_map(|track| &track.items) {
                if self.cache_edges_left == 0 {
                    return true;
                }
                self.cache_edges_left -= 1;
                if targets.contains(&item.item) {
                    return true;
                }
                if seen.insert(item.item) {
                    pending.push(item.item);
                }
            }
        }
        false
    }

    /// Revisit parents in deterministic waves, seeing earlier updated parents in each wave.
    fn restore_deferred_audio(&mut self) {
        let mut pending: Vec<_> = std::mem::take(&mut self.pending_audio).into_values().flatten().collect();
        pending.sort_by_key(|p| p.audio.id.0);
        'waves: while !pending.is_empty() {
            let mut waiting = Vec::new();
            let mut ready: HashMap<ItemId, Vec<PendingAudio<'a>>> = HashMap::new();
            for intent in pending {
                if self.cache_edges_left == 0 {
                    self.report_audio_work_limit();
                    break 'waves;
                }
                self.cache_edges_left -= 1;
                if self.b.p.sequence(intent.child).is_some_and(|s| s.audio_tracks.iter().any(|t| !t.items.is_empty())) {
                    ready.entry(intent.parent).or_default().push(intent);
                } else {
                    waiting.push(intent);
                }
            }
            let mut batches: Vec<_> = ready.into_values().collect();
            batches.sort_by_key(|batch| batch[0].audio.id.0);
            let mut progressed = false;
            for batch in batches {
                match self.replay_parent_audio(batch[0].parent, batch) {
                    Some(new_audio) => progressed |= new_audio,
                    None => {
                        self.report_audio_work_limit();
                        break 'waves;
                    }
                }
            }
            if !progressed {
                break;
            }
            pending = waiting;
        }
        // Report only final failures: a later replay may vacate a provisionally full lane.
        let mut parents: Vec<_> = self.audio_replay.iter().collect();
        parents.sort_by_key(|(parent, state)| (state.order.first().map(|p| p.0.0), parent.0));
        for (parent, state) in parents {
            // Some audio never reaches a successful replay (a silent child, refused
            // budget, or no deferred child at all). Its picture must not retain a link.
            let unplaced_links: HashMap<_, _> =
                state.unplaced.iter().filter_map(|a| state.paired.get(&a.id)).filter_map(|(video, link)| link.map(|link| (*video, link))).collect();
            if !unplaced_links.is_empty()
                && let Some(seq) = self.b.p.sequence_mut(*parent)
            {
                for video in seq.video_tracks.iter_mut().flat_map(|t| &mut t.items) {
                    if unplaced_links.get(&video.id).is_some_and(|link| video.link == Some(*link)) {
                        video.link = None;
                    }
                }
            }
            for _ in 0..state.unplaced.len().saturating_add(state.final_cap_skips) {
                self.report.warn_limited(
                    self.report_limit,
                    format!("FCPXML Audio lane exceeds the {MAX_IMPORTED_TRACKS}-track import limit; clip was skipped"),
                );
            }
            if state.diagnostics.is_empty() {
                continue;
            }
            let placed: HashSet<_> =
                self.b.p.sequence(*parent).into_iter().flat_map(|s| &s.audio_tracks).flat_map(|t| &t.items).map(|a| a.id).collect();
            // A later replay can displace audio again. Report adjustments only for the
            // final placed clips, once, in their original encounter order.
            for element in state.order.iter().filter(|(id, _)| placed.contains(id)).filter_map(|(id, _)| state.diagnostics.get(id)) {
                report_audio_adjustments(*element, &self.effects, self.b.catalog, self.report, self.report_limit);
            }
        }
    }

    fn report_audio_work_limit(&mut self) {
        self.report.warn_limited(self.report_limit, "FCPXML deferred audio exceeds the cache traversal budget; remaining audio was skipped");
    }

    /// Complete budgeted planning before taking ownership of the parent or its replay ledger.
    fn replay_parent_audio(&mut self, parent: ItemId, intents: Vec<PendingAudio<'a>>) -> Option<bool> {
        let Some(state) = self.audio_replay.get(&parent) else { return Some(false) };
        let Some(stored) = self.b.p.sequence(parent) else { return Some(false) };
        let had_audio = stored.audio_tracks.iter().any(|t| !t.items.is_empty());
        let count = stored.audio_tracks.iter().map(|t| t.items.len()).sum::<usize>();
        let video_work = if state.video_index.is_none() { stored.video_tracks.iter().map(|t| t.items.len()).sum::<usize>() } else { 0 };
        let work = count
            .saturating_mul(3)
            .saturating_add(state.unplaced.len().saturating_mul(4))
            .saturating_add(state.order.len().saturating_mul(4))
            .saturating_add(state.paired.len().saturating_mul(2))
            .saturating_add(intents.len().saturating_mul(4))
            .saturating_add(video_work)
            .saturating_add(stored.audio_tracks.len())
            .saturating_add(stored.video_tracks.len());
        if !spend_audio_work(&mut self.cache_edges_left, work) {
            return None;
        }
        let mut candidates: Vec<_> = state.unplaced.iter().map(|a| (a.id, a.item)).chain(intents.iter().map(|p| (p.audio.id, p.child))).collect();
        if !spend_audio_work(&mut self.cache_edges_left, audio_sort_work(candidates.len())) {
            return None;
        }
        candidates.sort_by_key(|p| p.0.0);
        let mut rejected = HashSet::new();
        let targets = HashSet::from([parent]);
        let mut reachability = HashMap::new();
        for (id, child) in candidates {
            let reaches_parent = match reachability.get(&child) {
                Some(&reaches) => reaches,
                None => {
                    let reaches = self.cache_reaches_any(child, &targets);
                    if self.cache_edges_left == 0 {
                        return None;
                    }
                    reachability.insert(child, reaches);
                    reaches
                }
            };
            if reaches_parent {
                self.report.warn_limited(
                    self.report_limit,
                    "FCPXML deferred audio could form a recursive graph or exceeds the cache traversal budget; audio was skipped",
                );
                rejected.insert(id);
            }
        }
        let state = self.audio_replay.get(&parent)?;
        let stored = self.b.p.sequence(parent)?;
        let items: HashMap<_, _> = stored
            .audio_tracks
            .iter()
            .flat_map(|track| &track.items)
            .chain(&state.unplaced)
            .chain(intents.iter().map(|p| &p.audio))
            .map(|a| (a.id, a))
            .collect();
        let audio: Vec<_> = state
            .order
            .iter()
            .filter(|placement| !rejected.contains(&placement.0))
            .filter_map(|&(id, preferred)| items.get(&id).map(|a| AudioInterval { id, preferred, start: a.start, end: a.end() }))
            .collect();
        drop(items);
        let transitions: Vec<_> = stored.audio_tracks.iter().map(|t| t.transitions.len()).collect();
        let plan = indexed_audio_plan(&audio, &transitions, &mut self.cache_edges_left)?;
        // No stored field has changed before this point. Work/refusal can only affect the budget/report.
        let Some(mut state) = self.audio_replay.remove(&parent) else { return Some(false) };
        for intent in intents {
            let id = intent.audio.id;
            state.unplaced.push(intent.audio);
            if let Some(element) = intent.diagnostics {
                state.diagnostics.insert(id, element);
            }
            if let Some(video) = intent.video {
                state.paired.insert(id, (video, None));
            }
        }
        state.unplaced.retain(|a| !rejected.contains(&a.id));
        state.paired.retain(|id, _| !rejected.contains(id));
        state.diagnostics.retain(|id, _| !rejected.contains(id));
        let Some(stored) = self.b.p.sequence_mut(parent) else {
            self.audio_replay.insert(parent, state);
            return Some(false);
        };
        let empty = empty_sequence(stored.settings.clone());
        let mut seq = std::mem::replace(stored, empty);
        let video_index = state.video_index.get_or_insert_with(|| {
            seq.video_tracks
                .iter()
                .enumerate()
                .flat_map(|(track, t)| t.items.iter().enumerate().map(move |(item, clip)| (clip.id, (track, item))))
                .collect()
        });
        for (audio, _) in &state.order {
            if let Some((video, Some(link))) = state.paired.get(audio)
                && let Some(&(track, item)) = video_index.get(video)
                && seq.video_tracks[track].items[item].link == Some(*link)
            {
                seq.video_tracks[track].items[item].link = None;
            }
        }
        let mut items: HashMap<_, _> = std::mem::take(&mut state.unplaced).into_iter().map(|a| (a.id, a)).collect();
        for track in &mut seq.audio_tracks {
            for item in std::mem::take(&mut track.items) {
                items.insert(item.id, item);
            }
        }
        for (id, track) in plan {
            let Some(mut audio) = items.remove(&id) else { continue };
            let pair = state.paired.get(&id).and_then(|(video, saved)| video_index.get(video).map(|&position| (position, *saved)));
            if let Some(track) = track
                && self.b.ensure_tracks(&mut seq, TrackKind::Audio, track + 1)
            {
                if let Some((position, saved)) = pair {
                    let (vtrack, item) = position;
                    let link = seq.video_tracks[vtrack].items[item].link.or(saved).unwrap_or_else(|| self.b.link_id());
                    audio.link = Some(link);
                    seq.audio_tracks[track].items.push(audio);
                    seq.video_tracks[vtrack].items[item].link = Some(link);
                    if let Some(pair) = state.paired.get_mut(&id) {
                        pair.1 = Some(link);
                    }
                } else {
                    seq.audio_tracks[track].items.push(audio);
                }
            } else {
                if pair.is_some() {
                    audio.link = None;
                }
                state.unplaced.push(audio);
            }
        }
        // Sorting was paid by the plan; video tracks were never reordered.
        for track in &mut seq.audio_tracks {
            track.sort();
        }
        let new_audio = !had_audio && seq.audio_tracks.iter().any(|t| !t.items.is_empty());
        if let Some(stored) = self.b.p.sequence_mut(parent) {
            *stored = seq;
        }
        self.audio_replay.insert(parent, state);
        Some(new_audio)
    }

    /// Nested sequence from a `<media>` resource.
    fn media(&mut self, id: &str) -> Option<ItemId> {
        // Check before the cache: a truncated resource may already have an item while it
        // is being rebuilt, but a back-edge must never turn that item into a self-cycle.
        if self.media_active.contains(id) {
            self.report.warn_limited(self.report_limit, format!("recursive FCPXML media resource {id:?} ignored"));
            return None;
        }
        let depth = self.media_active.len();
        let mut again = None;
        if let Some(i) = self.media_items.get(id).copied() {
            // Imported with a nested one cut off: import it again (into the same item) where that
            // one now fits.
            let shallower = self
                .media_truncated_at
                .get(id)
                .is_some_and(|&(media, xml)| depth <= media && self.depth <= xml && (depth < media || self.depth < xml))
                && !self.media_active.contains(id);
            if !shallower || self.reimports_left == 0 {
                if self.cache_reaches_active(i) {
                    self.depth_hits += 1;
                    self.report.warn_limited(
                        self.report_limit,
                        format!("FCPXML cached media {id:?} could form a recursive graph or exceeds the cache traversal budget; imported as a gap"),
                    );
                    return None;
                }
                return Some(i);
            }
            self.reimports_left -= 1;
            again = Some(i);
        }
        let n = *self.media_nodes.get(id)?;
        let s = child(n, "sequence")?;
        // The active set is the current nesting chain: a long acyclic chain of compound clips
        // would otherwise recurse once per resource.
        if depth >= MAX_MEDIA_DEPTH {
            self.depth_hits += 1;
            self.report.warn_limited(self.report_limit, format!("FCPXML media resource {id:?} is nested too deeply; imported as a gap"));
            return None;
        }
        self.media_active.insert(id.into());
        // Mark the resource before descending: a compound clip can reference itself through a
        // ref-clip before a completed sequence is available to cache.
        let name = n.attribute("name").unwrap_or("Compound Clip").to_string();
        let hits_before = self.depth_hits;
        let item = self.sequence(s, &name, None, again);
        self.media_active.remove(id);
        self.media_items.insert(id.into(), item);
        if self.depth_hits > hits_before {
            self.media_truncated_at.insert(id.into(), (depth, self.depth));
        } else {
            self.media_truncated_at.remove(id);
        }
        Some(item)
    }

    fn sequence(&mut self, s: Node<'a>, name: &str, bin: Option<BinId>, reuse: Option<ItemId>) -> ItemId {
        let (rate, w, h) = s.attribute("format").and_then(|f| self.formats.get(f)).copied().unwrap_or((FrameRate::default(), 1920, 1080));
        let df = s.attribute("tcFormat") == Some("DF");
        let mut settings = settings_for(rate, w, h, df);
        if let Some(sr) = s.attribute("audioRate").and_then(parse_audio_rate) {
            settings.sample_rate = sr;
        }
        let id = reuse.unwrap_or_else(|| self.b.reserve_sequence(name, settings.clone(), bin));
        // A rebuild replaces this parent's clips and their pending audio, not its item identity.
        self.pending_audio.remove(&id);
        self.audio_replay.remove(&id);
        let mut seq = empty_sequence(settings);
        let tc_start = attr_time(s, "tcStart").unwrap_or(Tick::ZERO);
        seq.start_timecode = rate.frame_at(tc_start);
        let ctx = Ctx { sequence: id, seq_rate: rate, frame: (w, h) };
        self.b.ensure_tracks(&mut seq, TrackKind::Video, 1);
        self.b.ensure_tracks(&mut seq, TrackKind::Audio, 1);
        if let Some(spine) = child(s, "spine") {
            // Sequence time origin is tcStart.
            self.storyline(&mut seq, &ctx, spine, -tc_start, 0, true);
        }
        seq.markers.sort_by_key(|m| m.start);
        self.b.put_sequence(id, seq);
        id
    }

    /// Lay out a storyline. `origin` maps the storyline's offsets to timeline time
    /// (`timeline = origin + offset`). `lane` is the storyline's lane (0 = primary).
    /// Refuses nesting past [`MAX_XML_DEPTH`].
    fn storyline(&mut self, seq: &mut Sequence, ctx: &Ctx, spine: Node<'a>, origin: Tick, lane: i32, primary: bool) {
        if self.depth >= MAX_XML_DEPTH {
            self.depth_hits += 1;
            self.report.warn_limited(
                self.report_limit,
                format!("XML storylines are nested too deeply (more than {MAX_XML_DEPTH}); deeper clips were skipped"),
            );
            return;
        }
        self.depth += 1;
        self.storyline_inner(seq, ctx, spine, origin, lane, primary);
        self.depth -= 1;
    }

    fn storyline_inner(&mut self, seq: &mut Sequence, ctx: &Ctx, spine: Node<'a>, origin: Tick, lane: i32, primary: bool) {
        // (timeline start, timeline end, video clip on this storyline's track)
        let mut laid: Vec<(Tick, Tick, Option<ClipId>)> = Vec::new();
        let mut transitions: Vec<(usize, Node)> = Vec::new();
        for el in elements(spine) {
            let tag = el.name();
            if tag == "transition" {
                transitions.push((laid.len(), el));
                continue;
            }
            let offset = attr_time(el, "offset").unwrap_or(Tick::ZERO);
            let t0 = origin + offset;
            let (end, clip) = self.element(seq, ctx, el, t0, lane, primary);
            laid.push((t0, end, clip));
        }
        let vidx = if lane >= 0 { lane as usize } else { usize::MAX };
        for (pos, el) in transitions {
            if vidx == usize::MAX {
                self.report.warn_limited(self.report_limit, "transitions on audio storylines are not supported");
                continue;
            }
            let start = origin + attr_time(el, "offset").unwrap_or(Tick::ZERO);
            let dur = attr_time(el, "duration").unwrap_or(Tick::ZERO);
            if dur <= Tick::ZERO {
                continue;
            }
            let name = child(el, "filter-video")
                .and_then(|f| f.attribute("ref").and_then(|r| self.effects.get(r).cloned()).or_else(|| f.attribute("name").map(str::to_string)))
                .filter(|n| !n.is_empty())
                .or_else(|| el.attribute("name").map(str::to_string))
                .unwrap_or_else(|| "Cross Dissolve".into());
            let prev = pos.checked_sub(1).and_then(|i| laid.get(i)).copied();
            let next = laid.get(pos).copied();
            let from = prev.and_then(|p| p.2.map(|c| (c, p.1)));
            let to = next.and_then(|n| n.2.map(|c| (c, n.0)));
            if from.is_none() && to.is_none() {
                continue;
            }
            let cut = to.map(|t| t.1).or(from.map(|f| f.1)).unwrap_or(start);
            let align = if cut == start {
                TransitionAlign::StartAtCut
            } else if cut == start + dur {
                TransitionAlign::EndAtCut
            } else {
                TransitionAlign::CenterAtCut
            };
            let mut temporary = Report::default();
            let effect = transition_effect(self.b.catalog, &name, false, &mut temporary);
            for e in temporary.entries {
                for _ in 0..e.count {
                    self.report.push_limited(e.level, self.report_limit, &e.message);
                }
            }
            let id = TransitionId(self.b.alloc());
            // The clips may have been moved to another track by placement; find the track holding them.
            let clip = from.or(to).map(|c| c.0);
            if let Some(t) = seq.video_tracks.iter_mut().find(|t| clip.is_some_and(|c| t.item(c).is_some())) {
                t.transitions.push(Transition {
                    id,
                    effect,
                    start,
                    duration: dur,
                    from: from.map(|f| f.0),
                    to: to.map(|t| t.0),
                    align,
                    reverse: false,
                    ext: None,
                });
                t.sort();
            }
        }
    }

    /// Lay out one storyline element at timeline time `t0`. Returns (timeline end, video clip).
    /// Refuses nesting past [`MAX_XML_DEPTH`].
    fn element(&mut self, seq: &mut Sequence, ctx: &Ctx, el: Node<'a>, t0: Tick, lane: i32, primary: bool) -> (Tick, Option<ClipId>) {
        if self.depth >= MAX_XML_DEPTH {
            self.depth_hits += 1;
            self.report.warn_limited(
                self.report_limit,
                format!("XML elements are nested too deeply (more than {MAX_XML_DEPTH}); deeper clips were skipped"),
            );
            return (t0, None);
        }
        self.depth += 1;
        let out = self.element_inner(seq, ctx, el, t0, lane, primary);
        self.depth -= 1;
        out
    }

    fn element_inner(&mut self, seq: &mut Sequence, ctx: &Ctx, el: Node<'a>, t0: Tick, lane: i32, primary: bool) -> (Tick, Option<ClipId>) {
        let tag = el.name();
        let dur = attr_time(el, "duration").unwrap_or(Tick::ZERO);
        let local_start = attr_time(el, "start").unwrap_or(Tick::ZERO);
        let mut vclip = None;
        let mut placed: Vec<(TrackKind, ClipId)> = Vec::new();
        let name = el.attribute("name").unwrap_or("Clip").to_string();
        // Media and source in.
        let mut incomplete_audio = false;
        let media: Option<(ItemId, Tick, bool, bool)> = match tag {
            "asset-clip" => el.attribute("ref").and_then(|r| self.asset(r, None)).map(|a| (a.item, a.start, a.has_video, a.has_audio)),
            "video" => {
                // FCPXML 1.10 DTD: video selects an asset's picture, or references a generator.
                // Our media model does not select additional video sources within an asset.
                let asset = el
                    .attribute("srcID")
                    .is_none_or(|id| id == "1")
                    .then(|| el.attribute("ref").and_then(|r| self.asset(r, None)))
                    .flatten()
                    .filter(|a| a.has_video);
                if asset.is_none() {
                    self.report
                        .warn_limited(self.report_limit, format!("FCPXML <video> \"{name}\" has no supported video asset/source; clip was skipped"));
                }
                asset.map(|a| (a.item, a.start, true, false))
            }
            "clip" => {
                // A clip wraps video/audio/asset-clip children that reference the asset.
                let inner = elements(el).find(|c| matches!(c.name(), "video" | "audio" | "asset-clip") && c.attribute("ref").is_some());
                inner.and_then(|c| {
                    let a = self.asset(c.attribute("ref").unwrap_or(""), None)?;
                    // inner local time = inner.start + (t - inner.offset)
                    let shift = attr_time(c, "start").unwrap_or(Tick::ZERO) - attr_time(c, "offset").unwrap_or(Tick::ZERO);
                    let v = a.has_video && c.name() != "audio";
                    let au = a.has_audio && (c.name() != "video" || elements(el).any(|x| x.has_tag_name("audio")));
                    Some((a.item, a.start - shift, v, au))
                })
            }
            "ref-clip" => el.attribute("ref").and_then(|r| {
                let item = self.media(r)?;
                let tc = self.b.p.sequence(item).map(|s| s.settings.frame_rate.tick_of(s.start_timecode)).unwrap_or(Tick::ZERO);
                let has_audio = self.b.p.sequence(item).is_some_and(|s| s.audio_tracks.iter().any(|t| !t.items.is_empty()));
                incomplete_audio = !has_audio && (self.media_truncated_at.contains_key(r) || self.pending_audio.contains_key(&item));
                Some((item, tc, true, has_audio))
            }),
            "gap" => None,
            "title" | "audio" | "sync-clip" | "mc-clip" | "live-drawing" => {
                self.report.warn_limited(self.report_limit, format!("FCPXML <{tag}> elements are not supported and were skipped"));
                None
            }
            _ => None,
        };
        let (speed, reverse, hold, src_base) = time_map(el, local_start);
        if let Some((item, origin, has_v, has_a)) = media
            && dur > Tick::ZERO
        {
            let src_in = src_base - origin;
            let src_enable = el.attribute("srcEnable").unwrap_or("all");
            let want_v = has_v && src_enable != "audio" && lane >= 0;
            let want_a = has_a && src_enable != "video";
            let defer_audio = incomplete_audio && src_enable != "video";
            let link = if want_v && want_a { Some(self.b.link_id()) } else { None };
            for (kind, want) in [(TrackKind::Video, want_v), (TrackKind::Audio, want_a)] {
                let deferred = kind == TrackKind::Audio && defer_audio;
                if !want && !deferred {
                    continue;
                }
                let mut ti = self.b.clip(item, kind, &name, t0, dur, src_in.max(Tick::ZERO));
                ti.speed = speed;
                ti.reverse = reverse;
                if hold {
                    ti.frame_hold = Some(src_in.max(Tick::ZERO));
                }
                ti.link = link;
                ti.enabled = flag(el, "enabled").unwrap_or(true);
                if let Some(l) = el.attribute("label").and_then(Label::from_name) {
                    ti.label = l;
                }
                self.adjustments(&mut ti, el, LocalTiming { local_start, src_in, speed }, ctx, kind, deferred);
                if !primary || kind == TrackKind::Audio {
                    ti.markers = self.markers(el, local_start, src_in, speed, ctx.seq_rate);
                }
                let pref = match kind {
                    TrackKind::Video => lane.max(0) as usize,
                    TrackKind::Audio => {
                        if lane < 0 {
                            lane.unsigned_abs() as usize - 1
                        } else {
                            lane as usize
                        }
                    }
                };
                if kind == TrackKind::Audio {
                    let replay = self.audio_replay.entry(ctx.sequence).or_default();
                    if pref >= MAX_IMPORTED_TRACKS {
                        replay.final_cap_skips += 1;
                        continue;
                    }
                    replay.order.push((ti.id, pref));
                    if !deferred && let Some(video) = vclip {
                        replay.paired.insert(ti.id, (video, link));
                    }
                }
                if deferred {
                    self.pending_audio.entry(ctx.sequence).or_default().push(PendingAudio {
                        parent: ctx.sequence,
                        child: item,
                        audio: ti,
                        video: vclip,
                        diagnostics: Some(el),
                    });
                    continue;
                }
                let id = ti.id;
                let track = if kind == TrackKind::Audio {
                    let (track, unplaced) = self.b.place_retaining(seq, kind, pref, ti);
                    if let Some(item) = unplaced {
                        self.audio_replay.entry(ctx.sequence).or_default().unplaced.push(item);
                    }
                    track
                } else {
                    self.b.place(seq, kind, pref, ti)
                };
                if track.is_none() {
                    if kind != TrackKind::Audio {
                        self.report.warn_limited(
                            self.report_limit,
                            format!("FCPXML {kind:?} lane exceeds the {MAX_IMPORTED_TRACKS}-track import limit; clip was skipped"),
                        );
                    }
                    continue;
                }
                placed.push((kind, id));
                if kind == TrackKind::Video {
                    vclip = Some(id);
                }
            }
        }
        if primary {
            // Markers on primary storyline elements are timeline markers.
            for m in self.markers(el, local_start, Tick::ZERO, 1.0, ctx.seq_rate) {
                seq.markers.push(Marker { start: t0 + m.start, ..m });
            }
        }
        // Connected clips and secondary storylines.
        for c in elements(el) {
            let Some(l) = c.attribute("lane").and_then(|l| l.parse::<i32>().ok()) else { continue };
            let off = attr_time(c, "offset").unwrap_or(Tick::ZERO);
            let ct0 = t0 + (off - local_start);
            let clane = if lane > 0 && l > 0 { lane.saturating_add(l) } else { l };
            if c.has_tag_name("spine") {
                // Secondary storyline: children offsets are in the storyline's local time.
                let first = elements(c).filter(|x| !x.has_tag_name("transition")).find_map(|x| attr_time(x, "offset")).unwrap_or(Tick::ZERO);
                self.storyline(seq, ctx, c, ct0 - first, clane, false);
            } else {
                self.element(seq, ctx, c, ct0, clane, false);
            }
        }
        let _ = placed;
        (t0 + dur, vclip)
    }

    fn markers(&mut self, el: Node, local_start: Tick, src_in: Tick, speed: f64, rate: FrameRate) -> Vec<Marker> {
        let mut out = Vec::new();
        // FCPXML has no zero-duration marker: a point is written as one frame at the sequence rate.
        let frame = rate.frame_duration();
        for m in elements(el).filter(|m| matches!(m.name(), "marker" | "chapter-marker")) {
            let rel = attr_time(m, "start").unwrap_or(Tick::ZERO) - local_start;
            let start = if src_in == Tick::ZERO && speed == 1.0 { rel } else { src_in + scaled(rel, speed) };
            let d = attr_time(m, "duration").unwrap_or(Tick::ZERO);
            out.push(Marker {
                url: String::new(),
                author: String::new(),
                ext: None,
                id: MarkerId(self.b.alloc()),
                start,
                duration: if d <= frame { Tick::ZERO } else { d },
                name: m.attribute("value").unwrap_or("").to_string(),
                comment: m.attribute("note").unwrap_or("").to_string(),
                kind: if m.has_tag_name("chapter-marker") { MarkerKind::Chapter } else { MarkerKind::Comment },
                color: if m.has_tag_name("chapter-marker") { Label::Mango } else { Label::Blue },
            });
        }
        out
    }

    fn adjustments(&mut self, ti: &mut TrackItem, el: Node, timing: LocalTiming, ctx: &Ctx, kind: TrackKind, defer_diagnostics: bool) {
        let LocalTiming { local_start, src_in, speed } = timing;
        // keyframe time (element-local) -> media time
        let kt = |t: Tick| src_in + scaled(t - local_start, speed);
        let (fw, fh) = (ctx.frame.0 as f64, ctx.frame.1 as f64);
        let _ = ctx.seq_rate;
        for a in elements(el) {
            match (a.name(), kind) {
                ("adjust-transform", TrackKind::Video) => {
                    let to_px = move |v: &str| {
                        let (x, y) = pair(v);
                        Vec2::new(fw / 2.0 + x * fh / 100.0, fh / 2.0 - y * fh / 100.0)
                    };
                    if let Some(p) = self.kparam(a, "position", &kt, |v| ParamValue::Vec2(to_px(v)))
                        && (p.is_animated() || p.value.as_vec2().is_some_and(|v| (v.x - fw / 2.0).abs() > 1e-9 || (v.y - fh / 2.0).abs() > 1e-9))
                    {
                        set_param(self.b.catalog, ti, "motion", "position", p);
                    }
                    let mut nonuniform = false;
                    if let Some(p) = self.kparam(a, "scale", &kt, |v| {
                        let (x, y) = pair(v);
                        nonuniform |= (x - y).abs() > 1e-9;
                        ParamValue::Float(x * 100.0)
                    }) {
                        set_param(self.b.catalog, ti, "motion", "scale", p);
                    }
                    if nonuniform {
                        self.report.warn_limited(self.report_limit, "non-uniform scale was imported as uniform scale");
                    }
                    if let Some(p) = self.kparam(a, "rotation", &kt, |v| ParamValue::Float(-v.trim().parse::<f64>().unwrap_or(0.0))) {
                        set_param(self.b.catalog, ti, "motion", "rotation", p);
                    }
                    if let Some(p) = self.kparam(a, "anchor", &kt, |v| ParamValue::Vec2(to_px(v)))
                        && p.value.as_vec2().is_some_and(|v| (v.x - fw / 2.0).abs() > 1e-9 || (v.y - fh / 2.0).abs() > 1e-9)
                    {
                        set_param(self.b.catalog, ti, "motion", "anchor", p);
                    }
                }
                ("adjust-blend", TrackKind::Video) => {
                    if let Some(p) = self.kparam(a, "amount", &kt, |v| ParamValue::Float(v.trim().parse::<f64>().unwrap_or(1.0) * 100.0)) {
                        set_param(self.b.catalog, ti, "opacity", "opacity", p);
                    }
                }
                ("adjust-volume", TrackKind::Audio) => {
                    if let Some(p) = self.kparam(a, "amount", &kt, |v| ParamValue::Float(parse_db(v))) {
                        set_param(self.b.catalog, ti, "volume", "level", p);
                    }
                }
                ("filter-video", TrackKind::Video) | ("filter-audio", TrackKind::Audio) => {
                    let name = filter_name(a, &self.effects);
                    match self.b.catalog.effect_by_name(&name).filter(|d| !d.intrinsic) {
                        Some(d) => {
                            let mut inst = d.instance();
                            inst.enabled = flag(a, "enabled").unwrap_or(true);
                            for p in children(a, "param") {
                                let (Some(k), Some(v)) = (p.attribute("key").or(p.attribute("name")), p.attribute("value")) else { continue };
                                if let (Some(pd), Ok(f)) = (d.params.iter().find(|x| x.id == k || x.label.eq_ignore_ascii_case(k)), v.parse::<f64>())
                                    && matches!(pd.default, ParamValue::Float(_))
                                {
                                    inst.params.insert(pd.id.clone(), Param::new(ParamValue::Float(f)));
                                }
                            }
                            ti.effects.push(inst);
                        }
                        None if !defer_diagnostics => {
                            self.report.warn_limited(self.report_limit, format!("effect \"{name}\" is not supported and was skipped"));
                        }
                        None => {}
                    }
                }
                _ => {}
            }
        }
    }

    /// A parameter from an adjustment attribute plus optional `<param name><keyframeAnimation>`.
    fn kparam(&mut self, a: Node, name: &str, kt: &dyn Fn(Tick) -> Tick, mut map: impl FnMut(&str) -> ParamValue) -> Option<Param> {
        let attr = a.attribute(name);
        let pnode = children(a, "param").find(|p| p.attribute("name") == Some(name));
        let mut p = Param::new(map(attr.or(pnode.and_then(|p| p.attribute("value")))?));
        if let Some(anim) = pnode.and_then(|p| child(p, "keyframeAnimation")) {
            for k in children(anim, "keyframe") {
                let (Some(t), Some(v)) = (attr_time(k, "time"), k.attribute("value")) else { continue };
                let mut kf = Keyframe::new(kt(t), map(v));
                if k.attribute("interp") == Some("hold") {
                    kf.interp = Interpolation::Hold;
                }
                p.keyframes.push(kf);
            }
            p.keyframes.sort_by_key(|k| k.time);
            if let Some(k) = p.keyframes.first() {
                p.value = k.value.clone();
            }
        }
        Some(p)
    }
}

fn filter_name(element: Node, effects: &HashMap<String, String>) -> String {
    element.attribute("ref").and_then(|r| effects.get(r).cloned()).or_else(|| element.attribute("name").map(str::to_string)).unwrap_or_default()
}

/// Keep only the source element while audio is pending. Applying the global limit here,
/// after placement, preserves counts of existing messages without retaining warning lists.
fn report_audio_adjustments(element: Node, effects: &HashMap<String, String>, catalog: &dyn Catalog, report: &mut Report, limit: Option<usize>) {
    for filter in children(element, "filter-audio") {
        let name = filter_name(filter, effects);
        if catalog.effect_by_name(&name).filter(|d| !d.intrinsic).is_none() {
            report.warn_limited(limit, format!("effect \"{name}\" is not supported and was skipped"));
        }
    }
}

fn pair(v: &str) -> (f64, f64) {
    let mut it = v.split_whitespace().filter_map(|s| s.parse::<f64>().ok());
    let x = it.next().unwrap_or(0.0);
    (x, it.next().unwrap_or(x))
}

fn parse_db(v: &str) -> f64 {
    let s = v.trim();
    let s = s.strip_suffix("dB").or_else(|| s.strip_suffix("db")).unwrap_or(s);
    s.trim().parse().unwrap_or(0.0)
}

/// (speed, reverse, freeze, source time at the element's local start) from a `timeMap`.
fn time_map(el: Node, local_start: Tick) -> (f64, bool, bool, Tick) {
    let Some(tm) = child(el, "timeMap") else { return (1.0, false, false, local_start) };
    let pts: Vec<(Tick, Tick)> = children(tm, "timept").filter_map(|p| Some((attr_time(p, "time")?, attr_time(p, "value")?))).collect();
    if pts.len() < 2 {
        return (1.0, false, false, pts.first().map(|p| p.1).unwrap_or(local_start));
    }
    let (t0, v0) = pts[0];
    let (t1, v1) = pts[pts.len() - 1];
    let dt = (t1 - t0).0 as f64;
    let dv = (v1 - v0).0 as f64;
    let at_start = |s: f64| v0 + Tick(((local_start - t0).0 as f64 * s).round() as i64);
    if dv == 0.0 {
        return (1.0, false, true, v0);
    }
    let s = (dv / dt * 1_000_000.0).round() / 1_000_000.0;
    if s < 0.0 {
        // Reverse: source runs from v0 down; our source_in is the lowest source time used.
        let speed = -s;
        let hi = at_start(s);
        let len = (t1 - local_start).max(Tick::ZERO);
        return (speed, true, false, hi - scaled(len, speed));
    }
    (s, false, false, at_start(s))
}

// ---------------------------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------------------------

struct Res {
    id: String,
}

struct Exp<'a, 'r> {
    p: &'a Project,
    opts: &'a ExportOptions<'a>,
    report: &'r mut Report,
    report_limit: Option<usize>,
    formats: Vec<(String, FrameRate, u32, u32)>,
    assets: HashMap<ItemId, Res>,
    medias: HashMap<ItemId, Res>,
    effects: HashMap<String, Res>,
    next: usize,
    /// Resource XML fragments, written before the library.
    res_out: Vec<String>,
    building: Vec<ItemId>,
}

/// Write FCPXML with an optional distinct-diagnostic limit ([`export`] has none).
pub fn export_with_report_limit(
    p: &Project,
    seq_id: ItemId,
    opts: &ExportOptions,
    report: &mut Report,
    report_limit: Option<usize>,
) -> Result<String> {
    let Some(seq) = p.sequence(seq_id) else { return Err(Error::NoSequence(seq_id)) };
    let mut x = Exp {
        p,
        opts,
        report,
        report_limit,
        formats: Vec::new(),
        assets: HashMap::new(),
        medias: HashMap::new(),
        effects: HashMap::new(),
        next: 1,
        res_out: Vec::new(),
        building: Vec::new(),
    };
    let fmt = x.format(seq.settings.frame_rate, seq.settings.width, seq.settings.height);
    let name = opts.name.map(str::to_string).unwrap_or_else(|| p.item(seq_id).map(|i| i.name.clone()).unwrap_or_default());
    let body = x.sequence_xml(seq_id, &fmt, 4);
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE fcpxml>\n<fcpxml version=\"{}\">\n  <resources>\n",
        opts.fcpxml_version.as_str()
    );
    for r in &x.res_out {
        out.push_str(r);
    }
    out.push_str("  </resources>\n");
    out.push_str(&format!("  <library>\n    <event name=\"{}\">\n", escape_attr(&p.name)));
    out.push_str(&format!("      <project name=\"{}\">\n", escape_attr(&name)));
    out.push_str(&body);
    out.push_str("      </project>\n    </event>\n  </library>\n</fcpxml>\n");
    Ok(out)
}

/// Write the sequence `seq_id` as FCPXML; what it cannot carry is in `report`.
pub fn export(p: &Project, seq_id: ItemId, opts: &ExportOptions, report: &mut Report) -> Result<String> {
    export_with_report_limit(p, seq_id, opts, report, None)
}

/// One storyline element being written.
struct El {
    start: Tick,
    end: Tick,
    /// Local time of the element at its start.
    local: Tick,
    xml_open: String,
    children: Vec<String>,
    close: String,
}

impl Exp<'_, '_> {
    fn rid(&mut self) -> String {
        let r = format!("r{}", self.next);
        self.next += 1;
        r
    }

    fn format(&mut self, rate: FrameRate, w: u32, h: u32) -> String {
        if let Some(f) = self.formats.iter().find(|f| f.1 == rate && f.2 == w && f.3 == h) {
            return f.0.clone();
        }
        let id = self.rid();
        self.res_out.push(format!(
            "    <format id=\"{id}\" name=\"FFVideoFormat{h}p{}\" frameDuration=\"{}/{}s\" width=\"{w}\" height=\"{h}\"/>\n",
            rate.label().replace('.', ""),
            rate.den,
            rate.num
        ));
        self.formats.push((id.clone(), rate, w, h));
        id
    }

    fn asset(&mut self, item: ItemId, rate: FrameRate) -> String {
        if let Some(r) = self.assets.get(&item) {
            return r.id.clone();
        }
        let Some(m) = item_media(self.p, item) else {
            self.report.warn_limited(self.report_limit, "clip without media skipped");
            return String::new();
        };
        let name = self.p.item(item).map(|i| i.name.clone()).unwrap_or_default();
        let fmt = m.info.video.as_ref().map(|v| self.format(m.interpret_frame_rate.unwrap_or(v.frame_rate), v.width, v.height));
        let id = self.rid();
        let mrate = m.info.video.as_ref().map(|v| v.frame_rate).unwrap_or(rate);
        let start = m.info.start_timecode.map(|f| mrate.tick_of(f)).unwrap_or(Tick::ZERO);
        let mut a = format!(
            "    <asset id=\"{id}\" name=\"{}\" start=\"{}\" duration=\"{}\" hasVideo=\"{}\" hasAudio=\"{}\"",
            escape_attr(&name),
            fmt_time(start, mrate),
            fmt_time(m.info.duration, mrate),
            m.info.video.is_some() as u8,
            m.info.has_audio() as u8
        );
        if let Some(f) = fmt {
            a.push_str(&format!(" format=\"{f}\""));
        }
        if let Some(au) = m.info.audio() {
            a.push_str(&format!(" audioSources=\"1\" audioChannels=\"{}\" audioRate=\"{}\"", au.channels, au.sample_rate));
        }
        let path = item_path(self.p, item).unwrap_or("");
        let src = if crate::path::is_absolute(path) { path_to_file_url(path, false) } else { path.to_string() };
        a.push_str(&format!(">\n      <media-rep kind=\"original-media\" src=\"{}\"/>\n    </asset>\n", escape_attr(&src)));
        self.res_out.push(a);
        self.assets.insert(item, Res { id: id.clone() });
        let _ = file_name;
        id
    }

    fn media(&mut self, item: ItemId) -> String {
        if let Some(r) = self.medias.get(&item) {
            return r.id.clone();
        }
        let Some(s) = self.p.sequence(item) else {
            self.report.warn_limited(self.report_limit, "missing nested sequence skipped");
            return String::new();
        };
        let fmt = self.format(s.settings.frame_rate, s.settings.width, s.settings.height);
        let id = self.rid();
        self.medias.insert(item, Res { id: id.clone() });
        self.building.push(item);
        let body = self.sequence_xml(item, &fmt, 3);
        self.building.pop();
        let name = self.p.item(item).map(|i| i.name.clone()).unwrap_or_default();
        self.res_out.push(format!("    <media id=\"{id}\" name=\"{}\">\n{body}    </media>\n", escape_attr(&name)));
        id
    }

    fn effect(&mut self, name: &str, id_hint: &str) -> String {
        if let Some(r) = self.effects.get(name) {
            return r.id.clone();
        }
        let id = self.rid();
        self.res_out.push(format!(
            "    <effect id=\"{id}\" name=\"{}\" uid=\"{}.{}\"/>\n",
            escape_attr(name),
            escape_attr(self.opts.namespace),
            escape_attr(id_hint)
        ));
        self.effects.insert(name.to_string(), Res { id: id.clone() });
        id
    }

    fn sequence_xml(&mut self, seq_id: ItemId, fmt: &str, indent: usize) -> String {
        let Some(seq) = self.p.sequence(seq_id) else { return String::new() };
        let rate = seq.settings.frame_rate;
        let df = seq.settings.drop_frame && rate.supports_drop_frame();
        let tc0 = rate.tick_of(seq.start_timecode);
        let pad = "  ".repeat(indent);
        let mut o = format!(
            "{pad}<sequence format=\"{fmt}\" duration=\"{}\" tcStart=\"{}\" tcFormat=\"{}\" audioLayout=\"stereo\" audioRate=\"{}\">\n{pad}  <spine>\n",
            fmt_time(seq.duration(), rate),
            fmt_time(tc0, rate),
            if df { "DF" } else { "NDF" },
            audio_rate_str(seq.settings.sample_rate)
        );
        // Which audio items merge into video clips (same index track, linked, identical timing).
        let mut merged: HashMap<ClipId, ClipId> = HashMap::new();
        for (vi, vt) in seq.video_tracks.iter().enumerate() {
            let Some(at) = seq.audio_tracks.get(vi) else { continue };
            for v in &vt.items {
                if let Some(a) = at.items.iter().find(|a| {
                    a.link.is_some()
                        && a.link == v.link
                        && a.item == v.item
                        && a.start == v.start
                        && a.duration == v.duration
                        && a.source_in == v.source_in
                        && a.speed == v.speed
                        && a.reverse == v.reverse
                        && a.frame_hold == v.frame_hold
                        && a.enabled == v.enabled
                }) {
                    merged.insert(v.id, a.id);
                }
            }
        }
        let merged_audio: std::collections::HashSet<_> = merged.values().copied().collect();
        if seq.all_tracks().skip(1).any(|t| !t.transitions.is_empty()) {
            self.report.warn_limited(self.report_limit, "FCPXML export keeps transitions on V1 only; transitions on other tracks were dropped");
        }
        // Primary storyline from V1 (gaps fill holes, plus a trailing gap covering connected clips).
        let v1 = seq.video_tracks.first();
        let mut els: Vec<El> = Vec::new();
        let mut trans_after: Vec<(usize, String)> = Vec::new();
        let mut t = Tick::ZERO;
        let push_gap = |els: &mut Vec<El>, a: Tick, b: Tick| {
            els.push(El {
                start: a,
                end: b,
                local: tc0 + a,
                xml_open: format!(
                    "<gap name=\"Gap\" offset=\"{}\" start=\"{}\" duration=\"{}\"",
                    fmt_time(tc0 + a, rate),
                    fmt_time(tc0 + a, rate),
                    fmt_time(b - a, rate)
                ),
                children: vec![],
                close: "gap".into(),
            })
        };
        if let Some(v1) = v1 {
            for it in &v1.items {
                if it.start > t {
                    push_gap(&mut els, t, it.start);
                }
                for tr in v1.transitions.iter().filter(|tr| tr.to == Some(it.id)) {
                    trans_after.push((els.len(), self.transition_xml(tr, tc0, rate)));
                }
                let audio = merged.get(&it.id).and_then(|a| seq.audio_tracks[0].item(*a));
                let el = self.clip_el(it, audio, TrackKind::Video, ClipPlacement { offset: tc0 + it.start, lane: None, primary: true }, seq);
                els.push(el);
                for tr in v1.transitions.iter().filter(|tr| tr.from == Some(it.id) && tr.to.is_none()) {
                    trans_after.push((els.len(), self.transition_xml(tr, tc0, rate)));
                }
                t = it.end();
            }
        }
        let last_start = seq.all_tracks().skip(1).flat_map(|tr| tr.items.iter().map(|i| i.start)).chain(seq.markers.iter().map(|m| m.start)).max();
        if let Some(ls) = last_start
            && ls >= t
        {
            let end = seq.duration().max(ls + rate.frame_duration());
            push_gap(&mut els, t, end);
        }
        // Connected clips.
        for (kind, tracks) in [(TrackKind::Video, &seq.video_tracks), (TrackKind::Audio, &seq.audio_tracks)] {
            for (ti, track) in tracks.iter().enumerate() {
                if kind == TrackKind::Video && ti == 0 {
                    continue;
                }
                let lane = if kind == TrackKind::Video { ti as i32 } else { -(ti as i32 + 1) };
                for it in &track.items {
                    if merged_audio.contains(&it.id) {
                        continue;
                    }
                    if let Some(what) = self.unsupported(it, rate) {
                        self.report.warn_limited(self.report_limit, format!("{what}: it is left out and is not read back on import"));
                        continue;
                    }
                    let Some(pi) = els.iter().position(|e| e.start <= it.start && it.start < e.end) else {
                        self.report.warn_limited(self.report_limit, "a connected clip could not be attached and was dropped");
                        continue;
                    };
                    let local = els[pi].local + (it.start - els[pi].start);
                    let audio = if kind == TrackKind::Video {
                        merged.get(&it.id).and_then(|a| seq.audio_tracks.get(ti).and_then(|t| t.item(*a)))
                    } else {
                        None
                    };
                    let el = self.clip_el(it, audio, kind, ClipPlacement { offset: local, lane: Some(lane), primary: false }, seq);
                    let s = self.render_el(&el, 0);
                    els[pi].children.push(s);
                }
            }
        }
        // Sequence markers on the covering primary element.
        for m in &seq.markers {
            match els.iter().position(|e| e.start <= m.start && m.start < e.end) {
                Some(pi) => {
                    let local = els[pi].local + (m.start - els[pi].start);
                    let s = marker_xml(m, local, rate);
                    els[pi].children.push(s);
                }
                None => self.report.warn_limited(self.report_limit, "a sequence marker past the end of the timeline was dropped"),
            }
        }
        let inner = format!("{pad}    ");
        let mut ti = 0;
        for (i, e) in els.iter().enumerate() {
            while ti < trans_after.len() && trans_after[ti].0 == i {
                o.push_str(&format!("{inner}{}\n", trans_after[ti].1));
                ti += 1;
            }
            o.push_str(&self.render_el(e, indent + 2));
        }
        while ti < trans_after.len() {
            o.push_str(&format!("{inner}{}\n", trans_after[ti].1));
            ti += 1;
        }
        o.push_str(&format!("{pad}  </spine>\n{pad}</sequence>\n"));
        o
    }

    fn render_el(&self, e: &El, indent: usize) -> String {
        let pad = "  ".repeat(indent);
        if e.children.is_empty() {
            return format!("{pad}{}/>\n", e.xml_open);
        }
        let mut s = format!("{pad}{}>\n", e.xml_open);
        for c in &e.children {
            for line in c.lines() {
                s.push_str(&format!("{pad}  {line}\n"));
            }
        }
        s.push_str(&format!("{pad}</{}>\n", e.close));
        s
    }

    fn transition_xml(&mut self, tr: &Transition, tc0: Tick, rate: FrameRate) -> String {
        let name = transition_name(self.opts.catalog, &tr.effect);
        let r = self.effect(&name, &tr.effect.effect);
        format!(
            "<transition name=\"{n}\" offset=\"{}\" duration=\"{}\"><filter-video ref=\"{r}\" name=\"{n}\"/></transition>",
            fmt_time(tc0 + tr.start, rate),
            fmt_time(tr.duration, rate),
            n = escape_attr(&name)
        )
    }

    /// How the report names a clip FCPXML cannot carry (a generator, a title, an adjustment layer); `None` for any
    /// other clip. Such a clip has no asset to reference, and an empty `ref` is not a valid IDREF: Final Cut Pro
    /// rejects the whole file. A `<gap>` exists only in a spine, so a connected one is left out instead.
    fn unsupported(&self, it: &TrackItem, rate: FrameRate) -> Option<String> {
        if let Some(g) = generator_of(self.p, base_item(self.p, it.item)) {
            return Some(format!("generator \"{}\" is not supported in FCPXML", g.label()));
        }
        crate::build::uncarried_clip(self.p, it, rate).map(|clip| format!("{clip} is not supported in FCPXML"))
    }

    /// A clip element at `offset` (in its parent's local time).
    fn clip_el(&mut self, it: &TrackItem, audio: Option<&TrackItem>, kind: TrackKind, at: ClipPlacement, seq: &Sequence) -> El {
        let ClipPlacement { offset, lane, primary } = at;
        let rate = seq.settings.frame_rate;
        let base = base_item(self.p, it.item);
        let is_seq = matches!(self.p.item(base).map(|i| &i.kind), Some(ItemKind::Sequence(_)));
        let lane_attr = lane.map(|l| format!(" lane=\"{l}\"")).unwrap_or_default();
        let name = escape_attr(&it.name);
        // In the primary storyline, a clip FCPXML cannot carry becomes a gap that keeps the timing
        // (connected ones never get here: see `unsupported`).
        if let Some(what) = self.unsupported(it, rate) {
            self.report.warn_limited(self.report_limit, format!("{what}: it is written as a gap and is not read back on import"));
            return gap_el(it, &name, offset, rate);
        }
        // Local timeline of the element: origin + source time (retimed clips use a timeMap).
        let (tag, r, origin) = if is_seq {
            let r = if self.building.contains(&base) {
                self.report.warn_limited(self.report_limit, "recursive nested sequence");
                String::new()
            } else {
                self.media(base)
            };
            let origin = self.p.sequence(base).map(|s| s.settings.frame_rate.tick_of(s.start_timecode)).unwrap_or(Tick::ZERO);
            ("ref-clip", r, origin)
        } else {
            let r = self.asset(base, rate);
            let m = item_media(self.p, base);
            let mrate = m.and_then(|m| m.info.video.as_ref().map(|v| v.frame_rate)).unwrap_or(rate);
            let st = m.and_then(|m| m.info.start_timecode).map(|f| mrate.tick_of(f)).unwrap_or(Tick::ZERO);
            ("asset-clip", r, st)
        };
        let retimed = it.speed != 1.0 || it.reverse || it.frame_hold.is_some();
        let local = origin + it.source_in;
        let has_audio = item_media(self.p, base).is_some_and(|m| m.info.has_audio())
            || self.p.sequence(base).is_some_and(|s| s.audio_tracks.iter().any(|t| !t.items.is_empty()));
        let has_video = item_media(self.p, base).is_none_or(|m| m.info.video.is_some());
        let src_enable = match (kind, audio.is_some()) {
            (TrackKind::Video, true) => "",
            (TrackKind::Video, false) if has_audio => " srcEnable=\"video\"",
            (TrackKind::Audio, _) if has_video => " srcEnable=\"audio\"",
            _ => "",
        };
        let enabled = if it.enabled { "" } else { " enabled=\"0\"" };
        let mut children = Vec::new();
        if retimed {
            let v0 = origin + it.frame_hold.unwrap_or(it.source_in);
            let (a, b) = if let Some(h) = it.frame_hold {
                (origin + h, origin + h)
            } else if it.reverse {
                (v0 + scaled(it.duration, it.speed), v0)
            } else {
                (v0, v0 + scaled(it.duration, it.speed))
            };
            children.push(format!(
                "<timeMap>\n  <timept time=\"{}\" value=\"{}\" interp=\"linear\"/>\n  <timept time=\"{}\" value=\"{}\" interp=\"linear\"/>\n</timeMap>",
                fmt_time(local, rate),
                fmt_time(a, rate),
                fmt_time(local + it.duration, rate),
                fmt_time(b, rate)
            ));
        }
        // keyframe media time -> element local time
        let to_local = |t: Tick| -> Tick {
            if it.speed == 0.0 || it.frame_hold.is_some() { local } else { local + Tick(((t - it.source_in).0 as f64 / it.speed).round() as i64) }
        };
        if kind == TrackKind::Video {
            self.transform(it, seq, &to_local, &mut children);
        }
        if let Some(a) = audio.or((kind == TrackKind::Audio).then_some(it)) {
            if param_modified(self.opts.catalog, a, "volume", "level") || a.gain_db != 0.0 {
                let g = a.gain_db;
                children.push(kparam_xml(
                    "adjust-volume",
                    "amount",
                    param(a, "volume", "level"),
                    rate,
                    &to_local,
                    |v| format!("{}dB", num(v.as_f64().unwrap_or(0.0) + g)),
                    "0dB",
                ));
            }
            if param_modified(self.opts.catalog, a, "panner", "balance")
                || param_modified(self.opts.catalog, a, "channel_volume", "left")
                || param_modified(self.opts.catalog, a, "channel_volume", "right")
            {
                self.report.warn_limited(self.report_limit, "pan and channel volume are not exported to FCPXML");
            }
        }
        for e in standard_effects(self.opts.catalog, it) {
            let n = self.opts.catalog.effect(&e.effect).map(|d| d.name).unwrap_or_else(|| e.effect.clone());
            let r = self.effect(&n, &e.effect);
            let tag = if kind == TrackKind::Video { "filter-video" } else { "filter-audio" };
            let mut s = format!("<{tag} ref=\"{r}\" name=\"{}\"", escape_attr(&n));
            if !e.enabled {
                s.push_str(" enabled=\"0\"");
            }
            let floats: Vec<_> =
                e.params.iter().filter_map(|(k, p)| p.value.as_f64().filter(|_| matches!(p.value, ParamValue::Float(_))).map(|v| (k, v))).collect();
            if floats.is_empty() {
                s.push_str("/>");
            } else {
                s.push_str(">\n");
                for (k, v) in floats {
                    s.push_str(&format!("  <param name=\"{k}\" key=\"{k}\" value=\"{}\"/>\n", num(v)));
                }
                s.push_str(&format!("</{tag}>"));
            }
            self.report.info_limited(self.report_limit, format!("effect \"{n}\" written as an FCPXML filter reference (FilmCraft-specific)"));
            children.push(s);
        }
        if !primary || kind == TrackKind::Audio {
            for m in &it.markers {
                children.push(marker_xml(m, to_local(m.start), rate));
            }
        } else if !it.markers.is_empty() {
            self.report.info_limited(self.report_limit, "clip markers on V1 are written as primary-storyline markers and import as sequence markers");
            for m in &it.markers {
                children.push(marker_xml(m, to_local(m.start), rate));
            }
        }
        El {
            start: it.start,
            end: it.end(),
            local,
            xml_open: format!(
                "<{tag} ref=\"{r}\"{lane_attr} name=\"{name}\" offset=\"{}\" start=\"{}\" duration=\"{}\"{src_enable}{enabled}",
                fmt_time(offset, rate),
                fmt_time(local, rate),
                fmt_time(it.duration, rate)
            ),
            children,
            close: tag.into(),
        }
    }

    fn transform(&mut self, it: &TrackItem, seq: &Sequence, to_local: &dyn Fn(Tick) -> Tick, out: &mut Vec<String>) {
        let rate = seq.settings.frame_rate;
        let (fw, fh) = (seq.settings.width as f64, seq.settings.height as f64);
        let pos = |v: &ParamValue| {
            let p = v.as_vec2().unwrap_or(Vec2::new(f64::NAN, f64::NAN));
            let x = if p.x.is_nan() { 0.0 } else { (p.x - fw / 2.0) / fh * 100.0 };
            let y = if p.y.is_nan() { 0.0 } else { -(p.y - fh / 2.0) / fh * 100.0 };
            format!("{} {}", num(x), num(y))
        };
        let changed = ["position", "scale", "rotation", "anchor"].iter().any(|p| param_modified(self.opts.catalog, it, "motion", p));
        if changed {
            if param_modified(self.opts.catalog, it, "motion", "scale_width")
                || param(it, "motion", "uniform_scale").and_then(|p| p.value.as_bool()) == Some(false)
            {
                self.report.warn_limited(self.report_limit, "non-uniform scale is not exported to FCPXML");
            }
            let mut attrs = Vec::new();
            let mut params = Vec::new();
            for (id, attr, f) in [
                ("position", "position", &pos as &dyn Fn(&ParamValue) -> String),
                ("scale", "scale", &|v: &ParamValue| {
                    let s = v.as_f64().unwrap_or(100.0) / 100.0;
                    format!("{} {}", num(s), num(s))
                }),
                ("rotation", "rotation", &|v: &ParamValue| num(-v.as_f64().unwrap_or(0.0))),
                ("anchor", "anchor", &pos),
            ] {
                let Some(p) = param(it, "motion", id) else { continue };
                if !param_modified(self.opts.catalog, it, "motion", id) {
                    continue;
                }
                attrs.push(format!(" {attr}=\"{}\"", f(&p.value)));
                if p.is_animated() {
                    let mut s = format!("  <param name=\"{attr}\">\n    <keyframeAnimation>\n");
                    for k in &p.keyframes {
                        let interp = if k.interp == Interpolation::Hold { " interp=\"hold\"" } else { "" };
                        s.push_str(&format!("      <keyframe time=\"{}\" value=\"{}\"{interp}/>\n", fmt_time(to_local(k.time), rate), f(&k.value)));
                    }
                    s.push_str("    </keyframeAnimation>\n  </param>");
                    params.push(s);
                }
            }
            out.push(wrap("adjust-transform", &attrs.concat(), &params));
        }
        if param_modified(self.opts.catalog, it, "opacity", "opacity") {
            out.push(kparam_xml(
                "adjust-blend",
                "amount",
                param(it, "opacity", "opacity"),
                rate,
                to_local,
                |v| num(v.as_f64().unwrap_or(100.0) / 100.0),
                "1",
            ));
        }
        if param(it, "opacity", "blend").and_then(|p| p.value.as_f64()).is_some_and(|b| b != 0.0) {
            self.report.warn_limited(self.report_limit, "blend modes are not exported to FCPXML");
        }
        let _ = (db_to_gain(0.0), gain_to_db(1.0));
    }
}

/// A primary-storyline `<gap>` standing in for a clip FCPXML cannot carry; it keeps the clip's place and length.
fn gap_el(it: &TrackItem, name: &str, offset: Tick, rate: FrameRate) -> El {
    El {
        start: it.start,
        end: it.end(),
        local: offset,
        xml_open: format!(
            "<gap name=\"{name}\" offset=\"{}\" start=\"{}\" duration=\"{}\"",
            fmt_time(offset, rate),
            fmt_time(offset, rate),
            fmt_time(it.duration, rate)
        ),
        children: vec![],
        close: "gap".into(),
    }
}

fn wrap(tag: &str, attrs: &str, params: &[String]) -> String {
    if params.is_empty() { format!("<{tag}{attrs}/>") } else { format!("<{tag}{attrs}>\n{}\n</{tag}>", params.join("\n")) }
}

fn kparam_xml(
    tag: &str,
    name: &str,
    p: Option<&Param>,
    rate: FrameRate,
    to_local: &dyn Fn(Tick) -> Tick,
    f: impl Fn(&ParamValue) -> String,
    default: &str,
) -> String {
    let Some(p) = p else { return format!("<{tag} {name}=\"{default}\"/>") };
    let attrs = format!(" {name}=\"{}\"", f(&p.value));
    let mut params = Vec::new();
    if p.is_animated() {
        let mut s = format!("  <param name=\"{name}\">\n    <keyframeAnimation>\n");
        for k in &p.keyframes {
            let interp = if k.interp == Interpolation::Hold { " interp=\"hold\"" } else { "" };
            s.push_str(&format!("      <keyframe time=\"{}\" value=\"{}\"{interp}/>\n", fmt_time(to_local(k.time), rate), f(&k.value)));
        }
        s.push_str("    </keyframeAnimation>\n  </param>");
        params.push(s);
    }
    wrap(tag, &attrs, &params)
}

fn marker_xml(m: &Marker, local: Tick, rate: FrameRate) -> String {
    let tag = if m.kind == MarkerKind::Chapter { "chapter-marker" } else { "marker" };
    let dur = if m.duration > Tick::ZERO { m.duration } else { rate.frame_duration() };
    let note = if m.comment.is_empty() { String::new() } else { format!(" note=\"{}\"", escape_attr(&m.comment)) };
    format!("<{tag} start=\"{}\" duration=\"{}\" value=\"{}\"{note}/>", fmt_time(local, rate), fmt_time(dur, rate), escape_attr(&m.name))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_ui::testing::trial;

    fn replay_fixture<'a, 'r>(opts: &'a ImportOptions<'a>, report: &'r mut Report) -> (Imp<'a, 'a, 'r>, ItemId, Vec<PendingAudio<'a>>) {
        let mut imp = Imp {
            b: Builder::new("Budget fixture", opts.catalog),
            opts,
            report,
            report_limit: None,
            formats: HashMap::new(),
            asset_nodes: HashMap::new(),
            assets: HashMap::new(),
            media_nodes: HashMap::new(),
            media_items: HashMap::new(),
            media_active: HashSet::new(),
            pending_audio: HashMap::new(),
            audio_replay: HashMap::new(),
            depth_hits: 0,
            media_truncated_at: HashMap::new(),
            reimports_left: MAX_REIMPORTS,
            cache_edges_left: MAX_CACHE_EDGES,
            depth: 0,
            effects: HashMap::new(),
        };
        let settings = settings_for(FrameRate::default(), 1920, 1080, false);
        let leaf = imp.b.reserve_sequence("Leaf", settings.clone(), None);
        let child = imp.b.reserve_sequence("Child", settings.clone(), None);
        let mut child_seq = empty_sequence(settings.clone());
        let sound = imp.b.clip(leaf, TrackKind::Audio, "Child sound", Tick(0), Tick(20), Tick(0));
        assert_eq!(imp.b.place(&mut child_seq, TrackKind::Audio, 0, sound), Some(0));
        imp.b.put_sequence(child, child_seq);
        let parent = imp.b.reserve_sequence("Parent", settings.clone(), None);
        let mut seq = empty_sequence(settings);
        let deferred = imp.b.clip(child, TrackKind::Audio, "Deferred", Tick(0), Tick(5), Tick(0));
        let mut video = imp.b.clip(child, TrackKind::Video, "Deferred video", Tick(0), Tick(5), Tick(0));
        // Explicit fixture coordinates make full-value equality meaningful (automatic points are NaN).
        for name in ["position", "anchor"] {
            video.effect_mut(crate::fx::MOTION).unwrap().param_mut(name).unwrap().value = ParamValue::Vec2(Vec2::new(960.0, 540.0));
        }
        let video_id = video.id;
        assert_eq!(imp.b.place(&mut seq, TrackKind::Video, 0, video), Some(0));
        let ordinary0 = imp.b.clip(leaf, TrackKind::Audio, "Ordinary 0", Tick(0), Tick(10), Tick(0));
        let ordinary1 = imp.b.clip(leaf, TrackKind::Audio, "Ordinary 1", Tick(9), Tick(3), Tick(0));
        let state = AudioReplay { order: vec![(deferred.id, 0), (ordinary0.id, 0), (ordinary1.id, 0)], ..AudioReplay::default() };
        assert_eq!(imp.b.place(&mut seq, TrackKind::Audio, 0, ordinary0), Some(0));
        assert_eq!(imp.b.place(&mut seq, TrackKind::Audio, 0, ordinary1), Some(1));
        seq.start_timecode = 7;
        seq.audio_tracks[0].name = "Original A1".into();
        seq.audio_tracks[0].locked = true;
        seq.audio_tracks[0].enabled = false;
        imp.b.put_sequence(parent, seq);
        imp.audio_replay.insert(parent, state);
        let pending = vec![PendingAudio { parent, child, audio: deferred, video: Some(video_id), diagnostics: None }];
        (imp, parent, pending)
    }

    #[trial]
    fn audio_replay_budget_refusal_preserves_parent_and_ledger() {
        let catalog = crate::fx::Standard;
        let opts = ImportOptions { base_dir: None, name: None, catalog: &catalog, namespace: "test" };
        let mut report = Report::default();
        let diagnostic_ids = |state: &AudioReplay<'_>| state.diagnostics.iter().map(|(id, n)| (*id, n.id())).collect::<HashMap<_, _>>();
        let (mut imp, parent, intents) = replay_fixture(&opts, &mut report);
        assert_eq!(intents.len(), 1);
        assert!(imp.b.p.sequence(intents[0].child).unwrap().audio_tracks.iter().any(|t| !t.items.is_empty()));
        let original = imp.b.p.sequence(parent).unwrap().clone();
        assert_eq!(imp.b.p.sequence(parent), Some(&original), "the fixture supports full-value clone equality");
        let state = imp.audio_replay.get(&parent).unwrap();
        let ledger = (state.order.clone(), state.unplaced.clone(), state.paired.clone(), state.video_index.clone(), diagnostic_ids(state));
        // Old linear charging admits this run; full indexed planning must also pay the final sort.
        imp.cache_edges_left = 54;
        let result = imp.replay_parent_audio(parent, intents);
        let parent_unchanged = imp.b.p.sequence(parent) == Some(&original);
        let state = imp.audio_replay.get(&parent).unwrap();
        let ledger_unchanged = (&state.order, &state.unplaced, &state.paired, &state.video_index, &diagnostic_ids(state))
            == (&ledger.0, &ledger.1, &ledger.2, &ledger.3, &ledger.4);
        assert!(
            result.is_none() && parent_unchanged && ledger_unchanged,
            "audio replay budget refusal: result={result:?}, parent_unchanged={parent_unchanged}, ledger_unchanged={ledger_unchanged}",
        );

        let mut empty_report = Report::default();
        let (mut empty_imp, empty_parent, empty_intents) = replay_fixture(&opts, &mut empty_report);
        let pending_id = empty_intents[0].audio.id;
        for track in &mut empty_imp.b.p.sequence_mut(empty_parent).unwrap().audio_tracks {
            track.items.clear();
        }
        empty_imp.audio_replay.get_mut(&empty_parent).unwrap().order.retain(|p| p.0 == pending_id);
        assert!(empty_imp.b.p.sequence(empty_parent).unwrap().audio_tracks.iter().all(|t| t.items.is_empty()));
        assert!(empty_imp.b.p.sequence(empty_intents[0].child).unwrap().audio_tracks.iter().any(|t| !t.items.is_empty()));
        let original = empty_imp.b.p.sequence(empty_parent).unwrap().clone();
        let state = empty_imp.audio_replay.get(&empty_parent).unwrap();
        let ledger = (state.order.clone(), state.unplaced.clone(), state.paired.clone(), state.video_index.clone(), diagnostic_ids(state));
        // Twelve metadata units leave no budget for the ready child's first graph edge.
        empty_imp.cache_edges_left = 12;
        let result = empty_imp.replay_parent_audio(empty_parent, empty_intents);
        let parent_unchanged = empty_imp.b.p.sequence(empty_parent) == Some(&original);
        let state = empty_imp.audio_replay.get(&empty_parent).unwrap();
        let ledger_unchanged = (&state.order, &state.unplaced, &state.paired, &state.video_index, &diagnostic_ids(state))
            == (&ledger.0, &ledger.1, &ledger.2, &ledger.3, &ledger.4);
        assert!(
            result.is_none() && parent_unchanged && ledger_unchanged,
            "empty parent audio replay budget refusal: result={result:?}, parent_unchanged={parent_unchanged}, ledger_unchanged={ledger_unchanged}",
        );
    }

    #[trial]
    fn repeated_audio_children_share_one_reachability_walk_per_replay() {
        let catalog = crate::fx::Standard;
        let opts = ImportOptions { base_dir: None, name: None, catalog: &catalog, namespace: "test" };
        let mut report = Report::default();
        let (mut imp, parent, initial) = replay_fixture(&opts, &mut report);
        let child = initial[0].child;
        let leaf = imp.b.p.sequence(child).unwrap().audio_tracks[0].items[0].item;
        for i in 0..1000 {
            let audio = imp.b.clip(leaf, TrackKind::Audio, "Child edge", Tick(i), Tick(1), Tick(0));
            imp.b.p.sequence_mut(child).unwrap().audio_tracks[0].items.push(audio);
        }
        for track in imp.b.p.sequence_mut(parent).unwrap().all_tracks_mut() {
            track.items.clear();
        }
        let mut state = AudioReplay::default();
        let mut intents = Vec::new();
        for i in 0..1100 {
            let audio = imp.b.clip(child, TrackKind::Audio, "Repeated child", Tick(i), Tick(1), Tick(0));
            state.order.push((audio.id, 0));
            intents.push(PendingAudio { parent, child, audio, video: None, diagnostics: None });
        }
        imp.audio_replay.insert(parent, state);
        assert_eq!(imp.replay_parent_audio(parent, intents), Some(true), "one shared child must not spend the graph budget once per reference");
        let seq = imp.b.p.sequence(parent).unwrap();
        assert_eq!(seq.audio_tracks.iter().map(|t| t.items.len()).sum::<usize>(), 1100);
        assert!(imp.cache_edges_left > 0);
    }

    #[trial]
    fn indexed_audio_plan_preserves_first_fit_boundaries_and_cap_recovery() {
        let intervals = [
            AudioInterval { id: ClipId(1), preferred: 254, start: Tick(0), end: Tick(5) },
            AudioInterval { id: ClipId(2), preferred: 254, start: Tick(0), end: Tick(10) },
            AudioInterval { id: ClipId(3), preferred: 254, start: Tick(9), end: Tick(12) },
            AudioInterval { id: ClipId(4), preferred: 255, start: Tick(11), end: Tick(13) },
        ];
        let mut work = MAX_CACHE_EDGES;
        let plan = indexed_audio_plan(&intervals, &[], &mut work).unwrap();
        assert_eq!(plan, vec![(ClipId(1), Some(254)), (ClipId(2), Some(255)), (ClipId(3), Some(254)), (ClipId(4), Some(255))]);
        let intervals = [
            AudioInterval { id: ClipId(1), preferred: 0, start: Tick(0), end: Tick(10) },
            AudioInterval { id: ClipId(2), preferred: 0, start: Tick(10), end: Tick(20) },
            AudioInterval { id: ClipId(3), preferred: 0, start: Tick(3), end: Tick(4) },
            AudioInterval { id: ClipId(4), preferred: 0, start: Tick(-2), end: Tick(2) },
            AudioInterval { id: ClipId(5), preferred: 0, start: Tick(0), end: Tick(0) },
            AudioInterval { id: ClipId(6), preferred: 0, start: Tick(5), end: Tick(5) },
        ];
        let mut work = MAX_CACHE_EDGES;
        let lanes: Vec<_> = indexed_audio_plan(&intervals, &[], &mut work).unwrap().into_iter().map(|p| p.1).collect();
        assert_eq!(lanes, vec![Some(0), Some(0), Some(1), Some(1), Some(0), Some(1)]);
    }

    #[trial]
    fn indexed_audio_replay_preserves_items_pairs_and_track_metadata() {
        let catalog = crate::fx::Standard;
        let opts = ImportOptions { base_dir: None, name: None, catalog: &catalog, namespace: "test" };
        let mut report = Report::default();
        let (mut imp, parent, intents) = replay_fixture(&opts, &mut report);
        let mut expected = intents[0].audio.clone();
        let original = imp.b.p.sequence(parent).unwrap().clone();
        assert!(imp.replay_parent_audio(parent, intents).is_some());
        let seq = imp.b.p.sequence(parent).unwrap();
        assert_eq!(seq.settings, original.settings);
        assert_eq!(seq.start_timecode, original.start_timecode);
        assert_eq!(seq.markers, original.markers);
        assert_eq!(seq.audio_tracks[0].id, original.audio_tracks[0].id);
        assert_eq!(seq.audio_tracks[0].name, "Original A1");
        assert!(seq.audio_tracks[0].locked);
        assert!(!seq.audio_tracks[0].enabled);
        assert_eq!(seq.video_tracks[0].id, original.video_tracks[0].id);
        assert_eq!(seq.video_tracks[0].transitions, original.video_tracks[0].transitions);
        assert_eq!(seq.audio_tracks[0].items.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), vec!["Deferred", "Ordinary 1"]);
        assert_eq!(seq.audio_tracks[1].items[0].name, "Ordinary 0");
        let actual = &seq.audio_tracks[0].items[0];
        expected.link = actual.link;
        assert_eq!(actual, &expected);
        assert!(actual.link.is_some());
        assert_eq!(actual.link, seq.video_tracks[0].items[0].link);
        assert!(imp.audio_replay.get(&parent).unwrap().unplaced.is_empty());
    }

    #[trial]
    fn rational_times_are_exact() {
        let r = FrameRate::FPS_23_976;
        assert_eq!(parse_time("1001/24000s"), Some(r.frame_duration()));
        assert_eq!(parse_time("3600s"), Some(Tick(3600 * TICKS_PER_SECOND)));
        assert_eq!(parse_time("0s"), Some(Tick::ZERO));
        assert_eq!(fmt_time(r.tick_of(1), r), "1001/24000s");
        assert_eq!(fmt_time(r.tick_of(24000), r), "1001s");
        assert_eq!(fmt_time(FrameRate::FPS_25.tick_of(50), FrameRate::FPS_25), "2s");
        for rate in FrameRate::COMMON {
            for f in [0, 1, 7, 1799, 1800, 86_313, 1_000_003] {
                let t = rate.tick_of(f);
                assert_eq!(parse_time(&fmt_time(t, rate)), Some(t), "{rate} {f}");
            }
        }
        let odd = Tick(12_345);
        assert_eq!(parse_time(&fmt_time(odd, r)), Some(odd));
        assert_eq!(rate_from_frame_duration("1001/30000s"), Some(FrameRate::FPS_29_97));
        assert_eq!(rate_from_frame_duration("100/2500s"), Some(FrameRate::FPS_25));
        for bad in ["100/0s", "0/100s", "-100/2500s", "100/-2500s", "0s", "x"] {
            assert_eq!(rate_from_frame_duration(bad), None, "{bad}");
        }
    }

    /// A zero frame duration denominator gave the format a zero frame rate, and export then
    /// divided by zero (`fmt_time`) as soon as the sequence had a start timecode or a marker.
    #[trial]
    fn zero_frame_duration_falls_back_to_the_default_rate() {
        for fd in ["100/0s", "0/100s", "-100/2500s"] {
            let xml = format!(
                r#"<?xml version="1.0"?><fcpxml version="1.10"><resources><format id="f1" frameDuration="{fd}" width="1920" height="1080"/>
<asset id="a1" name="c" start="0s" duration="10s" hasVideo="1" format="f1"><media-rep kind="original-media" src="file:///tmp/c.mov"/></asset></resources>
<library><event name="E"><project name="P"><sequence format="f1" tcStart="3600/1s" tcFormat="DF"><spine>
<asset-clip ref="a1" offset="3600s" duration="5s"><marker start="1s" duration="1/25s" value="m"/></asset-clip></spine></sequence></project></event></library></fcpxml>"#
            );
            let mut report = Report::default();
            let iopts = ImportOptions { base_dir: None, name: None, catalog: &crate::fx::Standard, namespace: "test" };
            let imported = import(&xml, &iopts, &mut report).expect("import succeeds");
            let xopts = ExportOptions {
                relative_to: None,
                name: None,
                catalog: &crate::fx::Standard,
                namespace: "test",
                xmeml_version: 4,
                fcpxml_version: Default::default(),
            };
            for seq in &imported.sequences {
                export(&imported.project, *seq, &xopts, &mut Report::default()).expect("export succeeds");
            }
            assert!(report.mentions("invalid frameDuration"), "{fd}: {report}");
        }
    }
}

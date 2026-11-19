//! Building a [`Project`] from a document, and the helpers importers and exporters share.

use std::collections::HashMap;

use crate::model::{
    AudioStreamInfo, BinId, Catalog, ClipId, EffectInstance, EffectKind, Generator, ItemId, ItemKind, Label, MediaClip, MediaInfo, MediaKind,
    MediaRef, Param, ParamValue, Project, Sequence, SequenceSettings, Track, TrackId, TrackItem, TrackKind, VideoStreamInfo,
};
use crate::path::extension;
use crate::rate::plausible_rate;
use crate::report::Report;
use crate::time::{FrameRate, Tick};

/// A timeline import may not materialise an unbounded number of sparse track
/// labels. This remains well above common editorial timelines while keeping
/// hostile lane/channel numbers from allocating every intervening track.
pub const MAX_IMPORTED_TRACKS: usize = 256;

/// Start timecodes beyond this many frames (about 100 hours at the highest accepted rate) are not
/// timecodes: a value near `i64::MAX` made an export overflow adding frame numbers to it.
pub const MAX_START_TIMECODE: u64 = 360_000_000_000;

/// The result of an import: a standalone project.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    /// Bins, media items and sequences read from the document (ids local to this project).
    pub project: Project,
    /// Top-level sequences, in document order (nested sequences are items too but not listed here).
    pub sequences: Vec<ItemId>,
}

impl Imported {
    /// Reset what a document carries beyond what frame arithmetic can hold, and say so: frame rates
    /// above [`crate::rate::MAX_RATE`] per second and start timecodes beyond [`MAX_START_TIMECODE`]
    /// frames. Every importer ends with this pass, so no format can let such a number through.
    pub fn bound(&mut self, report: &mut Report) {
        for item in self.project.items.values_mut() {
            let name = item.name.clone();
            if let Some(seq) = item.as_sequence_mut() {
                if !plausible_rate(seq.settings.frame_rate) {
                    report.warn(format!("sequence \"{name}\" has an implausible frame rate ({}); the default was used", seq.settings.frame_rate));
                    seq.settings.frame_rate = FrameRate::default();
                }
                if seq.start_timecode.unsigned_abs() > MAX_START_TIMECODE {
                    report.warn(format!("sequence \"{name}\" has an out-of-range start timecode ({} frames); it starts at 0", seq.start_timecode));
                    seq.start_timecode = 0;
                }
            } else if let Some(media) = item.as_media_mut() {
                if let Some(v) = media.info.video.as_mut()
                    && !plausible_rate(v.frame_rate)
                {
                    report.warn(format!("media \"{name}\" has an implausible frame rate ({}); the default was used", v.frame_rate));
                    v.frame_rate = FrameRate::default();
                }
                if media.info.start_timecode.is_some_and(|tc| tc.unsigned_abs() > MAX_START_TIMECODE) {
                    report.warn(format!("media \"{name}\" has an out-of-range start timecode; it was ignored"));
                    media.info.start_timecode = None;
                }
            }
        }
    }
}

/// Media kind guess from a file extension.
pub fn kind_from_ext(path: &str) -> MediaKind {
    match extension(path).as_str() {
        "wav" | "aif" | "aiff" | "mp3" | "aac" | "m4a" | "flac" | "ogg" | "opus" | "bwf" => MediaKind::AudioOnly,
        "png" | "jpg" | "jpeg" | "tif" | "tiff" | "psd" | "bmp" | "gif" | "webp" | "exr" | "dpx" | "tga" => MediaKind::Still,
        _ => MediaKind::Movie,
    }
}

/// What a document says about a media file.
#[derive(Clone, Debug, Default)]
pub struct MediaSpec {
    pub duration: Option<Tick>,
    /// (width, height, rate).
    pub video: Option<(u32, u32, FrameRate)>,
    /// (sample rate, channels).
    pub audio: Option<(u32, u32)>,
    pub start_tc: Option<i64>,
    pub kind: Option<MediaKind>,
}

pub fn media_info(name: &str, spec: &MediaSpec) -> MediaInfo {
    let kind = spec.kind.unwrap_or(match (spec.video.is_some(), spec.audio.is_some()) {
        (false, true) => MediaKind::AudioOnly,
        _ => MediaKind::Movie,
    });
    MediaInfo {
        name: name.to_string(),
        kind,
        duration: spec.duration.unwrap_or(Tick::ZERO),
        video: spec.video.map(|(w, h, r)| VideoStreamInfo { width: w, height: h, frame_rate: r, par: (1, 1) }),
        audio_streams: spec.audio.map(|(sr, ch)| AudioStreamInfo { sample_rate: sr, channels: ch, bits_per_sample: None }).into_iter().collect(),
        start_timecode: spec.start_tc,
    }
}

pub fn media_clip(media: MediaRef, info: MediaInfo) -> MediaClip {
    MediaClip { media, info, interpret_frame_rate: None, markers: Vec::new(), offline: false }
}

/// The file path behind an item (following subclips), if it is file media.
pub fn item_path(p: &Project, id: ItemId) -> Option<&str> {
    match &item_media(p, id)?.media {
        MediaRef::File { path } => Some(path),
        MediaRef::Generator(_) => None,
    }
}

/// The media clip behind an item (following subclips).
pub fn item_media(p: &Project, id: ItemId) -> Option<&MediaClip> {
    p.resolve_media(id).map(|(_, m, _)| m)
}

/// The underlying item for exports (subclip → parent media).
pub fn base_item(p: &Project, id: ItemId) -> ItemId {
    let mut cur = id;
    for _ in 0..16 {
        match p.item(cur).map(|i| &i.kind) {
            Some(ItemKind::Subclip { parent, .. }) => cur = *parent,
            _ => return cur,
        }
    }
    cur
}

/// How an export report names a clip whose source no format here can carry (a title, an
/// adjustment layer): `graphic clip "Title" at frame 48`. `None` for every other clip. A lost title
/// must be named, not folded into a count.
pub fn uncarried_clip(p: &Project, c: &TrackItem, rate: FrameRate) -> Option<String> {
    let ItemKind::Uncarried { what, .. } = &p.item(c.item)?.kind else { return None };
    Some(format!("{what} \"{}\" at frame {}", c.name, rate.frame_at(c.start)))
}

pub fn generator_of(p: &Project, id: ItemId) -> Option<&Generator> {
    match &item_media(p, id)?.media {
        MediaRef::Generator(g) => Some(g),
        MediaRef::File { .. } => None,
    }
}

/// Builds the project of an import.
pub struct Builder<'c> {
    pub p: Project,
    pub catalog: &'c dyn Catalog,
    media: HashMap<String, ItemId>,
    pub top: Vec<ItemId>,
    next_link: u64,
}

impl<'c> Builder<'c> {
    pub fn new(name: &str, catalog: &'c dyn Catalog) -> Self {
        Self { p: Project::new(name), catalog, media: HashMap::new(), top: Vec::new(), next_link: 1 }
    }

    pub fn bin(&mut self, name: &str, parent: Option<BinId>) -> BinId {
        self.p.add_bin(name, parent)
    }

    pub fn find_media(&self, key: &str) -> Option<ItemId> {
        self.media.get(key).copied()
    }

    pub fn register_media(&mut self, key: &str, id: ItemId) {
        self.media.insert(key.to_string(), id);
    }

    /// File media deduplicated by `key` (normally the resolved path).
    pub fn file_media(&mut self, key: &str, name: &str, path: &str, spec: &MediaSpec, bin: Option<BinId>) -> ItemId {
        if let Some(id) = self.media.get(key) {
            return *id;
        }
        let info = media_info(name, spec);
        let id = self.p.add_item(name, Label::Iris, ItemKind::Media(Box::new(media_clip(MediaRef::File { path: path.to_string() }, info))), bin);
        self.media.insert(key.to_string(), id);
        id
    }

    pub fn generator_media(&mut self, key: &str, name: &str, g: Generator, spec: &MediaSpec, bin: Option<BinId>) -> ItemId {
        if let Some(id) = self.media.get(key) {
            return *id;
        }
        let mut spec = spec.clone();
        spec.kind = Some(MediaKind::Synthetic);
        let info = media_info(name, &spec);
        let id = self.p.add_item(name, Label::Lavender, ItemKind::Media(Box::new(media_clip(MediaRef::Generator(g), info))), bin);
        self.media.insert(key.to_string(), id);
        id
    }

    /// Add an empty sequence item (filled later with [`Builder::put_sequence`]).
    pub fn reserve_sequence(&mut self, name: &str, settings: SequenceSettings, bin: Option<BinId>) -> ItemId {
        let seq = empty_sequence(settings);
        self.p.add_item(name, Label::Forest, ItemKind::Sequence(Box::new(seq)), bin)
    }

    pub fn put_sequence(&mut self, id: ItemId, mut seq: Sequence) {
        for t in seq.all_tracks_mut() {
            t.sort();
        }
        if let Some(s) = self.p.sequence_mut(id) {
            *s = seq;
        }
    }

    pub fn track(&mut self, kind: TrackKind, index: usize) -> Track {
        let id = TrackId(self.p.alloc_id());
        let name = match kind {
            TrackKind::Video => format!("Video {}", index + 1),
            TrackKind::Audio => format!("Audio {}", index + 1),
        };
        Track::new(id, kind, name)
    }

    /// Ensure `n` tracks of one kind exist. Returns false when an imported
    /// label would exceed the supported per-kind track count.
    pub fn ensure_tracks(&mut self, seq: &mut Sequence, kind: TrackKind, n: usize) -> bool {
        if n > MAX_IMPORTED_TRACKS {
            return false;
        }
        while seq.tracks(kind).len() < n {
            let i = seq.tracks(kind).len();
            let t = self.track(kind, i);
            seq.tracks_mut(kind).push(t);
        }
        true
    }

    pub fn link_id(&mut self) -> u64 {
        let l = self.next_link;
        self.next_link += 1;
        l
    }

    pub fn alloc(&mut self) -> u64 {
        self.p.alloc_id()
    }

    /// A track item with the intrinsic effects of its track kind.
    pub fn clip(&mut self, item: ItemId, kind: TrackKind, name: &str, start: Tick, duration: Tick, source_in: Tick) -> TrackItem {
        let label = self.p.item(item).map(|i| i.label).unwrap_or(Label::Iris);
        TrackItem {
            id: ClipId(self.p.alloc_id()),
            item,
            name: name.to_string(),
            label,
            start,
            duration,
            source_in,
            speed: 1.0,
            reverse: false,
            enabled: true,
            link: None,
            group: None,
            effects: self.catalog.intrinsic(kind),
            markers: Vec::new(),
            gain_db: 0.0,
            frame_hold: None,
            scale_to_frame: false,
            source_channels: Vec::new(),
            ext: None,
        }
    }

    /// Place `item` on the first track at or after `preferred` where it does not overlap anything,
    /// creating tracks as needed. Returns the track index.
    pub fn place(&mut self, seq: &mut Sequence, kind: TrackKind, preferred: usize, item: TrackItem) -> Option<usize> {
        let mut i = preferred;
        loop {
            if i >= MAX_IMPORTED_TRACKS || !self.ensure_tracks(seq, kind, i + 1) {
                return None;
            }
            let t = &mut seq.tracks_mut(kind)[i];
            if !t.items.iter().any(|o| o.start < item.end() && item.start < o.end()) {
                t.items.push(item);
                t.sort();
                return Some(i);
            }
            i += 1;
        }
    }

    /// Finish: grow media durations to cover every use, return the fragment.
    pub fn finish(mut self) -> Imported {
        let mut need: HashMap<ItemId, Tick> = HashMap::new();
        for it in self.p.items.values() {
            if let Some(seq) = it.as_sequence() {
                for t in seq.all_tracks() {
                    for ti in &t.items {
                        let e = need.entry(ti.item).or_insert(Tick::ZERO);
                        *e = (*e).max(ti.source_out()).max(ti.source_in + Tick(1));
                    }
                }
            }
        }
        for (id, end) in need {
            if let Some(m) = self.p.item_mut(id).and_then(|i| i.as_media_mut())
                && m.info.duration < end
            {
                m.info.duration = end;
            }
        }
        Imported { project: self.p, sequences: self.top }
    }
}

pub fn empty_sequence(settings: SequenceSettings) -> Sequence {
    Sequence::new(settings)
}

pub fn settings_for(rate: FrameRate, width: u32, height: u32, drop_frame: bool) -> SequenceSettings {
    SequenceSettings { width, height, frame_rate: rate, drop_frame: drop_frame && rate.supports_drop_frame(), ..SequenceSettings::default() }
}

/// Source time advanced by a timeline duration at `speed` (exact for integral speeds).
///
/// This is the one place the importers multiply a `Tick` by a number from a file: `Tick` has no
/// `Mul`, and its `Add`/`Sub` saturate, so every other sum of durations already clamps. It saturates
/// at the `i64` bounds like those operators do (not at `Tick::MAX`, which the editing code uses as
/// its "no next item" sentinel).
pub fn scaled(d: Tick, speed: f64) -> Tick {
    let s = speed.abs();
    // saturating: a duration read from a file times a speed from the same file can exceed an i64 of ticks
    if s == s.round() && s < 1e6 { Tick(d.0.saturating_mul(s as i64)) } else { Tick((d.0 as f64 * s).round() as i64) }
}

/// Timeline duration of a source duration at `speed`.
pub fn unscaled(d: Tick, speed: f64) -> Tick {
    let s = speed.abs();
    if s == 0.0 {
        return d;
    }
    if s == 1.0 { d } else { Tick((d.0 as f64 / s).round() as i64) }
}

pub fn param<'a>(ti: &'a TrackItem, effect: &str, param: &str) -> Option<&'a Param> {
    ti.effect(effect).and_then(|e| e.param(param))
}

/// Set a parameter, adding the effect instance (defaults) if the item lacks it.
pub fn set_param(catalog: &dyn Catalog, ti: &mut TrackItem, effect: &str, param: &str, value: Param) {
    if ti.effect(effect).is_none()
        && let Some(d) = catalog.effect(effect)
    {
        ti.effects.push(d.instance());
    }
    if let Some(e) = ti.effect_mut(effect) {
        e.params.insert(param.to_string(), value);
    }
}

/// Whether a parameter differs from its definition default or is animated.
pub fn param_modified(catalog: &dyn Catalog, ti: &TrackItem, effect: &str, param_id: &str) -> bool {
    let Some(p) = param(ti, effect, param_id) else { return false };
    if p.is_animated() {
        return true;
    }
    let def = catalog.effect(effect).and_then(|d| d.param(param_id).map(|p| p.default.clone()));
    match (&p.value, def) {
        (ParamValue::Vec2(v), Some(ParamValue::Vec2(_))) => !(v.x.is_nan() && v.y.is_nan()),
        (v, Some(d)) => *v != d,
        _ => false,
    }
}

/// Non-intrinsic effects on an item.
pub fn standard_effects<'a>(catalog: &'a dyn Catalog, ti: &'a TrackItem) -> impl Iterator<Item = &'a EffectInstance> {
    ti.effects.iter().filter(move |e| !catalog.effect(&e.effect).is_some_and(|d| d.intrinsic))
}

/// A transition effect for a document's transition name. Unknown names become Cross Dissolve
/// (Constant Power on audio) with a warning.
pub fn transition_effect(catalog: &dyn Catalog, name: &str, audio: bool, report: &mut Report) -> EffectInstance {
    let n = name.trim().to_ascii_lowercase();
    let alias = match n.as_str() {
        "dissolve" | "cross dissolve" | "smpte_dissolve" | "d" | "crossdissolve" => Some(if audio { "constant_power" } else { "cross_dissolve" }),
        "cross fade (+3db)" | "cross fade +3db" | "crossfade (+3db)" | "cross fade" => Some("constant_power"),
        "cross fade (0db)" | "cross fade 0db" | "crossfade (0db)" => Some("constant_gain"),
        "dip to color dissolve" | "fade in fade out dissolve" | "fade to color" => Some("dip_to_black"),
        "edge wipe" | "wipe" | "smpte_wipe" => Some("wipe"),
        _ => None,
    };
    if let Some(id) = alias
        && let Some(d) = catalog.effect(id)
    {
        return d.instance();
    }
    let want = if audio { EffectKind::AudioTransition } else { EffectKind::VideoTransition };
    if let Some(d) = catalog.effect_by_name(name.trim()).filter(|d| d.kind == want) {
        return d.instance();
    }
    let fallback = if audio { "constant_power" } else { "cross_dissolve" };
    let def = catalog.effect(fallback);
    report.warn(format!("transition \"{name}\" is not supported; imported as {}", def.as_ref().map(|d| d.name.as_str()).unwrap_or(fallback)));
    def.map(|d| d.instance()).unwrap_or_else(|| EffectInstance { effect: fallback.into(), enabled: true, params: Default::default(), ext: None })
}

/// Name of a transition in a document (the catalog's stable name, so a rename in an application's
/// interface does not change the file).
pub fn transition_name(catalog: &dyn Catalog, e: &EffectInstance) -> String {
    catalog.effect(&e.effect).map(|d| d.name).unwrap_or_else(|| e.effect.clone())
}

/// Label from a document label name (the sixteen label names; names such as "Good Take" are no label).
pub fn label_from(s: &str) -> Option<Label> {
    Label::from_name(s.trim())
}

/// dB ↔ linear gain.
pub fn db_to_gain(db: f64) -> f64 {
    if db <= -287.0 { 0.0 } else { 10f64.powf(db / 20.0) }
}
pub fn gain_to_db(g: f64) -> f64 {
    if g <= 1e-15 { -287.5 } else { 20.0 * g.log10() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fx::Standard;
    use crate::time::TimeRange;

    #[test]
    fn imported_tracks_are_bounded() {
        let mut b = Builder::new("tracks", &Standard);
        let mut seq = empty_sequence(Default::default());
        assert!(b.ensure_tracks(&mut seq, TrackKind::Video, MAX_IMPORTED_TRACKS));
        assert_eq!(seq.video_tracks.len(), MAX_IMPORTED_TRACKS);
        assert!(!b.ensure_tracks(&mut seq, TrackKind::Video, MAX_IMPORTED_TRACKS + 1));
        assert_eq!(seq.video_tracks.len(), MAX_IMPORTED_TRACKS);
    }

    /// A huge duration at an integral speed saturates instead of overflowing (an EDL with a record-out
    /// timecode of 2147483647 hours and an `M2 ... 72.0` speed change panicked here).
    #[test]
    fn scaled_saturates() {
        assert_eq!(scaled(Tick(i64::MAX / 2), 3.0), Tick(i64::MAX));
        assert_eq!(scaled(Tick(i64::MIN / 2), 3.0), Tick(i64::MIN));
        assert_eq!(scaled(Tick(1000), 3.0), Tick(3000));
        assert_eq!(scaled(Tick(1000), 0.5), Tick(500));
    }

    #[test]
    fn subclip_cycle_resolution_is_bounded() {
        let mut p = Project::new("Cycle Test");
        let spec = MediaSpec { duration: Some(Tick(100_000)), ..Default::default() };
        let media = media_clip(MediaRef::File { path: "/path/to/clip.mp4".into() }, media_info("clip.mp4", &spec));
        let media_id = p.add_item("Clip", Label::Iris, ItemKind::Media(Box::new(media)), None);
        let sub = |parent| ItemKind::Subclip { parent, range: TimeRange::default() };

        // Normal subclip and chained subclip
        let sub1 = p.add_item("Sub 1", Label::Iris, ItemKind::Subclip { parent: media_id, range: TimeRange::new(Tick::ZERO, Tick(10_000)) }, None);
        let sub2 = p.add_item("Sub 2", Label::Iris, ItemKind::Subclip { parent: sub1, range: TimeRange::new(Tick(1_000), Tick(5_000)) }, None);
        assert_eq!(base_item(&p, media_id), media_id);
        assert_eq!(base_item(&p, sub1), media_id);
        assert_eq!(base_item(&p, sub2), media_id);
        assert_eq!(item_path(&p, sub2), Some("/path/to/clip.mp4"));
        assert_eq!(p.resolve_media(sub2).map(|(id, _, range)| (id, range)), Some((media_id, Some(TimeRange::new(Tick(1_000), Tick(5_000))))));

        // Self-referencing subclip, and a mutual cycle (A -> B -> A)
        let self_cycle = p.add_item("Self Cycle", Label::Iris, sub(ItemId(9999)), None);
        p.item_mut(self_cycle).unwrap().kind = sub(self_cycle);
        let cycle_a = p.add_item("Cycle A", Label::Iris, sub(ItemId(9999)), None);
        let cycle_b = p.add_item("Cycle B", Label::Iris, sub(cycle_a), None);
        p.item_mut(cycle_a).unwrap().kind = sub(cycle_b);
        assert_eq!(base_item(&p, self_cycle), self_cycle);
        assert_eq!(item_media(&p, self_cycle), None);
        assert!(matches!(base_item(&p, cycle_a), x if x == cycle_a || x == cycle_b));
        assert_eq!(item_media(&p, cycle_a), None);
        assert_eq!(item_path(&p, cycle_b), None);
        assert_eq!(p.source_size(cycle_a), None);

        // Missing parent
        let missing = p.add_item("Missing Parent", Label::Iris, sub(ItemId(123456789)), None);
        assert_eq!(base_item(&p, missing), ItemId(123456789));
        assert_eq!(item_media(&p, missing), None);
    }

    /// An implausible rate or start timecode is reset, and the report says so.
    #[test]
    fn imports_are_bounded() {
        let mut b = Builder::new("bounds", &Standard);
        let id = b.reserve_sequence("S", settings_for(FrameRate::new(i64::MAX, 1), 1920, 1080, false), None);
        if let Some(s) = b.p.sequence_mut(id) {
            s.start_timecode = i64::MAX;
        }
        let spec = MediaSpec { video: Some((1920, 1080, FrameRate::new(i64::MAX, 1))), start_tc: Some(-i64::MAX), ..Default::default() };
        let m = b.file_media("m", "m.mov", "/m.mov", &spec, None);
        let mut imported = b.finish();
        let mut report = Report::default();
        imported.bound(&mut report);
        let s = imported.project.sequence(id).unwrap();
        assert_eq!((s.settings.frame_rate, s.start_timecode), (FrameRate::default(), 0));
        let media = imported.project.item(m).unwrap().as_media().unwrap();
        assert_eq!((media.info.frame_rate(), media.info.start_timecode), (FrameRate::default(), None));
        assert_eq!(report.warnings().count(), 4, "{report}");
    }
}

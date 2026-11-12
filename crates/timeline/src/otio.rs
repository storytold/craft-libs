//! OpenTimelineIO JSON (`.otio`).
//!
//! Schemas read and written: `Timeline.1`, `Stack.1`, `Track.1`, `Clip.1`/`Clip.2`, `Gap.1`,
//! `Transition.1`, `Marker.1`/`Marker.2`, `ExternalReference.1`, `MissingReference.1`,
//! `GeneratorReference.1`, `LinearTimeWarp.1`, `FreezeFrame.1`, `Effect.1`, `RationalTime.1`,
//! `TimeRange.1` and `SerializableCollection.1` (several timelines).
//!
//! Times are written as frames at the sequence rate (`rate` is the exact rate as a float, e.g.
//! 23.976023976023978) and read back exactly by snapping the rate to its rational form. Nested
//! sequences are `Stack` items inside tracks.
//!
//! What OTIO has no schema for round-trips through `metadata.<namespace>` ([`ImportOptions::namespace`]):
//! effects and their keyframes, labels, links, groups, clip gain, exact tick times, track state,
//! sequence settings, media info and generators, marker kinds. The model's `ext` of each object is
//! a JSON object whose keys are written into the same metadata object, after these, and every key
//! the reader does not know comes back in `ext`. An application keeps its own data that way (and
//! reads what older versions of it wrote there).
//!
//! JSON nesting is bounded by `serde_json`'s limit of 128 levels: a deeper document is a parse
//! error, and nothing after parsing recurses deeper than the document.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::build::{
    Builder, Imported, MediaSpec, base_item, empty_sequence, generator_of, item_media, settings_for, transition_effect, transition_name,
};
use crate::model::{
    AudioChannels, AudioStreamInfo, ClipId, EffectInstance, Ext, Generator, Interpolation, ItemId, ItemKind, Keyframe, Label, Marker, MarkerId,
    MarkerKind, MediaInfo, MediaKind, MediaRef, Param, ParamValue, Project, Sequence, SequenceSettings, Track, TrackItem, TrackKind, Transition,
    TransitionAlign, TransitionId, Vec2, VideoStreamInfo,
};
use crate::path::{file_name, is_absolute, path_to_file_url, relative_path, relative_url, resolve_path, resolve_url};
use crate::rate::{MAX_RATE, rate_from_f64};
use crate::report::Report;
use crate::time::{FrameRate, TICKS_PER_SECOND, Tick};
use crate::{Error, ExportOptions, Format, ImportOptions, Result};

// ---------------------------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------------------------

fn rt(t: Tick, rate: FrameRate) -> Value {
    let f = rate.frame_at(t);
    let value =
        if rate.tick_of(f) == t { json!(f as f64) } else { json!(t.0 as f64 * rate.num as f64 / (rate.den as f64 * TICKS_PER_SECOND as f64)) };
    json!({"OTIO_SCHEMA": "RationalTime.1", "rate": rate.as_f64(), "value": value})
}

fn tr(start: Tick, dur: Tick, rate: FrameRate) -> Value {
    json!({"OTIO_SCHEMA": "TimeRange.1", "start_time": rt(start, rate), "duration": rt(dur, rate)})
}

/// RationalTime → ticks (exact for integral values at broadcast rates).
fn read_rt(v: &Value) -> Option<Tick> {
    let rate = v.get("rate")?.as_f64()?;
    let value = v.get("value")?.as_f64()?;
    let r = rate_from_f64(rate);
    if value == value.round() && value.abs() < 9e15 {
        return Some(Tick::from_rational(value as i64, r.den, r.num));
    }
    Some(Tick((value * TICKS_PER_SECOND as f64 * r.den as f64 / r.num as f64).round() as i64))
}

/// Whether any `rate` in the document is beyond what [`rate_from_f64`] accepts.
fn has_absurd_rate(v: &Value) -> bool {
    match v {
        Value::Object(m) => m.iter().any(|(k, x)| if k == "rate" { x.as_f64().is_some_and(|r| r > MAX_RATE as f64) } else { has_absurd_rate(x) }),
        Value::Array(a) => a.iter().any(has_absurd_rate),
        _ => false,
    }
}

fn read_rate(v: &Value) -> Option<FrameRate> {
    v.get("rate").and_then(Value::as_f64).map(rate_from_f64)
}

fn read_tr(v: &Value) -> Option<(Tick, Tick)> {
    if v.is_null() {
        return None;
    }
    Some((read_rt(v.get("start_time")?)?, read_rt(v.get("duration")?)?))
}

fn schema(v: &Value) -> &str {
    v.get("OTIO_SCHEMA").and_then(Value::as_str).unwrap_or("")
}

fn schema_name(v: &Value) -> &str {
    schema(v).split('.').next().unwrap_or("")
}

fn name_of(v: &Value) -> String {
    v.get("name").and_then(Value::as_str).unwrap_or("").to_string()
}

/// OTIO's marker colour names.
fn color_name(l: Label) -> &'static str {
    match l {
        Label::Rose => "RED",
        Label::Mango | Label::Tan => "ORANGE",
        Label::Yellow => "YELLOW",
        Label::Green | Label::Forest => "GREEN",
        Label::Teal | Label::Caribbean => "CYAN",
        Label::Blue | Label::Iris | Label::Cerulean => "BLUE",
        Label::Violet | Label::Purple | Label::Lavender => "PURPLE",
        Label::Magenta => "MAGENTA",
        Label::Brown => "BLACK",
    }
}

fn color_label(s: &str) -> Label {
    match s.to_ascii_uppercase().as_str() {
        "RED" => Label::Rose,
        "PINK" | "MAGENTA" => Label::Magenta,
        "ORANGE" => Label::Mango,
        "YELLOW" => Label::Yellow,
        "GREEN" => Label::Green,
        "CYAN" => Label::Teal,
        "BLUE" => Label::Blue,
        "PURPLE" => Label::Purple,
        "BLACK" => Label::Brown,
        "WHITE" => Label::Lavender,
        _ => Label::Green,
    }
}

// ---------------------------------------------------------------------------------------------
// Metadata: the model's fields as JSON, and `ext`
// ---------------------------------------------------------------------------------------------

/// `fields` with the keys of `ext` (a JSON object) added after them; a key of ours wins.
fn with_ext(mut fields: Map<String, Value>, ext: &Ext, report: &mut Report) -> Value {
    if let Some(text) = ext {
        match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(extra)) => {
                for (k, v) in extra {
                    fields.entry(k).or_insert(v);
                }
            }
            _ => report.warn("extension data that is not a JSON object was not written to OTIO"),
        }
    }
    Value::Object(fields)
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// The keys of `meta` other than `known`, as the `ext` JSON object text (`None` when there are none).
fn ext_of(meta: Option<&Value>, known: &[&str]) -> Ext {
    let m = meta?.as_object()?;
    let rest: Map<String, Value> = m.iter().filter(|(k, _)| !known.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    (!rest.is_empty()).then(|| Value::Object(rest).to_string())
}

fn label_of(v: Option<&Value>) -> Option<Label> {
    v.and_then(Value::as_str).and_then(Label::from_name)
}

fn rate_json(r: FrameRate) -> Value {
    json!({"num": r.num, "den": r.den})
}

fn rate_of(v: &Value) -> Option<FrameRate> {
    let (num, den) = (v.get("num")?.as_i64()?, v.get("den")?.as_i64()?);
    (num > 0 && den > 0).then(|| FrameRate::new(num, den))
}

fn pair_of(v: Option<&Value>) -> Option<(u32, u32)> {
    let a = v?.as_array()?;
    Some((u32::try_from(a.first()?.as_u64()?).ok()?, u32::try_from(a.get(1)?.as_u64()?).ok()?))
}

fn u32_of(v: Option<&Value>) -> Option<u32> {
    v.and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok())
}

fn settings_json(s: &SequenceSettings) -> Map<String, Value> {
    obj(json!({
        "width": s.width, "height": s.height, "frame_rate": rate_json(s.frame_rate), "par": [s.par.0, s.par.1],
        "sample_rate": s.sample_rate, "drop_frame": s.drop_frame,
    }))
}

fn settings_of(v: &Value) -> Option<SequenceSettings> {
    let d = SequenceSettings::default();
    Some(SequenceSettings {
        width: u32_of(v.get("width")).filter(|w| *w > 0)?,
        height: u32_of(v.get("height")).filter(|h| *h > 0)?,
        frame_rate: v.get("frame_rate").and_then(rate_of)?,
        par: pair_of(v.get("par")).filter(|p| p.0 > 0 && p.1 > 0).unwrap_or(d.par),
        sample_rate: u32_of(v.get("sample_rate")).filter(|r| *r > 0).unwrap_or(d.sample_rate),
        drop_frame: v.get("drop_frame").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn kind_name(k: MediaKind) -> &'static str {
    match k {
        MediaKind::Movie => "Movie",
        MediaKind::AudioOnly => "AudioOnly",
        MediaKind::Still => "Still",
        MediaKind::ImageSequence => "ImageSequence",
        MediaKind::Synthetic => "Synthetic",
    }
}

fn kind_of(s: &str) -> Option<MediaKind> {
    [MediaKind::Movie, MediaKind::AudioOnly, MediaKind::Still, MediaKind::ImageSequence, MediaKind::Synthetic]
        .into_iter()
        .find(|k| kind_name(*k) == s)
}

fn info_json(i: &MediaInfo) -> Value {
    json!({
        "name": i.name, "kind": kind_name(i.kind), "duration": i.duration.0,
        "video": i.video.as_ref().map(|v| json!({"width": v.width, "height": v.height, "frame_rate": rate_json(v.frame_rate), "par": [v.par.0, v.par.1]})),
        "audio_streams": i.audio_streams.iter().map(|a| json!({"sample_rate": a.sample_rate, "channels": a.channels, "bits_per_sample": a.bits_per_sample})).collect::<Vec<_>>(),
        "start_timecode": i.start_timecode,
    })
}

fn info_of(v: &Value) -> Option<MediaInfo> {
    let video = match v.get("video") {
        Some(Value::Null) | None => None,
        Some(x) => Some(VideoStreamInfo {
            width: u32_of(x.get("width"))?,
            height: u32_of(x.get("height"))?,
            frame_rate: x.get("frame_rate").and_then(rate_of)?,
            par: pair_of(x.get("par")).unwrap_or((1, 1)),
        }),
    };
    let audio_streams = v
        .get("audio_streams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|a| {
            Some(AudioStreamInfo {
                sample_rate: u32_of(a.get("sample_rate"))?,
                channels: u32_of(a.get("channels"))?,
                bits_per_sample: u32_of(a.get("bits_per_sample")),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(MediaInfo {
        name: v.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
        kind: v.get("kind").and_then(Value::as_str).and_then(kind_of)?,
        duration: Tick(v.get("duration")?.as_i64()?),
        video,
        audio_streams,
        start_timecode: v.get("start_timecode").and_then(Value::as_i64),
    })
}

/// Generators as the same JSON FilmCraft's serde wrote: a unit variant's name, or `{"ColorMatte":
/// {"color": [r, g, b, a]}}`. `Other` writes its data as it is when it is JSON.
fn generator_json(g: &Generator) -> Value {
    match g {
        Generator::BlackVideo => json!("BlackVideo"),
        Generator::ColorMatte { color } => json!({"ColorMatte": {"color": color}}),
        Generator::TransparentVideo => json!("TransparentVideo"),
        Generator::CountingLeader => json!("CountingLeader"),
        Generator::BarsAndTone => json!("BarsAndTone"),
        Generator::Other { name, data } => serde_json::from_str(data).unwrap_or_else(|_| json!({"Other": {"name": name, "data": data}})),
    }
}

fn color_of(v: Option<&Value>) -> Option<[f32; 4]> {
    let a = v?.as_array()?;
    let c = |i: usize| a.get(i).and_then(Value::as_f64).map(|x| x as f32);
    Some([c(0)?, c(1)?, c(2)?, c(3)?])
}

fn generator_of_json(v: &Value, name: &str) -> Generator {
    match v.as_str() {
        Some("BlackVideo") => return Generator::BlackVideo,
        Some("TransparentVideo") => return Generator::TransparentVideo,
        Some("CountingLeader") => return Generator::CountingLeader,
        Some("BarsAndTone") => return Generator::BarsAndTone,
        _ => {}
    }
    if let Some(color) = color_of(v.get("ColorMatte").and_then(|c| c.get("color"))) {
        return Generator::ColorMatte { color };
    }
    if let Some(o) = v.get("Other") {
        let s = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        return Generator::Other { name: s("name"), data: s("data") };
    }
    Generator::Other { name: name.to_string(), data: v.to_string() }
}

fn interp_name(i: Interpolation) -> &'static str {
    match i {
        Interpolation::Linear => "Linear",
        Interpolation::Hold => "Hold",
        Interpolation::Bezier => "Bezier",
        Interpolation::AutoBezier => "AutoBezier",
        Interpolation::ContinuousBezier => "ContinuousBezier",
        Interpolation::EaseIn => "EaseIn",
        Interpolation::EaseOut => "EaseOut",
    }
}

fn interp_of(s: &str) -> Interpolation {
    use Interpolation::{AutoBezier, Bezier, ContinuousBezier, EaseIn, EaseOut, Hold, Linear};
    [Linear, Hold, Bezier, AutoBezier, ContinuousBezier, EaseIn, EaseOut].into_iter().find(|i| interp_name(*i) == s).unwrap_or_default()
}

/// A parameter value, externally tagged (`{"Float": 1.5}`); a NaN point component is `null`.
/// `Opaque` writes its text as it is when it is JSON.
fn value_json(v: &ParamValue) -> Value {
    let num = |x: f64| if x.is_nan() { Value::Null } else { json!(x) };
    match v {
        ParamValue::Float(x) => json!({"Float": num(*x)}),
        ParamValue::Vec2(p) => json!({"Vec2": {"x": num(p.x), "y": num(p.y)}}),
        ParamValue::Color(c) => json!({"Color": c}),
        ParamValue::Bool(b) => json!({"Bool": b}),
        ParamValue::Choice(c) => json!({"Choice": c}),
        ParamValue::Text(t) => json!({"Text": t}),
        ParamValue::Opaque(o) => serde_json::from_str(o).unwrap_or_else(|_| json!({"Opaque": o})),
    }
}

fn value_of(v: &Value) -> ParamValue {
    let f = |x: &Value| x.as_f64().unwrap_or(f64::NAN);
    if let Some(x) = v.get("Float") {
        return ParamValue::Float(f(x));
    }
    if let Some(p) = v.get("Vec2") {
        return ParamValue::Vec2(Vec2::new(p.get("x").map_or(f64::NAN, f), p.get("y").map_or(f64::NAN, f)));
    }
    if let Some(c) = color_of(v.get("Color")) {
        return ParamValue::Color(c);
    }
    if let Some(b) = v.get("Bool").and_then(Value::as_bool) {
        return ParamValue::Bool(b);
    }
    if let Some(c) = u32_of(v.get("Choice")) {
        return ParamValue::Choice(c);
    }
    if let Some(t) = v.get("Text").and_then(Value::as_str) {
        return ParamValue::Text(t.to_string());
    }
    if let Some(t) = v.get("Opaque").and_then(Value::as_str) {
        return ParamValue::Opaque(t.to_string());
    }
    ParamValue::Opaque(v.to_string())
}

const EFFECT_KEYS: [&str; 3] = ["effect", "enabled", "params"];

fn effects_json(effects: &[EffectInstance], report: &mut Report) -> Value {
    Value::Array(
        effects
            .iter()
            .map(|e| {
                let params: Map<String, Value> = e
                    .params
                    .iter()
                    .map(|(k, p)| {
                        let keys: Vec<Value> = p
                            .keyframes
                            .iter()
                            .map(|k| {
                                json!({"time": k.time.0, "value": value_json(&k.value), "interp": interp_name(k.interp),
                                    "out_influence": k.out_influence, "in_influence": k.in_influence})
                            })
                            .collect();
                        (k.clone(), json!({"value": value_json(&p.value), "keyframes": keys}))
                    })
                    .collect();
                with_ext(obj(json!({"effect": e.effect, "enabled": e.enabled, "params": params})), &e.ext, report)
            })
            .collect(),
    )
}

fn effects_of(v: Option<&Value>) -> Option<Vec<EffectInstance>> {
    let list = v?.as_array()?;
    let mut out = Vec::with_capacity(list.len());
    for e in list {
        let mut params = std::collections::BTreeMap::new();
        for (k, p) in e.get("params").and_then(Value::as_object).into_iter().flatten() {
            let keyframes = p
                .get("keyframes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|k| {
                    let mut kf = Keyframe::new(Tick(k.get("time")?.as_i64()?), value_of(k.get("value")?));
                    kf.interp = interp_of(k.get("interp").and_then(Value::as_str).unwrap_or(""));
                    kf.out_influence = k.get("out_influence").and_then(Value::as_f64).unwrap_or(kf.out_influence);
                    kf.in_influence = k.get("in_influence").and_then(Value::as_f64).unwrap_or(kf.in_influence);
                    Some(kf)
                })
                .collect();
            params.insert(k.clone(), Param { value: p.get("value").map_or(ParamValue::Opaque("null".into()), value_of), keyframes });
        }
        out.push(EffectInstance {
            effect: e.get("effect")?.as_str()?.to_string(),
            enabled: e.get("enabled").and_then(Value::as_bool).unwrap_or(true),
            params,
            ext: ext_of(Some(e), &EFFECT_KEYS),
        });
    }
    Some(out)
}

fn align_name(a: TransitionAlign) -> &'static str {
    match a {
        TransitionAlign::CenterAtCut => "CenterAtCut",
        TransitionAlign::StartAtCut => "StartAtCut",
        TransitionAlign::EndAtCut => "EndAtCut",
    }
}

fn align_of(v: Option<&Value>) -> Option<TransitionAlign> {
    match v?.as_str()? {
        "CenterAtCut" => Some(TransitionAlign::CenterAtCut),
        "StartAtCut" => Some(TransitionAlign::StartAtCut),
        "EndAtCut" => Some(TransitionAlign::EndAtCut),
        _ => None,
    }
}

fn marker_kind_name(k: MarkerKind) -> &'static str {
    match k {
        MarkerKind::Comment => "Comment",
        MarkerKind::Chapter => "Chapter",
        MarkerKind::Segmentation => "Segmentation",
        MarkerKind::WebLink => "WebLink",
        MarkerKind::FlashCue => "FlashCue",
    }
}

fn marker_kind_of(v: Option<&Value>) -> MarkerKind {
    use MarkerKind::{Chapter, Comment, FlashCue, Segmentation, WebLink};
    v.and_then(Value::as_str)
        .and_then(|s| [Comment, Chapter, Segmentation, WebLink, FlashCue].into_iter().find(|k| marker_kind_name(*k) == s))
        .unwrap_or_default()
}

fn channels_name(c: AudioChannels) -> &'static str {
    match c {
        AudioChannels::Mono => "Mono",
        AudioChannels::Stereo => "Stereo",
        AudioChannels::Surround51 => "Surround51",
        AudioChannels::Adaptive => "Adaptive",
    }
}

fn channels_of(v: Option<&Value>) -> Option<AudioChannels> {
    use AudioChannels::{Adaptive, Mono, Stereo, Surround51};
    v.and_then(Value::as_str).and_then(|s| [Mono, Stereo, Surround51, Adaptive].into_iter().find(|c| channels_name(*c) == s))
}

// ---------------------------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------------------------

/// Most clips an export writes inside nested sequences. OTIO has no references: every use of a nested
/// sequence is written in full, so a compound clip that holds three uses of another, nested ten deep,
/// is 3^10 copies, and a 3 KB document made the export run for minutes and grow to gigabytes. Each
/// clip is about 10 KB of JSON, so this bounds the extra output near 250 MB; real projects use a
/// few thousand. Past it, further nested sequences are written as gaps, with a warning.
const MAX_NESTED_ITEMS: usize = 25_000;

struct Exp<'a, 'r> {
    p: &'a Project,
    opts: &'a ExportOptions<'a>,
    report: &'r mut Report,
    stack: Vec<ItemId>,
    /// Clips written inside nested sequences so far, and the limit for it (see [`MAX_NESTED_ITEMS`]).
    nested_items: usize,
    nested_limit: usize,
}

/// Write `sequence` (and the media and nested sequences it uses) as an OTIO `Timeline`.
pub fn export(p: &Project, seq_id: ItemId, opts: &ExportOptions, report: &mut Report) -> Result<String> {
    let seq = p.sequence(seq_id).ok_or(Error::NoSequence(seq_id))?;
    let rate = seq.settings.frame_rate;
    let mut x = Exp { p, opts, report, stack: vec![seq_id], nested_items: 0, nested_limit: MAX_NESTED_ITEMS };
    let name = opts.name.map(str::to_string).unwrap_or_else(|| p.item(seq_id).map(|i| i.name.clone()).unwrap_or_default());
    let stack = x.stack_json(seq_id, &name, None);
    let mut meta = Map::new();
    meta.insert(opts.namespace.to_string(), x.seq_meta(seq_id));
    let tl = json!({
        "OTIO_SCHEMA": "Timeline.1",
        "name": name,
        "global_start_time": rt(rate.tick_of(seq.start_timecode), rate),
        "metadata": meta,
        "tracks": stack,
    });
    serde_json::to_string_pretty(&tl).map_err(|e| Error::Other(e.to_string()))
}

impl Exp<'_, '_> {
    /// `{namespace: m}`.
    fn ns(&self, m: Value) -> Value {
        let mut o = Map::new();
        o.insert(self.opts.namespace.to_string(), m);
        Value::Object(o)
    }

    /// A sequence's metadata: its item, label, settings and start timecode, then the item's `ext`.
    fn seq_meta(&mut self, id: ItemId) -> Value {
        let Some((it, s)) = self.p.item(id).and_then(|it| Some((it, it.as_sequence()?))) else { return json!({}) };
        let m = obj(json!({"item": id.0, "label": it.label.name(), "settings": settings_json(&s.settings), "start_timecode": s.start_timecode}));
        with_ext(m, &it.ext, self.report)
    }

    /// The `Stack` for a sequence; `range` = (source in, duration) when nested in a track.
    fn stack_json(&mut self, id: ItemId, name: &str, range: Option<(Tick, Tick, FrameRate)>) -> Value {
        let Some(seq) = self.p.sequence(id) else {
            self.report.warn("missing nested sequence skipped");
            return json!({"OTIO_SCHEMA": "Stack.1", "name": name, "children": [], "effects": [], "markers": [], "enabled": true, "metadata": {}});
        };
        let rate = seq.settings.frame_rate;
        let mut tracks = Vec::new();
        for kind in [TrackKind::Video, TrackKind::Audio] {
            for t in seq.tracks(kind) {
                tracks.push(self.track_json(t, kind, seq));
            }
        }
        let markers: Vec<Value> = seq.markers.iter().map(|m| self.marker_json(m, rate)).collect();
        let metadata = if range.is_some() {
            let m = self.seq_meta(id);
            self.ns(m)
        } else {
            json!({})
        };
        json!({
            "OTIO_SCHEMA": "Stack.1",
            "name": name,
            "children": tracks,
            "source_range": range.map(|(s, d, r)| tr(s, d, r)).unwrap_or(Value::Null),
            "effects": [],
            "markers": markers,
            "enabled": true,
            "metadata": metadata,
        })
    }

    fn track_json(&mut self, t: &Track, kind: TrackKind, seq: &Sequence) -> Value {
        let rate = seq.settings.frame_rate;
        let mut children = Vec::new();
        let mut pos = Tick::ZERO;
        for it in &t.items {
            if it.start > pos {
                children.push(json!({"OTIO_SCHEMA": "Gap.1", "name": "", "source_range": tr(Tick::ZERO, it.start - pos, rate), "effects": [], "markers": [], "enabled": true, "metadata": {}}));
            }
            for x in t.transitions.iter().filter(|x| x.to == Some(it.id)) {
                children.push(self.transition_json(x, it.start, rate));
            }
            children.push(self.item_json(it, kind, rate));
            for x in t.transitions.iter().filter(|x| x.from == Some(it.id) && x.to.is_none()) {
                children.push(self.transition_json(x, it.end(), rate));
            }
            pos = it.end();
        }
        let meta = with_ext(obj(json!({"locked": t.locked, "channels": channels_name(t.channels)})), &t.ext, self.report);
        json!({
            "OTIO_SCHEMA": "Track.1",
            "name": t.name,
            "kind": if kind == TrackKind::Video { "Video" } else { "Audio" },
            "children": children,
            "source_range": null,
            "effects": [],
            "markers": [],
            "enabled": t.enabled,
            "metadata": self.ns(meta),
        })
    }

    fn transition_json(&mut self, x: &Transition, cut: Tick, rate: FrameRate) -> Value {
        let dissolve = matches!(x.effect.effect.as_str(), "cross_dissolve" | "constant_power" | "constant_gain" | "film_dissolve");
        let effect = effects_json(std::slice::from_ref(&x.effect), self.report).as_array().and_then(|a| a.first().cloned()).unwrap_or(Value::Null);
        let meta = with_ext(obj(json!({"effect": effect, "align": align_name(x.align), "reverse": x.reverse})), &x.ext, self.report);
        json!({
            "OTIO_SCHEMA": "Transition.1",
            "name": transition_name(self.opts.catalog, &x.effect),
            "transition_type": if dissolve { "SMPTE_Dissolve" } else { "Custom_Transition" },
            "in_offset": rt(cut - x.start, rate),
            "out_offset": rt(x.end() - cut, rate),
            "metadata": self.ns(meta),
        })
    }

    fn media_ref(&mut self, item: ItemId, rate: FrameRate) -> Value {
        let name = self.p.item(item).map(|i| i.name.clone()).unwrap_or_default();
        let Some(m) = item_media(self.p, item) else {
            return json!({"OTIO_SCHEMA": "MissingReference.1", "name": name, "available_range": null, "metadata": {}});
        };
        let avail = tr(m.info.start_timecode.map(|f| m.frame_rate().tick_of(f)).unwrap_or(Tick::ZERO), m.info.duration, rate);
        let it = self.p.item(item);
        let mut fields = obj(json!({"item": item.0, "label": it.map(|i| i.label.name()), "info": info_json(&m.info), "offline": m.offline}));
        if let MediaRef::Generator(g) = &m.media {
            fields.insert("generator".into(), generator_json(g));
        }
        let ext = it.and_then(|i| i.ext.clone());
        let meta = with_ext(fields, &ext, self.report);
        match &m.media {
            MediaRef::Generator(g) => json!({
                "OTIO_SCHEMA": "GeneratorReference.1",
                "name": name,
                "generator_kind": match g { Generator::ColorMatte { .. } => "SolidColor", Generator::BlackVideo => "black", Generator::BarsAndTone => "SMPTEBars", _ => self.opts.namespace },
                "parameters": match g { Generator::ColorMatte { color } => json!({"color": color}), _ => json!({}) },
                "available_range": avail,
                "metadata": self.ns(meta),
            }),
            MediaRef::File { path } => {
                let url = match self.opts.relative_to {
                    Some(base) => relative_path(path, base).map(|r| relative_url(&r)).unwrap_or_else(|| abs_url(path)),
                    None => abs_url(path),
                };
                json!({"OTIO_SCHEMA": "ExternalReference.1", "name": name, "target_url": url, "available_range": avail, "available_image_bounds": null, "metadata": self.ns(meta)})
            }
        }
    }

    fn clip_meta(&mut self, it: &TrackItem) -> Value {
        let fields = obj(json!({
            "label": it.label.name(), "link": it.link, "group": it.group, "effects": effects_json(&it.effects, self.report),
            "gain_db": it.gain_db, "scale_to_frame": it.scale_to_frame, "source_in_ticks": it.source_in.0,
            "duration_ticks": it.duration.0,
        }));
        with_ext(fields, &it.ext, self.report)
    }

    fn item_json(&mut self, it: &TrackItem, kind: TrackKind, rate: FrameRate) -> Value {
        if self.stack.len() > 1 {
            self.nested_items = self.nested_items.saturating_add(1);
        }
        let base = base_item(self.p, it.item);
        let mut effects = Vec::new();
        if let Some(h) = it.frame_hold {
            let meta = self.ns(json!({"hold": h.0}));
            effects.push(json!({"OTIO_SCHEMA": "FreezeFrame.1", "name": "", "effect_name": "FreezeFrame", "time_scalar": 0.0, "metadata": meta}));
        } else if it.speed != 1.0 || it.reverse {
            let s = if it.reverse { -it.speed } else { it.speed };
            effects.push(json!({"OTIO_SCHEMA": "LinearTimeWarp.1", "name": "", "effect_name": "LinearTimeWarp", "time_scalar": s, "metadata": {}}));
        }
        let meta = self.clip_meta(it);
        let _ = kind;
        if matches!(self.p.item(base).map(|i| &i.kind), Some(ItemKind::Sequence(_))) {
            if self.stack.contains(&base) {
                self.report.warn("recursive nested sequence skipped");
                return json!({"OTIO_SCHEMA": "Gap.1", "name": "", "source_range": tr(Tick::ZERO, it.duration, rate), "effects": [], "markers": [], "enabled": true, "metadata": {}});
            }
            if self.nested_items >= self.nested_limit {
                // once: later nested sequences hit this too, and the message would repeat
                if self.nested_items < usize::MAX {
                    self.nested_items = usize::MAX;
                    self.report.warn(format!(
                        "more than {} clips written inside nested sequences: the rest are gaps (OTIO repeats a nested sequence for every use)",
                        self.nested_limit
                    ));
                }
                return json!({"OTIO_SCHEMA": "Gap.1", "name": "", "source_range": tr(Tick::ZERO, it.duration, rate), "effects": [], "markers": [], "enabled": true, "metadata": {}});
            }
            let markers: Vec<Value> = it.markers.iter().map(|m| self.marker_json(m, rate)).collect();
            self.stack.push(base);
            let mut s = self.stack_json(base, &it.name, Some((it.source_in, it.duration, rate)));
            self.stack.pop();
            s["effects"] = json!(effects);
            s["enabled"] = json!(it.enabled);
            let mut clip = meta;
            // Nested sequence markers stay on the Stack; clip instance markers live on the clip metadata.
            clip["markers"] = json!(markers);
            s["metadata"][self.opts.namespace]["clip"] = clip;
            return s;
        }
        let _ = generator_of(self.p, base);
        if let Some(clip) = crate::build::uncarried_clip(self.p, it, rate) {
            self.report.warn(format!(
                "{clip} has no OTIO equivalent: it is written as a clip with a missing media reference and is read back as offline media"
            ));
        }
        let mref = self.media_ref(base, rate);
        // OTIO source times are in the reference's time (which starts at its available_range start).
        let origin = item_media(self.p, base).and_then(|m| m.info.start_timecode.map(|f| m.frame_rate().tick_of(f))).unwrap_or(Tick::ZERO);
        let markers: Vec<Value> = it.markers.iter().map(|m| self.marker_json(&Marker { start: m.start + origin, ..m.clone() }, rate)).collect();
        json!({
            "OTIO_SCHEMA": "Clip.2",
            "name": it.name,
            "source_range": tr(origin + it.source_in, it.duration, rate),
            "effects": effects,
            "markers": markers,
            "enabled": it.enabled,
            "media_references": {"DEFAULT_MEDIA": mref},
            "active_media_reference_key": "DEFAULT_MEDIA",
            "metadata": self.ns(meta),
        })
    }

    fn marker_json(&mut self, m: &Marker, rate: FrameRate) -> Value {
        let fields = obj(
            json!({"kind": marker_kind_name(m.kind), "url": m.url, "label": m.color.name(), "author": m.author, "start_ticks": m.start.0, "duration_ticks": m.duration.0}),
        );
        let meta = with_ext(fields, &m.ext, self.report);
        json!({
            "OTIO_SCHEMA": "Marker.2",
            "name": m.name,
            "comment": m.comment,
            "color": color_name(m.color),
            "marked_range": tr(m.start, m.duration, rate),
            "metadata": self.ns(meta),
        })
    }
}

fn abs_url(path: &str) -> String {
    if is_absolute(path) { path_to_file_url(path, false) } else { path.to_string() }
}

// ---------------------------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------------------------

const SEQ_KEYS: [&str; 5] = ["item", "label", "settings", "start_timecode", "clip"];
const TRACK_KEYS: [&str; 2] = ["locked", "channels"];
const CLIP_KEYS: [&str; 9] = ["label", "link", "group", "effects", "gain_db", "scale_to_frame", "source_in_ticks", "duration_ticks", "markers"];
const MEDIA_KEYS: [&str; 5] = ["item", "label", "info", "offline", "generator"];
const MARKER_KEYS: [&str; 6] = ["kind", "url", "label", "author", "start_ticks", "duration_ticks"];
const TRANSITION_KEYS: [&str; 3] = ["effect", "align", "reverse"];

struct Imp<'a, 'c, 'r> {
    b: Builder<'c>,
    opts: &'a ImportOptions<'a>,
    report: &'r mut Report,
    links: HashMap<u64, u64>,
    groups: HashMap<u64, u64>,
}

/// Read an OTIO document: a `Timeline`, a `SerializableCollection` of them, a `Stack` or a bare `Track`.
pub fn import(text: &str, opts: &ImportOptions, report: &mut Report) -> Result<Imported> {
    let v: Value = serde_json::from_str(text).map_err(|e| Error::parse(Format::Otio, e.to_string()))?;
    let mut timelines = Vec::new();
    collect_timelines(&v, &mut timelines);
    if timelines.is_empty() {
        return Err(Error::parse(Format::Otio, format!("no Timeline in a {} document", schema(&v))));
    }
    if has_absurd_rate(&v) {
        report.warn(format!(
            "an OTIO rate above {MAX_RATE} per second is not a frame rate; the default 24 fps was used (this includes audio sample rates above it, such as DSD)"
        ));
    }
    let mut imp = Imp { b: Builder::new(&name_of(&v), opts.catalog), opts, report, links: HashMap::new(), groups: HashMap::new() };
    if imp.b.p.name.is_empty() {
        imp.b.p.name = "Imported OTIO".into();
    }
    for tl in timelines {
        let id = imp.timeline(tl);
        imp.b.top.push(id);
    }
    let mut imported = imp.b.finish();
    imported.bound(report);
    Ok(imported)
}

fn collect_timelines<'v>(v: &'v Value, out: &mut Vec<&'v Value>) {
    match schema_name(v) {
        "Timeline" | "Stack" => out.push(v),
        "SerializableCollection" => {
            for c in v.get("children").and_then(Value::as_array).into_iter().flatten() {
                collect_timelines(c, out);
            }
        }
        "Track" => out.push(v),
        _ => {}
    }
}

/// The rate of the first item a sequence can run at. Items at an audio sample rate (a gap or
/// clip at 48000 is valid OTIO) are skipped: no sequence runs at 48000 fps.
fn first_rate(v: &Value) -> Option<FrameRate> {
    if let Some(sr) = v.get("source_range").filter(|s| !s.is_null()) {
        return sr.get("duration").and_then(read_rate).filter(|&r| sequence_rate(r));
    }
    v.get("children").and_then(Value::as_array).into_iter().flatten().find_map(first_rate)
}

/// Whether a sequence can run at `rate`: positive, at most 1000 fps, with 32-bit parts.
fn sequence_rate(rate: FrameRate) -> bool {
    rate.num > 0
        && rate.den > 0
        && rate.as_f64() <= 1000.0
        && rate.num <= i64::from(u32::MAX)
        && rate.den <= i64::from(u32::MAX)
        && (i128::from(TICKS_PER_SECOND) * i128::from(rate.den) / i128::from(rate.num)) <= i128::from(Tick::MAX.0)
}

impl Imp<'_, '_, '_> {
    /// `metadata.<namespace>` of an object.
    fn meta<'v>(&self, v: &'v Value) -> Option<&'v Value> {
        v.get("metadata").and_then(|m| m.get(self.opts.namespace))
    }

    fn timeline(&mut self, tl: &Value) -> ItemId {
        let name = name_of(tl);
        let name = if name.is_empty() { self.opts.name.unwrap_or("Timeline").to_string() } else { name };
        let stack_owned;
        let stack = match schema_name(tl) {
            "Timeline" => tl.get("tracks").unwrap_or(&Value::Null),
            "Track" => {
                stack_owned = json!({"OTIO_SCHEMA": "Stack.1", "children": [tl.clone()]});
                &stack_owned
            }
            _ => tl,
        };
        let meta = self.meta(tl).or_else(|| self.meta(stack));
        let rate = tl.get("global_start_time").and_then(read_rate).filter(|&r| sequence_rate(r)).or_else(|| first_rate(stack)).unwrap_or_default();
        let id = self.sequence(stack, &name, meta, rate);
        if let Some(st) = tl.get("global_start_time").and_then(read_rt)
            && let Some(s) = self.b.p.sequence_mut(id)
        {
            s.start_timecode = s.settings.frame_rate.frame_at(st);
        }
        id
    }

    fn sequence(&mut self, stack: &Value, name: &str, meta: Option<&Value>, rate: FrameRate) -> ItemId {
        let settings = meta.and_then(|m| m.get("settings")).and_then(settings_of).unwrap_or_else(|| settings_for(rate, 1920, 1080, false));
        let id = self.b.reserve_sequence(name, settings.clone(), None);
        if let Some(it) = self.b.p.item_mut(id) {
            if let Some(l) = label_of(meta.and_then(|m| m.get("label"))) {
                it.label = l;
            }
            it.ext = ext_of(meta, &SEQ_KEYS);
        }
        let mut seq = empty_sequence(settings);
        seq.start_timecode = meta.and_then(|m| m.get("start_timecode")).and_then(Value::as_i64).unwrap_or(0);
        seq.markers = self.markers(stack);
        for t in stack.get("children").and_then(Value::as_array).into_iter().flatten() {
            match schema_name(t) {
                "Track" => {
                    let kind = if t.get("kind").and_then(Value::as_str) == Some("Audio") { TrackKind::Audio } else { TrackKind::Video };
                    self.track(&mut seq, t, kind);
                }
                other => self.report.warn(format!("OTIO {other} directly in a stack is not supported")),
            }
        }
        self.b.ensure_tracks(&mut seq, TrackKind::Video, 1);
        self.b.ensure_tracks(&mut seq, TrackKind::Audio, 1);
        self.b.put_sequence(id, seq);
        id
    }

    fn track(&mut self, seq: &mut Sequence, t: &Value, kind: TrackKind) {
        let idx = seq.tracks(kind).len();
        let mut track = self.b.track(kind, idx);
        let n = name_of(t);
        if !n.is_empty() {
            track.name = n;
        }
        track.enabled = t.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        if let Some(m) = self.meta(t) {
            track.locked = m.get("locked").and_then(Value::as_bool).unwrap_or(false);
            if let Some(c) = channels_of(m.get("channels")) {
                track.channels = c;
            }
            track.ext = ext_of(Some(m), &TRACK_KEYS);
        }
        if t.get("markers").and_then(Value::as_array).is_some_and(|m| !m.is_empty()) {
            let ms = self.markers(t);
            seq.markers.extend(ms);
            seq.markers.sort_by_key(|m| m.start);
        }
        // (clip at that slot or None for gaps, start, end)
        let mut slots: Vec<(Option<ClipId>, Tick, Tick)> = Vec::new();
        let mut pending: Vec<(usize, &Value)> = Vec::new();
        let mut pos = Tick::ZERO;
        let kids = t.get("children").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        for c in kids {
            match schema_name(c) {
                "Transition" => {
                    pending.push((slots.len(), c));
                    continue;
                }
                "Gap" => {
                    let d = c.get("source_range").and_then(read_tr).map(|r| r.1).unwrap_or(Tick::ZERO);
                    slots.push((None, pos, pos + d));
                    pos += d;
                }
                "Clip" | "Stack" => {
                    let item = if schema_name(c) == "Clip" { self.clip(c, kind, pos) } else { self.nested(c, kind, pos) };
                    match item {
                        Some(ti) => {
                            let (s, e, id) = (ti.start, ti.end(), ti.id);
                            track.items.push(ti);
                            slots.push((Some(id), s, e));
                            pos = e;
                        }
                        None => {
                            let d = c.get("source_range").and_then(read_tr).map(|r| r.1).unwrap_or(Tick::ZERO);
                            slots.push((None, pos, pos + d));
                            pos += d;
                        }
                    }
                }
                other => {
                    self.report.warn(format!("OTIO {other} items are not supported and were skipped"));
                }
            }
        }
        for (slot, c) in pending {
            let cut = slots.get(slot).map_or(pos, |s| s.1);
            let from = slot.checked_sub(1).and_then(|i| slots.get(i)).and_then(|s| s.0);
            let to = slots.get(slot).and_then(|s| s.0);
            if from.is_none() && to.is_none() {
                continue;
            }
            let inn = c.get("in_offset").and_then(read_rt).unwrap_or(Tick::ZERO);
            let out = c.get("out_offset").and_then(read_rt).unwrap_or(Tick::ZERO);
            let meta = self.meta(c);
            let effect: EffectInstance =
                match meta.and_then(|m| m.get("effect")).and_then(|e| effects_of(Some(&json!([e])))).and_then(|mut v| v.pop()) {
                    Some(e) => e,
                    None => {
                        let n = name_of(c);
                        let ty = c.get("transition_type").and_then(Value::as_str).unwrap_or("SMPTE_Dissolve");
                        transition_effect(self.b.catalog, if n.is_empty() { ty } else { &n }, kind == TrackKind::Audio, self.report)
                    }
                };
            let align = align_of(meta.and_then(|m| m.get("align"))).unwrap_or(if inn == Tick::ZERO {
                TransitionAlign::StartAtCut
            } else if out == Tick::ZERO {
                TransitionAlign::EndAtCut
            } else {
                TransitionAlign::CenterAtCut
            });
            let reverse = meta.and_then(|m| m.get("reverse")).and_then(Value::as_bool).unwrap_or(false);
            let ext = ext_of(meta, &TRANSITION_KEYS);
            track.transitions.push(Transition {
                id: TransitionId(self.b.alloc()),
                effect,
                start: cut - inn,
                duration: inn + out,
                from,
                to,
                align,
                reverse,
                ext,
            });
        }
        track.sort();
        seq.tracks_mut(kind).push(track);
    }

    fn apply_clip_meta(&mut self, ti: &mut TrackItem, c: &Value, meta: Option<&Value>) {
        ti.enabled = c.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        for e in c.get("effects").and_then(Value::as_array).into_iter().flatten() {
            match schema_name(e) {
                "LinearTimeWarp" => {
                    let s = e.get("time_scalar").and_then(Value::as_f64).unwrap_or(1.0);
                    if s == 0.0 {
                        ti.frame_hold = Some(ti.source_in);
                    } else {
                        ti.speed = s.abs();
                        ti.reverse = s < 0.0;
                    }
                }
                "FreezeFrame" => {
                    ti.frame_hold = Some(self.meta(e).and_then(|m| m.get("hold")).and_then(Value::as_i64).map(Tick).unwrap_or(ti.source_in));
                }
                _ => {
                    if meta.and_then(|m| m.get("effects")).is_none() {
                        let n = e.get("effect_name").and_then(Value::as_str).unwrap_or("effect");
                        self.report.warn(format!("OTIO effect \"{n}\" is not supported"));
                    }
                }
            }
        }
        let Some(m) = meta else { return };
        if let Some(l) = label_of(m.get("label")) {
            ti.label = l;
        }
        if let Some(effects) = effects_of(m.get("effects")) {
            ti.effects = effects;
        }
        if let Some(l) = m.get("link").and_then(Value::as_u64) {
            let next = self.b.link_id();
            ti.link = Some(*self.links.entry(l).or_insert(next));
        }
        if let Some(g) = m.get("group").and_then(Value::as_u64) {
            let next = self.b.alloc();
            ti.group = Some(*self.groups.entry(g).or_insert(next));
        }
        ti.gain_db = m.get("gain_db").and_then(Value::as_f64).unwrap_or(0.0);
        ti.scale_to_frame = m.get("scale_to_frame").and_then(Value::as_bool).unwrap_or(false);
        // Exact ticks for sub-frame values (audio) when the rational values were rounded.
        if let Some(s) = m.get("source_in_ticks").and_then(Value::as_i64)
            && (Tick(s) - ti.source_in).abs() < Tick(TICKS_PER_SECOND / 1000)
        {
            ti.source_in = Tick(s);
            if let Some(h) = &mut ti.frame_hold
                && c.get("effects").and_then(Value::as_array).is_some_and(|e| e.iter().any(|x| schema_name(x) == "LinearTimeWarp"))
            {
                *h = Tick(s);
            }
        }
        if let Some(d) = m.get("duration_ticks").and_then(Value::as_i64)
            && (Tick(d) - ti.duration).abs() < Tick(TICKS_PER_SECOND / 1000)
        {
            ti.duration = Tick(d);
        }
        ti.ext = ext_of(Some(m), &CLIP_KEYS);
    }

    fn clip(&mut self, c: &Value, kind: TrackKind, pos: Tick) -> Option<TrackItem> {
        let mref = match c.get("media_references") {
            Some(refs) => {
                let key = c.get("active_media_reference_key").and_then(Value::as_str).unwrap_or("DEFAULT_MEDIA");
                refs.get(key).or_else(|| refs.as_object().and_then(|o| o.values().next()))
            }
            None => c.get("media_reference"),
        }
        .unwrap_or(&Value::Null);
        let name = name_of(c);
        let avail = mref.get("available_range").and_then(read_tr);
        let (src_in, dur) = c.get("source_range").and_then(read_tr).or(avail)?;
        if dur <= Tick::ZERO {
            return None;
        }
        let item = self.media(mref, &name, kind, avail)?;
        // Sources are media time: subtract the media's start time (available_range start).
        let origin = avail.map(|a| a.0).unwrap_or(Tick::ZERO);
        let mut ti = self.b.clip(item, kind, if name.is_empty() { "Clip" } else { &name }, pos, dur, src_in - origin);
        ti.markers = self.markers(c);
        for m in &mut ti.markers {
            m.start -= origin;
        }
        let meta = self.meta(c);
        self.apply_clip_meta(&mut ti, c, meta);
        Some(ti)
    }

    fn nested(&mut self, c: &Value, kind: TrackKind, pos: Tick) -> Option<TrackItem> {
        let (src_in, dur) = c.get("source_range").and_then(read_tr)?;
        let meta = self.meta(c);
        let key = format!("stack:{}", meta.and_then(|m| m.get("item")).and_then(Value::as_u64).map(|i| i.to_string()).unwrap_or_else(|| name_of(c)));
        let item = match self.b.find_media(&key) {
            Some(i) => i,
            None => {
                let rate = first_rate(c).unwrap_or_default();
                let mut inner = c.clone();
                inner["source_range"] = Value::Null;
                let name = name_of(c);
                let id = self.sequence(&inner, if name.is_empty() { "Nested Sequence" } else { &name }, meta, rate);
                self.b.register_media(&key, id);
                id
            }
        };
        let name = name_of(c);
        let mut ti = self.b.clip(item, kind, if name.is_empty() { "Nested Sequence" } else { &name }, pos, dur, src_in);
        let clip_meta = meta.and_then(|m| m.get("clip"));
        // Stack markers belong to the nested sequence; instance markers are on <namespace>.clip.
        if let Some(cm) = clip_meta {
            ti.markers = self.markers_in(cm);
        }
        self.apply_clip_meta(&mut ti, c, clip_meta);
        Some(ti)
    }

    fn media(&mut self, r: &Value, clip_name: &str, kind: TrackKind, avail: Option<(Tick, Tick)>) -> Option<ItemId> {
        let meta = self.meta(r);
        let info = meta.and_then(|m| m.get("info")).and_then(info_of);
        let name = {
            let n = name_of(r);
            if n.is_empty() { clip_name.to_string() } else { n }
        };
        let spec = MediaSpec {
            duration: avail.map(|a| a.1),
            video: (kind == TrackKind::Video).then(|| (1920, 1080, FrameRate::default())),
            audio: (kind == TrackKind::Audio).then_some((48_000, 2)),
            start_tc: None,
            kind: None,
        };
        let id = match schema_name(r) {
            "ExternalReference" | "ImageSequenceReference" => {
                let url = r.get("target_url").and_then(Value::as_str).or_else(|| r.get("target_url_base").and_then(Value::as_str)).unwrap_or("");
                let path = resolve_url(url, self.opts.base_dir);
                let display = if name.is_empty() { file_name(&path).to_string() } else { name.clone() };
                let key = format!("file:{path}");
                let existed = self.b.find_media(&key);
                let id = self.b.file_media(&key, &display, &path, &spec, None);
                if existed.is_none() {
                    self.fill_media(id, info, meta);
                }
                id
            }
            "GeneratorReference" => {
                let g = match meta.and_then(|m| m.get("generator")) {
                    Some(g) => generator_of_json(g, &name),
                    None => match r.get("generator_kind").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase().as_str() {
                        "solidcolor" | "solid_color" => Generator::ColorMatte {
                            color: color_of(r.get("parameters").and_then(|p| p.get("color"))).unwrap_or([0.0, 0.0, 0.0, 1.0]),
                        },
                        "smptebars" | "bars" => Generator::BarsAndTone,
                        "black" => Generator::BlackVideo,
                        other => match self.b.catalog.generator_by_name(other) {
                            Some(g) => g,
                            None => {
                                self.report.warn(format!("OTIO generator \"{other}\" is not supported; imported as Black Video"));
                                Generator::BlackVideo
                            }
                        },
                    },
                };
                // Equal generator content can belong to distinct project items; repeated item IDs share media.
                let key = match meta.and_then(|m| m.get("item")).and_then(Value::as_u64) {
                    Some(item) => format!("gen:item:{item}"),
                    None => format!("gen:{}", generator_json(&g)),
                };
                let existed = self.b.find_media(&key);
                let id = self.b.generator_media(&key, if name.is_empty() { "Generator" } else { &name }, g, &spec, None);
                if existed.is_none() {
                    self.fill_media(id, info, meta);
                }
                id
            }
            _ => {
                if schema_name(r) != "MissingReference" && !r.is_null() {
                    self.report.warn(format!("OTIO media reference {} is not supported; imported offline", schema(r)));
                }
                let key = format!("missing:{name}");
                let existed = self.b.find_media(&key);
                let path = resolve_path(&name, self.opts.base_dir);
                let id = self.b.file_media(&key, &name, &path, &spec, None);
                if existed.is_none() {
                    self.fill_media(id, info, meta);
                    if let Some(m) = self.b.p.item_mut(id).and_then(|i| i.as_media_mut()) {
                        m.offline = true;
                    }
                }
                id
            }
        };
        Some(id)
    }

    fn fill_media(&mut self, id: ItemId, info: Option<MediaInfo>, meta: Option<&Value>) {
        let label = label_of(meta.and_then(|m| m.get("label")));
        let offline = meta.and_then(|m| m.get("offline")).and_then(Value::as_bool);
        let ext = ext_of(meta, &MEDIA_KEYS);
        let Some(it) = self.b.p.item_mut(id) else { return };
        it.ext = ext;
        if let Some(l) = label {
            it.label = l;
        }
        if let Some(m) = it.as_media_mut() {
            if let Some(i) = info {
                m.info = i;
            }
            if let Some(o) = offline {
                m.offline = o;
            }
        }
    }

    /// The markers of an OTIO object (`v.markers`).
    fn markers(&mut self, v: &Value) -> Vec<Marker> {
        self.markers_in(v)
    }

    fn markers_in(&mut self, v: &Value) -> Vec<Marker> {
        let mut out = Vec::new();
        for m in v.get("markers").and_then(Value::as_array).into_iter().flatten() {
            let (mut start, mut duration) = m.get("marked_range").and_then(read_tr).unwrap_or((Tick::ZERO, Tick::ZERO));
            let meta = self.meta(m);
            if let Some(s) = meta.and_then(|x| x.get("start_ticks")).and_then(Value::as_i64)
                && (Tick(s) - start).abs() < Tick(TICKS_PER_SECOND / 1000)
            {
                start = Tick(s);
            }
            if let Some(d) = meta.and_then(|x| x.get("duration_ticks")).and_then(Value::as_i64)
                && (Tick(d) - duration).abs() < Tick(TICKS_PER_SECOND / 1000)
            {
                duration = Tick(d);
            }
            let color =
                label_of(meta.and_then(|x| x.get("label"))).unwrap_or_else(|| color_label(m.get("color").and_then(Value::as_str).unwrap_or("")));
            out.push(Marker {
                url: meta.and_then(|x| x.get("url")).and_then(Value::as_str).unwrap_or("").to_string(),
                author: meta.and_then(|x| x.get("author")).and_then(Value::as_str).unwrap_or("").to_string(),
                id: MarkerId(self.b.alloc()),
                start,
                duration,
                name: name_of(m),
                comment: m.get("comment").and_then(Value::as_str).unwrap_or("").to_string(),
                kind: marker_kind_of(meta.and_then(|x| x.get("kind"))),
                color,
                ext: ext_of(meta, &MARKER_KEYS),
            });
        }
        out.sort_by_key(|m| m.start);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "xml")]
    fn iopts() -> ImportOptions<'static> {
        ImportOptions { base_dir: None, name: None, catalog: &crate::fx::Standard, namespace: "test" }
    }

    #[cfg(feature = "xml")]
    fn xopts() -> ExportOptions<'static> {
        ExportOptions {
            relative_to: None,
            name: None,
            catalog: &crate::fx::Standard,
            namespace: "test",
            xmeml_version: 4,
            fcpxml_version: Default::default(),
        }
    }

    #[test]
    fn rational_time() {
        for r in FrameRate::COMMON {
            for f in [0i64, 1, 1799, 86_400, 10_000_001] {
                let t = r.tick_of(f);
                assert_eq!(read_rt(&rt(t, r)), Some(t), "{r} {f}");
            }
        }
        // 23.976 written by other tools as a rounded float
        let v = json!({"OTIO_SCHEMA": "RationalTime.1", "rate": 23.976, "value": 48.0});
        assert_eq!(read_rt(&v), Some(FrameRate::FPS_23_976.tick_of(48)));
        // sub-frame audio time survives
        let t = Tick(TICKS_PER_SECOND / 48_000 * 12_345);
        let back = read_rt(&rt(t, FrameRate::FPS_24)).unwrap();
        assert!((back - t).abs() <= Tick(1));
    }

    #[cfg(feature = "xml")]
    /// A tree of compound clips: `uses` uses of the next level in each, `levels` media levels deep, leaf clips in the last.
    fn nested_tree(levels: usize, uses: usize) -> String {
        let mut xml = String::from(
            r#"<?xml version="1.0"?><fcpxml version="1.10"><resources><format id="f1" frameDuration="100/2500s" width="1920" height="1080"/>
<asset id="a1" name="c" start="0s" duration="10s" hasVideo="1" format="f1"><media-rep kind="original-media" src="file:///tmp/c.mov"/></asset>"#,
        );
        for level in 0..levels {
            let inner: String = (0..uses)
                .map(|i| {
                    if level + 1 < levels {
                        format!(r#"<ref-clip ref="m{}" offset="{i}s" duration="1s"/>"#, level + 1)
                    } else {
                        format!(r#"<asset-clip ref="a1" offset="{i}s" duration="1s"/>"#)
                    }
                })
                .collect();
            xml.push_str(&format!(
                r#"<media id="m{level}" name="L{level}"><sequence format="f1" tcStart="0s"><spine>{inner}</spine></sequence></media>"#
            ));
        }
        xml.push_str(r#"</resources><library><event name="E"><project name="P"><sequence format="f1" tcStart="0s"><spine><ref-clip ref="m0" offset="0s" duration="1s"/></spine></sequence></project></event></library></fcpxml>"#);
        xml
    }

    /// OTIO repeats a nested sequence for every use, so 10 uses nested 3 deep is 111 copies and a 3 KB
    /// document nested 12 deep made the export run for minutes and grow to gigabytes. Past the limit on clips
    /// written inside nested sequences the rest are gaps, with one warning. The test lowers the limit so it
    /// stays tiny: without the check all 112 stacks are written.
    #[cfg(feature = "xml")]
    #[test]
    fn nested_copies_stop_at_the_limit() {
        let imported = crate::fcpxml::import(&nested_tree(3, 10), &iopts(), &mut Report::default()).expect("import succeeds");
        let seq = imported.sequences[0];
        let opts = xopts();
        let write = |limit: usize| {
            let mut report = Report::default();
            let mut x = Exp { p: &imported.project, opts: &opts, report: &mut report, stack: vec![seq], nested_items: 0, nested_limit: limit };
            let stacks = x.stack_json(seq, "t", None).to_string().matches("\"Stack.1\"").count();
            (stacks, report)
        };
        let (all, report) = write(usize::MAX - 1);
        assert_eq!(all, 112);
        assert!(!report.mentions("nested sequences"), "{report}");
        let (capped, report) = write(200);
        assert!(capped < 60, "{capped} stacks");
        assert!(report.mentions("clips written inside nested sequences"), "{report}");
        assert_eq!(report.warnings().filter(|w| w.message.contains("inside nested sequences")).map(|w| w.count).sum::<u32>(), 1);
    }

    /// An ordinary project with nested sequences is far below the real limit and exports untouched.
    #[cfg(feature = "xml")]
    #[test]
    fn ordinary_nesting_is_not_capped() {
        let imported = crate::fcpxml::import(&nested_tree(3, 4), &iopts(), &mut Report::default()).expect("import succeeds");
        let mut report = Report::default();
        export(&imported.project, imported.sequences[0], &xopts(), &mut report).expect("export succeeds");
        assert!(!report.mentions("inside nested sequences"), "{report}");
    }
}

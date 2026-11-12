//! Test helpers: building projects, a catalog with a few application effects, and comparing edit
//! structure.
#![allow(dead_code)]

use std::ops::Range;

use craft_timeline::fx::Standard;
use craft_timeline::model::{
    AudioStreamInfo, Catalog, ClipId, EffectDef, EffectInstance, EffectKind, Generator, ItemId, ItemKind, Label, MediaClip, MediaInfo, MediaKind,
    MediaRef, ParamDef, ParamValue, Project, Sequence, SequenceSettings, Track, TrackId, TrackItem, TrackKind, Transition, TransitionAlign,
    TransitionId, VideoStreamInfo,
};
use craft_timeline::time::{FrameRate, TICKS_PER_SECOND, Tick, TimeRange};
use craft_timeline::{Error, ExportOptions, Format, ImportOptions, Imported, Report, fcp7, fcpxml, otio};

/// [`Standard`] and a few effects an application adds: a blur, a grade, two transitions.
pub struct TestCatalog;

fn extra() -> Vec<EffectDef> {
    let def = |id: &str, name: &str, kind, category: &str, params: Vec<ParamDef>| EffectDef {
        id: id.into(),
        name: name.into(),
        kind,
        category: category.into(),
        intrinsic: false,
        params,
    };
    let float = |id: &str, label: &str, v: f64| ParamDef { id: id.into(), label: label.into(), default: ParamValue::Float(v) };
    vec![
        def("gaussian_blur", "Gaussian Blur", EffectKind::Video, "Blur & Sharpen", vec![float("blurriness", "Blurriness", 0.0)]),
        def("lumetri", "Lumetri Color", EffectKind::Video, "Color Correction", vec![float("exposure", "Exposure", 0.0)]),
        def("dip_to_white", "Dip to White", EffectKind::VideoTransition, "Dissolve", vec![]),
        def("push", "Push", EffectKind::VideoTransition, "Slide", vec![]),
        def("iris_round", "Iris Round", EffectKind::VideoTransition, "Iris", vec![]),
    ]
}

impl Catalog for TestCatalog {
    fn effect(&self, id: &str) -> Option<EffectDef> {
        Standard.effect(id).or_else(|| extra().into_iter().find(|d| d.id == id))
    }
    fn effect_by_name(&self, name: &str) -> Option<EffectDef> {
        Standard.effect_by_name(name).or_else(|| extra().into_iter().find(|d| d.name.eq_ignore_ascii_case(name)))
    }
    fn intrinsic(&self, kind: TrackKind) -> Vec<EffectInstance> {
        Standard.intrinsic(kind)
    }
}

pub const NS: &str = "testapp";

pub fn iopts(base_dir: Option<&str>) -> ImportOptions<'_> {
    ImportOptions { base_dir, name: None, catalog: &TestCatalog, namespace: NS }
}

pub fn xopts() -> ExportOptions<'static> {
    ExportOptions { relative_to: None, name: None, catalog: &TestCatalog, namespace: NS, xmeml_version: 4, fcpxml_version: Default::default() }
}

pub fn instance(id: &str) -> EffectInstance {
    TestCatalog.effect(id).unwrap().instance()
}

pub fn media_info(name: &str, video: bool, audio: bool, rate: FrameRate, secs: i64) -> MediaInfo {
    MediaInfo {
        name: name.into(),
        kind: if video { MediaKind::Movie } else { MediaKind::AudioOnly },
        duration: Tick(secs * TICKS_PER_SECOND),
        video: video.then_some(VideoStreamInfo { width: 1920, height: 1080, frame_rate: rate, par: (1, 1) }),
        audio_streams: audio.then_some(AudioStreamInfo { sample_rate: 48_000, channels: 2, bits_per_sample: None }).into_iter().collect(),
        start_timecode: None,
    }
}

fn media_clip(media: MediaRef, info: MediaInfo) -> ItemKind {
    ItemKind::Media(Box::new(MediaClip { media, info, interpret_frame_rate: None, markers: vec![], offline: false }))
}

pub fn media(p: &mut Project, path: &str, video: bool, audio: bool, rate: FrameRate) -> ItemId {
    let name = path.rsplit('/').next().unwrap().to_string();
    let info = media_info(&name, video, audio, rate, 600);
    p.add_item(&name, Label::Iris, media_clip(MediaRef::File { path: path.into() }, info), None)
}

pub fn generator(p: &mut Project, name: &str, g: Generator) -> ItemId {
    let mut info = media_info(name, true, false, FrameRate::FPS_24, 600);
    info.kind = MediaKind::Synthetic;
    p.add_item(name, Label::Lavender, media_clip(MediaRef::Generator(g), info), None)
}

/// A sequence with three video and four audio tracks.
pub fn sequence(p: &mut Project, name: &str, rate: FrameRate, df: bool) -> ItemId {
    let mut seq = Sequence::new(SequenceSettings { frame_rate: rate, drop_frame: df, ..Default::default() });
    for (kind, n, word) in [(TrackKind::Video, 3, "Video"), (TrackKind::Audio, 4, "Audio")] {
        for i in 0..n {
            let id = TrackId(p.alloc_id());
            seq.tracks_mut(kind).push(Track::new(id, kind, format!("{word} {}", i + 1)));
        }
    }
    p.add_item(name, Label::Forest, ItemKind::Sequence(Box::new(seq)), None)
}

/// A clip of `item` as an editor puts one on a track: its intrinsic effects, its length snapped to frames.
pub fn track_item(p: &mut Project, item: ItemId, kind: TrackKind, start: Tick, source: TimeRange, rate: FrameRate) -> TrackItem {
    let (name, label) = p.item(item).map(|i| (i.name.clone(), i.label)).unwrap();
    TrackItem {
        id: ClipId(p.alloc_id()),
        item,
        name,
        label,
        start,
        duration: rate.snap_nearest(source.duration).max(rate.frame_duration()),
        source_in: source.start,
        speed: 1.0,
        reverse: false,
        enabled: true,
        link: None,
        group: None,
        effects: TestCatalog.intrinsic(kind),
        markers: vec![],
        gain_db: 0.0,
        frame_hold: None,
        scale_to_frame: false,
        source_channels: vec![],
        ext: None,
    }
}

/// Place a clip (frames at the sequence rate). Returns its id.
pub fn clip(p: &mut Project, seq: ItemId, kind: TrackKind, track: usize, item: ItemId, frames: Range<i64>, src: i64) -> ClipId {
    let rate = p.sequence(seq).unwrap().settings.frame_rate;
    let duration = frames.end - frames.start;
    let ti = track_item(p, item, kind, rate.tick_of(frames.start), TimeRange::new(rate.tick_of(src), rate.tick_of(duration)), rate);
    let id = ti.id;
    let t = &mut p.sequence_mut(seq).unwrap().tracks_mut(kind)[track];
    t.items.push(ti);
    t.sort();
    id
}

pub fn link(p: &mut Project, seq: ItemId, clips: &[ClipId]) {
    let l = p.alloc_id();
    let s = p.sequence_mut(seq).unwrap();
    for c in clips {
        s.find_item_mut(*c).unwrap().1.link = Some(l);
    }
}

pub fn transition(
    p: &mut Project,
    seq: ItemId,
    track: (TrackKind, usize),
    effect: &str,
    clips: (Option<ClipId>, Option<ClipId>),
    frames: Range<i64>,
    align: TransitionAlign,
) {
    let id = TransitionId(p.alloc_id());
    let rate = p.sequence(seq).unwrap().settings.frame_rate;
    let t = &mut p.sequence_mut(seq).unwrap().tracks_mut(track.0)[track.1];
    t.transitions.push(Transition {
        id,
        effect: instance(effect),
        start: rate.tick_of(frames.start),
        duration: rate.tick_of(frames.end - frames.start),
        from: clips.0,
        to: clips.1,
        align,
        reverse: false,
        ext: None,
    });
    t.sort();
}

/// A comparable description of one clip.
#[derive(Clone, Debug, PartialEq)]
pub struct C {
    pub start: Tick,
    pub dur: Tick,
    pub src: Tick,
    pub media: String,
    pub speed: String,
    pub reverse: bool,
    pub enabled: bool,
}

/// A comparable description of one transition (from/to as clip indices on the track).
#[derive(Clone, Debug, PartialEq)]
pub struct T {
    pub start: Tick,
    pub dur: Tick,
    pub effect: String,
    pub align: TransitionAlign,
    pub from: Option<usize>,
    pub to: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tr {
    pub clips: Vec<C>,
    pub transitions: Vec<T>,
}

pub fn media_key(p: &Project, item: ItemId) -> String {
    match &p.item(item).unwrap().kind {
        ItemKind::Media(m) => match &m.media {
            MediaRef::File { path } => path.clone(),
            MediaRef::Generator(g) => format!("gen:{}", g.label()),
        },
        ItemKind::Sequence(_) => format!("seq:{}", p.item(item).unwrap().name),
        _ => "other".into(),
    }
}

/// Non-empty tracks of a sequence (trailing empty tracks ignored).
pub fn structure(p: &Project, seq: ItemId, kind: TrackKind) -> Vec<Tr> {
    let s = p.sequence(seq).unwrap();
    let mut out: Vec<Tr> = s
        .tracks(kind)
        .iter()
        .map(|t| Tr {
            clips: t
                .items
                .iter()
                .map(|i| C {
                    start: i.start,
                    dur: i.duration,
                    src: i.frame_hold.unwrap_or(i.source_in),
                    media: media_key(p, i.item),
                    speed: if i.frame_hold.is_some() { "hold".into() } else { format!("{:.3}", i.speed) },
                    reverse: i.reverse,
                    enabled: i.enabled,
                })
                .collect(),
            transitions: t
                .transitions
                .iter()
                .map(|x| T {
                    start: x.start,
                    dur: x.duration,
                    effect: x.effect.effect.clone(),
                    align: x.align,
                    from: x.from.and_then(|f| t.items.iter().position(|i| i.id == f)),
                    to: x.to.and_then(|f| t.items.iter().position(|i| i.id == f)),
                })
                .collect(),
        })
        .collect();
    while out.last().is_some_and(|t| t.clips.is_empty()) {
        out.pop();
    }
    out
}

/// Link partition as sets of (kind, track, clip index).
pub fn links(p: &Project, seq: ItemId) -> Vec<Vec<(u8, usize, usize)>> {
    let s = p.sequence(seq).unwrap();
    let mut groups: std::collections::BTreeMap<u64, Vec<(u8, usize, usize)>> = Default::default();
    for (k, kind) in [(0u8, TrackKind::Video), (1, TrackKind::Audio)] {
        for (ti, t) in s.tracks(kind).iter().enumerate() {
            for (ci, c) in t.items.iter().enumerate() {
                if let Some(l) = c.link {
                    groups.entry(l).or_default().push((k, ti, ci));
                }
            }
        }
    }
    let mut v: Vec<Vec<_>> = groups.into_values().filter(|g| g.len() > 1).collect();
    for g in &mut v {
        g.sort();
    }
    v.sort();
    v
}

pub fn fcp7_export(p: &Project, seq: ItemId, opts: &ExportOptions) -> (String, Report) {
    let mut report = Report::default();
    let text = fcp7::export(p, seq, opts, &mut report).expect("export");
    (text, report)
}

pub fn fcp7_import(text: &str, base_dir: Option<&str>) -> (Imported, Report) {
    let mut report = Report::default();
    let imp = fcp7::import(text, &iopts(base_dir), &mut report).unwrap_or_else(|e| panic!("import failed: {e}\n{text}"));
    (imp, report)
}

pub fn fcp7_roundtrip(p: &Project, seq: ItemId, opts: &ExportOptions) -> (Imported, String, Report) {
    let (text, _) = fcp7_export(p, seq, opts);
    let (imp, rep) = fcp7_import(&text, None);
    (imp, text, rep)
}

pub fn only_seq(imp: &Imported) -> ItemId {
    assert_eq!(imp.sequences.len(), 1, "expected one top-level sequence");
    imp.sequences[0]
}

pub fn f(rate: FrameRate, n: i64) -> Tick {
    rate.tick_of(n)
}

/// Import `bytes` as `format`, as an application does: text decoded, the catalog and namespace of these tests.
pub fn import(bytes: &[u8], format: Format, base_dir: Option<&str>) -> Result<(Imported, Report), Error> {
    let text = craft_timeline::decode_text(bytes);
    let mut report = Report::default();
    let opts = iopts(base_dir);
    let imported = match format {
        Format::Fcp7Xml => fcp7::import(&text, &opts, &mut report),
        Format::Fcpxml => fcpxml::import(&text, &opts, &mut report),
        Format::Otio => otio::import(&text, &opts, &mut report),
        _ => Err(Error::Other(format!("{} is not read here", format.name()))),
    }?;
    Ok((imported, report))
}

/// Export `seq` as `format`.
pub fn export(p: &Project, seq: ItemId, format: Format, opts: &ExportOptions) -> Result<(Vec<u8>, Report), Error> {
    let mut report = Report::default();
    let text = match format {
        Format::Fcp7Xml => fcp7::export(p, seq, opts, &mut report),
        Format::Fcpxml => fcpxml::export(p, seq, opts, &mut report),
        Format::Otio => otio::export(p, seq, opts, &mut report),
        _ => Err(Error::Other(format!("{} is not written here", format.name()))),
    }?;
    Ok((text.into_bytes(), report))
}

pub fn roundtrip(p: &Project, seq: ItemId, format: Format, opts: &ExportOptions) -> (Imported, String, Report) {
    let (bytes, _rep) = export(p, seq, format, opts).expect("export");
    let text = String::from_utf8(bytes.clone()).unwrap();
    let (imp, rep) = import(&bytes, format, None).unwrap_or_else(|e| panic!("import failed: {e}\n{text}"));
    (imp, text, rep)
}

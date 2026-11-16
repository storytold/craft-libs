//! A neutral timeline: what Final Cut Pro 7 XML, FCPXML and OpenTimelineIO documents describe.
//!
//! A [`Project`] holds a tree of bins and a flat map of items: media (a file or a generator, with
//! what a document says about it), sequences, and subclips. A [`Sequence`] has video and audio
//! [`Track`]s of [`TrackItem`]s (clip instances with speed, reverse, frame hold, links, effects
//! with keyframes, markers) and [`Transition`]s. Timeline positions are sequence ticks;
//! `source_in` is media time.
//!
//! The shape is the one an editor's project has, cut down to what the formats carry. An
//! application converts its own project to and from it. Three things stay the application's:
//!
//! - **Effects.** An [`EffectInstance`] is an id with parameters. The formats know a few ids
//!   themselves (see [`fx`]); every other effect is looked up through a [`Catalog`] the caller
//!   supplies.
//! - **Generators** other than the ones every editor has: [`Generator::Other`] carries a name and
//!   an opaque payload.
//! - **Extensions.** `ext` on items (a sequence's is its item's), tracks, clips, transitions and markers is JSON
//!   text the application owns. A format that can carry application metadata (OpenTimelineIO)
//!   writes it under the application's namespace and gives it back on reading; the others ignore it.

use std::collections::BTreeMap;

use crate::time::{FrameRate, Tick, TimeRange};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub u64);
    };
}

id_type!(
    /// A project item (media, sequence, subclip).
    ItemId
);
id_type!(BinId);
id_type!(TrackId);
id_type!(
    /// A clip instance on a track.
    ClipId
);
id_type!(TransitionId);
id_type!(MarkerId);

/// A point or size in pixels (or a fraction, where a parameter says so).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const fn new(x: f64, y: f64) -> Vec2 {
        Vec2 { x, y }
    }
}

/// Application JSON carried through a document unread (see the module docs).
pub type Ext = Option<String>;

/// The sixteen label colours of Premiere Pro, which FCP7 XML writes by name (`label2`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Label {
    Violet,
    Iris,
    Caribbean,
    Lavender,
    Cerulean,
    Forest,
    Rose,
    Mango,
    Purple,
    Blue,
    Teal,
    Magenta,
    Tan,
    Green,
    Brown,
    Yellow,
}

impl Label {
    pub const ALL: [Label; 16] = [
        Label::Violet,
        Label::Iris,
        Label::Caribbean,
        Label::Lavender,
        Label::Cerulean,
        Label::Forest,
        Label::Rose,
        Label::Mango,
        Label::Purple,
        Label::Blue,
        Label::Teal,
        Label::Magenta,
        Label::Tan,
        Label::Green,
        Label::Brown,
        Label::Yellow,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Label::Violet => "Violet",
            Label::Iris => "Iris",
            Label::Caribbean => "Caribbean",
            Label::Lavender => "Lavender",
            Label::Cerulean => "Cerulean",
            Label::Forest => "Forest",
            Label::Rose => "Rose",
            Label::Mango => "Mango",
            Label::Purple => "Purple",
            Label::Blue => "Blue",
            Label::Teal => "Teal",
            Label::Magenta => "Magenta",
            Label::Tan => "Tan",
            Label::Green => "Green",
            Label::Brown => "Brown",
            Label::Yellow => "Yellow",
        }
    }

    /// The label of a name, in any case.
    pub fn from_name(s: &str) -> Option<Label> {
        Self::ALL.iter().copied().find(|l| l.name().eq_ignore_ascii_case(s))
    }
}

// ---------------------------------------------------------------------------------------------
// Media
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Movie,
    AudioOnly,
    Still,
    ImageSequence,
    /// A generator.
    Synthetic,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoStreamInfo {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    /// Pixel aspect ratio.
    pub par: (u32, u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioStreamInfo {
    pub sample_rate: u32,
    pub channels: u32,
    pub bits_per_sample: Option<u32>,
}

/// What a document says about a piece of media.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaInfo {
    pub name: String,
    pub kind: MediaKind,
    pub duration: Tick,
    pub video: Option<VideoStreamInfo>,
    pub audio_streams: Vec<AudioStreamInfo>,
    /// Timecode of the first frame, in frames at the media's rate.
    pub start_timecode: Option<i64>,
}

impl MediaInfo {
    pub fn has_video(&self) -> bool {
        self.video.is_some()
    }
    pub fn has_audio(&self) -> bool {
        !self.audio_streams.is_empty()
    }
    /// The first audio stream.
    pub fn audio(&self) -> Option<&AudioStreamInfo> {
        self.audio_streams.first()
    }
    /// The picture's rate, else the default rate.
    pub fn frame_rate(&self) -> FrameRate {
        self.video.as_ref().map(|v| v.frame_rate).unwrap_or_default()
    }
}

/// Synthetic media.
#[derive(Clone, Debug, PartialEq)]
pub enum Generator {
    BlackVideo,
    /// A solid colour (RGBA, 0..=1).
    ColorMatte {
        color: [f32; 4],
    },
    TransparentVideo,
    CountingLeader,
    BarsAndTone,
    /// A generator only the application knows: the name a document shows for it, and a payload
    /// the application reads back (formats that can, write it next to the name).
    Other {
        name: String,
        data: String,
    },
}

impl Generator {
    /// The name a document shows for it.
    pub fn label(&self) -> String {
        match self {
            Generator::BlackVideo => "Black Video".into(),
            Generator::ColorMatte { .. } => "Color Matte".into(),
            Generator::TransparentVideo => "Transparent Video".into(),
            Generator::CountingLeader => "Universal Counting Leader".into(),
            Generator::BarsAndTone => "Bars and Tone".into(),
            Generator::Other { name, .. } => name.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MediaRef {
    File { path: String },
    Generator(Generator),
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaClip {
    pub media: MediaRef,
    pub info: MediaInfo,
    /// A frame rate that overrides the file's (the editor's "interpret footage").
    pub interpret_frame_rate: Option<FrameRate>,
    pub markers: Vec<Marker>,
    /// The file is known to be missing.
    pub offline: bool,
}

impl MediaClip {
    pub fn frame_rate(&self) -> FrameRate {
        self.interpret_frame_rate.unwrap_or_else(|| self.info.frame_rate())
    }
    pub fn duration(&self) -> Tick {
        self.info.duration
    }
}

// ---------------------------------------------------------------------------------------------
// Items and bins
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    Media(Box<MediaClip>),
    Sequence(Box<Sequence>),
    /// A range of another item.
    Subclip {
        parent: ItemId,
        range: TimeRange,
    },
    /// Something no format here has an equivalent of (a title, an adjustment layer). Exports name
    /// each clip of one in the report (`<what> "<clip name>" at frame N`) and write what the
    /// format's module documents instead.
    Uncarried {
        what: String,
        width: u32,
        height: u32,
        rate: FrameRate,
        duration: Tick,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectItem {
    pub id: ItemId,
    pub name: String,
    pub label: Label,
    pub kind: ItemKind,
    /// Logging metadata by field name (`Scene`, `Tape Name`, …).
    pub metadata: BTreeMap<String, String>,
    pub ext: Ext,
}

impl ProjectItem {
    pub fn as_media(&self) -> Option<&MediaClip> {
        match &self.kind {
            ItemKind::Media(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_media_mut(&mut self) -> Option<&mut MediaClip> {
        match &mut self.kind {
            ItemKind::Media(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_sequence(&self) -> Option<&Sequence> {
        match &self.kind {
            ItemKind::Sequence(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_sequence_mut(&mut self) -> Option<&mut Sequence> {
        match &mut self.kind {
            ItemKind::Sequence(s) => Some(s),
            _ => None,
        }
    }
    pub fn has_video(&self) -> bool {
        match &self.kind {
            ItemKind::Media(m) => m.info.has_video(),
            _ => true,
        }
    }
    pub fn has_audio(&self) -> bool {
        match &self.kind {
            ItemKind::Media(m) => m.info.has_audio(),
            ItemKind::Sequence(s) => !s.audio_tracks.is_empty(),
            _ => false,
        }
    }
    pub fn duration(&self) -> Tick {
        match &self.kind {
            ItemKind::Media(m) => m.duration(),
            ItemKind::Sequence(s) => s.duration(),
            ItemKind::Subclip { range, .. } => range.duration,
            ItemKind::Uncarried { duration, .. } => *duration,
        }
    }
    pub fn frame_rate(&self) -> FrameRate {
        match &self.kind {
            ItemKind::Media(m) => m.frame_rate(),
            ItemKind::Sequence(s) => s.settings.frame_rate,
            ItemKind::Uncarried { rate, .. } => *rate,
            ItemKind::Subclip { .. } => FrameRate::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BinEntry {
    Item(ItemId),
    Bin(Bin),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bin {
    pub id: BinId,
    pub name: String,
    pub children: Vec<BinEntry>,
}

impl Bin {
    pub fn find_bin_mut(&mut self, id: BinId) -> Option<&mut Bin> {
        if self.id == id {
            return Some(self);
        }
        for c in &mut self.children {
            if let BinEntry::Bin(b) = c
                && let Some(f) = b.find_bin_mut(id)
            {
                return Some(f);
            }
        }
        None
    }
    pub fn find_bin(&self, id: BinId) -> Option<&Bin> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| if let BinEntry::Bin(b) = c { b.find_bin(id) } else { None })
    }
    pub fn remove_item(&mut self, id: ItemId) -> bool {
        let before = self.children.len();
        self.children.retain(|c| !matches!(c, BinEntry::Item(i) if *i == id));
        if self.children.len() != before {
            return true;
        }
        self.children.iter_mut().any(|c| if let BinEntry::Bin(b) = c { b.remove_item(id) } else { false })
    }
    /// Every item in this bin and the bins inside it, depth first.
    pub fn all_items(&self, out: &mut Vec<ItemId>) {
        for c in &self.children {
            match c {
                BinEntry::Item(i) => out.push(*i),
                BinEntry::Bin(b) => b.all_items(out),
            }
        }
    }
    /// The bin that directly holds item `id`.
    pub fn parent_of(&self, id: ItemId) -> Option<BinId> {
        for c in &self.children {
            match c {
                BinEntry::Item(i) if *i == id => return Some(self.id),
                BinEntry::Bin(b) => {
                    if let Some(p) = b.parent_of(id) {
                        return Some(p);
                    }
                }
                _ => {}
            }
        }
        None
    }
}

/// Bins, media and sequences. Ids are unique within one project.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    pub name: String,
    pub root: Bin,
    pub items: BTreeMap<ItemId, ProjectItem>,
    pub next_id: u64,
    pub ext: Ext,
}

impl Project {
    pub fn new(name: &str) -> Self {
        Self { name: name.into(), root: Bin { id: BinId(0), name: name.into(), children: Vec::new() }, items: BTreeMap::new(), next_id: 1, ext: None }
    }

    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// Add an item to `bin` (the root when `None` or not found).
    pub fn add_item(&mut self, name: &str, label: Label, kind: ItemKind, bin: Option<BinId>) -> ItemId {
        let id = ItemId(self.alloc_id());
        self.items.insert(id, ProjectItem { id, name: name.into(), label, kind, metadata: BTreeMap::new(), ext: None });
        if let Some(b) = bin.and_then(|b| self.root.find_bin_mut(b)) {
            b.children.push(BinEntry::Item(id));
        } else {
            self.root.children.push(BinEntry::Item(id));
        }
        id
    }

    pub fn add_bin(&mut self, name: &str, parent: Option<BinId>) -> BinId {
        let id = BinId(self.alloc_id());
        let bin = Bin { id, name: name.into(), children: Vec::new() };
        match parent.and_then(|p| self.root.find_bin_mut(p)) {
            Some(p) => p.children.push(BinEntry::Bin(bin)),
            None => self.root.children.push(BinEntry::Bin(bin)),
        }
        id
    }

    /// Move an item into `bin` (the root when `None`). False when the item or the bin is not there.
    pub fn move_item_to_bin(&mut self, item: ItemId, bin: Option<BinId>) -> bool {
        let target = bin.unwrap_or(self.root.id);
        if !self.items.contains_key(&item) || self.root.find_bin(target).is_none() || !self.root.remove_item(item) {
            return false;
        }
        match self.root.find_bin_mut(target) {
            Some(t) => t.children.push(BinEntry::Item(item)),
            None => self.root.children.push(BinEntry::Item(item)),
        }
        true
    }

    pub fn item(&self, id: ItemId) -> Option<&ProjectItem> {
        self.items.get(&id)
    }
    pub fn item_mut(&mut self, id: ItemId) -> Option<&mut ProjectItem> {
        self.items.get_mut(&id)
    }
    pub fn sequence(&self, id: ItemId) -> Option<&Sequence> {
        self.items.get(&id).and_then(ProjectItem::as_sequence)
    }
    pub fn sequence_mut(&mut self, id: ItemId) -> Option<&mut Sequence> {
        self.items.get_mut(&id).and_then(ProjectItem::as_sequence_mut)
    }

    /// The media behind an item, through at most sixteen subclips, with the outermost subclip's
    /// range. Missing parents, cycles and longer chains are no media.
    pub fn resolve_media(&self, item: ItemId) -> Option<(ItemId, &MediaClip, Option<TimeRange>)> {
        let mut id = item;
        let mut range = None;
        for _ in 0..16 {
            match &self.item(id)?.kind {
                ItemKind::Media(media) => return Some((id, media, range)),
                ItemKind::Subclip { parent, range: span } => {
                    range.get_or_insert(*span);
                    id = *parent;
                }
                _ => return None,
            }
        }
        None
    }

    /// Size in pixels of what an item shows; a subclip has the size of its parent.
    pub fn source_size(&self, item: ItemId) -> Option<(u32, u32)> {
        let mut id = item;
        for _ in 0..16 {
            return match &self.item(id)?.kind {
                ItemKind::Media(m) => m.info.video.as_ref().map(|v| (v.width, v.height)),
                ItemKind::Sequence(q) => Some((q.settings.width, q.settings.height)),
                ItemKind::Uncarried { width, height, .. } => Some((*width, *height)),
                ItemKind::Subclip { parent, .. } => {
                    id = *parent;
                    continue;
                }
            };
        }
        None
    }
}

// ---------------------------------------------------------------------------------------------
// Sequences
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct SequenceSettings {
    pub width: u32,
    pub height: u32,
    pub frame_rate: FrameRate,
    /// Pixel aspect ratio.
    pub par: (u32, u32),
    pub sample_rate: u32,
    /// Drop-frame timecode; only at rates that have it.
    pub drop_frame: bool,
}

impl Default for SequenceSettings {
    fn default() -> Self {
        Self { width: 1920, height: 1080, frame_rate: FrameRate::FPS_23_976, par: (1, 1), sample_rate: 48_000, drop_frame: false }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub settings: SequenceSettings,
    /// Bottom track first.
    pub video_tracks: Vec<Track>,
    pub audio_tracks: Vec<Track>,
    pub markers: Vec<Marker>,
    /// Timecode of the first frame, in frames.
    pub start_timecode: i64,
}

impl Sequence {
    pub fn new(settings: SequenceSettings) -> Self {
        Self { settings, video_tracks: Vec::new(), audio_tracks: Vec::new(), markers: Vec::new(), start_timecode: 0 }
    }
    /// The end of the last clip.
    pub fn duration(&self) -> Tick {
        self.all_tracks().map(Track::end).max().unwrap_or(Tick::ZERO)
    }
    pub fn tracks(&self, kind: TrackKind) -> &Vec<Track> {
        match kind {
            TrackKind::Video => &self.video_tracks,
            TrackKind::Audio => &self.audio_tracks,
        }
    }
    pub fn tracks_mut(&mut self, kind: TrackKind) -> &mut Vec<Track> {
        match kind {
            TrackKind::Video => &mut self.video_tracks,
            TrackKind::Audio => &mut self.audio_tracks,
        }
    }
    pub fn all_tracks(&self) -> impl Iterator<Item = &Track> {
        self.video_tracks.iter().chain(self.audio_tracks.iter())
    }
    pub fn all_tracks_mut(&mut self) -> impl Iterator<Item = &mut Track> {
        self.video_tracks.iter_mut().chain(self.audio_tracks.iter_mut())
    }
    pub fn find_item_mut(&mut self, id: ClipId) -> Option<(TrackId, &mut TrackItem)> {
        self.all_tracks_mut().find_map(|t| {
            let tid = t.id;
            t.item_mut(id).map(|i| (tid, i))
        })
    }
    pub fn find_item(&self, id: ClipId) -> Option<(TrackId, &TrackItem)> {
        self.all_tracks().find_map(|t| t.item(id).map(|i| (t.id, i)))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TrackKind {
    #[default]
    Video,
    Audio,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioChannels {
    Mono,
    #[default]
    Stereo,
    Surround51,
    Adaptive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub name: String,
    /// Sorted by start; items do not overlap.
    pub items: Vec<TrackItem>,
    pub transitions: Vec<Transition>,
    pub locked: bool,
    pub enabled: bool,
    pub channels: AudioChannels,
    pub ext: Ext,
}

impl Track {
    pub fn new(id: TrackId, kind: TrackKind, name: String) -> Self {
        Self { id, kind, name, items: Vec::new(), transitions: Vec::new(), locked: false, enabled: true, channels: AudioChannels::Stereo, ext: None }
    }
    pub fn end(&self) -> Tick {
        self.items.iter().map(|i| i.end()).max().unwrap_or(Tick::ZERO)
    }
    pub fn item(&self, id: ClipId) -> Option<&TrackItem> {
        self.items.iter().find(|i| i.id == id)
    }
    pub fn item_mut(&mut self, id: ClipId) -> Option<&mut TrackItem> {
        self.items.iter_mut().find(|i| i.id == id)
    }
    pub fn sort(&mut self) {
        self.items.sort_by_key(|i| i.start);
        self.transitions.sort_by_key(|t| t.start);
    }
}

/// A clip instance on a track.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackItem {
    pub id: ClipId,
    /// The item it shows.
    pub item: ItemId,
    pub name: String,
    pub label: Label,
    pub start: Tick,
    pub duration: Tick,
    /// Media time of the first frame shown (of the last, when reversed, plus the clip's span).
    pub source_in: Tick,
    /// Playback speed, positive; `reverse` gives the direction.
    pub speed: f64,
    pub reverse: bool,
    pub enabled: bool,
    /// Clips with the same link id are one clip's picture and sound.
    pub link: Option<u64>,
    pub group: Option<u64>,
    pub effects: Vec<EffectInstance>,
    /// Clip markers, in media time.
    pub markers: Vec<Marker>,
    pub gain_db: f64,
    /// A freeze frame: the media time held for the whole clip.
    pub frame_hold: Option<Tick>,
    /// Scale the picture to the sequence frame.
    pub scale_to_frame: bool,
    /// Sound: the channels of the media this clip plays (0-based); empty for all of them. One
    /// channel is a mono clip split off a multichannel file.
    pub source_channels: Vec<u16>,
    pub ext: Ext,
}

impl TrackItem {
    pub fn end(&self) -> Tick {
        self.start + self.duration
    }
    pub fn range(&self) -> TimeRange {
        TimeRange::new(self.start, self.duration)
    }
    /// Media time one past the last frame shown.
    pub fn source_out(&self) -> Tick {
        self.source_in + Tick((self.duration.0 as f64 * self.speed.abs()).round() as i64)
    }
    pub fn effect(&self, id: &str) -> Option<&EffectInstance> {
        self.effects.iter().find(|e| e.effect == id)
    }
    pub fn effect_mut(&mut self, id: &str) -> Option<&mut EffectInstance> {
        self.effects.iter_mut().find(|e| e.effect == id)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransitionAlign {
    #[default]
    CenterAtCut,
    StartAtCut,
    EndAtCut,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub id: TransitionId,
    pub effect: EffectInstance,
    pub start: Tick,
    pub duration: Tick,
    /// The outgoing clip (`None`: a fade in from nothing).
    pub from: Option<ClipId>,
    /// The incoming clip (`None`: a fade out).
    pub to: Option<ClipId>,
    pub align: TransitionAlign,
    pub reverse: bool,
    pub ext: Ext,
}

impl Transition {
    pub fn end(&self) -> Tick {
        self.start + self.duration
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkerKind {
    #[default]
    Comment,
    Chapter,
    Segmentation,
    WebLink,
    FlashCue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub id: MarkerId,
    pub start: Tick,
    pub duration: Tick,
    pub name: String,
    pub comment: String,
    pub kind: MarkerKind,
    pub color: Label,
    pub url: String,
    pub author: String,
    pub ext: Ext,
}

// ---------------------------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum ParamValue {
    Float(f64),
    /// A point; NaN components mean "automatic" (the centre of the frame or source).
    Vec2(Vec2),
    /// RGBA, 0..=1.
    Color([f32; 4]),
    Bool(bool),
    Choice(u32),
    Text(String),
    /// A value only the application reads (a curve, a mask path), as it serialises it.
    Opaque(String),
}

impl ParamValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ParamValue::Float(v) => Some(*v),
            ParamValue::Choice(c) => Some(*c as f64),
            ParamValue::Bool(b) => Some(*b as u8 as f64),
            _ => None,
        }
    }
    pub fn as_vec2(&self) -> Option<Vec2> {
        match self {
            ParamValue::Vec2(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_color(&self) -> Option<[f32; 4]> {
        match self {
            ParamValue::Color(c) => Some(*c),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ParamValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interpolation {
    #[default]
    Linear,
    Hold,
    Bezier,
    AutoBezier,
    ContinuousBezier,
    EaseIn,
    EaseOut,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    /// Media time.
    pub time: Tick,
    pub value: ParamValue,
    /// How the value moves from this keyframe to the next.
    pub interp: Interpolation,
    pub out_influence: f64,
    pub in_influence: f64,
}

impl Keyframe {
    pub fn new(time: Tick, value: ParamValue) -> Self {
        Self { time, value, interp: Interpolation::Linear, out_influence: 1.0 / 3.0, in_influence: 1.0 / 3.0 }
    }
}

/// A parameter: its value, or keyframes sorted by time.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub value: ParamValue,
    pub keyframes: Vec<Keyframe>,
}

impl Param {
    pub fn new(value: ParamValue) -> Self {
        Self { value, keyframes: Vec::new() }
    }
    pub fn is_animated(&self) -> bool {
        !self.keyframes.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectInstance {
    /// The effect's id (see [`fx`] and [`Catalog`]).
    pub effect: String,
    pub enabled: bool,
    pub params: BTreeMap<String, Param>,
    pub ext: Ext,
}

impl EffectInstance {
    pub fn param(&self, id: &str) -> Option<&Param> {
        self.params.get(id)
    }
    pub fn param_mut(&mut self, id: &str) -> Option<&mut Param> {
        self.params.get_mut(id)
    }
    pub fn is_animated(&self) -> bool {
        self.params.values().any(Param::is_animated)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Video,
    Audio,
    VideoTransition,
    AudioTransition,
}

/// What the formats need to know about an effect.
#[derive(Clone, Debug, PartialEq)]
pub struct EffectDef {
    pub id: String,
    /// The name a document writes and other applications read back. It must not change when the
    /// application renames the effect in its interface.
    pub name: String,
    pub kind: EffectKind,
    /// The folder a document files it under (`Dissolve`, `Blur & Sharpen`, …); may be empty.
    pub category: String,
    /// Fixed effects every clip has (motion, opacity, volume): never written as a filter of their own.
    pub intrinsic: bool,
    /// Parameters, in the effect's order.
    pub params: Vec<ParamDef>,
}

/// A parameter of an effect: its id, the name a document shows for it, and its default, whose
/// variant is the parameter's type.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamDef {
    pub id: String,
    pub label: String,
    pub default: ParamValue,
}

impl EffectDef {
    pub fn param(&self, id: &str) -> Option<&ParamDef> {
        self.params.iter().find(|p| p.id == id)
    }
    /// An instance with every parameter at its default.
    pub fn instance(&self) -> EffectInstance {
        EffectInstance {
            effect: self.id.clone(),
            enabled: true,
            params: self.params.iter().map(|p| (p.id.clone(), Param::new(p.default.clone()))).collect(),
            ext: None,
        }
    }
}

/// The application's effects and generators, for what a format does not know itself.
pub trait Catalog {
    /// The effect with this id.
    fn effect(&self, id: &str) -> Option<EffectDef>;
    /// The effect a document names (its [`EffectDef::name`], or a name the application also answers to).
    fn effect_by_name(&self, name: &str) -> Option<EffectDef>;
    /// The fixed effects a new clip on a track of `kind` starts with.
    fn intrinsic(&self, kind: TrackKind) -> Vec<EffectInstance>;
    /// A generator a document names, when it is one of the application's own.
    fn generator_by_name(&self, _name: &str) -> Option<Generator> {
        None
    }
}

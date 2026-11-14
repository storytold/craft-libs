//! A format-neutral mob-style composition model shared by AAF and OMF.
//!
//! Both formats describe a timeline as a composition of slots (tracks) whose segments are
//! sequences of fillers, source clips and transitions, where a transition *overlaps* the end of
//! the segment before it and the start of the segment after it (the sequence's length is the sum of
//! its segment lengths minus its transition lengths). Source clips reference media through master
//! mobs and file source mobs: here, a [`Source`] per essence, grouped into one media item by its
//! key. All times are ticks; the format serialisers convert to and from edit units.

use craft_timeline::{FrameRate, TICKS_PER_SECOND, Tick};

/// Picture or sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CKind {
    /// Moving or still images.
    Picture,
    /// Audio.
    Sound,
}

/// A whole document: compositions and the media they reference.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    /// The top-level compositions.
    pub compositions: Vec<Composition>,
    /// The essences the compositions reference.
    pub sources: Vec<Source>,
    /// Compositions that are only used inside others (nested sequences); a [`Source`] refers to
    /// one by its index here. They may nest further.
    pub nested: Vec<Composition>,
}

/// One composition: its tracks, markers and timing.
#[derive(Clone, Debug, PartialEq)]
pub struct Composition {
    /// The name.
    pub name: String,
    /// AAF CompositionMob identifier as 64 lowercase hexadecimal digits. OMF has no equivalent.
    /// It is a read identity for applications that let a user choose among compositions; writers
    /// generate fresh format identifiers and do not preserve it.
    pub aaf_mob_id: Option<String>,
    /// The edit rate.
    pub rate: FrameRate,
    /// Sound: samples per second.
    pub sample_rate: u32,
    /// Picture: width in pixels.
    pub width: u32,
    /// Picture: height in pixels.
    pub height: u32,
    /// Start timecode in frames at `rate`.
    pub start_tc: i64,
    /// Whether the timecode is drop-frame.
    pub drop: bool,
    /// The tracks, picture and sound.
    pub tracks: Vec<CTrack>,
    /// The markers, at times in the composition.
    pub markers: Vec<CMarker>,
}

/// One track of a composition.
#[derive(Clone, Debug, PartialEq)]
pub struct CTrack {
    /// Whether the track carries picture or sound.
    pub kind: CKind,
    /// The name.
    pub name: String,
    /// Physical track number (1-based, per kind).
    pub number: u32,
    /// Sound: channels of the track (1 mono, 2 stereo, 6 5.1).
    pub channels: u32,
    /// The segments in order.
    pub items: Vec<CItem>,
}

/// One segment of a track.
#[derive(Clone, Debug, PartialEq)]
pub enum CItem {
    /// Nothing for the given length.
    Filler(Tick),
    /// A stretch of a source.
    Clip(CClip),
    /// A transition overlapping its neighbours.
    Transition(CTransition),
}

impl CItem {
    /// The item's length on the track.
    pub fn len(&self) -> Tick {
        match self {
            CItem::Filler(l) => *l,
            CItem::Clip(c) => c.len,
            CItem::Transition(t) => t.len,
        }
    }
}

/// A clip: a stretch of one source on a track.
#[derive(Clone, Debug, PartialEq)]
pub struct CClip {
    /// The length on the track.
    pub len: Tick,
    /// Index into [`Document::sources`].
    pub source: usize,
    /// Source (file) time of the segment's first frame.
    pub start: Tick,
    /// Sound: the clip gain, if any.
    pub gain: Option<Gain>,
    /// The name.
    pub name: String,
}

/// Clip gain (linear amplitude).
#[derive(Clone, Debug, PartialEq)]
pub enum Gain {
    /// One gain for the whole segment.
    Constant(f64),
    /// Points at offsets from the segment start; `linear` false = hold (step) interpolation.
    Varying {
        /// Whether the points join by straight lines rather than steps.
        linear: bool,
        /// The gain at each offset.
        points: Vec<(Tick, f64)>,
    },
}

/// A transition between the segments before and after it.
#[derive(Clone, Debug, PartialEq)]
pub struct CTransition {
    /// The length on the track.
    pub len: Tick,
    /// Where the cut lies, from the transition start.
    pub cut: Tick,
    /// The transition, by a name of the writing application's choosing; it is kept in a private tag
    /// and comes back unchanged. Names the formats themselves can say: `cross_dissolve` and
    /// `dip_to_black` (picture), any name containing `wipe` (a SMPTE wipe), `constant_power` (sound).
    /// A file without the tag reads as one of these.
    pub effect: String,
}

/// A marker or note at a time in a composition or source.
#[derive(Clone, Debug, PartialEq)]
pub struct CMarker {
    /// Where it starts.
    pub start: Tick,
    /// How long it lasts; zero for a point.
    pub duration: Tick,
    /// The name.
    pub name: String,
    /// The free-text comment.
    pub comment: String,
    /// The colour name the writing application gave it.
    pub color: Option<String>,
}

/// One essence: a file (or embedded data) carrying the picture, or some channels of sound, of a
/// media item.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    /// Groups the sources of one media item (one master mob).
    pub key: String,
    /// The name.
    pub name: String,
    /// Whether the track carries picture or sound.
    pub kind: CKind,
    /// The file the essence lives in; `None` for embedded data.
    pub path: Option<String>,
    /// Sound: the (0-based) channel of the file this source exposes (broken out to mono), or
    /// `None` for all of them.
    pub channel: Option<u32>,
    /// Sound: channels this source exposes (1 when `channel` is set, else `file_channels`).
    pub channels: u32,
    /// Sound: channels of the file / embedded data.
    pub file_channels: u32,
    /// Picture: width in pixels.
    pub width: u32,
    /// Picture: height in pixels.
    pub height: u32,
    /// Picture: the frame rate of the media.
    pub frame_rate: FrameRate,
    /// Sound: samples per second.
    pub sample_rate: u32,
    /// Sound: bits per sample.
    pub bits: u16,
    /// The length of the essence.
    pub length: Tick,
    /// Media time of the essence's time 0 (trimmed / consolidated essence).
    pub offset: Tick,
    /// Start timecode in frames at `tc_rate`.
    pub start_tc: Option<i64>,
    /// The rate `start_tc` counts in.
    pub tc_rate: FrameRate,
    /// Essence data stored in the file itself.
    pub embedded: Option<Vec<u8>>,
    /// The markers, at times in the composition.
    pub markers: Vec<CMarker>,
    /// Not media but a nested composition: its index in [`Document::nested`]. `start` of a clip
    /// that uses it is a time in that composition.
    pub nested: Option<usize>,
}

impl Source {
    /// A picture source with no length: a still image.
    pub fn is_still(&self) -> bool {
        self.kind == CKind::Picture && self.length == Tick::ZERO
    }
}

/// The largest time read from a document (about 3.3 days).
pub const MAX_TIME: Tick = Tick(1 << 56);

/// Ticks of `n` edit units at rate `num/den` (rounded to the nearest tick).
pub fn units_to_ticks(n: i64, num: i64, den: i64) -> Tick {
    if num <= 0 || den <= 0 {
        return Tick::ZERO;
    }
    let t = n as i128 * den as i128 * TICKS_PER_SECOND as i128;
    let q = t.div_euclid(num as i128);
    let r = t.rem_euclid(num as i128);
    // documents are untrusted: keep every time far from overflow (±3 days)
    Tick((q + if r * 2 >= num as i128 { 1 } else { 0 }).clamp(-MAX_TIME.0 as i128, MAX_TIME.0 as i128) as i64)
}

/// Edit units at rate `num/den` nearest to `t`; the flag is false when `t` is not on a unit.
pub fn ticks_to_units(t: Tick, num: i64, den: i64) -> (i64, bool) {
    if num <= 0 || den <= 0 {
        return (0, false);
    }
    let x = t.0 as i128 * num as i128;
    let d = den as i128 * TICKS_PER_SECOND as i128;
    let q = x.div_euclid(d);
    let r = x.rem_euclid(d);
    let n = q + if r * 2 >= d { 1 } else { 0 };
    (n.clamp(i64::MIN as i128, i64::MAX as i128) as i64, r == 0)
}

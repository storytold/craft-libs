//! Documents the tests write: what an editor's two-clip sequence with a dissolve looks like in
//! the model.

use craft_timeline::time::{FrameRate, Tick};

use crate::Identity;
use crate::model::{CClip, CItem, CKind, CTrack, CTransition, Composition, Document, Gain, Source};

pub(crate) const IDENTITY: Identity<'static> =
    Identity { company: "Craft", product: "craft-aaf tests", version: "1.2.3", platform: "test", product_id: *b"craft-aaf-tests." };

/// A source of `kind` for the media item `key` at `path`: 1280×720 picture at `rate`, or stereo
/// 48 kHz sound of `bits` bits; 100 seconds long.
pub(crate) fn source(key: &str, path: &str, kind: CKind, rate: FrameRate, bits: u16, start_tc: Option<i64>) -> Source {
    let sound = kind == CKind::Sound;
    Source {
        key: key.into(),
        name: path.rsplit('/').next().unwrap_or(path).into(),
        kind,
        path: Some(path.into()),
        channel: None,
        channels: if sound { 2 } else { 0 },
        file_channels: if sound { 2 } else { 0 },
        width: if sound { 0 } else { 1280 },
        height: if sound { 0 } else { 720 },
        frame_rate: rate,
        sample_rate: 48_000,
        bits,
        length: rate.tick_of(100 * rate.timecode_base()),
        offset: Tick::ZERO,
        start_tc,
        tc_rate: rate,
        embedded: None,
        markers: Vec::new(),
        nested: None,
    }
}

pub(crate) fn clip(rate: FrameRate, source: usize, start: i64, len: i64, name: &str) -> CItem {
    CItem::Clip(CClip { len: rate.tick_of(len), source, start: rate.tick_of(start), gain: None, name: name.into() })
}

/// Two clips of `source` cut at frame `cut`, each `cut` frames long on the timeline, joined by a
/// transition of `dur` frames centred on the cut: each clip runs half the transition past the cut.
pub(crate) fn two_clips(rate: FrameRate, source: usize, cut: i64, dur: i64, (in_a, in_b): (i64, i64), effect: &str) -> Vec<CItem> {
    let half = dur / 2;
    vec![
        clip(rate, source, in_a, cut + half, "a"),
        CItem::Transition(CTransition { len: rate.tick_of(dur), cut: rate.tick_of(half), effect: effect.into() }),
        clip(rate, source, in_b - half, cut + half, "a"),
    ]
}

pub(crate) fn composition(name: &str, rate: FrameRate, tracks: Vec<CTrack>) -> Composition {
    Composition {
        name: name.into(),
        aaf_mob_id: None,
        rate,
        sample_rate: 48_000,
        width: 1920,
        height: 1080,
        start_tc: 0,
        drop: false,
        tracks,
        markers: Vec::new(),
    }
}

pub(crate) fn track(kind: CKind, name: &str, number: u32, items: Vec<CItem>) -> CTrack {
    CTrack { kind, name: name.into(), number, channels: if kind == CKind::Sound { 2 } else { 0 }, items }
}

/// "S" at `rate`: V1 and A1 each hold two clips of `a.mov`, `cut` frames each on the timeline (from
/// source frames `ins`), with a transition of `dur` frames centred on the cut; the second sound
/// clip is 3 dB down.
pub(crate) fn picture_and_sound_cut(rate: FrameRate, cut: i64, dur: i64, ins: (i64, i64)) -> Document {
    let sources = vec![
        source("item:1", "/m/a.mov", CKind::Picture, rate, 24, Some(86_400)),
        source("item:1", "/m/a.mov", CKind::Sound, rate, 24, Some(86_400)),
    ];
    let mut sound = two_clips(rate, 1, cut, dur, ins, "constant_power");
    if let Some(CItem::Clip(c)) = sound.last_mut() {
        c.gain = Some(Gain::Constant(10f64.powf(-3.0 / 20.0)));
    }
    let tracks = vec![track(CKind::Picture, "V1", 1, two_clips(rate, 0, cut, dur, ins, "cross_dissolve")), track(CKind::Sound, "A1", 1, sound)];
    Document { compositions: vec![composition("S", rate, tracks)], sources, nested: Vec::new() }
}

/// [`picture_and_sound_cut`] with 50-frame clips from source frames 10 and 205 and a 10-frame
/// transition: every time is a multiple of five frames, which is a whole number of 48 kHz samples
/// at the NTSC rates too.
pub(crate) fn picture_and_sound(rate: FrameRate) -> Document {
    picture_and_sound_cut(rate, 50, 10, (10, 205))
}

/// 16-bit PCM: `frames` frames of `channels` channels, every sample different from its neighbours.
pub(crate) fn pcm_ramp(frames: usize, channels: usize, seed: i32) -> Vec<u8> {
    (0..frames * channels).flat_map(|i| ((i as i32 * 37 + seed * 1000) as i16).to_le_bytes()).collect()
}

/// "S" at 25 fps, sound only: A1 holds two 50-frame clips of `a.wav` (from source frames 10 and
/// 300) with a 10-frame crossfade centred on the cut; the first clip is 6 dB up. The sound is
/// embedded when `embedded` is given (480 frames of 16-bit stereo).
pub(crate) fn sound_only(embedded: Option<Vec<u8>>) -> Document {
    let rate = FrameRate::FPS_25;
    let mut s = source("item:1", "/m/a.wav", CKind::Sound, rate, 16, None);
    if let Some(pcm) = embedded {
        s.path = None;
        s.length = Tick::from_units((pcm.len() / 4) as i64, 48_000);
        s.embedded = Some(pcm);
    }
    let mut items = two_clips(rate, 0, 50, 10, (10, 300), "constant_power");
    if let Some(CItem::Clip(c)) = items.first_mut() {
        c.gain = Some(Gain::Constant(10f64.powf(6.0 / 20.0)));
    }
    Document { compositions: vec![composition("S", rate, vec![track(CKind::Sound, "A1", 1, items)])], sources: vec![s], nested: Vec::new() }
}

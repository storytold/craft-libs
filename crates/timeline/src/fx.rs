//! The effects the formats name themselves, and a [`Catalog`] of just those.
//!
//! Final Cut Pro 7 XML and FCPXML have their own elements for a clip's motion, opacity, speed,
//! audio level and pan, and for the common dissolves. The readers and writers map those onto the
//! effect and parameter ids below, so an application's [`Catalog`] has to answer for at least
//! these ids (with whatever defaults it uses) and for anything else it wants carried.

use crate::model::{Catalog, EffectDef, EffectInstance, EffectKind, ParamDef, ParamValue, TrackKind, Vec2};

/// Motion: `position` and `anchor` (points in pixels; NaN = automatic, the centre), `scale` and
/// `scale_width` (percent), `uniform_scale`, `rotation` (degrees), `anti_flicker` (0..=1).
pub const MOTION: &str = "motion";
/// Opacity: `opacity` (percent), `blend` (blend mode index).
pub const OPACITY: &str = "opacity";
/// Time remapping: `speed` (percent).
pub const TIME_REMAP: &str = "time_remap";
/// Audio level: `level` (dB).
pub const VOLUME: &str = "volume";
/// Per-channel level: `left`, `right` (dB).
pub const CHANNEL_VOLUME: &str = "channel_volume";
/// Pan: `balance` (−100..=100).
pub const PANNER: &str = "panner";

fn def(id: &str, name: &str, kind: EffectKind, category: &str, intrinsic: bool, params: Vec<(&str, &str, ParamValue)>) -> EffectDef {
    EffectDef {
        id: id.into(),
        name: name.into(),
        kind,
        category: category.into(),
        intrinsic,
        params: params.into_iter().map(|(id, label, default)| ParamDef { id: id.into(), label: label.into(), default }).collect(),
    }
}

/// The definitions of [`Standard`], in order.
pub fn standard_defs() -> Vec<EffectDef> {
    use EffectKind::{Audio, AudioTransition, Video, VideoTransition};
    use ParamValue::{Bool, Choice, Float};
    let auto = || ParamValue::Vec2(Vec2::new(f64::NAN, f64::NAN));
    vec![
        def(
            MOTION,
            "Motion",
            Video,
            "",
            true,
            vec![
                ("position", "Position", auto()),
                ("scale", "Scale", Float(100.0)),
                ("scale_width", "Scale Width", Float(100.0)),
                ("uniform_scale", "Uniform Scale", Bool(true)),
                ("rotation", "Rotation", Float(0.0)),
                ("anchor", "Anchor Point", auto()),
                ("anti_flicker", "Anti-flicker Filter", Float(0.0)),
            ],
        ),
        def(OPACITY, "Opacity", Video, "", true, vec![("opacity", "Opacity", Float(100.0)), ("blend", "Blend Mode", Choice(0))]),
        def(TIME_REMAP, "Time Remapping", Video, "", true, vec![("speed", "Speed", Float(100.0))]),
        def(VOLUME, "Volume", Audio, "", true, vec![("bypass", "Bypass", Bool(false)), ("level", "Level", Float(0.0))]),
        def(
            CHANNEL_VOLUME,
            "Channel Volume",
            Audio,
            "",
            true,
            vec![("bypass", "Bypass", Bool(false)), ("left", "Left", Float(0.0)), ("right", "Right", Float(0.0))],
        ),
        def(PANNER, "Panner", Audio, "", true, vec![("balance", "Balance", Float(0.0))]),
        def("cross_dissolve", "Cross Dissolve", VideoTransition, "Dissolve", false, vec![]),
        def("dip_to_black", "Dip to Black", VideoTransition, "Dissolve", false, vec![]),
        def("film_dissolve", "Film Dissolve", VideoTransition, "Dissolve", false, vec![]),
        def("additive_dissolve", "Additive Dissolve", VideoTransition, "Dissolve", false, vec![]),
        def("non_additive_dissolve", "Non-Additive Dissolve", VideoTransition, "Video Transitions", false, vec![]),
        def("wipe", "Wipe", VideoTransition, "Video Transitions", false, vec![("direction", "Direction", Choice(3))]),
        def("constant_power", "Constant Power", AudioTransition, "Crossfade", false, vec![]),
        def("constant_gain", "Constant Gain", AudioTransition, "Crossfade", false, vec![]),
        def("exponential_fade", "Exponential Fade", AudioTransition, "Crossfade", false, vec![]),
    ]
}

/// A catalog of the effects in this module and nothing else: enough for an application without
/// an effect registry of its own, and for tests. Names match in any case.
#[derive(Clone, Copy, Debug, Default)]
pub struct Standard;

impl Catalog for Standard {
    fn effect(&self, id: &str) -> Option<EffectDef> {
        standard_defs().into_iter().find(|d| d.id == id)
    }
    fn effect_by_name(&self, name: &str) -> Option<EffectDef> {
        standard_defs().into_iter().find(|d| d.name.eq_ignore_ascii_case(name))
    }
    fn intrinsic(&self, kind: TrackKind) -> Vec<EffectInstance> {
        let ids: &[&str] = match kind {
            TrackKind::Video => &[MOTION, OPACITY, TIME_REMAP],
            TrackKind::Audio => &[VOLUME, CHANNEL_VOLUME, PANNER],
        };
        ids.iter().filter_map(|id| self.effect(id)).map(|d| d.instance()).collect()
    }
}

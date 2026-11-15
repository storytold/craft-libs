//! Borrowed views of native XFL frame-sound and envelope records.
//!
//! These parse the frame fields, effect-name spelling, and envelope wire values. Resolving sound
//! library items and mapping effects/envelopes into a consumer's audio model remain application
//! responsibilities.

use crate::shape::{XflElement, integer, number};

/// Native frame-sound synchronization mode. Unknown values use Animate's event behavior.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncMode {
    Start,
    Stop,
    Stream,
    #[default]
    Event,
}

/// A native sound instruction stored on a `DOMFrame`.
#[derive(Clone, Copy, Debug)]
pub struct FrameSound<'a, E: XflElement> {
    frame: &'a E,
}

impl<'a, E: XflElement> FrameSound<'a, E> {
    pub fn new(frame: &'a E) -> Self {
        Self { frame }
    }

    pub fn name(&self) -> Option<&'a str> {
        self.frame.attr("soundName")
    }

    pub fn sync_mode(&self) -> SyncMode {
        match self.frame.attr("soundSync").map(str::trim) {
            Some("start") => SyncMode::Start,
            Some("stop") => SyncMode::Stop,
            Some("stream") => SyncMode::Stream,
            _ => SyncMode::Event,
        }
    }

    pub fn looping(&self) -> bool {
        self.frame.attr("soundLoopMode").map(str::trim) == Some("loop")
    }

    pub fn repeat(&self) -> Option<u32> {
        integer(self.frame, "soundLoop")
    }

    pub fn has_in_point(&self) -> bool {
        self.frame.attr("inPoint44").is_some()
    }

    pub fn has_out_point(&self) -> bool {
        self.frame.attr("outPoint44").is_some()
    }

    /// The effect menu label normalized to lowercase ASCII letters and digits, matching the
    /// legacy reader's recognition of labels such as `Fade Left to Right`.
    pub fn effect_key(&self) -> String {
        self.frame
            .attr("soundEffect")
            .unwrap_or("")
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|character| character.to_ascii_lowercase())
            .collect()
    }

    pub fn envelope_points(&self) -> impl Iterator<Item = EnvelopePoint<'a, E>> + 'a {
        self.frame
            .child("SoundEnvelope")
            .into_iter()
            .flat_map(|envelope| envelope.elements_named("SoundEnvelopePoint"))
            .map(|element| EnvelopePoint { element })
    }
}

/// One native `SoundEnvelopePoint` record. Levels are raw SWF/XFL integer units.
#[derive(Clone, Copy, Debug)]
pub struct EnvelopePoint<'a, E: XflElement> {
    element: &'a E,
}

impl<E: XflElement> EnvelopePoint<'_, E> {
    pub fn mark44(&self) -> Option<f32> {
        number(self.element, "mark44")
    }

    pub fn level0(&self) -> Option<f32> {
        number(self.element, "level0")
    }

    pub fn level1(&self) -> Option<f32> {
        number(self.element, "level1")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &[u8]) -> craft_xml::El {
        craft_xml::parse(source).expect("well-formed test XML")
    }

    #[test]
    fn reads_frame_sound_modes_repeat_trim_and_effect_name() {
        let frame = parse(br#"<x:DOMFrame xmlns:x="urn:xfl" soundName="Music" soundSync=" stream " soundLoopMode="loop" soundLoop="12" inPoint44="2" outPoint44="100" soundEffect="Fade Left to Right!"/>"#);
        let sound = FrameSound::new(&frame);
        assert_eq!(sound.name(), Some("Music"));
        assert_eq!(sound.sync_mode(), SyncMode::Stream);
        assert!(sound.looping());
        assert_eq!(sound.repeat(), Some(12));
        assert!(sound.has_in_point());
        assert!(sound.has_out_point());
        assert_eq!(sound.effect_key(), "fadelefttoright");
    }

    #[test]
    fn defaults_and_envelope_records_keep_native_optional_values() {
        let frame = parse(br#"<DOMFrame soundSync="unknown" soundLoopMode="repeat" soundEffect="CuStOm"><SoundEnvelope><SoundEnvelopePoint mark44="44100" level0="32768"/><Other/><SoundEnvelopePoint mark44="bad" level0="0" level1="nan"/></SoundEnvelope></DOMFrame>"#);
        let sound = FrameSound::new(&frame);
        assert_eq!(sound.sync_mode(), SyncMode::Event);
        assert!(!sound.looping());
        assert_eq!(sound.repeat(), None);
        assert!(!sound.has_in_point());
        assert!(!sound.has_out_point());
        assert_eq!(sound.effect_key(), "custom");
        let points: Vec<_> = sound.envelope_points().collect();
        assert_eq!(points.len(), 2);
        assert_eq!((points[0].mark44(), points[0].level0(), points[0].level1()), (Some(44100.0), Some(32768.0), None));
        assert_eq!((points[1].mark44(), points[1].level0(), points[1].level1()), (None, Some(0.0), None));
    }
}

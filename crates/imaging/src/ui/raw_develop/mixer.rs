//! The Color Mixer and Color Grading sections of the control column.

use craft_ui::choice::Segmented;
use craft_ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft_ui::runtime::env::Label;
use craft_ui::sections::{HeaderStrength, Section};
use craft_ui::style::DataColor;
use egui::{Color32, Ui};

use super::controls::{GRADING_TAIL, Rows, section};
use super::words::RawDevelopWords;
use crate::raw::{GradeWheel, RawDevelopSettings};

/// Reads a band's value of a tab from the settings.
type BandGet = fn(&RawDevelopSettings, usize) -> f32;
/// Writes a band's value of a tab into the settings.
type BandSet = fn(&mut RawDevelopSettings, usize, f32);
/// A band's word.
type BandWord = fn(&RawDevelopWords) -> &Label;

/// The Color Mixer's tabs: the band property a row edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum MixerTab {
    Hue,
    Saturation,
    Luminance,
}

impl MixerTab {
    /// Reads and writes the tab's value of band `k`.
    fn accessors(self) -> (BandGet, BandSet) {
        match self {
            MixerTab::Hue => (|s, k| s.hsl_hue[k], |s, k, v| s.hsl_hue[k] = v),
            MixerTab::Saturation => (|s, k| s.hsl_sat[k], |s, k, v| s.hsl_sat[k] = v),
            MixerTab::Luminance => (|s, k| s.hsl_lum[k], |s, k, v| s.hsl_lum[k] = v),
        }
    }
}

/// The eight bands: their word and the hue (degrees) their track is drawn around.
const BANDS: [(BandWord, f32); 8] = [
    (|w| &w.band_reds, 0.0),
    (|w| &w.band_oranges, 30.0),
    (|w| &w.band_yellows, 60.0),
    (|w| &w.band_greens, 120.0),
    (|w| &w.band_aquas, 180.0),
    (|w| &w.band_blues, 240.0),
    (|w| &w.band_purples, 270.0),
    (|w| &w.band_magentas, 300.0),
];

/// The track of a band's row on `tab`: the hue shifting around the band, the saturation rising, the luminance rising.
fn band_stops(tab: MixerTab, hue: f32) -> Vec<DataColor> {
    let color = |h: f32, s: f32, v: f32| {
        let unit = (h / 360.0).rem_euclid(1.0);
        DataColor::new(Color32::from(egui::ecolor::Hsva::new(unit, s, v, 1.0)))
    };
    match tab {
        MixerTab::Hue => vec![color(hue - 30.0, 0.9, 1.0), color(hue, 0.9, 1.0), color(hue + 30.0, 0.9, 1.0)],
        MixerTab::Saturation => vec![color(hue, 0.15, 0.9), color(hue, 1.0, 1.0)],
        MixerTab::Luminance => vec![color(hue, 0.9, 0.25), color(hue, 0.9, 0.7), color(hue, 0.3, 1.0)],
    }
}

/// One grading wheel: its word and its settings field.
#[derive(Clone, Copy)]
struct Wheel {
    word: fn(&RawDevelopWords) -> &Label,
    get: fn(&RawDevelopSettings) -> &GradeWheel,
    get_mut: fn(&mut RawDevelopSettings) -> &mut GradeWheel,
}

const WHEELS: [Wheel; 4] = [
    Wheel { word: |w| &w.wheel_shadows, get: |s| &s.grade_shadows, get_mut: |s| &mut s.grade_shadows },
    Wheel { word: |w| &w.wheel_midtones, get: |s| &s.grade_midtones, get_mut: |s| &mut s.grade_midtones },
    Wheel { word: |w| &w.wheel_highlights, get: |s| &s.grade_highlights, get_mut: |s| &mut s.grade_highlights },
    Wheel { word: |w| &w.wheel_global, get: |s| &s.grade_global, get_mut: |s| &mut s.grade_global },
];

/// The Color Mixer section: the Hue, Saturation and Luminance tabs over the eight bands. The tabs keep the chosen tab,
/// Hue until one is picked.
pub(super) fn color_mixer(ui: &mut Ui, rows: &mut Rows<'_>, words: &RawDevelopWords) {
    section(&words.section_color_mixer, "color_mixer", false).show(ui, |ui, _| {
        let tabs = [
            Entry::new(MixerTab::Hue, words.mixer_hue.clone()),
            Entry::new(MixerTab::Saturation, words.mixer_saturation.clone()),
            Entry::new(MixerTab::Luminance, words.mixer_luminance.clone()),
        ];
        let shown = Segmented::new(words.section_color_mixer.clone(), Bind::kept().reset(MixerTab::Hue), &tabs).show(ui);
        let kept = shown.shown.as_ref().and_then(|held| held.same()).map(|tab| **tab);
        let tab = shown.value().copied().or(kept).unwrap_or(MixerTab::Hue);
        let (get, set) = tab.accessors();
        let (base, defaults) = (rows.base, rows.defaults);
        for (k, (word, hue)) in BANDS.iter().enumerate() {
            let stops = Some(band_stops(tab, *hue));
            rows.slider(ui, word(words), get(base, k), get(defaults, k), -100.0..=100.0, 1.0, stops, move |settings, v| set(settings, k, v));
        }
    });
}

/// The Color Grading section: Hue, Saturation and Luminance rows for each wheel, then Blending and Balance.
pub(super) fn color_grading(ui: &mut Ui, rows: &mut Rows<'_>, words: &RawDevelopWords) {
    section(&words.section_color_grading, "color_grading", false).show(ui, |ui, _| {
        for wheel in WHEELS {
            Section::fixed((wheel.word)(words).clone()).strength(HeaderStrength::Caption).show(ui, |ui, _| {
                let (now, was) = ((wheel.get)(rows.base), (wheel.get)(rows.defaults));
                let (now, was) = (*now, *was);
                rows.slider(ui, &words.mixer_hue, now.hue, was.hue, 0.0..=360.0, 1.0, None, move |s, v| (wheel.get_mut)(s).hue = v);
                rows.slider(ui, &words.mixer_saturation, now.sat, was.sat, 0.0..=100.0, 1.0, None, move |s, v| (wheel.get_mut)(s).sat = v);
                rows.slider(ui, &words.mixer_luminance, now.lum, was.lum, -100.0..=100.0, 1.0, None, move |s, v| (wheel.get_mut)(s).lum = v);
            });
        }
        rows.table(ui, words, GRADING_TAIL);
    });
}

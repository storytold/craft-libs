//! Stories of the Curves panel: points mode with the other channels as overlays, and pencil mode.

use super::{CurvesMode, CurvesPanel, ToneCurves};
use crate::ui::adjust::{ToneChannels, TonePicker, ToneSpace};
use crate::ui::curve::Curve;
use crate::ui::curve::stories::story_bins;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The host's values of the panel story.
#[derive(Clone, Debug, PartialEq)]
struct Rig {
    curves: ToneCurves,
    armed: Option<TonePicker>,
}

fn counts(shift: f32) -> [u32; 256] {
    let mut counts = [0u32; 256];
    for (slot, bin) in counts.iter_mut().zip(story_bins(shift)) {
        *slot = bin;
    }
    counts
}

#[story(CurvesPanel<'static>, covers = [Default], drive = Drive::Never)]
fn curves_panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let channels = ToneChannels::of(ToneSpace::Rgb, false);
    let mut curves = ToneCurves::identity(&channels);
    curves.curves[0] = Curve::smooth(vec![[0.0, 0.0], [64.0, 80.0], [192.0, 170.0], [255.0, 255.0]]);
    curves.curves[1] = Curve::smooth(vec![[0.0, 10.0], [255.0, 245.0]]);
    let rig = data.value("rig", Rig { curves, armed: None });
    let histograms = [counts(0.0), counts(0.0), counts(12.0), counts(-12.0)];
    CurvesPanel::new("Curves", Bind::bound(&mut rig.curves), channels).histograms(&histograms).pickers(Bind::bound(&mut rig.armed)).show(ui).into()
}

#[story(CurvesPanel<'static>, covers = [], drive = Drive::Never)]
fn curves_panel_pencil(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let channels = ToneChannels::of(ToneSpace::Rgb, false);
    let drawn = ToneCurves { curves: vec![Curve::Map((0..=255).map(|level| level as f32).collect()); channels.len()], mode: CurvesMode::Pencil };
    let curves = data.value("drawn", drawn);
    CurvesPanel::new("Curves", Bind::bound(curves), channels).histograms(&[counts(0.0), counts(0.0), counts(0.0), counts(0.0)]).show(ui).into()
}

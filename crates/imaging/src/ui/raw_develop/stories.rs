//! Stories of the RAW Develop workspace and its components.
//!
//! The source's pictures are a generated photo (after) and a flat grey (before); its scope frame is one fixed
//! histogram; its develop finishes at once.

use std::sync::Arc;
use std::time::Duration;

use craft_behavior::PixelSize;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::contract::{Bind, Role};
use craft_ui::dialog::{HostedWorkspace, stories::hosted};
use craft_ui::primitives::Revision;
use craft_ui::runtime::pending::Job;
use craft_ui::testing::input::key_press;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::{Color32, ColorImage, Key, Modifiers, Ui};

use super::{
    Demosaic, RawDevelopOpen, RawDevelopPanel, RawDevelopSource, RawDevelopWorkspace, RawFilmItem, RawFilmstrip, RawFloatingScope, RawPreviewRequest,
    RawScopeStrip, RawTarget,
};
use crate::raw::RawDevelopSettings;
use crate::scopes::ClippingMask;
use crate::ui::histogram::ChannelColor;
use crate::ui::mock_image::MockImage;
use crate::ui::scopes::{HistogramSet, ScopeChannel, ScopeFrame, ScopePanelState, ScopeReadout, ScopeSettings, ScopeView};
use crate::ui::wide_angle::PreviewDone;

/// The picture's size in pixels.
const IMAGE: PixelSize = PixelSize::new(480, 320);

/// The before picture: one flat grey.
struct Flat;

impl RasterSource for Flat {
    fn size(&self) -> PixelSize {
        IMAGE
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        Some(PixelRect::default())
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        out.pixels.fill(Color32::from_gray(0x60));
    }
}

/// The story's source.
struct Pictures {
    after: MockImage,
    before: Flat,
}

impl RawDevelopSource for Pictures {
    fn after(&self) -> &dyn RasterSource {
        &self.after
    }

    fn before(&self) -> &dyn RasterSource {
        &self.before
    }

    fn develop(&self, _: &RawPreviewRequest) -> Job<PreviewDone, String> {
        Job::new(|_, _| Ok(PreviewDone::new(Duration::from_millis(12)).backend("CPU")))
    }

    fn auto(&self, settings: &RawDevelopSettings) -> RawDevelopSettings {
        settings.clone()
    }

    fn scope(&self, _: bool, _: bool, _: Option<[f64; 2]>, show: &mut dyn FnMut(Option<ScopeFrame<'_>>)) {
        let bins = bins();
        let channels = [ScopeChannel::new("rgb", "RGB", ChannelColor::Neutral, &bins, false)];
        show(Some(ScopeFrame::histogram(HistogramSet::new(&channels))));
    }

    fn clipping(&self) -> Option<(Arc<ClippingMask>, Revision)> {
        None
    }

    fn sample(&self, _: [f64; 2], _: bool, _: bool) -> Option<ScopeReadout> {
        Some(ScopeReadout::Rgb([0.5, 0.4, 0.3]))
    }

    fn has_selection(&self) -> bool {
        false
    }
}

/// A fixed histogram: a hill and a smaller one beside it.
fn bins() -> Vec<u32> {
    (0..256)
        .map(|level| {
            let x = level as f32;
            let main = (-((x - 110.0) / 40.0).powi(2) / 2.0).exp();
            let side = 0.4 * (-((x - 190.0) / 25.0).powi(2) / 2.0).exp();
            ((main + side) * 3000.0) as u32 + 8
        })
        .collect()
}

fn source() -> Arc<dyn RawDevelopSource> {
    Arc::new(Pictures { after: MockImage::new([IMAGE.width, IMAGE.height]), before: Flat })
}

fn raws() -> Vec<RawFilmItem> {
    ["IMG_0001.CR3", "IMG_0002.CR3", "IMG_0003.CR3"].into_iter().map(RawFilmItem::new).collect()
}

fn open(target: RawTarget) -> RawDevelopOpen {
    RawDevelopOpen::new("IMG_0001.CR3", IMAGE, target, RawDevelopSettings::default())
}

fn workspace(ui: &mut Ui, data: &mut StoryData, knobs: &'static str, open: impl FnOnce() -> RawDevelopOpen) -> StoryOutput {
    hosted(ui, data, knobs, || HostedWorkspace::new(RawDevelopWorkspace::new(open()), source()))
}

#[story(RawDevelopWorkspace, covers = [Default], drive = Drive::Never)]
fn filter(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    workspace(ui, data, "filter", || open(RawTarget::Filter))
}

#[story(RawDevelopWorkspace, covers = [], drive = Drive::Never)]
fn opening_with_three_raws(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    workspace(ui, data, "opening", || open(RawTarget::OpenRaw).demosaic(Demosaic::Ahd).filmstrip(raws(), 0))
}

#[story(RawDevelopWorkspace, covers = [], drive = Drive::Never)]
fn smart_filter_with_edits(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    workspace(ui, data, "smart filter", || {
        let mut workspace = open(RawTarget::SmartFilter);
        workspace.settings.exposure = 1.0;
        workspace
    })
}

/// Y goes down and stays down: the workspace shows the picture before the settings.
fn hold_before(fixture: &mut Fixture) {
    fixture.frame(vec![key_press(Key::Y, Modifiers::NONE)]);
}

#[story(RawDevelopWorkspace, covers = [], drive = Drive::Custom(hold_before))]
fn before_on(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    workspace(ui, data, "before", || open(RawTarget::Filter))
}

/// A click on the scope's Float action.
fn float_scope(fixture: &mut Fixture) {
    if let Some(float) = fixture.by_label("Float") {
        fixture.click(&float);
    }
}

#[story(RawDevelopWorkspace, covers = [], drive = Drive::Custom(float_scope))]
fn floating_scope(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    workspace(ui, data, "floating", || open(RawTarget::Filter))
}

#[story(RawDevelopPanel<'static>, covers = [Default], drive = Drive::Never)]
fn panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (settings, settings_at) = data.value_at("settings", RawDevelopSettings::default());
    RawDevelopPanel::new(settings_at.bind(settings)).show(ui).into()
}

#[story(RawDevelopPanel<'static>, covers = [], drive = Drive::Never)]
fn panel_with_demosaic(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (settings, settings_at) = data.value_at("settings", RawDevelopSettings::default());
    RawDevelopPanel::new(settings_at.bind(settings)).demosaic(Demosaic::Ahd).show(ui).into()
}

/// A click on the Color Mixer's Saturation tab, the second of its three.
fn saturation_tab(fixture: &mut Fixture) {
    if let Some(tab) = fixture.by_role(Role::TAB).get(1).cloned() {
        fixture.click(&tab);
    }
}

#[story(RawDevelopPanel<'static>, covers = [], drive = Drive::Custom(saturation_tab))]
fn panel_with_color_mixer_saturation(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (settings, settings_at) = data.value_at("settings", RawDevelopSettings::default());
    RawDevelopPanel::new(settings_at.bind(settings)).show(ui).into()
}

/// The scope state of the strip stories: Expanded, so the zone values and statistics show.
fn scope_state() -> ScopePanelState {
    ScopePanelState { settings: ScopeSettings { view: ScopeView::Expanded, ..ScopeSettings::default() }, range: None }
}

#[story(RawScopeStrip<'static>, covers = [Default], drive = Drive::Never)]
fn scope_strip(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = data.value("settings", RawDevelopSettings::default()).clone();
    let mut state = data.value("scope", scope_state()).clone();
    let mut output = StoryOutput::default();
    Pictures { after: MockImage::new([IMAGE.width, IMAGE.height]), before: Flat }.scope(false, false, None, &mut |frame| {
        output =
            RawScopeStrip::new(&mut state, frame.as_ref(), Bind::given(settings.clone())).show(ui).panel.map(StoryOutput::from).unwrap_or_default();
    });
    *data.value("scope", scope_state()) = state;
    output
}

#[story(RawFloatingScope<'static>, covers = [Default], drive = Drive::Never)]
fn floating_scope_window(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = data.value("settings", RawDevelopSettings::default()).clone();
    let mut state = data.value("scope", scope_state()).clone();
    let mut floating = true;
    let mut output = StoryOutput::default();
    Pictures { after: MockImage::new([IMAGE.width, IMAGE.height]), before: Flat }.scope(false, false, None, &mut |frame| {
        output = RawFloatingScope::new(&mut state, frame.as_ref(), Bind::bound(&mut floating), Bind::given(settings.clone()))
            .show(ui)
            .panel
            .map(StoryOutput::from)
            .unwrap_or_default();
    });
    *data.value("scope", scope_state()) = state;
    output
}

#[story(RawFilmstrip<'static>, covers = [Default], drive = Drive::Never)]
fn filmstrip(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let items = raws();
    let (current, current_at) = data.value_at("current", 1_usize);
    RawFilmstrip::new(&items, current_at.bind(current)).show(ui);
    StoryOutput::default()
}

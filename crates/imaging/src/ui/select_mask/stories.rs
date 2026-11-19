//! Stories of the Select and Mask workspace, its options bar and its Properties panel.
//!
//! The workspace stories run it over `MockImage` through a fake `SelectMaskSource`: the picture is the preview (the
//! engine's composite is the app's), the house is the refined selection. The stroke trail, the open polygon, the
//! brush outline and the Alt + right-drag resize are the pointer's: draw on the picture to see them.

use std::sync::Arc;
use std::time::Duration;

use egui::Ui;

use craft_ui::canvas::RasterSource;
use craft_ui::dialog::HostedWorkspace;
use craft_ui::dialog::stories::hosted;
use craft_ui::runtime::pending::Job;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};

use super::{
    RefineSettings, SelectMaskBar, SelectMaskLimits, SelectMaskOpen, SelectMaskPanel, SelectMaskPreviewRequest, SelectMaskSettings, SelectMaskSource,
    SelectMaskTool, SelectMaskToolOptions, SelectMaskWorkspace,
};
use crate::trace::MaskSource;
use crate::ui::mask_view::{MaskViewMode, MaskViewState};
use crate::ui::mock_image::{MockImage, MockSubject};
use crate::ui::wide_angle::PreviewDone;

/// The limits a story runs under: PhotoCraft's.
const LIMITS: SelectMaskLimits = SelectMaskLimits { edits: 4096, edit_points: 100_000, strokes: 4096, stroke_points: 100_000 };

/// How often a never-finishing preview job looks for its cancellation.
const POLL: Duration = Duration::from_millis(50);

/// The app's side over `MockImage`: the picture as the view, the house as the selection.
struct StorySource {
    image: MockImage,
    subject: MockSubject,
    /// The preview job never finishes: the loading state.
    loading: bool,
}

impl SelectMaskSource for StorySource {
    fn view(&self) -> &dyn RasterSource {
        &self.image
    }

    fn image(&self) -> &dyn RasterSource {
        &self.image
    }

    fn ants(&self) -> Option<(&dyn MaskSource, u32)> {
        Some((&self.subject, 1))
    }

    fn preview_job(&self, _: &SelectMaskPreviewRequest) -> Job<PreviewDone, String> {
        if self.loading {
            Job::new(|_, cancel| {
                while !cancel.is_cancelled() {
                    std::thread::sleep(POLL);
                }
                Err("cancelled".to_owned())
            })
        } else {
            Job::new(|_, _| Ok(PreviewDone::new(Duration::ZERO)))
        }
    }
}

/// Shows the workspace over the mock picture with `settings`; `loading` leaves the preview unfinished. It runs on the
/// story's dialog host with the open flag the story keeps.
fn show_workspace(ui: &mut Ui, data: &mut StoryData, settings: SelectMaskSettings, loading: bool) -> StoryOutput {
    // A pass's `WorkspaceCx` is built by the host only, so this runs the workspace on a `DialogHost`.
    hosted(ui, data, (settings.clone(), loading), || {
        let image = MockImage::default();
        let size = image.size();
        let open = SelectMaskOpen::new([size.width, size.height], "House", LIMITS).settings(settings);
        HostedWorkspace::new(SelectMaskWorkspace::new(open), Arc::new(StorySource { image, subject: MockSubject(image), loading }))
    })
}

fn viewing(view: MaskViewMode) -> SelectMaskSettings {
    SelectMaskSettings { view, ..SelectMaskSettings::default() }
}

#[story(SelectMaskWorkspace, covers = [Default], drive = Drive::Never)]
fn workspace(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, SelectMaskSettings::default(), false)
}

#[story(SelectMaskWorkspace, covers = [], drive = Drive::Never)]
fn view_marching_ants(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, viewing(MaskViewMode::MarchingAnts), false)
}
#[story(SelectMaskWorkspace, covers = [], drive = Drive::Never)]
fn view_overlay(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, viewing(MaskViewMode::Overlay), false)
}
#[story(SelectMaskWorkspace, covers = [], drive = Drive::Never)]
fn view_on_black(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, viewing(MaskViewMode::OnBlack), false)
}
#[story(SelectMaskWorkspace, covers = [], drive = Drive::Never)]
fn view_on_white(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, viewing(MaskViewMode::OnWhite), false)
}
#[story(SelectMaskWorkspace, covers = [], drive = Drive::Never)]
fn view_black_and_white(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, viewing(MaskViewMode::BlackAndWhite), false)
}
#[story(SelectMaskWorkspace, covers = [], drive = Drive::Never)]
fn view_on_layers(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, viewing(MaskViewMode::OnLayers), false)
}

#[story(SelectMaskWorkspace, covers = [], drive = Drive::Never)]
fn workspace_with_refinements(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refine = RefineSettings {
        radius: 12.0,
        smart_radius: true,
        smooth: 20.0,
        feather: 4.0,
        contrast: 30.0,
        shift_edge: -10.0,
        ..RefineSettings::default()
    };
    show_workspace(ui, data, SelectMaskSettings { refine, ..SelectMaskSettings::default() }, false)
}

#[story(SelectMaskWorkspace, covers = [], drive = Drive::Never)]
fn workspace_with_the_preview_loading(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, SelectMaskSettings::default(), true)
}

/// The bar for `tool` over the default options.
fn bar(ui: &mut Ui, data: &mut StoryData, tool: SelectMaskTool) -> StoryOutput {
    let options = data.value("options", SelectMaskToolOptions::default());
    SelectMaskBar::new(tool, options).show(ui).into()
}

#[story(SelectMaskBar<'static>, covers = [Default], drive = Drive::Never)]
fn bar_for_quick_selection(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, SelectMaskTool::Quick)
}

#[story(SelectMaskBar<'static>, covers = [], drive = Drive::Never)]
fn bar_for_the_brush(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, SelectMaskTool::Brush)
}

#[story(SelectMaskBar<'static>, covers = [], drive = Drive::Never)]
fn bar_for_object_selection(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, SelectMaskTool::Object)
}

#[story(SelectMaskBar<'static>, covers = [], drive = Drive::Never)]
fn bar_for_the_lasso(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, SelectMaskTool::Lasso)
}

#[story(SelectMaskBar<'static>, covers = [], drive = Drive::Never)]
fn bar_for_the_hand(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, SelectMaskTool::Hand)
}

#[story(SelectMaskPanel<'static>, covers = [Default], drive = Drive::Never)]
fn panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = data.value("settings", SelectMaskSettings::default()).clone();
    let mut settings = settings;
    let mut refine = RefineSettings::default();
    let mut view = MaskViewState { mode: MaskViewMode::Overlay, ..MaskViewState::default() };
    SelectMaskPanel::new(&mut settings, &mut refine, &mut view).show(ui).into()
}

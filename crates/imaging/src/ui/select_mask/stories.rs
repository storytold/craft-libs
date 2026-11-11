//! Stories of the Select and Mask workspace, its options bar and its Properties panel.
//!
//! The workspace stories run it over `MockImage` through a fake `SelectMaskSource`: the picture is the preview (the
//! engine's composite is the app's), the house is the refined selection. The stroke trail, the open polygon, the
//! brush outline and the Alt + right-drag resize are the pointer's: draw on the picture to see them.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use egui::Ui;

use craft_ui::canvas::RasterSource;
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

/// The workspace a story keeps between passes, compared by identity.
#[derive(Clone)]
struct Kept(Arc<Mutex<SelectMaskWorkspace>>);

impl PartialEq for Kept {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Shows the workspace over the mock picture with `settings`; `loading` leaves the preview unfinished.
fn show_workspace(ui: &mut Ui, data: &mut StoryData, settings: SelectMaskSettings, loading: bool) -> StoryOutput {
    let image = MockImage::default();
    let size = image.size();
    let open = SelectMaskOpen::new([size.width, size.height], "House", LIMITS).settings(settings);
    let kept = data.value("workspace", Kept(Arc::new(Mutex::new(SelectMaskWorkspace::new(open))))).clone();
    let mut workspace = kept.0.lock().unwrap_or_else(PoisonError::into_inner);
    workspace.show(ui, &StorySource { image, subject: MockSubject(image), loading }).into()
}

fn viewing(view: MaskViewMode) -> SelectMaskSettings {
    SelectMaskSettings { view, ..SelectMaskSettings::default() }
}

/// One story per view: the workspace with that view chosen.
macro_rules! view_story {
    ($name:ident, $view:expr) => {
        #[story(SelectMaskWorkspace, covers = [], drive = Drive::Never)]
        fn $name(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
            show_workspace(ui, data, viewing($view), false)
        }
    };
}

#[story(SelectMaskWorkspace, covers = [Default], drive = Drive::Never)]
fn workspace(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, SelectMaskSettings::default(), false)
}

view_story!(view_marching_ants, MaskViewMode::MarchingAnts);
view_story!(view_overlay, MaskViewMode::Overlay);
view_story!(view_on_black, MaskViewMode::OnBlack);
view_story!(view_on_white, MaskViewMode::OnWhite);
view_story!(view_black_and_white, MaskViewMode::BlackAndWhite);
view_story!(view_on_layers, MaskViewMode::OnLayers);

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

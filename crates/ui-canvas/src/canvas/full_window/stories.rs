//! The story of the full-window dialog shell.

use std::ops::RangeInclusive;
use std::sync::Arc;

use super::{FullWindowDialog, FullWindowWorkspace, HostedWorkspace, WorkspaceCx, WorkspacePass};
use crate::canvas::{PixelRect, RasterSource};
use crate::contract::{Bind, Show as _, ShowContainer as _};
use crate::dialog::stories::hosted;
use crate::dialog::{DialogWords, FieldRefusal, FullWindowCanvas, FullWindowSide, FullWindowView, FullWindowZoomBar};
use crate::display::Text;
use crate::form::SliderField;
use crate::primitives::Revision;
use crate::runtime::env::{Env, Label};
use crate::runtime::windows::WindowSpec;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use egui::{Color32, ColorImage, Context, Ui};

/// The drive of the workspace: a click on Cancel.
fn cancel(fixture: &mut Fixture) {
    if let Some(cancel) = fixture.by_label("Cancel") {
        fixture.click(&cancel);
    }
}

/// A workspace as Liquify shows one: the view in the body, a side column with the brush size, and the footer among
/// the side columns, with Apply when the story asks for it.
#[story(FullWindowDialog<'static>, covers = [Default], drive = Drive::Custom(cancel))]
fn full_window_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    liquify_shell(ui, data, false)
}

/// The same workspace in an OS window of its own, as RAW Develop opens; the story kit has no viewports, so it fills
/// the window.
#[story(FullWindowDialog<'static>, covers = [], drive = Drive::Custom(cancel))]
fn own_window_dialog(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    liquify_shell(ui, data, true)
}

/// The Liquify shell of the stories above, in its own window when `own_window` says so.
fn liquify_shell(ui: &mut Ui, data: &mut StoryData, own_window: bool) -> StoryOutput {
    let apply = *data.value("apply", false);
    let mut brush = *data.value("brush size", 50.0_f64);
    let env = Env::of(ui);
    let common = env.words::<CommonWords>();
    let mut buttons = vec![FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
    if apply {
        buttons.push(FooterButton::new(env.words::<DialogWords>().apply.clone(), DialogRole::Other).stays_open(true));
    }
    let mut dialog = FullWindowDialog::new("Liquify");
    if own_window {
        dialog = dialog.own_window(WindowSpec::dialog());
    }
    let shown: StoryOutput = dialog
        .opened(data.bind("open", true))
        .buttons(buttons)
        .sides(|ui, _| {
            FullWindowSide::new("Properties").show(ui, |ui, _| {
                SliderField::new("Brush size", Bind::bound(&mut brush), 1.0..=100.0).step(1.0).show(ui);
            });
        })
        .show(ui, |ui, _| {
            Text::new("The view fills the rest of the window.").show(ui);
        })
        .into();
    *data.value("brush size", 50.0_f64) = brush;
    shown
}

/// The sample workspace's picture: one flat colour.
pub(super) struct Flat;

impl RasterSource for Flat {
    fn size(&self) -> PixelSize {
        PixelSize::new(320, 200)
    }

    fn revision(&self) -> Revision {
        Revision::new(1)
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        Some(PixelRect::default())
    }

    fn read(&self, _: PixelRect, _: u32, out: &mut ColorImage) {
        out.pixels.fill(Color32::from_rgb(70, 110, 160));
    }
}

/// A brush size change the sample workspace reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BrushChanged(pub(super) f64);

/// The brush sizes the sample workspace's slider and its `brush` field take.
const BRUSH_SIZES: RangeInclusive<f64> = 1.0..=100.0;

/// The index of OK among the sample workspace's buttons.
const ACCEPT: usize = 0;

/// A workspace as Liquify shows one, for the hosted story and trials: the zoom bar over the canvas in the body, the
/// brush size in a side column, OK and Cancel. OK answers the brush size; automation reads and sets it as `brush`.
#[derive(Default)]
pub(super) struct BrushWorkspace {
    pub(super) view: FullWindowView,
    pub(super) brush: f64,
    /// The source revision the last pass showed.
    pub(super) revision: u64,
    /// Why the app refused the last held OK.
    pub(super) failed: Option<Label>,
    /// What automation changed since the last pass, reported by the next one.
    pub(super) queued: Vec<BrushChanged>,
}

impl FullWindowWorkspace for BrushWorkspace {
    type Source = dyn RasterSource + Send + Sync;
    type Event = BrushChanged;
    type Result = f64;

    const KEY: &'static str = "story.brushWorkspace";

    fn show(&mut self, ui: &mut Ui, cx: WorkspaceCx<'_, Self::Source>) -> WorkspacePass<BrushChanged, f64> {
        let WorkspaceCx { source, revision, open, hold_ok, ok_waits, .. } = cx;
        self.revision = revision;
        let common = Env::of(ui).words::<CommonWords>();
        let mut events = std::mem::take(&mut self.queued);
        let before = self.brush;
        let (view, brush, failed) = (&mut self.view, &mut self.brush, &self.failed);
        let ok = FooterButton::new(common.ok.clone(), DialogRole::Accept).stays_open(hold_ok).enabled(!ok_waits);
        let shown = FullWindowDialog::new("Liquify")
            .opened(open)
            .buttons([ok, FooterButton::new(common.cancel.clone(), DialogRole::Cancel)])
            .sides(|ui, _| {
                FullWindowSide::new("Properties").show(ui, |ui, _| {
                    SliderField::new("Brush size", Bind::bound(&mut *brush), BRUSH_SIZES).step(1.0).show(ui);
                    if let Some(reason) = failed {
                        Text::new(reason.clone()).show(ui);
                    }
                });
            })
            .show(ui, |ui, _| {
                FullWindowZoomBar::new(Bind::bound(&mut *view)).show(ui);
                FullWindowCanvas::new(Bind::bound(&mut *view), source).show(ui);
            });
        if self.brush != before {
            events.push(BrushChanged(self.brush));
        }
        let held = (hold_ok && shown.output.picked == Some(ACCEPT)).then_some(self.brush);
        let result = shown.output.result(|| self.brush);
        WorkspacePass::new(shown.output.response, events).result(result).held(held)
    }

    fn json_fields(&self, _source: &Self::Source) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "brush": self.brush }))
    }

    fn set_json_field(&mut self, _: &Context, _: &Self::Source, field: &str, value: serde_json::Value) -> Result<(), String> {
        if field != "brush" {
            return Err(FieldRefusal::Unknown.message(field));
        }
        let Some(size) = value.as_f64().filter(|size| BRUSH_SIZES.contains(size)) else {
            return Err(FieldRefusal::Rejects(&value).message(field));
        };
        if size != self.brush {
            self.brush = size;
            self.queued.push(BrushChanged(size));
        }
        Ok(())
    }

    fn confirm(&mut self, _: &Context, _: &Self::Source) -> Option<f64> {
        Some(self.brush)
    }

    fn ok_failed(&mut self, reason: Label) {
        self.failed = Some(reason);
    }
}

/// The sample workspace on the story's dialog host, opened as an app opens one.
#[story(FullWindowDialog<'static>, covers = [], drive = Drive::Custom(cancel))]
fn hosted_workspace(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || HostedWorkspace::new(BrushWorkspace { brush: 50.0, ..BrushWorkspace::default() }, Arc::new(Flat)))
}

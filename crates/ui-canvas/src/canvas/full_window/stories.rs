//! The story of the full-window dialog shell.

use std::sync::Arc;

use super::{FullWindowDialog, FullWindowWorkspace, HostedWorkspace, WorkspacePass};
use crate::canvas::{PixelRect, RasterSource};
use crate::contract::{Bind, Show as _, ShowContainer as _};
use crate::dialog::stories::hosted;
use crate::dialog::{DialogWords, FullWindowCanvas, FullWindowSide, FullWindowView, FullWindowZoomBar};
use crate::display::Text;
use crate::form::SliderField;
use crate::primitives::Revision;
use crate::runtime::env::Env;
use crate::runtime::words::CommonWords;
use crate::sections::{DialogRole, FooterButton};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::PixelSize;
use egui::{Color32, ColorImage, Ui};

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
    let apply = *data.value("apply", false);
    let mut brush = *data.value("brush size", 50.0_f64);
    let env = Env::of(ui);
    let common = env.words::<CommonWords>();
    let mut buttons = vec![FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)];
    if apply {
        buttons.push(FooterButton::new(env.words::<DialogWords>().apply.clone(), DialogRole::Other).stays_open(true));
    }
    let shown: StoryOutput = FullWindowDialog::new("Liquify")
        .opened(Bind::bound(data.value("open", true)))
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

/// A workspace as Liquify shows one, for the hosted story and trials: the zoom bar over the canvas in the body, the
/// brush size in a side column, OK and Cancel. OK answers the brush size.
#[derive(Default)]
pub(super) struct BrushWorkspace {
    pub(super) view: FullWindowView,
    pub(super) brush: f64,
}

impl FullWindowWorkspace for BrushWorkspace {
    type Source = dyn RasterSource + Send + Sync;
    type Event = BrushChanged;
    type Result = f64;

    const KEY: &'static str = "story.brushWorkspace";

    fn show(&mut self, ui: &mut Ui, source: &Self::Source, open: Bind<'_, bool>) -> WorkspacePass<BrushChanged, f64> {
        let common = Env::of(ui).words::<CommonWords>();
        let before = self.brush;
        let (view, brush) = (&mut self.view, &mut self.brush);
        let shown = FullWindowDialog::new("Liquify")
            .opened(open)
            .buttons([FooterButton::new(common.ok.clone(), DialogRole::Accept), FooterButton::new(common.cancel.clone(), DialogRole::Cancel)])
            .sides(|ui, _| {
                FullWindowSide::new("Properties").show(ui, |ui, _| {
                    SliderField::new("Brush size", Bind::bound(&mut *brush), 1.0..=100.0).step(1.0).show(ui);
                });
            })
            .show(ui, |ui, _| {
                FullWindowZoomBar::new(Bind::bound(&mut *view)).show(ui);
                FullWindowCanvas::new(Bind::bound(&mut *view), source).show(ui);
            });
        let events = if self.brush == before { Vec::new() } else { vec![BrushChanged(self.brush)] };
        let result = shown.output.result(|| self.brush);
        WorkspacePass::new(shown.output.response, events).result(result)
    }
}

/// The sample workspace on the story's dialog host, opened as an app opens one.
#[story(FullWindowDialog<'static>, covers = [], drive = Drive::Custom(cancel))]
fn hosted_workspace(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    hosted(ui, data, (), || HostedWorkspace::new(BrushWorkspace { brush: 50.0, ..BrushWorkspace::default() }, Arc::new(Flat)))
}

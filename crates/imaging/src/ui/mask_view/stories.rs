//! Stories of the mask views: the View Mode controls with and without an opacity, and the composite the engine would
//! build for each view that veils the picture.
//!
//! The story composites the recipe itself over `MockImage` with `MockImage::subject` as the soft mask (the house), so
//! every view shows without an engine ([`Composite`]).

use super::{MaskCoverage, MaskViewControls, MaskViewMode, MaskViewRecipe, MaskViewState, VeilMask};
use crate::ui::mock_image::{MockImage, MockSubject, StoryView, story_canvas};
use crate::ui::selection::SelectionOverlay;
use craft_behavior::PixelSize;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::primitives::Revision;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::{Color32, ColorImage, Ui};

/// Show Edge's band: its half width as a fraction of the picture's width.
const BAND: f64 = 0.008;

/// The view the engine would build from a [`MaskViewRecipe`], drawn on the spot: the layer (the picture) with its mask
/// raised to the recipe's floor, then each veil over it.
pub(crate) struct Composite {
    pub(crate) image: MockImage,
    pub(crate) recipe: MaskViewRecipe,
    pub(crate) revision: Revision,
}

impl Composite {
    /// The coverage the recipe shows at `(x, y)`.
    fn coverage(&self, x: f64, y: f64) -> f32 {
        match self.recipe.coverage {
            MaskCoverage::Refined => self.image.subject(x, y),
            // The selection before refinement: the hard outline.
            MaskCoverage::Original => f32::from(self.image.subject_distance(x, y) < 0.0),
            // The band either side of the edge.
            MaskCoverage::EdgeBand => {
                let reach = BAND * f64::from(RasterSource::size(&self.image).width);
                (1.0 - (self.image.subject_distance(x, y).abs() / reach).clamp(0.0, 1.0)) as f32
            }
        }
    }

    /// The composite at `(x, y)`: straight sRGBA.
    fn at(&self, x: f64, y: f64) -> Color32 {
        let [r, g, b, _] = self.image.pixel(x, y);
        if self.recipe.hidden {
            return Color32::from_rgb(r, g, b);
        }
        let c = self.coverage(x, y);
        let mut colour = [f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0];
        let mut alpha = self.recipe.layer_floor.map_or(1.0, |floor| c + (1.0 - c) * floor);
        for veil in &self.recipe.veils {
            let mask = match veil.mask {
                VeilMask::Everywhere => 1.0,
                VeilMask::Coverage => c,
                VeilMask::Inverse => 1.0 - c,
            };
            let on = veil.opacity * mask * veil.color.a;
            let out = on + alpha * (1.0 - on);
            if out > 0.0 {
                let over = [veil.color.r, veil.color.g, veil.color.b];
                for (channel, over) in colour.iter_mut().zip(over) {
                    *channel = (over * on + *channel * alpha * (1.0 - on)) / out;
                }
            }
            alpha = out;
        }
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        Color32::from_rgba_unmultiplied(byte(colour[0]), byte(colour[1]), byte(colour[2]), byte(alpha))
    }
}

impl RasterSource for Composite {
    fn size(&self) -> PixelSize {
        RasterSource::size(&self.image)
    }

    fn revision(&self) -> Revision {
        self.revision
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let [columns, rows] = out.size;
        let factor = f64::from(factor.max(1));
        for row in 0..rows {
            let y = f64::from(region.y0) + (row as f64 + 0.5) * factor;
            for column in 0..columns {
                out.pixels[row * columns + column] = self.at(f64::from(region.x0) + (column as f64 + 0.5) * factor, y);
            }
        }
    }
}

/// A canvas of the recipe's view over the picture, with the ants when the recipe has them.
fn composite(ui: &mut Ui, mode: MaskViewMode) -> StoryOutput {
    let image = MockImage::default();
    let recipe = MaskViewState { mode, ..MaskViewState::default() }.recipe();
    let ants = recipe.ants && !recipe.hidden;
    let view = Composite { image, recipe, revision: Revision::new(1) };
    story_canvas(ui, Some(&view), StoryView::Fit, |ui, canvas| {
        SelectionOverlay::new("Selection", &MockSubject(image)).visible(ants).show_at(ui, canvas);
        StoryOutput::default()
    })
}

#[story(MaskViewControls<'static>, covers = [Default], drive = Drive::Never)]
fn controls_with_opacity(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = data.value("state", MaskViewState { mode: MaskViewMode::Overlay, ..MaskViewState::default() });
    let shown = MaskViewControls::new(state).show(ui);
    let _ = shown.changed;
    StoryOutput::default()
}

#[story(MaskViewControls<'static>, covers = [], drive = Drive::Never)]
fn controls_without_opacity(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let state = data.value("state", MaskViewState { mode: MaskViewMode::MarchingAnts, ..MaskViewState::default() });
    let shown = MaskViewControls::new(state).show(ui);
    let _ = shown.changed;
    StoryOutput::default()
}

#[story(MaskViewControls<'static>, covers = [], drive = Drive::Never)]
fn overlay_composite(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    composite(ui, MaskViewMode::Overlay)
}

#[story(MaskViewControls<'static>, covers = [], drive = Drive::Never)]
fn on_black_composite(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    composite(ui, MaskViewMode::OnBlack)
}

#[story(MaskViewControls<'static>, covers = [], drive = Drive::Never)]
fn on_white_composite(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    composite(ui, MaskViewMode::OnWhite)
}

#[story(MaskViewControls<'static>, covers = [], drive = Drive::Never)]
fn black_and_white_composite(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    composite(ui, MaskViewMode::BlackAndWhite)
}

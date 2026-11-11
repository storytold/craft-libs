//! Stories of the Liquify workspace and its Properties panel.
//!
//! The workspace stories run it over `MockImage` through a fake `LiquifySource`: the picture itself (the strokes
//! only reach the app's field, which the stories do not have), the house as the freeze mask, a backdrop in grey and a
//! gently displaced mesh. The stroke in progress, the brush outline and the Alt + right-drag resize with its readout
//! are the pointer's: draw on the picture to see them.

use std::sync::{Arc, Mutex, PoisonError};

use egui::{Color32, ColorImage, Ui};

use craft_behavior::PixelSize;
use craft_behavior::kurbo::Point;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::contract::Bind;
use craft_ui::primitives::Revision;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};

use super::{BackdropMode, BackdropUse, LiquifyMesh, LiquifyOpen, LiquifyPanel, LiquifySettings, LiquifySource, LiquifyWorkspace, MeshSize};
use crate::ui::mock_image::MockImage;

/// The freeze mask's pixels per texel: coarse, since the mask is soft.
const MASK_TEXEL: u32 = 4;
/// The mesh's node spacing in document pixels at Small, Medium and Large (a story's picture is
/// 1600 px wide).
const MESH_SPACING: [f64; 3] = [32.0, 64.0, 128.0];
/// The displacement the mesh shows, as a share of the picture's width.
const BULGE: f64 = 0.03;
/// The reach of the mesh's bulge round the house, as a share of the picture's width.
const BULGE_REACH: f64 = 0.18;
/// The ids of the two layers the backdrop can use.
const LAYER_SKY: u64 = 1;
const LAYER_HOUSE: u64 = 2;
/// The backdrop opacity the view-options stories use.
const BACKDROP_OPACITY: f64 = 60.0;

/// The freeze mask: the house, white with the frozen amount as alpha, one texel per [`MASK_TEXEL`] pixels.
struct FreezeMask(MockImage);

impl RasterSource for FreezeMask {
    fn size(&self) -> PixelSize {
        let size = self.0.size();
        PixelSize::new(size.width.div_ceil(MASK_TEXEL), size.height.div_ceil(MASK_TEXEL))
    }

    fn revision(&self) -> Revision {
        self.0.revision()
    }

    fn damage(&self, _: Revision) -> Option<PixelRect> {
        None
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        let [columns, rows] = out.size;
        let (factor, texel) = (f64::from(factor.max(1)), f64::from(MASK_TEXEL));
        for row in 0..rows {
            let y = (f64::from(region.y0) + (row as f64 + 0.5) * factor) * texel;
            for column in 0..columns {
                let x = (f64::from(region.x0) + (column as f64 + 0.5) * factor) * texel;
                out.pixels[row * columns + column] = Color32::from_white_alpha((self.0.subject(x, y) * 255.0).round() as u8);
            }
        }
    }
}

/// The picture in grey: the backdrop is another rendering of the same document.
struct GreyBackdrop(MockImage);

impl RasterSource for GreyBackdrop {
    fn size(&self) -> PixelSize {
        self.0.size()
    }

    fn revision(&self) -> Revision {
        self.0.revision()
    }

    fn damage(&self, since: Revision) -> Option<PixelRect> {
        self.0.damage(since)
    }

    fn read(&self, region: PixelRect, factor: u32, out: &mut ColorImage) {
        self.0.read(region, factor, out);
        for pixel in &mut out.pixels {
            let [r, g, b, a] = pixel.to_array();
            let grey = ((u32::from(r) * 30 + u32::from(g) * 59 + u32::from(b) * 11) / 100) as u8;
            *pixel = Color32::from_rgba_unmultiplied(grey, grey, grey, a);
        }
    }
}

/// The app's side over `MockImage`.
struct StorySource {
    image: MockImage,
    mask: FreezeMask,
    backdrop: GreyBackdrop,
    layers: [(u64, String); 2],
}

impl StorySource {
    fn new(image: MockImage) -> Self {
        Self {
            image,
            mask: FreezeMask(image),
            backdrop: GreyBackdrop(image),
            layers: [(LAYER_SKY, "Sky".to_owned()), (LAYER_HOUSE, "House".to_owned())],
        }
    }
}

impl LiquifySource for StorySource {
    fn picture(&self) -> &dyn RasterSource {
        &self.image
    }

    fn freeze_mask(&self) -> Option<(&dyn RasterSource, u32)> {
        Some((&self.mask, MASK_TEXEL))
    }

    fn mesh(&self, size: MeshSize) -> Option<LiquifyMesh> {
        let image = self.image.size();
        let (width, height) = (f64::from(image.width), f64::from(image.height));
        let spacing = MESH_SPACING[MeshSize::ALL.iter().position(|s| *s == size).unwrap_or(0)];
        let columns = (width / spacing).ceil() as usize + 1;
        let rows = (height / spacing).ceil() as usize + 1;
        let bounds = self.image.subject_bounds();
        let center = Point::new(f64::from(bounds.x0 + bounds.x1) / 2.0, f64::from(bounds.y0 + bounds.y1) / 2.0);
        let (bulge, reach) = (BULGE * width, BULGE_REACH * width);
        // A Bloat over the house: the nodes move away from its middle, most near it.
        let points = (0..rows * columns)
            .map(|i| {
                let p = Point::new((i % columns) as f64 * spacing, (i / columns) as f64 * spacing);
                let (dx, dy) = (p.x - center.x, p.y - center.y);
                let pull = (-(dx * dx + dy * dy) / (2.0 * reach * reach)).exp();
                let length = dx.hypot(dy).max(1.0);
                Point::new(p.x + dx / length * bulge * pull, p.y + dy / length * bulge * pull)
            })
            .collect();
        Some(LiquifyMesh::new(columns, rows, points))
    }

    fn backdrop(&self) -> Option<&dyn RasterSource> {
        Some(&self.backdrop)
    }

    fn layers(&self) -> Vec<(u64, String)> {
        self.layers.to_vec()
    }
}

/// The workspace a story keeps between passes, compared by identity.
#[derive(Clone)]
struct Kept(Arc<Mutex<LiquifyWorkspace>>);

impl PartialEq for Kept {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Shows the workspace over the mock picture with `settings`.
fn show_workspace(ui: &mut Ui, data: &mut StoryData, settings: &LiquifySettings) -> StoryOutput {
    let image = MockImage::default();
    let size = image.size();
    let open = LiquifyOpen::new([size.width, size.height], "House").settings(settings.clone());
    let kept = data.value("workspace", Kept(Arc::new(Mutex::new(LiquifyWorkspace::new(open))))).clone();
    let mut workspace = kept.0.lock().unwrap_or_else(PoisonError::into_inner);
    workspace.show(ui, &StorySource::new(image)).into()
}

#[story(LiquifyWorkspace, covers = [Default], drive = Drive::Never)]
fn workspace(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, &LiquifySettings::default())
}

#[story(LiquifyWorkspace, covers = [], drive = Drive::Never)]
fn workspace_with_the_mask_shown(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, &LiquifySettings { show_mask: true, ..LiquifySettings::default() })
}

#[story(LiquifyWorkspace, covers = [], drive = Drive::Never)]
fn workspace_with_the_mesh_shown(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show_workspace(ui, data, &LiquifySettings { show_mesh: true, mesh_size: MeshSize::Medium, ..LiquifySettings::default() })
}

#[story(LiquifyPanel<'static>, covers = [Default], drive = Drive::Never)]
fn panel(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let settings = data.value("settings", LiquifySettings::default());
    LiquifyPanel::new(Bind::bound(settings)).show(ui).into()
}

#[story(LiquifyPanel<'static>, covers = [], drive = Drive::Never)]
fn panel_with_the_backdrop_shown(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let shown = LiquifySettings {
        show_backdrop: true,
        backdrop: BackdropUse::Layer(LAYER_SKY),
        backdrop_mode: BackdropMode::Blend,
        backdrop_opacity: BACKDROP_OPACITY,
        ..LiquifySettings::default()
    };
    let settings = data.value("settings", shown);
    let layers = [(LAYER_SKY, "Sky".to_owned()), (LAYER_HOUSE, "House".to_owned())];
    LiquifyPanel::new(Bind::bound(settings)).layers(&layers).show(ui).into()
}

//! Stories of the Filter Gallery workspace and its columns.
//!
//! The stories use `MockGallery`: a six-filter catalog in two folders with `Param` lists (a Range, a Choice, a
//! Bool, a color), thumbnails from `FakeThumbSource`, and a preview that is `MockImage` (no engine). The
//! workspace keeps its session between passes in the story's data, so a pick or a change stays.

use std::time::Duration;

use craft_theme::Srgba;
use craft_ui::canvas::{PixelRect, RasterSource};
use craft_ui::contract::Job;
use craft_ui::patterns::form_dialogs::{Param, ParamKind};
use craft_ui::primitives::textures::{FakeThumbSource, ThumbnailOptions, Thumbnails};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use craft_ui::value_drag::SliderScale;
use egui::Ui;

use super::{
    EffectChange, EffectLayer, EffectParameters, EffectStack, FilterCatalog, FilterCategory, FilterGalleryOpen, FilterGallerySession,
    FilterGallerySource, FilterGalleryWorkspace, FilterIndex, FilterTree, GalleryFilter, GalleryThumb,
};
use crate::ui::mock_image::MockImage;
use crate::ui::wide_angle::PreviewDone;

/// The time the zoom bar's note reports.
const NOTE: Duration = Duration::from_millis(120);
/// How far the loading render has come, as a fraction.
const PROGRESS: f32 = 0.4;
/// How often the loading render looks for its cancellation.
const POLL: Duration = Duration::from_millis(20);

// --- the fake app ---

/// The app's catalog: six filters in two folders.
struct MockGallery;

impl MockGallery {
    const ARTISTIC: [&'static str; 3] = ["cutout", "sponge", "neon"];
    const SKETCH: [&'static str; 3] = ["chalk", "photocopy", "paper"];

    fn range(key: &str, label: &'static str, min: f64, max: f64, default: f64) -> Param {
        Param::new(key, label, ParamKind::range(min, max, default, SliderScale::default()))
    }
}

impl FilterCatalog for MockGallery {
    fn categories(&self) -> Vec<FilterCategory> {
        let keys = |keys: &[&str]| keys.iter().map(|key| (*key).to_owned()).collect();
        vec![FilterCategory::new("artistic", "Artistic", keys(&Self::ARTISTIC)), FilterCategory::new("sketch", "Sketch", keys(&Self::SKETCH))]
    }

    fn filter(&self, key: &str) -> Option<GalleryFilter> {
        let (label, category, params, uses_colors) = match key {
            "cutout" => {
                let options = vec![("1".to_owned(), "Low".into()), ("2".to_owned(), "Medium".into()), ("3".to_owned(), "High".into())];
                let fidelity = Param::new("fidelity", "Edge Fidelity", ParamKind::choice(options, Some("2".to_owned())));
                (
                    "Cutout",
                    "artistic",
                    vec![
                        Self::range("levels", "Number of Levels", 2.0, 8.0, 6.0),
                        Self::range("simplicity", "Edge Simplicity", 0.0, 10.0, 4.0),
                        fidelity,
                    ],
                    false,
                )
            }
            "sponge" => {
                let smooth = Param::new("smooth", "Smooth edges", ParamKind::bool(true));
                (
                    "Sponge",
                    "artistic",
                    vec![Self::range("size", "Brush Size", 0.0, 10.0, 2.0), Self::range("detail", "Detail", 1.0, 25.0, 5.0), smooth],
                    false,
                )
            }
            "neon" => {
                let glow = Param::new("glow", "Glow Color", ParamKind::color(Some([0.9, 0.3, 0.6])));
                ("Neon Glow", "artistic", vec![Self::range("size", "Glow Size", -24.0, 24.0, 5.0), glow], true)
            }
            "chalk" => (
                "Chalk & Charcoal",
                "sketch",
                vec![Self::range("darkness", "Darkness", 0.0, 20.0, 6.0), Self::range("detail", "Detail", 0.0, 20.0, 1.0)],
                true,
            ),
            "photocopy" => (
                "Photocopy",
                "sketch",
                vec![Self::range("detail", "Detail", 1.0, 24.0, 24.0), Self::range("darkness", "Darkness", 1.0, 50.0, 5.0)],
                true,
            ),
            "paper" => ("Note Paper", "sketch", vec![Self::range("balance", "Image Balance", 0.0, 50.0, 25.0)], true),
            _ => return None,
        };
        Some(GalleryFilter::new(key, label, category, params).uses_colors(uses_colors))
    }
}

/// The start: Cutout, Sponge with its eye off, and Neon Glow on top, selected.
fn start() -> FilterGallerySession {
    let catalog = MockGallery;
    let mut session = FilterGallerySession::new("cutout", &catalog);
    let mut selected = 0;
    for (index, filter) in [(1, "sponge"), (2, "neon")] {
        session.apply(EffectChange::Add, &catalog, &mut selected);
        session.apply(EffectChange::SetFilter { index, filter: filter.to_owned() }, &catalog, &mut selected);
    }
    session.apply(EffectChange::set_visible(1, false), &catalog, &mut selected);
    session
}

/// The picture under the effects, with a render that is done at once or never finishes.
struct MockSource {
    image: MockImage,
    rendering: bool,
}

impl FilterGallerySource for MockSource {
    fn preview(&self) -> &dyn RasterSource {
        &self.image
    }

    fn render(&self, _: &[EffectLayer], _: PixelRect, _: f64) -> Job<PreviewDone, String> {
        if self.rendering {
            Job::new(|progress, cancel| {
                progress.report(PROGRESS);
                while !cancel.is_cancelled() {
                    std::thread::sleep(POLL);
                }
                Err(String::new())
            })
        } else {
            Job::new(|_, _| Ok(PreviewDone::new(NOTE)))
        }
    }
}

/// Shows the workspace over the session the story keeps.
fn workspace(ui: &mut Ui, data: &mut StoryData, rendering: bool) -> StoryOutput {
    let image = MockImage::default();
    let session = data.value("session", start()).clone();
    let mut workspace = FilterGalleryWorkspace::new(FilterGalleryOpen::new(image.size(), "Photo", session));
    let thumbs = Thumbnails::<GalleryThumb>::in_context(ui.ctx(), "gallery-story-thumbs", ThumbnailOptions::default());
    let out = workspace.show(ui, &MockGallery, &thumbs, &FakeThumbSource::<GalleryThumb>::new(), &MockSource { image, rendering });
    // A swatch press asks the app for a color; the story has no picker.
    if let Some(request) = out.pick_color {
        workspace.set_color(&request.key, Srgba::from_rgb_array([0.2, 0.7, 0.9]));
    }
    *data.value("session", start()) = workspace.session().clone();
    out.dialog.into()
}

#[story(FilterGalleryWorkspace, covers = [Default], drive = Drive::Never)]
fn filter_gallery(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    workspace(ui, data, false)
}

#[story(FilterGalleryWorkspace, covers = [], drive = Drive::Never)]
fn filter_gallery_rendering(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    workspace(ui, data, true)
}

#[story(FilterTree<'static>, covers = [Default], drive = Drive::Never)]
fn filter_tree(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let index = FilterIndex::of(&MockGallery);
    let thumbs = Thumbnails::<GalleryThumb>::in_context(ui.ctx(), "gallery-story-tree-thumbs", ThumbnailOptions::default());
    FilterTree::new(&index, "sponge", &thumbs, &FakeThumbSource::<GalleryThumb>::new()).show(ui).into()
}

#[story(EffectStack<'static>, covers = [Default], drive = Drive::Never)]
fn effect_stack(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let index = FilterIndex::of(&MockGallery);
    EffectStack::new(&start().effects, 2, &index).show(ui).into()
}

#[story(EffectParameters<'static>, covers = [Default], drive = Drive::Never)]
fn effect_parameters(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let index = FilterIndex::of(&MockGallery);
    let session = start();
    EffectParameters::new(2, &session.effects[2], &index).show(ui).into()
}

//! `LayersSettings`, `LayersOptions` and `LayersOptionsDialog`: Layers Panel Options.
//!
//! `LayersSettings` is the panel's persisted section (`.settings(&mut ..)` on the panel);
//! `LayersOptions` is the value the dialog edits and returns, held by the settings.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, FieldLayout, LabelKind, Props as _, Show as _, ShowContainer as _, SideLabel};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::display::{Picture, PictureSize};
use craft_ui::layout_components::Stack;
use craft_ui::list::{ListWords, ThumbSize};
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::runtime::env::Env;
use craft_ui::sections::GroupBox;
use egui::{Color32, ColorImage, Context, Ui};

use super::thumbs::ThumbContents;

/// The Layers panel's persisted section: the Panel Options and the panel menu's Show Filters.
/// Every field has a serde default, so a section saved by an older version loads.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct LayersSettings {
    /// The Panel Options dialog's value.
    pub options: LayersOptions,
    /// The panel menu's Filter Options › Show: the filter bar shows (default true).
    pub show_filters: bool,
}

impl Default for LayersSettings {
    /// [`LayersOptions::default`], filters shown.
    fn default() -> Self {
        Self { options: LayersOptions::default(), show_filters: true }
    }
}

/// Layers Panel Options: what the dialog edits and returns. Every field has a serde default.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct LayersOptions {
    /// Thumbnail Size: [`ThumbSize::None`], `Small`, `Medium` (default) or `Large`; the row
    /// height follows the size's `size.thumb.*` token.
    pub thumbnail_size: ThumbSize,
    /// Thumbnail Contents: Layer Bounds or Entire Document (default). The panel menu's Clip
    /// Thumbnails to Layer Bounds is the same setting; it is part of every
    /// [`LayerThumb`](super::thumbs::LayerThumb) key of a pixel thumbnail.
    pub thumbnail_contents: ThumbContents,
    /// Use Default Masks on Fill Layers (default true): the app reads it when it makes a fill
    /// layer.
    pub use_default_masks_on_fill_layers: bool,
    /// Expand New Effects (default true): the app opens a layer's effects when it adds one.
    pub expand_new_effects: bool,
    /// Add "copy" to Copied Layers and Groups (default true): the app reads it when it
    /// duplicates.
    pub add_copy_to_copied_layers_and_groups: bool,
    /// Preview on canvas when reordering layers (default true): the app reads it to decide whether
    /// its canvas shows the move of a layer drag before the drop. The panel reports the move only
    /// as the drop's [`LayersEvent::Drop`](super::panel::LayersEvent::Drop).
    pub preview_on_canvas_when_reordering: bool,
    /// Show layer mask badges (default false): a small badge on each mask thumbnail's corner
    /// (a pen for a vector mask, a dot for a pixel mask).
    pub show_layer_mask_badges: bool,
}

impl Default for LayersOptions {
    /// Photoshop's defaults: Medium, Entire Document, the first four switches on, badges off.
    fn default() -> Self {
        Self {
            thumbnail_size: ThumbSize::Medium,
            thumbnail_contents: ThumbContents::EntireDocument,
            use_default_masks_on_fill_layers: true,
            expand_new_effects: true,
            add_copy_to_copied_layers_and_groups: true,
            preview_on_canvas_when_reordering: true,
            show_layer_mask_badges: false,
        }
    }
}

craft_ui::words! {
    /// The words of Layers Panel Options. The size names are [`ThumbSize::label`]'s.
    pub struct LayersOptionsWords {
        title: "layersOptionsDialog.title" = "Layers Panel Options";
        thumbnail_size: "layersOptionsDialog.thumbnailSize" = "Thumbnail Size";
        /// The name of the sample picture beside a size.
        sample: "layersOptionsDialog.sample" = "Sample thumbnail";
        thumbnail_contents: "layersOptionsDialog.thumbnailContents" = "Thumbnail Contents";
        layer_bounds: "layersOptionsDialog.layerBounds" = "Layer Bounds";
        entire_document: "layersOptionsDialog.entireDocument" = "Entire Document";
        default_masks: "layersOptionsDialog.defaultMasks" = "Use Default Masks on Fill Layers";
        expand_new_effects: "layersOptionsDialog.expandNewEffects" = "Expand New Effects";
        add_copy: "layersOptionsDialog.addCopy" = "Add \u{201c}copy\u{201d} to Copied Layers and Groups";
        preview_on_canvas: "layersOptionsDialog.previewOnCanvas" = "Preview on canvas when reordering layers";
        mask_badges: "layersOptionsDialog.maskBadges" = "Show layer mask badges";
    }
}

/// The key of the sample picture in the texture store.
const SAMPLE_KEY: u64 = 0x6c61_7965_7273_7361;
/// The side of the sample picture in pixels.
const SAMPLE_SIDE: usize = 96;
/// The sky's grey at the top and the grey it gains toward the bottom.
const SKY_TOP: f32 = 119.0;
const SKY_GAIN: f32 = 30.0;
/// The grey of the far hills, the near hills and the outline.
const FAR_HILLS: u8 = 80;
const NEAR_HILLS: u8 = 64;
const OUTLINE: u8 = 92;
/// The ridge of the far hills and of the near hills as `(x, y)` points in fractions of the picture.
const FAR_RIDGE: [(f32, f32); 5] = [(0.0, 0.60), (0.22, 0.36), (0.46, 0.56), (0.72, 0.33), (1.0, 0.62)];
const NEAR_RIDGE: [(f32, f32); 4] = [(0.0, 0.76), (0.32, 0.60), (0.57, 0.69), (1.0, 0.51)];

/// The height of the piecewise-linear `ridge` at `x`.
fn ridge_at(ridge: &[(f32, f32)], x: f32) -> f32 {
    let Some(&(first_x, first_y)) = ridge.first() else { return 1.0 };
    let mut before = (first_x, first_y);
    for &(px, py) in ridge {
        if x <= px {
            let span = (px - before.0).max(f32::EPSILON);
            return before.1 + (py - before.1) * ((x - before.0) / span).clamp(0.0, 1.0);
        }
        before = (px, py);
    }
    before.1
}

/// The meadow's grass strokes: each runs from the bottom edge up to a tip `GRASS_LEAN` to the right, `GRASS_WIDTH`
/// wide, in fractions of the picture.
const GRASS: u8 = 104;
const GRASS_BLADES: usize = 16;
const GRASS_LEAN: f32 = 0.02;
const GRASS_WIDTH: f32 = 0.01;
/// The daisy: its centre, and its unit (a petal's reach) as a fraction of the picture.
const DAISY_CENTRE: (f32, f32) = (0.49, 0.57);
const DAISY_UNIT: f32 = 0.34;
/// Petals, in daisy units: how many, where each starts and how far it reaches from the centre, and its half width
/// at the middle. Every third petal is the shaded one.
const PETALS: usize = 15;
const PETAL_START: f32 = 0.22;
const PETAL_REACH: f32 = 0.83;
const PETAL_HALF_WIDTH: f32 = 0.22;
const PETAL: u8 = 238;
const PETAL_SHADED: u8 = 213;
/// The petal's edge, in fractions of the picture, and its grey.
const PETAL_EDGE: f32 = 0.005;
const PETAL_EDGE_GREY: u8 = 155;
/// The centre disc and its darker highlight (offset toward the top left), in daisy units.
const DISC_RADIUS: f32 = 0.27;
const DISC: u8 = 33;
const DISC_HIGHLIGHT: (f32, f32, f32) = (-0.065, -0.055, 0.16);
const DISC_HIGHLIGHT_GREY: u8 = 55;
/// The seed dots: a ring of 16 alternating between two radii (daisy units), each dot `SEED_RADIUS` of the picture.
const SEEDS: usize = 16;
const SEED_RINGS: (f32, f32) = (0.16, 0.21);
const SEED_RADIUS: f32 = 0.008;
const SEED: u8 = 119;
/// Samples per pixel along each axis.
const SUPERSAMPLE: usize = 2;

/// The distance from `p` to the segment `a`-`b`.
fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = (dx * dx + dy * dy).max(f32::EPSILON);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length).clamp(0.0, 1.0);
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

/// The grey of the daisy at `(u, v)` (fractions of the picture), or `None` off the flower. Later petals lie over
/// earlier ones, and the disc and seeds over the petals.
fn daisy_at(u: f32, v: f32) -> Option<f32> {
    let (dx, dy) = ((u - DAISY_CENTRE.0) / DAISY_UNIT, (v - DAISY_CENTRE.1) / DAISY_UNIT);
    let near = |x: f32, y: f32, radius: f32| (dx - x).powi(2) + (dy - y).powi(2) <= radius * radius;
    for n in 0..SEEDS {
        let theta = n as f32 * std::f32::consts::TAU / SEEDS as f32;
        let ring = if n % 2 == 0 { SEED_RINGS.0 } else { SEED_RINGS.1 };
        if near(theta.cos() * ring, theta.sin() * ring, SEED_RADIUS / DAISY_UNIT) {
            return Some(f32::from(SEED));
        }
    }
    if near(DISC_HIGHLIGHT.0, DISC_HIGHLIGHT.1, DISC_HIGHLIGHT.2) {
        return Some(f32::from(DISC_HIGHLIGHT_GREY));
    }
    if near(0.0, 0.0, DISC_RADIUS) {
        return Some(f32::from(DISC));
    }
    let edge = PETAL_EDGE / DAISY_UNIT;
    (0..PETALS).rev().find_map(|n| {
        let theta = (n as f32 + 0.5) * std::f32::consts::TAU / PETALS as f32;
        let along = dx * theta.cos() + dy * theta.sin();
        let across = (dy * theta.cos() - dx * theta.sin()).abs();
        let t = (along - PETAL_START) / PETAL_REACH;
        let half = PETAL_HALF_WIDTH * (std::f32::consts::PI * t.clamp(0.0, 1.0)).sin();
        if !(0.0..=1.0).contains(&t) || across > half {
            return None;
        }
        let grey = if across > half - edge {
            PETAL_EDGE_GREY
        } else if n % 3 == 0 {
            PETAL_SHADED
        } else {
            PETAL
        };
        Some(f32::from(grey))
    })
}

/// The grey of the landscape at `(u, v)`, fractions of the picture: sky, the two ranges of hills, the meadow's
/// grass and the daisy, each over the one before.
fn landscape_at(u: f32, v: f32) -> f32 {
    if let Some(grey) = daisy_at(u, v) {
        return grey;
    }
    let grass = (0..GRASS_BLADES).any(|n| {
        let x = ((n * 43 + 13) % 97) as f32 / 96.0;
        let tip = 0.78 + (n * 11 % 19) as f32 / 100.0;
        segment_distance((u, v), (x, 1.0), ((x + GRASS_LEAN).min(1.0), tip)) <= GRASS_WIDTH / 2.0
    });
    if grass {
        f32::from(GRASS)
    } else if v >= ridge_at(&NEAR_RIDGE, u) {
        f32::from(NEAR_HILLS)
    } else if v >= ridge_at(&FAR_RIDGE, u) {
        f32::from(FAR_HILLS)
    } else {
        SKY_TOP + v * SKY_GAIN
    }
}

/// The sample picture of the thumbnail sizes: a monochrome landscape of a pale sky, two ranges of hills and a meadow
/// with a daisy, outlined, each pixel the mean of `SUPERSAMPLE` x `SUPERSAMPLE` samples.
fn sample_image() -> ColorImage {
    let mut image = ColorImage::filled([SAMPLE_SIDE, SAMPLE_SIDE], Color32::from_gray(0));
    let last = SAMPLE_SIDE - 1;
    let step = 1.0 / (SAMPLE_SIDE * SUPERSAMPLE) as f32;
    for y in 0..SAMPLE_SIDE {
        for x in 0..SAMPLE_SIDE {
            let grey = if x == 0 || y == 0 || x == last || y == last {
                f32::from(OUTLINE)
            } else {
                let mut sum = 0.0;
                for sy in 0..SUPERSAMPLE {
                    for sx in 0..SUPERSAMPLE {
                        let u = ((x * SUPERSAMPLE + sx) as f32 + 0.5) * step;
                        let v = ((y * SUPERSAMPLE + sy) as f32 + 0.5) * step;
                        sum += landscape_at(u, v);
                    }
                }
                sum / (SUPERSAMPLE * SUPERSAMPLE) as f32
            };
            image.pixels[y * SAMPLE_SIDE + x] = Color32::from_gray(grey.round() as u8);
        }
    }
    image
}

/// The Layers Panel Options dialog, a [`StackDialog`] whose result is the edited
/// [`LayersOptions`].
///
/// ```ignore
/// dialogs.open(LayersOptionsDialog::new(prefs.layers.options.clone()));
/// // later
/// if let Some(DialogResult::Ok(options)) = dialogs.take::<LayersOptionsDialog>() { prefs.layers.options = options; }
/// ```
///
/// - **Layout**: a group box "Thumbnail Size" with a radio group of None, then Small, Medium and
///   Large, each shown by a sample picture of its size in place of its name; a group box "Thumbnail
///   Contents" with Layer Bounds and Entire Document; then the five switches as check boxes, in
///   PhotoCraft's order.
/// - **Keyboard**: the radio groups take arrows; Tab walks the groups and check boxes; Enter is
///   OK, Escape is Cancel.
/// - Returns `Ok(options)` on OK, `Cancel` on Cancel or Escape; the app applies nothing while it
///   is open.
#[derive(craft_ui::contract::Pattern)]
pub struct LayersOptionsDialog {
    id: DialogId,
    options: LayersOptions,
}

impl LayersOptionsDialog {
    /// The dialog over `initial`: the panel's current Panel Options.
    pub fn new(initial: LayersOptions) -> Self {
        Self { id: dialog_id("layers-options"), options: initial }
    }
}

impl StackDialog for LayersOptionsDialog {
    type Result = LayersOptions;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _stack: &DialogStack) -> Option<DialogResult<LayersOptions>> {
        let env = Env::of(ui);
        let words = env.words::<LayersOptionsWords>();
        let list = env.words::<ListWords>();
        let shown = FormDialog::new(words.title.clone(), Bind::bound(&mut self.options)).show(ui, |ui, _, options| {
            GroupBox::new(words.thumbnail_size.clone(), None).show(ui, |ui, _| {
                RadioGroup::new(words.thumbnail_size.clone(), Bind::bound(&mut options.thumbnail_size)).show(ui, |ui, cx| {
                    Stack::new().show(ui, |ui, _| {
                        // Each size's sample is its option's label: a click on it picks the size, and the size's
                        // name names the radio and is the sample's tooltip.
                        for size in ThumbSize::ALL {
                            let side = SideLabel::new().layout(FieldLayout::After);
                            if size == ThumbSize::None {
                                cx.labelled(ui, size, size.label(&list), side, Radio::new(size));
                            } else {
                                let sample = Picture::new(words.sample.clone(), SAMPLE_KEY, &sample_image).extent(PictureSize::Thumb(size));
                                cx.labelled_with(ui, size, size.label(&list), side.kind(LabelKind::Hidden), Radio::new(size), sample);
                            }
                        }
                    });
                });
            });
            GroupBox::new(words.thumbnail_contents.clone(), None).show(ui, |ui, _| {
                RadioGroup::new(words.thumbnail_contents.clone(), Bind::bound(&mut options.thumbnail_contents)).show(ui, |ui, _| {
                    Stack::new().show(ui, |ui, _| {
                        Radio::new(ThumbContents::LayerBounds).label(words.layer_bounds.clone()).show(ui);
                        Radio::new(ThumbContents::EntireDocument).label(words.entire_document.clone()).show(ui);
                    });
                });
            });
            let switches = [
                (&words.default_masks, &mut options.use_default_masks_on_fill_layers),
                (&words.expand_new_effects, &mut options.expand_new_effects),
                (&words.add_copy, &mut options.add_copy_to_copied_layers_and_groups),
                (&words.preview_on_canvas, &mut options.preview_on_canvas_when_reordering),
                (&words.mask_badges, &mut options.show_layer_mask_badges),
            ];
            for (label, value) in switches {
                Checkbox::new(Bind::bound(value)).label(label.clone()).show(ui);
            }
            true
        });
        match (shown.accepted, shown.closed) {
            (Some(options), _) => Some(DialogResult::Ok(options)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _ctx: &Context) -> Option<LayersOptions> {
        Some(self.options.clone())
    }
}

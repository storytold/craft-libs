//! `LayersSettings`, `LayersOptions` and `LayersOptionsDialog`: Layers Panel Options.
//!
//! `LayersSettings` is the panel's persisted section (`.settings(&mut ..)` on the panel);
//! `LayersOptions` is the value the dialog edits and returns, held by the settings.

#[cfg(feature = "screen-testing")]
mod stories;

use craft_ui::choice::{Checkbox, Radio, RadioGroup};
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::display::{Picture, PictureSize};
use craft_ui::layout_components::{Row, Stack};
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

/// The sample picture of the thumbnail sizes: a monochrome landscape of a pale sky and two ranges
/// of hills, outlined.
fn sample_image() -> ColorImage {
    let mut image = ColorImage::filled([SAMPLE_SIDE, SAMPLE_SIDE], Color32::from_gray(0));
    let last = SAMPLE_SIDE - 1;
    for y in 0..SAMPLE_SIDE {
        for x in 0..SAMPLE_SIDE {
            let (u, v) = (x as f32 / last as f32, y as f32 / last as f32);
            let grey = if x == 0 || y == 0 || x == last || y == last {
                OUTLINE
            } else if v >= ridge_at(&NEAR_RIDGE, u) {
                NEAR_HILLS
            } else if v >= ridge_at(&FAR_RIDGE, u) {
                FAR_HILLS
            } else {
                (SKY_TOP + v * SKY_GAIN) as u8
            };
            image.pixels[y * SAMPLE_SIDE + x] = Color32::from_gray(grey);
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
///   Large, each with a sample picture of its size beside its label; a group box "Thumbnail
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
            Stack::new().show(ui, |ui, _| {
                GroupBox::new(words.thumbnail_size.clone(), None).show(ui, |ui, _| {
                    RadioGroup::new(words.thumbnail_size.clone(), Bind::bound(&mut options.thumbnail_size)).show(ui, |ui, _| {
                        Stack::new().show(ui, |ui, _| {
                            for size in ThumbSize::ALL {
                                Row::new().show(ui, |ui, _| {
                                    Radio::new(size).label(size.label(&list)).show(ui);
                                    if size != ThumbSize::None {
                                        Picture::new(words.sample.clone(), SAMPLE_KEY, &sample_image).extent(PictureSize::Thumb(size)).show(ui);
                                    }
                                });
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
            });
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

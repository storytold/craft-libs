//! `PointerLooks`: the painted I-beam, type and path-text pointers as the window shows them, at three display scales.

use craft_behavior::PaintedCursor;
use craft_ui::contract::{Gap, Pattern, Props, Show as _, ShowContainer as _};
use craft_ui::display::{Picture, PictureSize, Shade, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::primitives::painted_image;
use craft_ui::runtime::env::Env;
use craft_ui::style::TypeRole;
use egui::{Color32, ColorImage, Ui};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};

/// The display scales, in device pixels per point, the pointers are drawn at.
const SCALES: [f32; 3] = [1.0, 1.25, 1.5];

/// What a card row shows: the pointer's name and its look.
fn looks() -> [(&'static str, PaintedCursor); 3] {
    [("I-beam", PaintedCursor::IBeam), ("Type", PaintedCursor::type_on(false)), ("Path text", PaintedCursor::path_text(true, true))]
}

/// The painted pointers `IBeam`, `TypeOn` and `PathText`, one row each, at scales 1.0, 1.25 and 1.5. Each is the
/// bitmap the window takes as its cursor image, so the pixel rule of the I-beam's strokes shows as it ships.
#[derive(Pattern, Props)]
pub struct PointerLooks {}

impl PointerLooks {
    /// The card over the three pointers at the three scales.
    pub fn new() -> Self {
        PointerLooks {}
    }

    /// Shows the rows one under the other, each pointer at each scale side by side at its device pixel size.
    pub fn show(self, ui: &mut Ui) {
        let env = Env::of(ui);
        Stack::new().fill_width().gap(Gap::Section).show(ui, |ui, _| {
            for (name, cursor) in looks() {
                Stack::new().fill_width().gap(Gap::Tight).show(ui, |ui, _| {
                    Text::new(name).role(TypeRole::BodyStrong).show(ui);
                    Row::new().gap(Gap::Section).show(ui, |ui, _| {
                        for scale in SCALES {
                            Stack::new().gap(Gap::Tight).show(ui, |ui, _| {
                                // The art takes its colours from the token set and its marks from the reading
                                // direction, so both are part of the key; the bitmap is drawn only when the key is new.
                                let mut hasher = DefaultHasher::new();
                                (name, scale.to_bits(), env.tokens.key(), env.direction.is_rtl()).hash(&mut hasher);
                                let image = || match painted_image(&env, &cursor, scale) {
                                    Some(bitmap) => {
                                        let [w, h] = bitmap.size;
                                        ColorImage::from_rgba_unmultiplied([usize::from(w), usize::from(h)], &bitmap.rgba)
                                    }
                                    None => ColorImage::new([1, 1], vec![Color32::TRANSPARENT]),
                                };
                                Picture::new(name, hasher.finish(), &image).extent(PictureSize::DevicePixels).show(ui);
                                Text::new(format!("{scale}×")).role(TypeRole::Mono).shade(Shade::Muted).show(ui);
                            });
                        }
                    });
                });
            }
        });
    }
}

impl Default for PointerLooks {
    fn default() -> Self {
        PointerLooks::new()
    }
}

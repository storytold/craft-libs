//! `CanvasSize`: the Canvas Size dialog.

use super::{CanvasExtension, Kit, LinkedSize, SizeForm, SizingOutput, show_form};
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::form::Field;
use crate::pickers::{Anchor9, ReferencePoint, Select};
use crate::runtime::env::UnitKind;
use craft_behavior::PixelSize;
use egui::Ui;

/// What Canvas Size answers with at OK.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CanvasSizeResult {
    /// The new canvas size in pixels.
    pub size: PixelSize,
    /// The unit the width and height showed at OK.
    pub unit: UnitKind,
    /// Whether the fields showed the change in size (the Relative box) at OK.
    pub relative: bool,
    /// Which part of the current canvas stays put.
    pub anchor: Anchor9,
    /// What fills the added area; Transparent for a document with no background.
    pub extension: CanvasExtension,
}

/// What Canvas Size reports: its response, how it closed and the [`CanvasSizeResult`] at OK.
pub type CanvasSizeOutput = SizingOutput<CanvasSizeResult>;

/// The values Canvas Size edits: the linked size, the anchor and the extension. The app builds them when it opens
/// the dialog and keeps them in the binding the dialog edits.
#[derive(Clone, Debug, PartialEq)]
pub struct CanvasSizeValues {
    size: LinkedSize,
    anchor: Anchor9,
    extension: CanvasExtension,
    has_background: bool,
}

impl CanvasSizeValues {
    /// The values for a canvas of `size` pixels, at 72 pixels per inch, in pixels, anchored at the centre and filling
    /// with the background colour.
    pub fn new(size: PixelSize) -> Self {
        CanvasSizeValues {
            size: LinkedSize::new(size).chain(false),
            anchor: Anchor9::Centre,
            extension: CanvasExtension::Background,
            has_background: true,
        }
    }

    /// The resolution of the document, in pixels per inch, for the physical units.
    #[must_use]
    pub fn pixels_per_inch(mut self, pixels_per_inch: f64) -> Self {
        self.size = self.size.opening_resolution(pixels_per_inch);
        self
    }

    /// The points in an inch of the Points and Picas units: 72, or 72.27 for traditional points.
    #[must_use]
    pub fn points_per_inch(mut self, points_per_inch: f64) -> Self {
        self.size = self.size.points_per_inch(points_per_inch);
        self
    }

    /// The unit the width and height open in.
    #[must_use]
    pub fn unit(mut self, unit: UnitKind) -> Self {
        self.size.set_unit(unit);
        self
    }

    /// Whether the fields open showing the change in size.
    #[must_use]
    pub fn relative(mut self, on: bool) -> Self {
        self.size.relative(on);
        self
    }

    /// The anchor the dialog opens with.
    #[must_use]
    pub fn anchor(mut self, anchor: Anchor9) -> Self {
        self.anchor = anchor;
        self
    }

    /// The extension the dialog opens with.
    #[must_use]
    pub fn extension(mut self, extension: CanvasExtension) -> Self {
        self.extension = extension;
        self
    }

    /// Whether the document has a background layer; without one the extension is Transparent and the list is off.
    #[must_use]
    pub fn has_background(mut self, has_background: bool) -> Self {
        self.has_background = has_background;
        self
    }
}

impl Default for CanvasSizeValues {
    /// The values of an empty canvas, which the dialog refuses to answer with.
    fn default() -> Self {
        CanvasSizeValues::new(PixelSize::new(0, 0))
    }
}

impl SizeForm for CanvasSizeValues {
    fn pixels(&self) -> PixelSize {
        self.size.pixels()
    }
}

/// Canvas Size: the current size, the new width and height with a unit each (no chain: the sides are independent), the
/// Relative box, the 3 by 3 anchor and the colour that fills a larger canvas.
///
/// - Keys: Tab walks the fields, Enter is OK unless a field owns it, Escape cancels. Holding Alt turns Cancel into
///   Reset, which restores the values from when the dialog opened and keeps the dialog open. The anchor takes the arrow
///   keys while it is focused.
/// - With Relative the fields hold the change in size, not the size.
/// - The anchor's arrows point where the canvas grows or shrinks.
/// - OK is refused while a side is under 1 or over `MAX_DOCUMENT_SIDE` pixels.
/// - Values: the `values` binding holds them; the app builds them with [`CanvasSizeValues::new`] when it opens the
///   dialog.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct CanvasSize<'a> {
    values: Bind<'a, CanvasSizeValues>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// The bytes one pixel takes, for the file size readouts.
    #[prop(quantity = "bytes per pixel")]
    bytes_per_pixel: f64,
}

impl<'a> CanvasSize<'a> {
    /// The dialog over `values`.
    pub fn new(values: Bind<'a, CanvasSizeValues>) -> Self {
        CanvasSize { values, open: None, bytes_per_pixel: 4.0 }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> CanvasSizeOutput {
        let CanvasSize { values, open, bytes_per_pixel } = self;
        let kit = Kit::new(ui);
        let words = kit.words();

        let title = words.canvas_size_title.clone();
        show_form(
            ui,
            title,
            open,
            values,
            |ui, state| {
                let size = state.size.original();
                let has_background = state.has_background;
                if !has_background {
                    state.extension = CanvasExtension::Transparent;
                }
                kit.readout(ui, &words.current_size, kit.file_size(size, bytes_per_pixel), None);
                kit.readout(ui, &words.current_width, kit.pixels(size.width), None);
                kit.readout(ui, &words.current_height, kit.pixels(size.height), None);
                let now = state.size.pixels();
                kit.readout(ui, &words.new_size, kit.file_size(now, bytes_per_pixel), kit.tone_of(now));
                kit.refusal(ui, now);
                kit.sides(ui, &mut state.size);

                let check = Checkbox::new(Bind::given(state.size.is_relative())).label(words.relative.clone()).id_salt("relative").show(ui);
                if let Some(on) = check.value().copied() {
                    state.size.relative(on);
                }
                let growth = (f64::from(now.width) - f64::from(size.width), f64::from(now.height) - f64::from(size.height));
                Field::new(words.anchor.clone()).id_salt("anchor").show(ui, |ui, _| {
                    let picked = ReferencePoint::new(words.anchor.clone(), Bind::given(state.anchor)).growth(growth).show(ui);
                    if let Some(anchor) = picked.value().copied() {
                        state.anchor = anchor;
                    }
                });
                Field::new(words.extension.clone()).id_salt("extension").enabled(has_background).show(ui, |ui, _| {
                    let entries = CanvasExtension::entries(words, &state.extension, has_background);
                    let picked = Select::new(words.extension.clone(), Bind::given(state.extension.clone()), &entries).show(ui);
                    if let (true, Some(extension)) = (has_background, picked.value().cloned()) {
                        state.extension = extension;
                    }
                });
            },
            |state| CanvasSizeResult {
                size: state.size.pixels(),
                unit: state.size.unit(),
                relative: state.size.is_relative(),
                anchor: state.anchor,
                extension: state.extension.clone(),
            },
        )
    }
}

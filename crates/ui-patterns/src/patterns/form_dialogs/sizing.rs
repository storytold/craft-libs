//! The size dialogs `ImageSize` and `CanvasSize`: the rows, the readouts and the dialog around them that the two share,
//! and the `Resample` and `CanvasExtension` choices.

use super::FormDialog;
use super::size_model::{LinkedSize, MAX_DOCUMENT_SIDE};
use crate::contract::{Bind, Entry, NoValue, Outcome, Props as _, Show as _, ShowContainer as _};
use crate::display::Text;
use crate::form::{Field, FieldGroup};
use crate::layout_components::{Row, Stack};
use crate::pickers::UnitSelect;
use crate::runtime::env::{Env, Label, UnitKind, Units};
use crate::runtime::overlay::CloseReason;
use crate::text::{LinkMode, LinkedWords, link_toggle};
use craft_behavior::PixelSize;
use craft_theme::variant::Tone;
use egui::{Response, Ui};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

mod canvas;
mod image;
#[cfg(feature = "testing")]
mod stories;

pub use canvas::{CanvasSize, CanvasSizeOutput, CanvasSizeResult, CanvasSizeValues};
pub use image::{HostedImageSize, ImageSize, ImageSizeOutput, ImageSizeResult, ImageSizeValues};

craft_ui::words! {
    /// The words of the size dialogs beside the common OK and Cancel.
    pub struct SizingWords {
        image_size_title: "sizing.imageSizeTitle" = "Image Size";
        canvas_size_title: "sizing.canvasSizeTitle" = "Canvas Size";
        image_size: "sizing.imageSize" = "Image Size";
        /// `{size}` is the new file size, `{old}` the one the dialog opened with.
        size_was: template "sizing.sizeWas" = "{size} (was {old})";
        dimensions: "sizing.dimensions" = "Dimensions";
        /// `{width}` and `{height}` are the sides with their unit.
        dimensions_value: template "sizing.dimensionsValue" = "{width} × {height}";
        current_size: "sizing.currentSize" = "Current Size";
        current_width: "sizing.currentWidth" = "Current width";
        current_height: "sizing.currentHeight" = "Current height";
        new_size: "sizing.newSize" = "New Size";
        width: "sizing.width" = "Width";
        height: "sizing.height" = "Height";
        resolution: "sizing.resolution" = "Resolution";
        width_unit: "sizing.widthUnit" = "Width unit";
        height_unit: "sizing.heightUnit" = "Height unit";
        resolution_unit: "sizing.resolutionUnit" = "Resolution unit";
        resample: "sizing.resample" = "Resample";
        resample_method: "sizing.resampleMethod" = "Resample method";
        scale_styles: "sizing.scaleStyles" = "Scale Styles";
        relative: "sizing.relative" = "Relative";
        anchor: "sizing.anchor" = "Anchor";
        extension: "sizing.extension" = "Canvas extension color";
        /// Why a number of pixels cannot be typed while Resample is off.
        resample_off: "sizing.resampleOff" = "Turn on Resample to change the number of pixels.";
        /// `{max}` is the greatest side in pixels.
        too_large: template "sizing.tooLarge" = "Each side must be 1 to {max} pixels.";
        automatic: "sizing.automatic" = "Automatic";
        preserve_details: "sizing.preserveDetails" = "Preserve Details";
        bicubic_smoother: "sizing.bicubicSmoother" = "Bicubic Smoother (enlargement)";
        bicubic_sharper: "sizing.bicubicSharper" = "Bicubic Sharper (reduction)";
        bicubic: "sizing.bicubic" = "Bicubic (smooth gradients)";
        nearest_neighbor: "sizing.nearestNeighbor" = "Nearest Neighbor (hard edges)";
        bilinear: "sizing.bilinear" = "Bilinear";
        lanczos: "sizing.lanczos" = "Lanczos (sharp)";
        foreground: "sizing.foreground" = "Foreground";
        background: "sizing.background" = "Background";
        white: "sizing.white" = "White";
        black: "sizing.black" = "Black";
        gray: "sizing.gray" = "Gray";
        other: "sizing.other" = "Other";
        transparent: "sizing.transparent" = "Transparent";
    }
}

/// How the pixels are made when the size changes: Photoshop's Resample list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Resample {
    /// The method suited to the change.
    Automatic,
    /// Enlarges and keeps the detail.
    PreserveDetails,
    /// Smooth gradients for an enlargement.
    BicubicSmoother,
    /// Sharp edges for a reduction.
    BicubicSharper,
    /// Smooth gradients.
    Bicubic,
    /// Hard edges.
    NearestNeighbor,
    /// The average of the four nearest pixels.
    Bilinear,
    /// Sharp results.
    Lanczos,
}

impl Resample {
    /// Every method, in the order of the list.
    pub const ALL: [Resample; 8] = [
        Resample::Automatic,
        Resample::PreserveDetails,
        Resample::BicubicSmoother,
        Resample::BicubicSharper,
        Resample::Bicubic,
        Resample::NearestNeighbor,
        Resample::Bilinear,
        Resample::Lanczos,
    ];

    /// The method's name in the list.
    pub fn label(self, words: &SizingWords) -> Label {
        match self {
            Resample::Automatic => words.automatic.clone(),
            Resample::PreserveDetails => words.preserve_details.clone(),
            Resample::BicubicSmoother => words.bicubic_smoother.clone(),
            Resample::BicubicSharper => words.bicubic_sharper.clone(),
            Resample::Bicubic => words.bicubic.clone(),
            Resample::NearestNeighbor => words.nearest_neighbor.clone(),
            Resample::Bilinear => words.bilinear.clone(),
            Resample::Lanczos => words.lanczos.clone(),
        }
    }

    /// The list entries of the methods in `allowed`, in the order of [`Resample::ALL`]; all of them when `allowed` names
    /// none.
    pub fn entries(words: &SizingWords, allowed: &[Resample]) -> Vec<Entry<Resample>> {
        let listed = |method: &Resample| allowed.is_empty() || allowed.contains(method);
        Resample::ALL.iter().copied().filter(listed).map(|method| Entry::new(method, method.label(words))).collect()
    }
}

/// What fills the area a larger canvas adds.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum CanvasExtension {
    /// The foreground colour.
    Foreground,
    /// The background colour.
    Background,
    /// White.
    White,
    /// Black.
    Black,
    /// Mid gray.
    Gray,
    /// A colour the app chose, as red, green and blue between 0 and 1.
    Other([f32; 3]),
    /// Nothing: the added area is transparent (a document with no background).
    Transparent,
}

impl CanvasExtension {
    /// The extension's name in the list.
    pub fn label(&self, words: &SizingWords) -> Label {
        match self {
            CanvasExtension::Foreground => words.foreground.clone(),
            CanvasExtension::Background => words.background.clone(),
            CanvasExtension::White => words.white.clone(),
            CanvasExtension::Black => words.black.clone(),
            CanvasExtension::Gray => words.gray.clone(),
            CanvasExtension::Other(_) => words.other.clone(),
            CanvasExtension::Transparent => words.transparent.clone(),
        }
    }

    /// The list entries for a canvas whose current extension is `current`: the five named colours, `Other` while it is
    /// the current one, and Transparent for a document with no background (`has_background` false).
    pub fn entries(words: &SizingWords, current: &CanvasExtension, has_background: bool) -> Vec<Entry<CanvasExtension>> {
        let mut list =
            vec![CanvasExtension::Foreground, CanvasExtension::Background, CanvasExtension::White, CanvasExtension::Black, CanvasExtension::Gray];
        if matches!(current, CanvasExtension::Other(_)) {
            list.push(current.clone());
        }
        if !has_background {
            list.push(CanvasExtension::Transparent);
        }
        list.into_iter().map(|extension| Entry::new(extension.clone(), extension.label(words))).collect()
    }
}

/// What a size dialog reports.
#[derive(Debug)]
#[non_exhaustive]
pub struct SizingOutput<R> {
    /// The dialog's response.
    pub response: Response,
    /// How the dialog closed in this pass; `None` while it stays open.
    pub closed: Option<CloseReason>,
    /// The answer, the pass the dialog closed as OK.
    pub result: Option<R>,
}

impl<R> Outcome for SizingOutput<R> {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The state a size dialog edits.
pub(super) trait SizeForm: Clone + PartialEq + Send + Sync + 'static {
    /// The pixel size the dialog would answer with.
    fn pixels(&self) -> PixelSize;
}

/// Whether both sides are 1 to `MAX_DOCUMENT_SIDE` pixels.
fn size_valid(size: PixelSize) -> bool {
    let side = |pixels: u32| (1.0..=MAX_DOCUMENT_SIDE).contains(&f64::from(pixels));
    side(size.width) && side(size.height)
}

/// A size dialog on `FormDialog`: `rows` edits the values and `answer` makes the result from them at OK. OK, Cancel
/// and Alt-for-Reset follow `ParamDialog`; OK is refused while the pixel size is empty or over the limit.
pub(super) fn show_form<'a, S: SizeForm + Default, R>(
    ui: &mut Ui,
    title: Label,
    open: Option<Bind<'a, bool>>,
    values: Bind<'a, S>,
    mut rows: impl FnMut(&mut Ui, &mut S),
    answer: impl FnOnce(&S) -> R,
) -> SizingOutput<R> {
    let mut dialog = FormDialog::new(title.clone(), values).outer_gesture(true);
    if let Some(open) = open {
        dialog = dialog.opened(open);
    }
    let shown = dialog.show(ui, |ui, _, state| {
        FieldGroup::new(title.clone()).show(ui, |ui, _| rows(ui, state));
        size_valid(state.pixels())
    });
    SizingOutput { response: shown.response, closed: shown.closed, result: shown.accepted.map(|state| answer(&state)) }
}

/// The rows both dialogs show, with the words and units they need.
pub(super) struct Kit {
    words: Arc<SizingWords>,
    units: Units,
    link: Label,
    lengths: Vec<Entry<UnitKind>>,
    resolutions: Vec<Entry<UnitKind>>,
}

/// The units the width and height are shown in.
const LENGTH_UNITS: [UnitKind; 7] =
    [UnitKind::Pixels, UnitKind::Percent, UnitKind::Inches, UnitKind::Centimetres, UnitKind::Millimetres, UnitKind::Points, UnitKind::Picas];

/// The units the resolution is shown in.
const RESOLUTION_UNITS: [UnitKind; 2] = [UnitKind::PixelsPerInch, UnitKind::PixelsPerCentimetre];

/// Which of the width, height and resolution a row edits.
#[derive(Clone, Copy)]
enum Dimension {
    Width,
    Height,
    Resolution,
}

impl Kit {
    pub(super) fn new(ui: &Ui) -> Self {
        let env = Env::of(ui);
        let units = env.units();
        let entries = |kinds: &[UnitKind]| kinds.iter().map(|kind| Entry::new(*kind, Label::from(units.symbol(*kind)))).collect();
        Kit {
            words: env.words::<SizingWords>(),
            link: LinkMode::Ratio.label(&env.words::<LinkedWords>()),
            lengths: entries(&LENGTH_UNITS),
            resolutions: entries(&RESOLUTION_UNITS),
            units,
        }
    }

    pub(super) fn words(&self) -> &SizingWords {
        &self.words
    }

    /// A line of text named `label`.
    pub(super) fn readout(&self, ui: &mut Ui, label: &Label, text: String, tone: Option<Tone>) {
        Field::new(label.clone()).show(ui, |ui, _| {
            let text = Text::new(text.clone());
            match tone {
                Some(tone) => text.tone(tone).show(ui),
                None => text.show(ui),
            };
        });
    }

    /// The size of a picture of `size` pixels at `bytes_per_pixel`, as a file size.
    pub(super) fn file_size(&self, size: PixelSize, bytes_per_pixel: f64) -> String {
        let bytes = f64::from(size.width) * f64::from(size.height) * bytes_per_pixel;
        self.units.file_size(bytes.max(0.0).round() as u64)
    }

    /// `size` as "width × height" with the pixel unit.
    pub(super) fn dimensions(&self, size: PixelSize) -> String {
        let (width, height) = (self.units.pixels(f64::from(size.width)), self.units.pixels(f64::from(size.height)));
        self.words.dimensions_value.fill(&[("width", width.as_str()), ("height", height.as_str())])
    }

    /// `pixels` in the pixel unit.
    pub(super) fn pixels(&self, pixels: u32) -> String {
        self.units.pixels(f64::from(pixels))
    }

    /// The readout of a new size: its text, in the tone of an error while the size cannot be answered.
    pub(super) fn tone_of(&self, size: PixelSize) -> Option<Tone> {
        (!size_valid(size)).then_some(Tone::Bad)
    }

    /// The line that says why OK is refused, when it is.
    pub(super) fn refusal(&self, ui: &mut Ui, size: PixelSize) {
        if !size_valid(size) {
            let max = self.units.number(MAX_DOCUMENT_SIDE, 0..=0);
            Text::new(self.words.too_large.fill(&[("max", max.as_str())])).tone(Tone::Bad).show(ui);
        }
    }

    /// The width and height rows, with the chain beside them when the model draws one.
    pub(super) fn sides(&self, ui: &mut Ui, size: &mut LinkedSize) {
        let chain = size.has_chain();
        Row::new().show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                self.row(ui, size, Dimension::Width);
                self.row(ui, size, Dimension::Height);
            });
            if chain {
                let toggle = link_toggle(self.link.clone(), Bind::given(size.is_linked())).enabled(size.is_resampling()).id_salt("link").show(ui);
                if let Some(on) = toggle.value().copied() {
                    size.lock(on);
                }
            }
        });
    }

    /// The resolution row.
    pub(super) fn resolution(&self, ui: &mut Ui, size: &mut LinkedSize) {
        self.row(ui, size, Dimension::Resolution);
    }

    /// One number with its unit, over the model.
    fn row(&self, ui: &mut Ui, size: &mut LinkedSize, which: Dimension) {
        let words = &self.words;
        let (name, unit_name, value, range, unit, entries, salt) = match which {
            Dimension::Width => (&words.width, &words.width_unit, size.width(), size.width_range(), size.unit(), &self.lengths, "width"),
            Dimension::Height => (&words.height, &words.height_unit, size.height(), size.height_range(), size.unit(), &self.lengths, "height"),
            Dimension::Resolution => (
                &words.resolution,
                &words.resolution_unit,
                size.resolution(),
                size.resolution_range(),
                size.resolution_unit(),
                &self.resolutions,
                "resolution",
            ),
        };
        let editable = !matches!(which, Dimension::Width | Dimension::Height) || size.is_editable();
        let mut field = Field::new(name.clone()).id_salt(salt);
        if !editable {
            field = field.tooltip(words.resample_off.clone());
        }
        field.show(ui, |ui, _| {
            let shown = UnitSelect::new(name.clone(), Bind::given(Some(value)), Bind::given(unit), entries)
                .range(range.clone())
                .unit_label(unit_name.clone())
                .number_enabled(editable)
                .id_salt("field")
                .show(ui);
            if let Some(number) = shown.edit.value().copied().flatten() {
                match which {
                    Dimension::Width => size.set_width(number),
                    Dimension::Height => size.set_height(number),
                    Dimension::Resolution => size.set_resolution(number),
                }
            }
            if let Some(kind) = shown.unit.value().copied() {
                match which {
                    Dimension::Width | Dimension::Height => size.set_unit(kind),
                    Dimension::Resolution => size.set_resolution_unit(kind),
                };
            }
        });
    }
}

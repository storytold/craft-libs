//! `ImageSize`: the Image Size dialog.

use super::{Kit, LinkedSize, Resample, SizeForm, SizingOutput, show_form};
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use crate::form::Field;
use crate::pickers::Select;
use crate::runtime::env::UnitKind;
use craft_behavior::PixelSize;
use egui::Ui;

/// What Image Size answers with at OK.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ImageSizeResult {
    /// The new size in pixels.
    pub size: PixelSize,
    /// The new resolution, in pixels per inch.
    pub pixels_per_inch: f64,
    /// The unit the width and height showed at OK.
    pub unit: UnitKind,
    /// The unit the resolution showed at OK.
    pub resolution_unit: UnitKind,
    /// The method the pixels are made with; `None` when Resample is off and the pixel count stays.
    pub resample: Option<Resample>,
    /// Whether layer effects scale with the image.
    pub scale_styles: bool,
}

/// What Image Size reports: its response, how it closed and the [`ImageSizeResult`] at OK.
pub type ImageSizeOutput = SizingOutput<ImageSizeResult>;

/// The values Image Size edits: the linked size, the Resample method and Scale Styles. The app builds them when it
/// opens the dialog and keeps them in the binding the dialog edits.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageSizeValues {
    size: LinkedSize,
    method: Resample,
    scale_styles: bool,
}

impl ImageSizeValues {
    /// The values for a document of `size` pixels, at 72 pixels per inch, in pixels, resampling automatically with
    /// Scale Styles on.
    pub fn new(size: PixelSize) -> Self {
        ImageSizeValues { size: LinkedSize::new(size), method: Resample::Automatic, scale_styles: true }
    }

    /// The resolution the document opens with, in pixels per inch.
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

    /// The unit the resolution opens in.
    #[must_use]
    pub fn resolution_unit(mut self, unit: UnitKind) -> Self {
        self.size.set_resolution_unit(unit);
        self
    }

    /// Whether Resample opens on; the default is on.
    #[must_use]
    pub fn resampling(mut self, on: bool) -> Self {
        self.size.resampling(on);
        self
    }

    /// The method Resample opens with, and returns to when it is turned on again.
    #[must_use]
    pub fn method(mut self, method: Resample) -> Self {
        self.method = method;
        self
    }

    /// Whether Scale Styles opens on.
    #[must_use]
    pub fn scale_styles(mut self, on: bool) -> Self {
        self.scale_styles = on;
        self
    }
}

impl Default for ImageSizeValues {
    /// The values of an empty document, which the dialog refuses to answer with.
    fn default() -> Self {
        ImageSizeValues::new(PixelSize::new(0, 0))
    }
}

impl SizeForm for ImageSizeValues {
    fn pixels(&self) -> PixelSize {
        self.size.pixels()
    }
}

/// Image Size: the width, height and resolution with a unit each, the chain that keeps the proportions, Resample with
/// its method list and Scale Styles, over a readout of the file size and the dimensions.
///
/// - Keys: Tab walks the fields, Enter is OK unless a field owns it, Escape cancels. Holding Alt turns Cancel into
///   Reset, which restores the values from when the dialog opened and keeps the dialog open. Ctrl/Cmd+Z steps the
///   dialog's own settings.
/// - With Resample off the pixel count is fixed: the width and height in pixels or percent are disabled, and a
///   physical size changes the resolution.
/// - OK is refused while a side is under 1 or over `MAX_DOCUMENT_SIDE` pixels.
/// - Values: the `values` binding holds them; the app builds them with [`ImageSizeValues::new`] when it opens the
///   dialog.
#[must_use = "show the dialog with `show`"]
#[derive(crate::contract::Pattern, crate::contract::Props)]
pub struct ImageSize<'a> {
    values: Bind<'a, ImageSizeValues>,
    /// Binds the open state to `open`.
    #[prop(name = "opened")]
    open: Option<Bind<'a, bool>>,
    /// The methods the list offers, in the order of [`Resample::ALL`]; all of them when empty.
    #[prop]
    methods: Vec<Resample>,
    /// The bytes one pixel takes, for the file size readout.
    #[prop(quantity = "bytes per pixel")]
    bytes_per_pixel: f64,
}

impl<'a> ImageSize<'a> {
    /// The dialog over `values`.
    pub fn new(values: Bind<'a, ImageSizeValues>) -> Self {
        ImageSize { values, open: None, methods: Vec::new(), bytes_per_pixel: 4.0 }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> ImageSizeOutput {
        let ImageSize { values, open, methods, bytes_per_pixel } = self;
        let kit = Kit::new(ui);
        let entries = Resample::entries(kit.words(), &methods);
        let first_method = entries.first().and_then(|entry| entry.value().copied());

        let words = kit.words();
        let title = words.image_size_title.clone();
        show_form(
            ui,
            title,
            open,
            values,
            |ui, state| {
                if let (false, Some(first)) = (entries.iter().any(|entry| entry.value() == Some(&state.method)), first_method) {
                    state.method = first;
                }
                let was = kit.file_size(state.size.original(), bytes_per_pixel);
                let now = state.size.pixels();
                let tone = kit.tone_of(now);
                let shown = words.size_was.fill(&[("size", kit.file_size(now, bytes_per_pixel).as_str()), ("old", was.as_str())]);
                kit.readout(ui, &words.image_size, shown, tone);
                kit.readout(ui, &words.dimensions, kit.dimensions(now), tone);
                kit.refusal(ui, now);
                kit.sides(ui, &mut state.size);
                kit.resolution(ui, &mut state.size);

                let on = Checkbox::new(Bind::given(state.size.is_resampling())).label(words.resample.clone()).id_salt("resample").show(ui);
                if let Some(on) = on.value().copied() {
                    state.size.resampling(on);
                }
                if state.size.is_resampling() {
                    Field::new(words.resample_method.clone()).id_salt("method").show(ui, |ui, _| {
                        let picked = Select::new(words.resample_method.clone(), Bind::given(state.method), &entries).show(ui);
                        if let Some(method) = picked.value().copied() {
                            state.method = method;
                        }
                    });
                }
                let styles = Checkbox::new(Bind::given(state.scale_styles))
                    .label(words.scale_styles.clone())
                    .id_salt("scale-styles")
                    .enabled(state.size.is_linked())
                    .show(ui);
                if let Some(on) = styles.value().copied() {
                    state.scale_styles = on;
                }
            },
            |state| ImageSizeResult {
                size: state.size.pixels(),
                pixels_per_inch: state.size.pixels_per_inch(),
                unit: state.size.unit(),
                resolution_unit: state.size.resolution_unit(),
                resample: state.size.is_resampling().then_some(state.method),
                scale_styles: state.scale_styles,
            },
        )
    }
}

/// Builds the dialog over the host-owned values of a [`HostedImageSize`] for one pass.
type Build = dyn for<'b> Fn(Bind<'b, ImageSizeValues>) -> ImageSize<'b> + Send;

/// A hosted Image Size: the `DialogHost` owns the values, and `build` makes the dialog over them each pass. The app
/// opens it with the values of its document and takes the [`ImageSizeResult`] from the host.
///
/// ```ignore
/// host.open(HostedImageSize::new(ImageSizeValues::new(document_size), |values| ImageSize::new(values)));
/// if let Some(DialogResult::Ok(result)) = host.take::<HostedImageSize>() {
///     resize(result);
/// }
/// ```
pub struct HostedImageSize {
    id: DialogId,
    values: ImageSizeValues,
    build: Box<Build>,
}

impl HostedImageSize {
    /// A dialog opening with `values`; `build` makes it over the values (`ImageSize::new` with its settings).
    pub fn new(values: ImageSizeValues, build: impl for<'b> Fn(Bind<'b, ImageSizeValues>) -> ImageSize<'b> + Send + 'static) -> Self {
        HostedImageSize { id: dialog_id(KEY), values, build: Box::new(build) }
    }

    /// The values the dialog shows.
    pub fn values(&self) -> &ImageSizeValues {
        &self.values
    }
}

/// The key of the hosted dialog.
const KEY: &str = "sizing.imageSize";

impl StackDialog for HostedImageSize {
    type Result = ImageSizeResult;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ImageSizeResult>> {
        let shown = (self.build)(Bind::bound(&mut self.values)).show(ui);
        match (shown.result, shown.closed) {
            (Some(result), _) => Some(DialogResult::Ok(result)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }
}

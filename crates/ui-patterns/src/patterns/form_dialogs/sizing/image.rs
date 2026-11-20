//! `ImageSize`: the Image Size dialog.

use super::{Hold, Kit, LENGTH_UNITS, LinkedSize, Resample, SizeForm, SizingOutput, set_size_json, show_form, size_json, size_valid};
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use crate::form::Field;
use crate::pickers::Select;
use crate::runtime::env::{ColumnSize, Label, UnitKind};
use crate::runtime::overlay::CloseReason;
use crate::runtime::{keys, overlay};
use craft_behavior::PixelSize;
use craft_keymap::{Chord, Mods};
use egui::Ui;
use serde_json::json;
use std::sync::{Arc, Mutex};

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
    /// Whether the resolution differs from the opening one. The opening resolution is held to the dialog's range, so
    /// while this is `false` the app keeps the document's own resolution, which may lie outside it.
    pub resolution_changed: bool,
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
    physical: LastPhysical,
}

/// The physical length the sides showed last, which turning Resample off switches pixels and percent to. It is a
/// memory of the dialog, not a value it shows: the clones the dialog makes of its values (the copy a pass edits, the
/// opening values Reset puts back, the undo steps) share one memory, so a change to it is never lost with a copy and
/// Reset and undo leave it. It takes no part in equality, which compares what the dialog shows.
#[derive(Clone, Debug)]
struct LastPhysical(Arc<Mutex<UnitKind>>);

impl LastPhysical {
    fn new(unit: UnitKind) -> Self {
        LastPhysical(Arc::new(Mutex::new(unit)))
    }

    /// The unit remembered; Inches if the memory cannot be read.
    fn get(&self) -> UnitKind {
        self.0.lock().map_or(UnitKind::Inches, |unit| *unit)
    }

    /// Remembers `unit` when it is a physical length.
    fn note(&self, unit: UnitKind) {
        if is_physical(unit)
            && let Ok(mut remembered) = self.0.lock()
        {
            *remembered = unit;
        }
    }
}

impl PartialEq for LastPhysical {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// Whether `unit` is a physical length the width and height can show: a length unit of the rows other than pixels
/// and percent.
fn is_physical(unit: UnitKind) -> bool {
    !matches!(unit, UnitKind::Pixels | UnitKind::Percent) && LENGTH_UNITS.contains(&unit)
}

impl ImageSizeValues {
    /// Remembers the shared unit when it is a physical length.
    fn note_unit(&self) {
        self.physical.note(self.size.unit());
    }

    /// The physical unit turning Resample off switches pixels and percent to until the sides show another (the unit
    /// used last, which the app keeps); Inches without one. A kind that is not a physical length is ignored.
    #[must_use]
    pub fn physical_unit(self, unit: UnitKind) -> Self {
        self.physical.note(unit);
        self
    }

    /// The document's column size (the Units preference's column width and gutter): the Width row offers Columns,
    /// converted through it. Without one the row offers no Columns.
    #[must_use]
    pub fn column_size(mut self, size: ColumnSize) -> Self {
        self.size = self.size.column_size(size);
        self
    }

    /// What OK answers with for these values.
    fn result(&self) -> ImageSizeResult {
        ImageSizeResult {
            size: self.size.pixels(),
            pixels_per_inch: self.size.pixels_per_inch(),
            unit: self.size.unit(),
            resolution_unit: self.size.resolution_unit(),
            resample: self.size.is_resampling().then_some(self.method),
            scale_styles: self.scale_styles,
            resolution_changed: self.size.resolution_changed(),
        }
    }

    /// The values for a document of `size` pixels, at 72 pixels per inch, in pixels, resampling automatically with
    /// Scale Styles on.
    pub fn new(size: PixelSize) -> Self {
        ImageSizeValues {
            size: LinkedSize::new(size),
            method: Resample::Automatic,
            scale_styles: true,
            physical: LastPhysical::new(UnitKind::Inches),
        }
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
        self.note_unit();
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
///   physical size changes the resolution. Turning the checkbox off switches them to the physical unit shown last
///   ([`LinkedSize::check_resampling`]); Pixels is greyed in their unit menus, and the method and Dimensions are
///   greyed.
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
    /// The bytes one pixel of the flat image takes, for the file size readout: the mode's colour channels at the
    /// document's depth, no alpha (1/8 for Bitmap, 1 for Indexed).
    #[prop(quantity = "bytes per pixel")]
    bytes_per_pixel: f64,
    /// The held OK of a hosted dialog ([`Hold`]).
    #[prop(name = "hold_ok", vis = "pub(crate)")]
    hold: Hold<ImageSizeValues>,
}

impl<'a> ImageSize<'a> {
    /// The dialog over `values`.
    pub fn new(values: Bind<'a, ImageSizeValues>) -> Self {
        ImageSize { values, open: None, methods: Vec::new(), bytes_per_pixel: 4.0, hold: Hold::default() }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> ImageSizeOutput {
        let ImageSize { values, open, methods, bytes_per_pixel, hold } = self;
        let kit = Kit::new(ui).resample_off_greys_pixels();
        let entries = Resample::entries(kit.words(), &methods);
        let first_method = entries.first().and_then(|entry| entry.value().copied());

        let words = kit.words();
        let title = words.image_size_title.clone();
        show_form(
            ui,
            title,
            open,
            values,
            hold,
            |ui, state| {
                if let (false, Some(first)) = (entries.iter().any(|entry| entry.value() == Some(&state.method)), first_method) {
                    state.method = first;
                }
                let (now, original) = (state.size.pixels(), state.size.original());
                let tone = kit.tone_of(now);
                let size = kit.file_size(now, bytes_per_pixel);
                // "(was …)" only once the pixel size differs from the document's.
                let shown = if now == original {
                    size
                } else {
                    words.size_was.fill(&[("size", size.as_str()), ("old", kit.file_size(original, bytes_per_pixel).as_str())])
                };
                kit.readout(ui, &words.image_size, shown, tone);
                // Without resampling the pixel dimensions cannot change, so their line is greyed.
                kit.readout_enabled(ui, &words.dimensions, kit.dimensions(now), tone, state.size.is_resampling());
                kit.refusal(ui, now);
                kit.sides(ui, &mut state.size);
                state.note_unit();
                kit.resolution(ui, &mut state.size);

                if !ui.is_sizing_pass() && overlay::drawing_top_modal(ui.ctx()) {
                    let owner = ui.scope_id().with("resample-keys");
                    for (key, method) in Resample::KEYS {
                        let chord = Chord::new(Mods::ALT, key);
                        keys::capture(ui.ctx(), owner, chord);
                        while keys::take_claimed(ui.ctx(), owner, chord) {
                            let offered = |method: &Resample| entries.iter().any(|entry| entry.value() == Some(method));
                            if let Some(method) = method.filter(|method| state.size.is_resampling() && offered(method)) {
                                state.method = method;
                            }
                        }
                    }
                }
                let on = Checkbox::new(Bind::given(state.size.is_resampling())).label(words.resample.clone()).id_salt("resample").show(ui);
                if let Some(on) = on.value().copied() {
                    state.size.check_resampling(on, state.physical.get());
                }
                // Greyed, not hidden, while Resample is off: it keeps the method it returns to.
                let resampling = state.size.is_resampling();
                Field::new(words.resample_method.clone()).id_salt("method").enabled(resampling).show(ui, |ui, _| {
                    let picked = Select::new(words.resample_method.clone(), Bind::given(state.method), &entries).show(ui);
                    if let (true, Some(method)) = (resampling, picked.value().copied()) {
                        state.method = method;
                    }
                });
                let styles = Checkbox::new(Bind::given(state.scale_styles))
                    .label(words.scale_styles.clone())
                    .id_salt("scale-styles")
                    .enabled(state.size.is_linked())
                    .show(ui);
                if let Some(on) = styles.value().copied() {
                    state.scale_styles = on;
                }
            },
            ImageSizeValues::result,
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
    open: bool,
    values: ImageSizeValues,
    build: Box<Build>,
    /// Whether OK files its answer for the host instead of closing ([`held_ok`](HostedImageSize::held_ok)).
    hold: bool,
    /// The OK the host holds back until it answers `answer_ok`.
    held: HeldAnswer<ImageSizeResult>,
    /// The values the host refused the held OK for: the refusal shows until they change.
    refused_at: Option<ImageSizeValues>,
    #[cfg(feature = "testing")]
    facts: crate::testing::StoryOutput,
}

impl HostedImageSize {
    /// The dialog's key: its [`DialogId`] is `dialog_id(KEY)`, so an app finds the open dialog by it.
    pub const KEY: &'static str = "sizing.imageSize";

    /// A dialog opening with `values`; `build` makes it over the values (`ImageSize::new` with its settings).
    pub fn new(values: ImageSizeValues, build: impl for<'b> Fn(Bind<'b, ImageSizeValues>) -> ImageSize<'b> + Send + 'static) -> Self {
        HostedImageSize {
            id: dialog_id(Self::KEY),
            open: true,
            values,
            build: Box::new(build),
            hold: false,
            held: HeldAnswer::new(),
            refused_at: None,
            #[cfg(feature = "testing")]
            facts: crate::testing::StoryOutput::default(),
        }
    }

    /// The values the dialog shows.
    pub fn values(&self) -> &ImageSizeValues {
        &self.values
    }

    /// Makes OK file its answer for the host to take instead of closing: the host checks it and answers with
    /// [`answer_ok`](Self::answer_ok). OK stays disabled until then.
    #[must_use]
    pub fn held_ok(mut self) -> Self {
        self.hold = true;
        self
    }

    /// [`held_ok`](Self::held_ok) for a dialog already open.
    pub fn hold_ok(&mut self) {
        self.hold = true;
    }

    /// The OK filed by a held dialog, once.
    pub fn take_ok(&mut self) -> Option<ImageSizeResult> {
        self.held.take()
    }

    /// The host's answer to the held OK: `Ok` closes the dialog with the held answer in the next pass; `Err` enables
    /// OK again and shows the label under the rows until a value changes. Without a held OK it does nothing.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        if answer.is_err() {
            self.refused_at = Some(self.values.clone());
        }
        self.held.answer(answer);
    }
}

impl StackDialog for HostedImageSize {
    type Result = ImageSizeResult;

    /// Answers what OK answers; `None` while OK is refused (a side out of range). A held dialog files the answer for the
    /// host, as its button does, and answers none; once the host accepted the held answer, it answers that.
    fn confirm(&mut self, _ctx: &egui::Context) -> Option<Self::Result> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if self.held.is_waiting() {
            return None;
        }
        let result = size_valid(self.values.pixels()).then(|| self.values.result())?;
        if self.hold {
            self.held.file(result);
            return None;
        }
        Some(result)
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    /// `width`, `height`, `unit` and `widthUnit` (as the size rows show them), `resolution`, `resolutionUnit`,
    /// `resample` (the method, `null` while Resample is off), `constrain` and `scaleStyles`.
    fn json_fields(&self) -> Option<serde_json::Value> {
        let values = &self.values;
        let mut fields = size_json(&values.size);
        fields.insert("resolution".into(), json!(values.size.resolution()));
        fields.insert("resolutionUnit".into(), json!(values.size.resolution_unit()));
        fields.insert("resample".into(), json!(values.size.is_resampling().then_some(values.method)));
        fields.insert("constrain".into(), json!(values.size.is_linked()));
        fields.insert("scaleStyles".into(), json!(values.scale_styles));
        Some(serde_json::Value::Object(fields))
    }

    /// Sets a field through the setter its row calls. The chain and Scale Styles refuse as inactive while their
    /// rows are disabled; `resample` takes a method the dialog lists or `null` (Resample off).
    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        let methods = (self.build)(Bind::bound(&mut self.values)).methods;
        let values = &mut self.values;
        if let Some(done) = set_size_json(&mut values.size, field, &value) {
            values.note_unit();
            return done;
        }
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        match field {
            "resolution" => {
                let number = value.as_f64().ok_or_else(rejects)?;
                values.size.set_resolution(number);
            }
            "resolutionUnit" => {
                let kind = serde_json::from_value::<UnitKind>(value.clone()).map_err(|_| rejects())?;
                if !values.size.set_resolution_unit(kind) {
                    return Err(rejects());
                }
            }
            "resample" if value.is_null() => values.size.resampling(false),
            "resample" => {
                let method = serde_json::from_value::<Resample>(value.clone()).map_err(|_| rejects())?;
                if !methods.is_empty() && !methods.contains(&method) {
                    return Err(rejects());
                }
                values.size.resampling(true);
                values.method = method;
            }
            "constrain" => {
                let on = value.as_bool().ok_or_else(rejects)?;
                if !values.size.is_resampling() {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                values.size.lock(on);
            }
            "scaleStyles" => {
                let on = value.as_bool().ok_or_else(rejects)?;
                if !values.size.is_linked() {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                values.scale_styles = on;
            }
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ImageSizeResult>> {
        let finish = self.held.is_accepted();
        let hold = Hold {
            on: self.hold,
            finish,
            waiting: self.held.is_waiting(),
            refusal: self.held.refusal().zip(self.refused_at.as_ref()).map(|((reason, _), at)| (reason.clone(), at.clone())),
        };
        let mut shown = (self.build)(Bind::bound(&mut self.values)).hold_ok(hold).opened(Bind::bound(&mut self.open)).show(ui);
        #[cfg(feature = "testing")]
        {
            self.facts = crate::contract::Outcome::story_facts(&shown);
        }
        if let Some(result) = shown.held.take() {
            self.held.file(result);
        }
        if self.refused_at.as_ref().is_some_and(|at| *at != self.values) {
            self.refused_at = None;
            self.held.clear_refusal();
        }
        // The host accepted the held answer: the pass that closes the dialog answers it.
        if finish && shown.result.is_none() && matches!(shown.closed, Some(CloseReason::Activated)) {
            return Some(self.held.finish().map_or(DialogResult::Cancel, DialogResult::Ok));
        }
        shown.hosted()
    }

    fn key(&self) -> Option<&str> {
        Some(Self::KEY)
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        self.facts.clone()
    }
}

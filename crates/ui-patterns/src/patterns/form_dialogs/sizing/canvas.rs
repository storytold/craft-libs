//! `CanvasSize`: the Canvas Size dialog.

use super::{CanvasExtension, Kit, LinkedSize, SizeForm, SizingOutput, set_size_json, show_form, size_json, size_valid};
use crate::choice::Checkbox;
use crate::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use crate::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use crate::display::Text;
use crate::form::Field;
use crate::pickers::{Anchor9, ReferencePoint, Select};
use crate::runtime::env::{ColumnSize, Label, UnitKind};
use crate::sections::Section;
use craft_behavior::PixelSize;
use craft_theme::variant::Tone;
use egui::Ui;
use serde_json::json;

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
    /// The document's column size (the Units preference's column width and gutter): the Width row offers Columns,
    /// converted through it as in Image Size. Without one the row offers no Columns.
    #[must_use]
    pub fn column_size(mut self, size: ColumnSize) -> Self {
        self.size = self.size.column_size(size);
        self
    }

    /// What OK answers with for these values.
    fn result(&self) -> CanvasSizeResult {
        CanvasSizeResult {
            size: self.size.pixels(),
            unit: self.size.unit(),
            relative: self.size.is_relative(),
            anchor: self.anchor,
            extension: self.fill(),
        }
    }

    /// The extension the canvas fills with: Transparent for a document with no background, whatever was set.
    fn fill(&self) -> CanvasExtension {
        if self.has_background { self.extension.clone() } else { CanvasExtension::Transparent }
    }

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
    /// Files OK's answer in `held` for the host instead of closing; the host answers it.
    #[prop(vis = "pub(crate)")]
    held: Option<&'a mut HeldAnswer<CanvasSizeValues>>,
}

impl<'a> CanvasSize<'a> {
    /// The dialog over `values`.
    pub fn new(values: Bind<'a, CanvasSizeValues>) -> Self {
        CanvasSize { values, open: None, bytes_per_pixel: 4.0, held: None }
    }

    /// Shows the dialog.
    pub fn show(self, ui: &mut Ui) -> CanvasSizeOutput {
        let CanvasSize { values, open, bytes_per_pixel, held } = self;
        let kit = Kit::for_canvas(ui);
        let words = kit.words();

        let title = words.canvas_size_title.clone();
        show_form(
            ui,
            title,
            open,
            values,
            held,
            |ui, state| {
                let size = state.size.original();
                let has_background = state.has_background;
                state.extension = state.fill();
                sized_section(ui, "current", &words.current_size, kit.file_size(size, bytes_per_pixel), None, |ui| {
                    kit.readout(ui, &words.width, kit.pixels(size.width), None);
                    kit.readout(ui, &words.height, kit.pixels(size.height), None);
                });
                let now = state.size.pixels();
                sized_section(ui, "new", &words.new_size, kit.file_size(now, bytes_per_pixel), kit.tone_of(now), |ui| {
                    kit.refusal(ui, now);
                    kit.sides(ui, &mut state.size);
                    let check = Checkbox::new(Bind::given(state.size.is_relative())).label(words.relative.clone()).id_salt("relative").show(ui);
                    if let Some(on) = check.value().copied() {
                        state.size.relative(on);
                    }
                    let growth = (f64::from(now.width) - f64::from(size.width), f64::from(now.height) - f64::from(size.height));
                    Field::new(words.anchor.clone()).id_salt("anchor").show(ui, |ui, _| {
                        let picked = ReferencePoint::new(words.anchor.clone(), Bind::given(Some(state.anchor))).growth(growth).show(ui);
                        if let Some(anchor) = picked.value().copied().flatten() {
                            state.anchor = anchor;
                        }
                    });
                });
                Field::new(words.extension.clone()).id_salt("extension").enabled(has_background).show(ui, |ui, _| {
                    let entries = CanvasExtension::entries(words, &state.extension, has_background);
                    let picked = Select::new(words.extension.clone(), Bind::given(state.extension.clone()), &entries).show(ui);
                    if let (true, Some(extension)) = (has_background, picked.value().cloned()) {
                        state.extension = extension;
                    }
                });
            },
            CanvasSizeValues::result,
        )
    }
}

/// A section titled `title` with the file size `size` at the end of its header, in `tone` when given.
fn sized_section(ui: &mut Ui, salt: &str, title: &Label, size: String, tone: Option<Tone>, mut body: impl FnMut(&mut Ui)) {
    Section::fixed(title.clone())
        .id_salt(salt)
        .controls(move |ui, _| {
            let text = Text::new(size.clone());
            match tone {
                Some(tone) => text.tone(tone).show(ui),
                None => text.show(ui),
            };
        })
        .show(ui, |ui, _| body(ui));
}

/// Builds the dialog over the host-owned values of a [`HostedCanvasSize`] for one pass.
type Build = dyn for<'b> Fn(Bind<'b, CanvasSizeValues>) -> CanvasSize<'b> + Send;

/// A hosted Canvas Size: the `DialogHost` owns the values, and `build` makes the dialog over them each pass. The app
/// opens it with the values of its document and takes the [`CanvasSizeResult`] from the host.
///
/// ```ignore
/// host.open(HostedCanvasSize::new(CanvasSizeValues::new(document_size), |values| CanvasSize::new(values)));
/// if let Some(DialogResult::Ok(result)) = host.take::<HostedCanvasSize>() {
///     resize_canvas(result);
/// }
/// ```
pub struct HostedCanvasSize {
    id: DialogId,
    open: bool,
    values: CanvasSizeValues,
    build: Box<Build>,
    /// Whether OK files its answer for the host instead of closing ([`held_ok`](HostedCanvasSize::held_ok)).
    hold: bool,
    /// The OK the host holds back until it answers `answer_ok`.
    held: HeldAnswer<CanvasSizeValues>,
    #[cfg(feature = "testing")]
    facts: crate::testing::StoryOutput,
}

impl HostedCanvasSize {
    /// The dialog's key: its [`DialogId`] is `dialog_id(KEY)`, so an app finds the open dialog by it.
    pub const KEY: &'static str = "sizing.canvasSize";

    /// A dialog opening with `values`; `build` makes it over the values (`CanvasSize::new` with its settings).
    pub fn new(values: CanvasSizeValues, build: impl for<'b> Fn(Bind<'b, CanvasSizeValues>) -> CanvasSize<'b> + Send + 'static) -> Self {
        HostedCanvasSize {
            id: dialog_id(Self::KEY),
            open: true,
            values,
            build: Box::new(build),
            hold: false,
            held: HeldAnswer::new(),
            #[cfg(feature = "testing")]
            facts: crate::testing::StoryOutput::default(),
        }
    }

    /// The values the dialog shows.
    pub fn values(&self) -> &CanvasSizeValues {
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
    pub fn take_ok(&mut self) -> Option<CanvasSizeResult> {
        self.held.take().map(|values| values.result())
    }

    /// The host's answer to the held OK: `Ok` closes the dialog with the held answer in the next pass; `Err` enables
    /// OK again and shows the label under the rows until a value changes. Without a held OK it does nothing.
    pub fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }
}

impl StackDialog for HostedCanvasSize {
    type Result = CanvasSizeResult;

    /// Answers what OK answers; `None` while OK is refused (a side out of range). A held dialog files the answer for the
    /// host, as its button does, and answers none; once the host accepted the held answer, it answers that.
    fn confirm(&mut self, _ctx: &egui::Context) -> Option<Self::Result> {
        if self.held.is_accepted() {
            return self.held.finish().map(|values| values.result());
        }
        if self.held.is_waiting() {
            return None;
        }
        if !size_valid(self.values.pixels()) {
            return None;
        }
        if self.hold {
            self.held.file(self.values.clone());
            return None;
        }
        Some(self.values.result())
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    /// `width`, `height`, `unit` and `widthUnit` (as the size rows show them), `relative`, `anchor` and `extension`.
    fn json_fields(&self) -> Option<serde_json::Value> {
        let values = &self.values;
        let mut fields = size_json(&values.size);
        fields.insert("relative".into(), json!(values.size.is_relative()));
        fields.insert("anchor".into(), json!(values.anchor));
        fields.insert("extension".into(), json!(values.fill()));
        Some(serde_json::Value::Object(fields))
    }

    /// Sets a field through the setter its row calls. `extension` takes one the list offers, and refuses as
    /// inactive for a document with no background (the list is off).
    fn set_json_field(&mut self, _ctx: &egui::Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        let values = &mut self.values;
        if let Some(done) = set_size_json(&mut values.size, field, &value) {
            return done;
        }
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        match field {
            "relative" => values.size.relative(value.as_bool().ok_or_else(rejects)?),
            "anchor" => values.anchor = serde_json::from_value::<Anchor9>(value.clone()).map_err(|_| rejects())?,
            "extension" => {
                if !values.has_background {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                let extension = serde_json::from_value::<CanvasExtension>(value.clone()).map_err(|_| rejects())?;
                if !CanvasExtension::offered(&values.extension, true).contains(&extension) {
                    return Err(rejects());
                }
                values.extension = extension;
            }
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<CanvasSizeResult>> {
        let built = (self.build)(Bind::bound(&mut self.values));
        let built = match self.hold.then_some(&mut self.held) {
            Some(held) => built.held(held),
            None => built,
        };
        let shown = built.opened(Bind::bound(&mut self.open)).show(ui);
        #[cfg(feature = "testing")]
        {
            self.facts = crate::contract::Outcome::story_facts(&shown);
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

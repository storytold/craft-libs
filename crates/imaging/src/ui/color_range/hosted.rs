//! [`HostedColorRange`]: Color Range on the dialog host.

use std::sync::Arc;

use craft_ui::canvas::RasterSource;
use craft_ui::contract::Bind;
use craft_ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id, serde_fields, set_serde_field};
use craft_ui::patterns::form_dialogs::FormDialog;
use craft_ui::runtime::env::Label;
use egui::{Context, Ui};
use serde_json::{Value, json};

use super::{ColorRangeDialog, ColorRangeSelect, ColorRangeValue, SampleTool};
use crate::selection::PreviewMode;

/// The key of the hosted Color Range dialog.
const KEY: &str = "select.colorRange";

/// The preview the app rendered, as the dialog host keeps it.
type Preview = Arc<dyn RasterSource + Send + Sync>;

/// Color Range as a dialog the dialog host owns. It is not registered: the app opens it. It answers with the value at OK.
///
/// The sample tool, the Selection / Image view and the Selection Preview mode are dialog state, not part of the
/// value: they start at the eyedropper, Selection and None at each open, and automation reaches them as the `tool`,
/// `view` and `selectionPreview` fields beside the value's own.
pub struct HostedColorRange {
    id: DialogId,
    title: Label,
    open: bool,
    value: ColorRangeValue,
    size: [u32; 2],
    tool: SampleTool,
    show_image: bool,
    preview: PreviewMode,
    canvas: Option<egui::Id>,
    mask: Option<Preview>,
    image: Option<Preview>,
    picture_scale: u32,
    error: Option<Label>,
}

impl HostedColorRange {
    /// The dialog titled `title` over `value`, for a document of `size` pixels.
    pub fn new(title: impl Into<Label>, value: ColorRangeValue, size: [u32; 2]) -> Self {
        HostedColorRange {
            id: dialog_id(KEY),
            title: title.into(),
            open: true,
            value,
            size,
            tool: SampleTool::default(),
            show_image: false,
            preview: PreviewMode::default(),
            canvas: None,
            mask: None,
            image: None,
            picture_scale: 1,
            error: None,
        }
    }

    /// The canvas the sampling presses reach.
    #[must_use]
    pub fn canvas(mut self, canvas: egui::Id) -> Self {
        self.canvas = Some(canvas);
        self
    }

    /// The value the dialog shows.
    pub fn value(&self) -> &ColorRangeValue {
        &self.value
    }

    /// The sample tool chosen in the dialog, before Shift or Alt change it.
    pub fn tool(&self) -> SampleTool {
        self.tool
    }

    /// Sets the previews the app rendered: the selection `mask` and the `image`, shown at `picture_scale`, or the
    /// `error` that kept them from rendering.
    pub fn set_preview(&mut self, mask: Option<Preview>, image: Option<Preview>, picture_scale: u32, error: Option<Label>) {
        self.mask = mask;
        self.image = image;
        self.picture_scale = picture_scale;
        self.error = error;
    }

    /// The Selection Preview mode the dialog shows on the canvas.
    pub fn preview_mode(&self) -> PreviewMode {
        self.preview
    }

    /// Whether the dialog's view shows the image rather than the selection.
    pub fn shows_image(&self) -> bool {
        self.show_image
    }

    /// A press on the canvas at `at` with `mods` held samples a colour. `false` for a press outside the document or
    /// while the select takes no samples.
    pub fn sample_canvas(&mut self, at: [f64; 2], mods: craft_keymap::Mods) -> bool {
        let inside = |v: f64, extent: u32| v.is_finite() && v >= 0.0 && v < f64::from(extent);
        if !self.value.controls().sampling || !inside(at[0], self.size[0]) || !inside(at[1], self.size[1]) {
            return false;
        }
        self.value.samples.pick(at, self.size, self.tool, mods);
        true
    }

    /// Whether the row of `field` is shown and enabled for the select.
    fn takes(&self, field: &str) -> bool {
        let c = self.value.controls();
        match field {
            "localized" | "samples" | "tool" => c.sampling,
            "fuzziness" => c.fuzziness && !c.tonal,
            "toneFuzziness" => c.tonal,
            "range" => c.range,
            "shadowsLevel" => self.value.select == ColorRangeSelect::Shadows,
            "highlightsLevel" => self.value.select == ColorRangeSelect::Highlights,
            "midtonesLow" | "midtonesHigh" => self.value.select == ColorRangeSelect::Midtones,
            _ => true,
        }
    }
}

impl StackDialog for HostedColorRange {
    type Result = ColorRangeValue;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<ColorRangeValue>> {
        let HostedColorRange { title, open, value, size, tool, show_image, preview, canvas, mask, image, picture_scale, error, .. } = self;
        let shown = FormDialog::new(title.clone(), Bind::bound(value)).opened(Bind::bound(open)).show(ui, |ui, _, value| {
            let mut body = ColorRangeDialog::new(title.clone(), Bind::bound(value))
                .picture_scale(*picture_scale)
                .document(*size)
                .tool(*tool)
                .show_image(*show_image)
                .preview_mode(*preview);
            if let Some(mask) = mask.as_deref() {
                body = body.mask(mask);
            }
            if let Some(image) = image.as_deref() {
                body = body.image(image);
            }
            if let Some(error) = error {
                body = body.error(error.clone());
            }
            if let Some(canvas) = *canvas {
                body = body.canvas(canvas);
            }
            let out = body.show(ui);
            if let Some(next) = out.tool {
                *tool = next;
            }
            if let Some(next) = out.show_image {
                *show_image = next;
            }
            if let Some(next) = out.preview {
                *preview = next;
            }
            true
        });
        let accepted = shown.accepted.map(DialogResult::Ok);
        accepted.or(shown.closed.map(|_| DialogResult::Cancel))
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn confirm(&mut self, ctx: &Context) -> Option<ColorRangeValue> {
        let _ = ctx;
        Some(self.value.clone())
    }

    fn json_fields(&self) -> Option<Value> {
        let mut fields = serde_fields(&self.value)?;
        let map = fields.as_object_mut()?;
        map.insert("tool".into(), serde_json::to_value(self.tool).ok()?);
        map.insert("view".into(), json!(if self.show_image { "image" } else { "selection" }));
        map.insert("selectionPreview".into(), serde_json::to_value(self.preview).ok()?);
        Some(fields)
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        let _ = ctx;
        if !self.takes(field) {
            return Err(FieldRefusal::Inactive.message(field));
        }
        let rejected = |value: &Value| FieldRefusal::Rejects(value).message(field);
        match field {
            "tool" => self.tool = serde_json::from_value(value.clone()).map_err(|_| rejected(&value))?,
            "view" => {
                self.show_image = match value.as_str() {
                    Some("image") => true,
                    Some("selection") => false,
                    _ => return Err(rejected(&value)),
                }
            }
            "selectionPreview" => self.preview = serde_json::from_value(value.clone()).map_err(|_| rejected(&value))?,
            _ => return set_serde_field(&mut self.value, field, value),
        }
        Ok(())
    }
}

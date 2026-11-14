//! [`HostedColorRange`]: Color Range on the dialog host.

use craft_ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft_ui::runtime::env::Label;
use egui::{Context, Ui};

use super::ColorRangeValue;

/// The key of the hosted Color Range dialog.
const KEY: &str = "select.colorRange";

/// Color Range as a dialog the dialog host owns. It is not registered: the app opens it. It answers with the value at OK.
pub struct HostedColorRange {
    id: DialogId,
    value: ColorRangeValue,
}

impl HostedColorRange {
    /// The dialog titled `title` over `value`, for a document of `size` pixels.
    pub fn new(title: impl Into<Label>, value: ColorRangeValue, size: [u32; 2]) -> Self {
        let _ = (title.into(), size);
        HostedColorRange { id: dialog_id(KEY), value }
    }

    /// The canvas the sampling presses reach.
    #[must_use]
    pub fn canvas(self, canvas: egui::Id) -> Self {
        let _ = canvas;
        todo!("dl.K54")
    }

    /// The value the dialog shows.
    pub fn value(&self) -> &ColorRangeValue {
        &self.value
    }

    /// Sets the previews the app rendered: the selection `mask` and the `image`, shown at `picture_scale`, or the
    /// `error` that kept them from rendering.
    // P3(dialogs-w9): dl.K54 `set_preview` takes the four arguments
    pub fn set_preview(
        &mut self,
        mask: Option<std::sync::Arc<dyn craft_ui::canvas::RasterSource + Send + Sync>>,
        image: Option<std::sync::Arc<dyn craft_ui::canvas::RasterSource + Send + Sync>>,
        picture_scale: u32,
        error: Option<Label>,
    ) {
        let _ = (mask, image, picture_scale, error);
        todo!("dl.K54")
    }

    /// The Selection Preview mode the dialog shows on the canvas.
    // P3(dialogs-w9): dl.K54 `preview_mode`
    pub fn preview_mode(&self) -> crate::selection::PreviewMode {
        todo!("dl.K54")
    }

    /// Whether the dialog's view shows the image rather than the selection.
    // P3(dialogs-w9): dl.K54 `shows_image`
    pub fn shows_image(&self) -> bool {
        todo!("dl.K54")
    }

    /// A press on the canvas at `at` with `mods` held samples a colour. `false` for a press outside the document.
    pub fn sample_canvas(&mut self, at: [f64; 2], mods: craft_keymap::Mods) -> bool {
        let _ = (at, mods);
        todo!("dl.K54")
    }
}

impl StackDialog for HostedColorRange {
    type Result = ColorRangeValue;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, stack: &DialogStack) -> Option<DialogResult<ColorRangeValue>> {
        let _ = (ui, stack);
        todo!("dl.K54")
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn confirm(&mut self, ctx: &Context) -> Option<ColorRangeValue> {
        let _ = ctx;
        todo!("dl.K54")
    }

    fn json_fields(&self) -> Option<serde_json::Value> {
        todo!("dl.K54")
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: serde_json::Value) -> Result<(), String> {
        let _ = (ctx, field, value);
        todo!("dl.K54")
    }
}

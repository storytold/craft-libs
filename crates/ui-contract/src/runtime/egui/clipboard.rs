//! Clipboard output commands.

use egui::{ColorImage, Context};

/// Puts `text` on the system clipboard.
pub(crate) fn copy_text(ctx: &Context, text: String) {
    ctx.copy_text(text);
}

/// Puts `image` on the system clipboard.
pub(crate) fn copy_image(ctx: &Context, image: ColorImage) {
    ctx.copy_image(image);
}

/// Asks the platform for the clipboard text, which arrives as a paste event.
pub(crate) fn request_paste(ctx: &Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
}

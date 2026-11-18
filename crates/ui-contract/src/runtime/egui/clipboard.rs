//! Clipboard output commands.

use craft_platform::clipboard::{Clipboard, ClipboardBackend, ClipboardError, ClipboardImage, Flavor};
use egui::{ColorImage, Context};
use std::path::PathBuf;

/// The backend the runtime installs when the host gives no clipboard: egui's own copy and paste commands.
#[allow(dead_code)] // P3(platform-w3): pf.C-R2
pub(crate) struct EguiClipboard;

impl ClipboardBackend for EguiClipboard {
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        let _ = flavors;
        todo!("pf.C-R2")
    }

    /// egui delivers pasted text only as an input event, so a read finds nothing.
    fn read(&mut self, mimes: &[&str], limit: u64) -> Result<Option<Flavor>, ClipboardError> {
        let _ = (mimes, limit);
        Ok(None)
    }

    fn read_image(&mut self, limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        let _ = limit;
        todo!("pf.C-R2")
    }

    fn read_files(&mut self, limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        let _ = limit;
        todo!("pf.C-R2")
    }

    fn has(&mut self, mimes: &[&str]) -> bool {
        let _ = mimes;
        todo!("pf.C-R2")
    }
}

/// Runs `f` on the clipboard installed in `ctx`'s runtime memory.
#[allow(dead_code)] // P3(platform-w3): pf.C-R2
pub(crate) fn with<R>(ctx: &Context, f: impl FnOnce(&mut Clipboard) -> R) -> R {
    let _ = (ctx, f);
    todo!("pf.C-R2")
}

/// Once per frame, in the pass that receives input: takes a late paste offer for this frame.
#[allow(dead_code)] // P3(platform-w3): pf.C-R2
pub(crate) fn begin_frame(ctx: &Context) {
    let _ = ctx;
    todo!("pf.C-R2")
}

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

//! The installed clipboard, its egui backend, and the once-per-frame late paste flag.

use crate::runtime::pass_state;
use craft_platform::clipboard::{Clipboard, ClipboardBackend, ClipboardError, ClipboardImage, Flavor, RGBA8, TEXT};
use egui::{ColorImage, Context};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

/// The backend the runtime installs when the host gives no clipboard: egui's own copy and paste commands. egui
/// carries one value of text or of an image, and the last copy command of a frame wins, so a write acts on the first
/// flavour egui carries and skips the rest. It holds no `Context` (the context owns the clipboard, so a clone would
/// be a cycle): a write is left in a slot that `with` hands to the context's output.
pub(crate) struct EguiClipboard {
    copies: Copies,
}

/// The copy command an `EguiClipboard` write leaves for `with`.
enum EguiCopy {
    Text(String),
    Image(ColorImage),
}

/// Where an `EguiClipboard` leaves its write until `with` sends it to egui.
type Copies = Arc<Mutex<Option<EguiCopy>>>;

impl EguiClipboard {
    /// The backend that leaves its writes in `copies`.
    fn new(copies: Copies) -> Self {
        EguiClipboard { copies }
    }

    fn leave(&self, copy: EguiCopy) {
        *self.copies.lock().unwrap_or_else(PoisonError::into_inner) = Some(copy);
    }
}

impl ClipboardBackend for EguiClipboard {
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        if flavors.is_empty() {
            self.leave(EguiCopy::Text(String::new()));
            return Ok(0);
        }
        for (at, flavor) in flavors.iter().enumerate() {
            if flavor.mime == TEXT
                && let Ok(text) = std::str::from_utf8(&flavor.data)
            {
                self.leave(EguiCopy::Text(text.to_owned()));
                return Ok(at);
            }
            if flavor.mime == RGBA8
                && let Some(image) = flavor.rgba8_image().and_then(|image| color_image(&image))
            {
                self.leave(EguiCopy::Image(image));
                return Ok(at);
            }
        }
        Err(ClipboardError::NoFormat)
    }

    /// egui delivers pasted text only as an input event, so a read finds nothing.
    fn read(&mut self, mimes: &[&str], limit: u64) -> Result<Option<Flavor>, ClipboardError> {
        let _ = (mimes, limit);
        Ok(None)
    }

    /// egui has no clipboard read.
    fn read_image(&mut self, limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        let _ = limit;
        Ok(None)
    }

    /// egui has no clipboard read.
    fn read_files(&mut self, limit: usize) -> Result<Option<Vec<PathBuf>>, ClipboardError> {
        let _ = limit;
        Ok(None)
    }

    /// egui cannot look at the clipboard, so it holds nothing it could name.
    fn has(&mut self, mimes: &[&str]) -> bool {
        let _ = mimes;
        false
    }
}

/// The clipboard of a context and whether a late paste arrived this frame. The clipboard is an install-time handle,
/// shared so `with` can lend it after the pass-state borrow ends.
#[derive(Default)]
pub(crate) struct Installed {
    clipboard: Option<Arc<Mutex<Clipboard>>>,
    /// What the egui backend wrote, until `with` sends it to the context.
    copies: Copies,
    /// Assigned once per frame by `begin_frame`, so it is a fact of that frame alone.
    arrived: bool,
}

/// Keeps `clipboard` as the context's clipboard; `None` leaves the choice to `with`, which builds egui's.
pub(crate) fn install(ctx: &Context, clipboard: Option<Clipboard>) {
    pass_state::with::<Installed, _>(ctx, |installed| {
        installed.clipboard = clipboard.map(|clipboard| Arc::new(Mutex::new(clipboard)));
        installed.arrived = false;
    });
}

/// Runs `f` on the clipboard installed in `ctx`'s runtime memory: the host's, else egui's, built here once.
pub(crate) fn with<R>(ctx: &Context, f: impl FnOnce(&mut Clipboard) -> R) -> R {
    let (shared, copies) = pass_state::with::<Installed, _>(ctx, |installed| {
        let copies = Arc::clone(&installed.copies);
        let clipboard = installed.clipboard.get_or_insert_with(|| Arc::new(Mutex::new(Clipboard::new(EguiClipboard::new(Arc::clone(&copies))))));
        (Arc::clone(clipboard), copies)
    });
    let result = {
        let mut clipboard = shared.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut clipboard)
    };
    let copy = copies.lock().unwrap_or_else(PoisonError::into_inner).take();
    match copy {
        Some(EguiCopy::Text(text)) => ctx.copy_text(text),
        Some(EguiCopy::Image(image)) => ctx.copy_image(image),
        None => {}
    }
    result
}

/// Once per frame, from `set_input`: takes a late paste offer for this frame. The flag is assigned, never or-ed, so it
/// holds in every pass of this frame (passes a frame reruns do not call `set_input`) and goes with the next frame.
pub(crate) fn begin_frame(ctx: &Context) {
    let arrived = with(ctx, Clipboard::take_arrived_paste);
    pass_state::with::<Installed, _>(ctx, |installed| installed.arrived = arrived);
}

/// Whether a late paste offer arrived this frame.
pub(crate) fn arrived(ctx: &Context) -> bool {
    pass_state::with::<Installed, _>(ctx, |installed| installed.arrived)
}

/// `image` as straight-alpha RGBA8 pixels, the form every clipboard flavour of a bitmap carries. A side too long for
/// a `u32` gives an image that is not whole, which a write refuses.
pub(crate) fn clipboard_image(image: &ColorImage) -> ClipboardImage {
    let [width, height] = image.size;
    ClipboardImage {
        width: u32::try_from(width).unwrap_or(0),
        height: u32::try_from(height).unwrap_or(0),
        rgba: image.pixels.iter().flat_map(|pixel| pixel.to_srgba_unmultiplied()).collect(),
    }
}

/// The pixels of `image` for egui, `None` when its pixels do not match its size.
fn color_image(image: &ClipboardImage) -> Option<ColorImage> {
    if !image.is_whole() {
        return None;
    }
    let size = [usize::try_from(image.width).ok()?, usize::try_from(image.height).ok()?];
    Some(ColorImage::from_rgba_unmultiplied(size, &image.rgba))
}

/// Puts `text` on the installed clipboard.
pub(crate) fn copy_text(ctx: &Context, text: String) {
    with(ctx, |clipboard| {
        let _ = clipboard.write_text(&text);
    });
}

/// Asks the platform for the clipboard text, which arrives as a paste event.
pub(crate) fn request_paste(ctx: &Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
}

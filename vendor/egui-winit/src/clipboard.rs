use raw_window_handle::RawDisplayHandle;

/// Handles interfacing with the OS clipboard.
///
/// If the "clipboard" feature is off, or we cannot connect to the OS clipboard,
/// then a fallback clipboard that just works within the same app is used instead.
pub struct Clipboard {
    #[cfg(all(target_os = "linux", feature = "wayland", feature = "clipboard"))]
    native: Option<craft_clipboard::Clipboard>,
    #[cfg(all(target_os = "linux", feature = "wayland", feature = "clipboard"))]
    native_bound: bool,

    #[cfg(all(not(any(target_os = "android", target_os = "ios")), feature = "arboard",))]
    arboard: Option<arboard::Clipboard>,

    #[cfg(all(
        any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd"),
        feature = "smithay-clipboard"
    ))]
    smithay: Option<smithay_clipboard::Clipboard>,

    /// Fallback manual clipboard.
    clipboard: String,
}

impl Clipboard {
    /// Construct a new instance
    pub fn new(_raw_display_handle: Option<RawDisplayHandle>) -> Self {
        let _use_native = false;
        #[cfg(all(target_os = "linux", feature = "wayland", feature = "clipboard"))]
        let _use_native = matches!(_raw_display_handle, Some(RawDisplayHandle::Wayland(_)));
        Self {
            #[cfg(all(target_os = "linux", feature = "wayland", feature = "clipboard"))]
            native: None,
            #[cfg(all(target_os = "linux", feature = "wayland", feature = "clipboard"))]
            native_bound: false,
            #[cfg(all(not(any(target_os = "android", target_os = "ios")), feature = "arboard",))]
            arboard: if _use_native { None } else { init_arboard() },

            #[cfg(all(
                any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd"),
                feature = "smithay-clipboard"
            ))]
            smithay: if _use_native { None } else { init_smithay_clipboard(_raw_display_handle) },

            clipboard: Default::default(),
        }
    }

    pub fn bind_window(&mut self, _window: &winit::window::Window) {
        #[cfg(all(target_os = "linux", feature = "wayland", feature = "clipboard"))]
        if !self.native_bound {
            self.native_bound = true;
            match craft_clipboard::Clipboard::from_window(_window) {
                Ok(clipboard) => {
                    if clipboard.is_some() {
                        #[cfg(feature = "smithay-clipboard")]
                        {
                            self.smithay = None;
                        }
                        #[cfg(feature = "arboard")]
                        {
                            self.arboard = None;
                        }
                    }
                    self.native = clipboard;
                }
                Err(error) => log::warn!("Native Wayland clipboard: {error}"),
            }
        }
    }

    pub fn get(&mut self) -> Option<String> {
        #[cfg(all(target_os = "linux", feature = "wayland", feature = "clipboard"))]
        if let Some(clipboard) = &self.native {
            return clipboard.text();
        }

        #[cfg(all(
            any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd"),
            feature = "smithay-clipboard"
        ))]
        if let Some(clipboard) = &mut self.smithay {
            match clipboard.load() {
                Ok(text) => return Some(text),
                Err(err) => {
                    log::error!("smithay paste error: {err}");
                }
            }
        }

        #[cfg(all(not(any(target_os = "android", target_os = "ios")), feature = "arboard",))]
        if let Some(clipboard) = &mut self.arboard {
            return match clipboard.get_text() {
                Ok(text) => Some(text),
                Err(err) => {
                    log::error!("arboard paste error: {err}");
                    None
                }
            };
        }

        Some(self.clipboard.clone())
    }

    pub fn set_text(&mut self, text: String) {
        #[cfg(all(target_os = "linux", feature = "wayland", feature = "clipboard"))]
        if let Some(clipboard) = &self.native {
            if let Err(error) = clipboard.write_text(&text) {
                log::warn!("Native Wayland copy: {error}");
            }
            return;
        }

        #[cfg(all(
            any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd"),
            feature = "smithay-clipboard"
        ))]
        if let Some(clipboard) = &mut self.smithay {
            clipboard.store(text);
            return;
        }

        #[cfg(all(not(any(target_os = "android", target_os = "ios")), feature = "arboard",))]
        if let Some(clipboard) = &mut self.arboard {
            if let Err(err) = clipboard.set_text(text) {
                log::error!("arboard copy/cut error: {err}");
            }
            return;
        }

        self.clipboard = text;
    }

    pub fn set_image(&mut self, image: &egui::ColorImage) {
        #[cfg(all(target_os = "linux", feature = "wayland", feature = "clipboard"))]
        if let Some(clipboard) = &self.native {
            match native_png(image).and_then(|bytes| clipboard.write(vec![("image/png".into(), bytes)])) {
                Ok(()) => {}
                Err(error) => log::warn!("Native Wayland image copy: {error}"),
            }
            return;
        }

        #[cfg(all(not(any(target_os = "android", target_os = "ios")), feature = "arboard",))]
        if let Some(clipboard) = &mut self.arboard {
            if let Err(err) = clipboard.set_image(arboard::ImageData {
                width: image.width(),
                height: image.height(),
                bytes: std::borrow::Cow::Borrowed(bytemuck::cast_slice(&image.pixels)),
            }) {
                log::error!("arboard copy/cut error: {err}");
            }
            log::debug!("Copied image to clipboard");
            return;
        }

        log::error!("Copying images is not supported. Enable the 'clipboard' feature of `egui-winit` to enable it.");
        _ = image;
    }
}

#[cfg(all(not(any(target_os = "android", target_os = "ios")), feature = "arboard",))]
fn init_arboard() -> Option<arboard::Clipboard> {
    profiling::function_scope!();

    log::trace!("Initializing arboard clipboard…");
    match arboard::Clipboard::new() {
        Ok(clipboard) => Some(clipboard),
        Err(err) => {
            log::warn!("Failed to initialize arboard clipboard: {err}");
            None
        }
    }
}

#[cfg(all(
    any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "netbsd", target_os = "openbsd"),
    feature = "smithay-clipboard"
))]
fn init_smithay_clipboard(raw_display_handle: Option<RawDisplayHandle>) -> Option<smithay_clipboard::Clipboard> {
    #![expect(clippy::undocumented_unsafe_blocks)]

    profiling::function_scope!();

    if let Some(RawDisplayHandle::Wayland(display)) = raw_display_handle {
        log::trace!("Initializing smithay clipboard…");
        #[expect(unsafe_code)]
        Some(unsafe { smithay_clipboard::Clipboard::new(display.display.as_ptr()) })
    } else {
        #[cfg(feature = "wayland")]
        log::debug!("Cannot init smithay clipboard without a Wayland display handle");
        #[cfg(not(feature = "wayland"))]
        log::debug!("Cannot init smithay clipboard: the 'wayland' feature of 'egui-winit' is not enabled");
        None
    }
}

#[cfg(all(target_os = "linux", feature = "wayland", feature = "clipboard"))]
fn native_png(image: &egui::ColorImage) -> Result<Vec<u8>, String> {
    if image.pixels.len().checked_mul(4).is_none_or(|bytes| bytes > 64 << 20) {
        return Err("Clipboard image exceeds 64 MiB".into());
    }
    let width = u32::try_from(image.width()).map_err(|error| error.to_string())?;
    let height = u32::try_from(image.height()).map_err(|error| error.to_string())?;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|pixel| pixel.to_srgba_unmultiplied()).collect();
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer.write_image_data(&rgba).map_err(|error| error.to_string())?;
    }
    Ok(bytes)
}

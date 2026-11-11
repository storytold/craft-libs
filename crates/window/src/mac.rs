//! The macOS traffic lights, measured, so an app-drawn title bar fits around them.
//!
//! AppKit places the lights; their size and position change between macOS versions (they grew
//! in macOS 26). [`measure`] reads the zoom button of a titled window with a full-size content
//! view and gives the [`MacInset`] the title bar reserves. [`Cache`] keeps the result: the AppKit
//! calls run again only after leaving full screen or when the scale changes. Before a window
//! exists [`os_default`] gives [`default_inset`] for the running macOS version.
//!
//! # Checklist on a Mac
//!
//! Run an app that reserves this inset on macOS 26 and 27, one window with the
//! app-drawn bar, and compare against the real lights:
//!
//! - The reserved leading space ends one gap after the green button; the bar's content centres
//!   on the lights' centre line.
//! - A second window of the same app gets the same inset.
//! - Full screen: the inset is `None` and the bar's content starts at the left edge. Leaving
//!   full screen restores the inset with no frame at the default.
//! - Dragging the window between a Retina and a non-Retina display re-measures; the inset stays
//!   equal to the lights.

use crate::policy::MacInset;

/// The three buttons as AppKit laid them out, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lights {
    /// The zoom (rightmost) button's right edge from the window's left edge.
    pub right: f32,
    /// The lights' vertical centre below the window's top.
    pub centre_from_top: f32,
    /// A button's diameter.
    pub size: f32,
}

/// Largest plausible distance of the lights' centre from the window top, in points.
const MAX_CENTRE_FROM_TOP: f32 = 40.0;
/// Largest plausible distance of the zoom button's right edge from the window's left, in points.
const MAX_RIGHT: f32 = 200.0;

/// The inset for measured lights: the lights are centred in a band `2 * centre_from_top` tall,
/// and content starts the band's side gap after them, the same gap they keep from the top.
/// `None` for a measurement that is not a plausible button.
pub fn inset_from_lights(lights: Lights) -> Option<MacInset> {
    let Lights { right, centre_from_top, size } = lights;
    // The bounds keep out a full-screen window, whose lights sit in the toolbar window about
    // 1000 pt from the left.
    let sane = [right, centre_from_top, size].iter().all(|v| v.is_finite() && *v > 0.0)
        && size <= 2.0 * centre_from_top
        && centre_from_top <= MAX_CENTRE_FROM_TOP
        && right <= MAX_RIGHT;
    sane.then(|| MacInset { leading: right + (centre_from_top - size / 2.0), centre_from_top })
}

/// The inset for macOS `major` (26 for Tahoe) before a window exists to measure.
///
/// Before 26 the lights are 12 pt, 20 pt apart, 7 pt from the left, in a 28 pt band: 67 / 14.
/// DeckCraft measured macOS 27 at 14 pt, 23 pt apart, 9 pt from the left, in a 32 pt band:
/// 78 / 16. macOS 26 has not been measured; it shares 27's redesigned lights, so it uses the same.
pub fn default_inset(major: u32) -> MacInset {
    let lights = if major >= 26 {
        Lights { right: 9.0 + 2.0 * 23.0 + 14.0, centre_from_top: 16.0, size: 14.0 }
    } else {
        Lights { right: 7.0 + 2.0 * 20.0 + 12.0, centre_from_top: 14.0, size: 12.0 }
    };
    inset_from_lights(lights).unwrap_or_default()
}

/// The measured inset, kept as app state and re-measured when the scale changes or full screen ends.
#[derive(Clone, Debug, Default)]
pub struct Cache {
    measured: Option<MacInset>,
    scale: Option<u32>,
}

impl Cache {
    /// The inset for this frame. `measure` runs only when there is no measurement, the scale
    /// changed since it, or full screen ended since it; it returns `None` while no window can be
    /// measured, and the next frame tries again. Without a measurement the result is `default`.
    /// `None` in full screen.
    pub fn inset(
        &mut self,
        scale: f32,
        fullscreen: bool,
        default: impl FnOnce() -> MacInset,
        measure: impl FnOnce() -> Option<MacInset>,
    ) -> Option<MacInset> {
        if fullscreen {
            // The lights move or hide in full screen; what was measured is out of date.
            self.measured = None;
            return None;
        }
        let key = scale.to_bits();
        if self.scale != Some(key) {
            self.measured = None;
        }
        if self.measured.is_none() {
            self.measured = measure();
            self.scale = self.measured.map(|_| key);
        }
        Some(self.measured.unwrap_or_else(default))
    }
}

/// The inset for the running macOS version before a window exists to measure; [`MacInset::default`] elsewhere.
#[cfg(all(feature = "appkit", target_os = "macos"))]
pub fn os_default() -> MacInset {
    use objc2_foundation::NSProcessInfo;
    let major = NSProcessInfo::processInfo().operatingSystemVersion().majorVersion;
    default_inset(u32::try_from(major).unwrap_or(0))
}

/// The inset for the running macOS version before a window exists to measure; [`MacInset::default`] elsewhere.
#[cfg(not(all(feature = "appkit", target_os = "macos")))]
pub fn os_default() -> MacInset {
    MacInset::default()
}

/// One measurement of the first titled full-size-content window, without the cache. `None` when no such window
/// exists, off the main thread, and off macOS.
#[cfg(all(feature = "appkit", target_os = "macos"))]
pub fn measure() -> Option<MacInset> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSWindowButton, NSWindowStyleMask};

    // eframe runs the app on the main thread; anywhere else AppKit is out of reach.
    let mtm = MainThreadMarker::new()?;
    // Every titled window with a full-size content view has the same geometry; floating panels
    // hide the buttons and are skipped.
    // A window in full screen is skipped: its buttons live in the full-screen toolbar window.
    let window = NSApplication::sharedApplication(mtm).windows().to_vec().into_iter().find(|w| {
        let mask = w.styleMask();
        mask.contains(NSWindowStyleMask::Titled)
            && mask.contains(NSWindowStyleMask::FullSizeContentView)
            && !mask.contains(NSWindowStyleMask::FullScreen)
            && w.standardWindowButton(NSWindowButton::CloseButton).is_some_and(|close| !close.isHidden())
            && w.standardWindowButton(NSWindowButton::ZoomButton).and_then(|zoom| zoom.window()).is_some_and(|owner| owner == *w)
    })?;
    let zoom = window.standardWindowButton(NSWindowButton::ZoomButton)?;
    // Window coordinates count up from the bottom.
    let frame = zoom.convertRect_toView(zoom.bounds(), None);
    let window_height = window.frame().size.height;
    inset_from_lights(Lights {
        right: (frame.origin.x + frame.size.width) as f32,
        centre_from_top: (window_height - (frame.origin.y + frame.size.height / 2.0)) as f32,
        size: frame.size.height as f32,
    })
}

/// One measurement of the first titled full-size-content window, without the cache. `None` when no such window
/// exists, off the main thread, and off macOS.
#[cfg(not(all(feature = "appkit", target_os = "macos")))]
pub fn measure() -> Option<MacInset> {
    None
}

/// Sets the `movable` flag of the key window when it has a full-size content view and visible window buttons;
/// floating panels in their own windows keep their own flag, which AppKit also needs to resize them. `applied`
/// holds the value last set. Does nothing off macOS.
pub fn set_movable(movable: bool, applied: &mut Option<bool>) {
    #[cfg(all(feature = "appkit", target_os = "macos"))]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::{NSApplication, NSWindowButton, NSWindowStyleMask};

        // eframe runs the app on the main thread; anywhere else AppKit is out of reach.
        let Some(mtm) = MainThreadMarker::new() else { return };
        let Some(window) = NSApplication::sharedApplication(mtm).keyWindow() else { return };
        let buttons = window.standardWindowButton(NSWindowButton::CloseButton).is_some_and(|close| !close.isHidden());
        if window.styleMask().contains(NSWindowStyleMask::FullSizeContentView) && buttons {
            window.setMovable(movable);
            *applied = Some(movable);
        }
    }
    #[cfg(not(all(feature = "appkit", target_os = "macos")))]
    let _ = (movable, applied);
}

/// Sets the opacity (1 is opaque) of the AppKit windows titled `title`. egui has no viewport command for it, and a
/// transparent surface would need the main window transparent too (eframe picks one alpha mode for every viewport),
/// so this goes to AppKit, which fades content, corners and shadow together. A window's native handle is a raw
/// `NSView` pointer and turning that into an `NSWindow` needs `unsafe`, so the window is found by title; two
/// windows with one title fade together. Returns whether a window was found. Does nothing off macOS.
pub fn set_opacity(title: &str, opacity: f32) -> bool {
    #[cfg(all(feature = "appkit", target_os = "macos"))]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        use objc2_foundation::NSString;

        let Some(mtm) = MainThreadMarker::new() else { return false };
        let title = NSString::from_str(title);
        let opacity = f64::from(opacity);
        let mut found = false;
        for window in NSApplication::sharedApplication(mtm).windows().iter() {
            if window.title().isEqualToString(&title) {
                found = true;
                if window.alphaValue() != opacity {
                    window.setAlphaValue(opacity);
                }
            }
        }
        found
    }
    #[cfg(not(all(feature = "appkit", target_os = "macos")))]
    {
        let _ = (title, opacity);
        false
    }
}

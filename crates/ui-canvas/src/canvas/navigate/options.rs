//! `NavigationOptions`: the user's navigation choices (Photoshop's Preferences, General, Tools and Enhanced Controls
//! rows), which the app owns and hands to the navigation pieces and the view tools.

/// The navigation choices, with Photoshop's defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NavigationOptions {
    /// General, Zoom with Scroll Wheel: the wheel zooms and Alt + wheel scrolls. Off.
    pub zoom_with_wheel: bool,
    /// General, Animated Zoom: a zoom command, a Zoom tool click and a Navigator zoom ease on the canvas, and a held Zoom
    /// tool press zooms continuously; the wheel and a pinch follow the input at once. On.
    pub animated_zoom: bool,
    /// General, Zoom Resizes Windows (also the Zoom tool's Resize Windows to Fit). Off.
    pub zoom_resizes_windows: bool,
    /// General, Zoom Clicked Point to Center: a Zoom tool click centres the clicked point. Off.
    pub zoom_clicked_point_to_center: bool,
    /// Tools, Enable Flick Panning: a Hand drag released while moving glides on. On.
    pub flick_panning: bool,
    /// Tools, Overscroll: the view may scroll the document past the canvas edge. Off.
    pub overscroll: bool,
    /// The Zoom tool's Scrubby Zoom option: a Zoom tool drag zooms continuously instead of drawing a rectangle. On.
    pub scrubby_zoom: bool,
    /// Enhanced Controls, Zoom with Trackpad Pinch. On.
    pub pinch_zooms: bool,
    /// Enhanced Controls, Rotate View with Trackpad. Off.
    pub trackpad_rotate: bool,
    /// Enhanced Controls, Enable Gestures: a touch screen's two-finger pinch and twist. On.
    pub touch_gestures: bool,
}

impl Default for NavigationOptions {
    fn default() -> Self {
        Self {
            zoom_with_wheel: false,
            animated_zoom: true,
            zoom_resizes_windows: false,
            zoom_clicked_point_to_center: false,
            flick_panning: true,
            overscroll: false,
            scrubby_zoom: true,
            pinch_zooms: true,
            trackpad_rotate: false,
            touch_gestures: true,
        }
    }
}

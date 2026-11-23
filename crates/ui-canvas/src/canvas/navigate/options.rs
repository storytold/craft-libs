//! `NavigationOptions`: the user's navigation choices (Photoshop's Preferences, General, Tools and Enhanced Controls
//! rows), which the app owns and hands to the navigation pieces and the view tools.

<<<<<<< HEAD
use super::PageKey;
=======
/// How the wheel navigates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum WheelScheme {
    /// Photoshop's: the wheel scrolls, and Zoom with Scroll Wheel decides what zooms.
    #[default]
    Photoshop,
    /// A document's: Ctrl/Cmd+wheel zooms by zoom ladder rungs and Shift+wheel scrolls sideways.
    Document,
}

/// What the middle button does on the canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MiddleButton {
    /// A middle drag pans.
    #[default]
    Pan,
    /// A middle click starts autoscroll and a middle drag pans.
    Autoscroll,
}
>>>>>>> origin/next

/// The navigation choices, with Photoshop's defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct NavigationOptions {
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
    /// The Hand tool's Scroll All Windows option: a Hand drag scrolls every open document's view by the same amount. Off.
    pub scroll_all_windows: bool,
    /// Enhanced Controls, Zoom with Trackpad Pinch. On.
    pub pinch_zooms: bool,
    /// Enhanced Controls, Rotate View with Trackpad. Off.
    pub trackpad_rotate: bool,
    /// Enhanced Controls, Enable Gestures: a touch screen's two-finger pinch and twist. On.
    pub touch_gestures: bool,
<<<<<<< HEAD
    /// What a plain and a modified wheel do. `Scroll`.
    pub wheel: WheelScheme,
    /// How many points a wheel line moves the view. 16.
    pub wheel_line: f32,
    /// How many points a key move of the view steps. [`PageKey::SMALL_STEP`].
    pub key_line: f32,
    /// A wheel pan eases to its target through the wheel service instead of jumping. Off.
    pub eased_wheel: bool,
    /// A touchpad's scroll glides on after the fingers lift. Off.
    pub touchpad_glide: bool,
    /// What the middle button does over the canvas background. `Pan`.
    pub middle_button: craft_behavior::MiddleButton,
    /// The middle-button scroll runs the other way. Off.
    pub invert_middle: bool,
}

/// What a plain and a modified wheel do over a canvas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum WheelScheme {
    /// The wheel scrolls; Alt and the zoom modifiers zoom.
    #[default]
    Scroll,
    /// The wheel zooms; Alt scrolls.
    Zoom,
    /// Cmd or Ctrl zooms about the pointer by [`ACROBAT_NOTCH`](Self::ACROBAT_NOTCH) per line; Shift scrolls sideways.
    Acrobat,
}

impl WheelScheme {
    /// The zoom factor of one wheel line under [`Acrobat`](Self::Acrobat).
    pub const ACROBAT_NOTCH: f64 = 1.221_402_758;
=======
    /// How the wheel navigates. Photoshop's.
    pub wheel: WheelScheme,
    /// What the middle button does. A pan.
    pub middle_button: MiddleButton,
>>>>>>> origin/next
}

impl Default for NavigationOptions {
    fn default() -> Self {
        Self {
            animated_zoom: true,
            zoom_resizes_windows: false,
            zoom_clicked_point_to_center: false,
            flick_panning: true,
            overscroll: false,
            scrubby_zoom: true,
            scroll_all_windows: false,
            pinch_zooms: true,
            trackpad_rotate: false,
            touch_gestures: true,
            wheel: WheelScheme::default(),
<<<<<<< HEAD
            wheel_line: 16.0,
            key_line: PageKey::SMALL_STEP,
            eased_wheel: false,
            touchpad_glide: false,
            middle_button: craft_behavior::MiddleButton::default(),
            invert_middle: false,
=======
            middle_button: MiddleButton::default(),
>>>>>>> origin/next
        }
    }
}

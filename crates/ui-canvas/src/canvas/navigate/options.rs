//! `NavigationOptions`: the user's navigation choices (Photoshop's Preferences, General, Tools and Enhanced Controls
//! rows), which the app owns and hands to the navigation pieces and the view tools.

use super::PageKey;

pub use craft_behavior::MiddleButton;

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
    /// What a plain and a modified wheel do. `Scroll`.
    pub wheel: WheelScheme,
    /// How many points a wheel line moves the view. 16.
    pub wheel_line: f32,
    /// How many points a key move of the view steps. [`PageKey::SMALL_STEP`].
    pub key_line: f32,
    /// A wheel pan eases to its target through the wheel service instead of jumping. Off.
    pub eased_wheel: bool,
    /// A wheel zoom (a zoom-modifier wheel notch, not a pinch or a trackpad's small steps) eases to its factor over the
    /// passes after the notch, as egui eases a wheel, instead of zooming at once. Off.
    pub eased_wheel_zoom: bool,
    /// A touchpad's scroll glides on after the fingers lift. Off.
    pub touchpad_glide: bool,
    /// What the middle button does over the canvas background. `Pan`.
    pub middle_button: craft_behavior::MiddleButton,
    /// The middle-button scroll runs the other way. Off.
    pub invert_middle: bool,
    /// How a middle-button autoscroll feels under [`MiddleButton::AutoScroll`]. [`MiddleAutoscrollFeel::Edge`].
    pub middle_autoscroll: MiddleAutoscrollFeel,
}

/// How a middle-button autoscroll runs: its speed, what its release does and what ends it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MiddleAutoscrollFeel {
    /// The canvas's one autoscroll rule (`Autoscroll::step`) on the pointer's offset past the dead zone; letting go
    /// after a drag past the drag threshold ends the run.
    #[default]
    Edge,
    /// A document reader's (PdfCraft): the speed grows as the pointer's whole distance from the mark raised to the
    /// power 2.2, times 0.04, in points per second; letting go ends the run only after the held pointer rested past
    /// the dead zone for two stepped passes (a shorter hold is a click, which keeps it); and the run ends when the
    /// canvas loses the focus (a modal over it, another control taking the keys).
    Reader,
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
    /// A document's: Ctrl/Cmd+wheel zooms by zoom ladder rungs and Shift+wheel scrolls sideways.
    Document,
}

impl WheelScheme {
    /// The zoom factor of one wheel line under [`Acrobat`](Self::Acrobat).
    pub const ACROBAT_NOTCH: f64 = 1.221_402_758;
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
            wheel_line: 16.0,
            key_line: PageKey::SMALL_STEP,
            eased_wheel: false,
            eased_wheel_zoom: false,
            touchpad_glide: false,
            middle_button: craft_behavior::MiddleButton::default(),
            invert_middle: false,
            middle_autoscroll: MiddleAutoscrollFeel::Edge,
        }
    }
}

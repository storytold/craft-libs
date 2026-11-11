//! The Blur Gallery's pins (Filter › Blur Gallery › Field Blur, Iris Blur, Spin Blur,
//! Tilt-Shift): an overlay on a canvas and the rows of the filter dialog.
//!
//! As PhotoCraft (lead call 48), the Blur Gallery is the ordinary filter dialog: no full-window
//! frame. [`BlurPins`] is the typed state (PhotoCraft kept a JSON map in the dialog's fields);
//! [`BlurPinsOverlay`] draws and edits the pins over a canvas while the dialog is open, and
//! [`BlurPinsPanel`] adds the selected pin's amount and the hints to the dialog's body. The app
//! maps the pins to the filter's parameters and scales them for its proxy preview.
//!
//! ```ignore
//! // In the canvas body, after the document:
//! let out = BlurPinsOverlay::new("Blur pins", Bind::bound(&mut app.blur_pins)).show_at(ui, body.rect);
//! // In the dialog body:
//! BlurPinsPanel::new(&mut app.blur_pins).show(body_ui);
//! ```
//!
//! Files: `model` and `grab`; `overlay` and `panel`; `style` and `words`.

mod grab;
mod model;
mod overlay;
mod panel;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;
mod style;
mod words;

pub use grab::{AXIS_HANDLES, BlurGrab, PinReach, drag, feather_handles, grab, iris_point, round_handle, tilt_point, turn_handle};
pub use model::{BlurKind, BlurPin, BlurPins, DEFAULT_AMOUNT, IrisShape, MAX_PINS, TiltBand};
pub use overlay::{BlurPinsCommand, BlurPinsOutput, BlurPinsOverlay};
pub use panel::{BlurPinsPanel, BlurPinsPanelOutput};
pub use style::BlurPinsStyle;
pub use words::BlurPinsWords;

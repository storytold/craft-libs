//! Direction-aware building blocks for hand-painted chrome: scrubby labels, sliders, value
//! fields, buttons, panel helpers, progress bars, tab strips and text-edit helpers.
//!
//! egui's `Style::right_to_left` flag mirrors only egui-owned containers; these widgets are
//! hand-painted, so each one mirrors its own geometry through the [`crate::direction`] helpers.
//! The rule everywhere: a widget owns its direction branch — callers pass intent ("after in
//! reading order"), never a pre-mirrored boolean — pointer semantics (scrub drags, wheel
//! notches, slider value↔x mapping) stay physical, and the add order of focusable children is
//! never reversed, so Tab order follows registration, not geometry.
//!
//! Applications own their themes: every widget takes its colors and fonts as parameters and
//! returns plain egui responses.

pub mod buttons;
pub mod fields;
pub mod panel;
pub mod progress;
pub mod scrub;
pub mod slider;
pub mod tab_strip;
pub mod text_field;

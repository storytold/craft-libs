//! [`BlurPinsStyle`]: the pins' marks, from tokens.

use craft_behavior::Metrics;
use craft_ui::style::{ComponentStyle, FocusLook, HandleLook, MarkerLook, StyleColor, StyleCx};

use super::grab::PinReach;

/// How far a feather handle sits beyond a plain handle's radius, in screen points.
const FEATHER_EXTRA: f32 = 1.0;
/// How far a turn handle sits beyond a plain handle's radius, in screen points.
const TURN_EXTRA: f32 = 0.5;
/// The corner radius of the rounding handle, in screen points.
const ROUND_CORNER: f32 = 1.0;

/// The pins' sizes in screen points and colours.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct BlurPinsStyle {
    #[style(metrics)]
    metrics: Metrics,
    /// The handles' look; a shape handle is half its side in radius.
    pub(super) handle: HandleLook,
    /// How near a press grabs each mark.
    pub(super) reach: PinReach,
    /// A dash's length and the gap after it.
    pub(super) dash: f32,
    pub(super) dash_gap: f32,
    /// A feather handle's extra radius.
    pub(super) feather_extra: f32,
    /// A turn handle's extra radius.
    pub(super) turn_extra: f32,
    /// The rounding handle's corner radius.
    pub(super) round_corner: f32,
    /// The pins, the ring and the lines: the marker's ring over its halo; a handle is filled with the ring colour and
    /// edged with the halo.
    pub(super) marker: MarkerLook,
    /// The ring's amount arc.
    pub(super) amount: StyleColor,
    pub(super) focus: FocusLook,
}

impl BlurPinsStyle {
    /// The sizes and colours in `cx`.
    pub(super) fn of(cx: &StyleCx<'_>) -> BlurPinsStyle {
        // pc: crates/ui-egui/src/dialogs/blur_pins.rs:44-51 (the sizes), :689 `draw` (the colours).
        let tokens = cx.tokens;
        let (size, stroke) = (&tokens.size, &tokens.stroke);
        BlurPinsStyle {
            metrics: Metrics::new(),
            handle: cx.handle(),
            reach: PinReach::new(size.hit.target, size.handle.ring, size.hit.target, size.hit.target),
            dash: stroke.dash,
            dash_gap: stroke.dash_gap,
            feather_extra: FEATHER_EXTRA,
            turn_extra: TURN_EXTRA,
            round_corner: ROUND_CORNER,
            marker: cx.marker(),
            amount: cx.color(tokens.color.accent.solid),
            focus: cx.focus_look(),
        }
    }

    /// How near a press grabs each mark.
    pub fn reach(&self) -> PinReach {
        self.reach
    }
}

//! [`BlurPinsWords`]: the pins' words.

craft_ui::words! {
    /// The Blur Gallery pins' words, English by default.
    pub struct BlurPinsWords {
        /// The pins' group on the canvas.
        pins: "blurPins.pins" = "Blur pins";
        /// A pin as a keyboard part: `{n}` its number, `{amount}` its blur in pixels with the unit.
        pin_part: template "blurPins.pinPart" = "Pin {n}, Blur {amount}";
        /// A Spin Blur pin as a keyboard part: `{n}` its number, `{amount}` its angle with the unit.
        pin_part_angle: template "blurPins.pinPartAngle" = "Pin {n}, Blur Angle {amount}";
        /// The amount row of a pin.
        blur: "blurPins.blur" = "Blur";
        /// The amount row of a Spin Blur pin.
        blur_angle: "blurPins.blurAngle" = "Blur Angle";
        /// The panel without a selected pin.
        no_pin: "blurPins.noPin" = "No pin selected";
        /// The hint of Field, Iris and Spin Blur.
        hint_pins: "blurPins.hintPins" = "Click the image to add a pin; drag a pin to move it, or its ring to change its blur. Delete removes the selected pin.";
        /// The hint of Tilt-Shift.
        hint_tilt: "blurPins.hintTilt" = "Drag the pin to move the band, or its ring to change its blur. Drag the solid lines to widen the sharp band, the dashed lines to set the transition, and the dots to turn it.";
        /// The extra hint of Iris Blur.
        hint_iris: "blurPins.hintIris" = "Drag the handles on the outline to resize and turn it, the square to round its corners, and the dots inside to set where the blur starts.";
        /// The extra hint of Spin Blur.
        hint_spin: "blurPins.hintSpin" = "Drag the handles on the outline to resize and turn it; the ring sets the blur angle.";
    }
}

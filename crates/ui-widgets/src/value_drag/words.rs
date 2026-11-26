//! Built-in words of the value drag family.

craft_ui::words! {
    /// The words of range sliders, thumb rows and point pickers.
    pub struct ValueDragWords {
        lower: "rangeSlider.lower" = "Minimum";
        upper: "rangeSlider.upper" = "Maximum";
        thumb_low: template "thumbRow.low" = "{name} low";
        thumb_high: template "thumbRow.high" = "{name} high";
        range: template "rangeSlider.range" = "{from} to {to}";
    }
}

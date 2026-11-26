//! The words of the corner widgets ([`CornerWords`]).

craft_ui::words! {
    /// The words of [`CornerWidgets`](super::CornerWidgets), English by default. Templates take `{i}`, `{n}` and
    /// `{radius}` (the radius in the canvas's ruler units, with the unit).
    pub struct CornerWords {
        /// The AccessKit group of widgets.
        group: "cornerWidgets.group" = "Corner radius";
        /// A widget's accessible name: "Corner 2 of 4".
        corner: template "cornerWidgets.corner" = "Corner {i} of {n}";
        /// A widget's accessible value: "radius 12 px".
        value: template "cornerWidgets.value" = "radius {radius}";
        /// The readout's label beside the pointer while dragging ("R: 25.0 px").
        readout: "cornerWidgets.readout" = "R:";
    }
}

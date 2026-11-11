//! The words of the gradient stop bar, its fields and the canvas gradient line.

craft_ui::words! {
    /// The gradient editor's words. Templates take `{i}`, `{n}`, `{location}`, `{hex}`, `{opacity}`, `{colors}`,
    /// `{opacities}`, `{x}` and `{y}`.
    pub struct GradientWords {
        /// A colour stop's accessible name: "Color stop {i} of {n}".
        color_stop: template "gradient.colorStop" = "Color stop {i} of {n}";
        /// A colour stop's accessible value: "{location}, {hex}".
        color_value: template "gradient.colorValue" = "{location}, {hex}";
        /// A colour stop's accessible value under `StopRows::Combined`.
        color_opacity_value: template "gradient.colorOpacityValue" = "{location}, {hex}, opacity {opacity}";
        /// An opacity stop's accessible name: "Opacity stop {i} of {n}".
        opacity_stop: template "gradient.opacityStop" = "Opacity stop {i} of {n}";
        /// An opacity stop's accessible value: "{location}, opacity {opacity}".
        opacity_value: template "gradient.opacityValue" = "{location}, opacity {opacity}";
        /// A colour midpoint's accessible name: "Midpoint {i}".
        midpoint: template "gradient.midpoint" = "Midpoint {i}";
        /// An opacity midpoint's accessible name: "Opacity midpoint {i}".
        opacity_midpoint: template "gradient.opacityMidpoint" = "Opacity midpoint {i}";
        /// The count of colour stops in the bar's value under `StopRows::Split`.
        color_stops: count "gradient.colorStops" = "{n} color stop" | "{n} color stops";
        /// The count of opacity stops in the bar's value under `StopRows::Split`.
        opacity_stops: count "gradient.opacityStops" = "{n} opacity stop" | "{n} opacity stops";
        /// The bar's value under `StopRows::Split`: "{colors}, {opacities}".
        split_value: template "gradient.splitValue" = "{colors}, {opacities}";
        /// The count of stops in the bar's value under `StopRows::Combined`.
        bar_stops: count "gradient.barStops" = "{n} stop" | "{n} stops";
        /// A canvas gradient line's end handle value, a point in document units: "{x}, {y}".
        point_value: template "gradient.pointValue" = "{x}, {y}";
        /// The Color field's label.
        color: "gradient.color" = "Color";
        /// The Opacity field's label.
        opacity: "gradient.opacity" = "Opacity";
        /// The Location field's label.
        location: "gradient.location" = "Location";
        /// The Midpoint field's label.
        midpoint_field: "gradient.midpointField" = "Midpoint";
        /// The Delete button's accessible name and tooltip: "Delete stop".
        delete: "gradient.deleteStop" = "Delete stop";
        /// The name of the selected stop's fields.
        stop_fields: "gradient.stopFields" = "Selected stop";
        /// The name of the colour popover.
        popover: "gradient.colorPopover" = "Stop color";
    }
}

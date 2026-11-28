//! Built-in words of the text family.

craft_ui::words! {
    /// The words of text fields.
    pub struct TextWords {
        clear: "textField.clear" = "Clear";
        presets: "textField.presets" = "Presets";
        automatic: "textField.automatic" = "Auto";
        slider: "textField.slider" = "Slider";
        mixed: "textField.mixed" = "Mixed";
        range_integer: template "textField.rangeInteger" = "An integer between {low} and {high} is required. Closest value inserted.";
        range_value: template "textField.rangeValue" = "A value between {low} and {high} is required. Closest value inserted.";
        range_number: template "textField.rangeNumber" = "A number between {low} and {high} is required. Closest value inserted.";
        range_end: template "textField.rangeEnd" = "{n} {unit}";
    }
}

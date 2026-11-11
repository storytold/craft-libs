//! The picker's colour logic: the values a colour is shown in, and each edit as a new [`PickedColor`].

use std::rc::Rc;

use craft_theme::{ColorModel, ModelValues, Rgba, Srgba};

use super::PickedColor;
use crate::ui::channels::{WEB_LEVELS, snapped};

/// The host's constraint on every colour the picker reports.
#[derive(Clone)]
pub struct ColorConstraint<'a>(Rc<dyn Fn(Srgba) -> Srgba + 'a>);

impl<'a> ColorConstraint<'a> {
    /// A constraint that maps every colour through `constrain`.
    pub fn new(constrain: impl Fn(Srgba) -> Srgba + 'a) -> Self {
        ColorConstraint(Rc::new(constrain))
    }
}

/// What every colour an edit makes goes through: Only Web Colors' snap, then the host's constraint.
#[derive(Clone)]
pub(super) struct Rules<'a> {
    pub web: bool,
    pub constrain: Option<ColorConstraint<'a>>,
}

impl Rules<'_> {
    pub(super) fn apply(&self, color: Srgba) -> Srgba {
        let color = if self.web { snapped(color, WEB_LEVELS) } else { color };
        self.constrain.as_ref().map_or(color, |constrain| (constrain.0)(color))
    }
}

impl PickedColor {
    /// The colour in `model`: the kept values while they are of `model` and still give the colour's 8-bit sRGB (greys
    /// keep their hue, black its hue and saturation, a typed Lab colour outside sRGB its values), otherwise converted.
    pub(super) fn values_in(&self, model: &dyn ColorModel) -> ModelValues {
        match self.model {
            Some(values) if values.model == model.id() && model.to_srgb(&values.values).same_rgb8(self.color) => values,
            _ => model.from_srgb(self.color),
        }
    }

    /// The colour `values` of `model` give, with this alpha, through the rules. The values stay with the colour while
    /// the rules left it alone.
    pub(super) fn with_values(&self, values: &ModelValues, model: &dyn ColorModel, rules: &Rules<'_>) -> PickedColor {
        let made = model.to_srgb(&values.values).with_alpha(self.color.a);
        let color = rules.apply(made);
        PickedColor { color, model: color.same_rgb8(made).then_some(*values) }
    }

    /// Channel `channel` of `model` set to `value`.
    pub(super) fn with_channel(&self, model: &dyn ColorModel, channel: usize, value: f64, rules: &Rules<'_>) -> PickedColor {
        let mut values = self.values_in(model);
        values.values[channel] = value as f32;
        self.with_values(&values, model, rules)
    }

    /// The alpha set to `percent`.
    pub(super) fn with_alpha(&self, percent: f64, rules: &Rules<'_>) -> PickedColor {
        PickedColor { color: rules.apply(self.color.with_alpha((percent / 100.0) as f32)), model: self.model }
    }

    /// A colour set as such (a sample, a click on a swatch): no model values stay with it.
    pub(super) fn with_color(&self, color: Srgba, rules: &Rules<'_>) -> PickedColor {
        PickedColor { color: rules.apply(color), model: None }
    }

    /// The hex field's colour; this value itself while the field shows it.
    pub(super) fn with_hex(&self, rgba: Rgba, rules: &Rules<'_>) -> PickedColor {
        if rgba == self.color.to_rgba() { *self } else { self.with_color(rgba.into(), rules) }
    }

    /// The alpha in percent.
    pub(super) fn alpha_percent(&self) -> f64 {
        f64::from(self.color.a) * 100.0
    }
}

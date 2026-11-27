//! The picker's colour logic: the values a colour is shown in, and each edit as a new [`PickedColor`].

use std::rc::Rc;

use craft_theme::{ChannelPrecision, ColorModel, ModelValues, Rgba, Srgba};

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
    pub exact_rgb: Option<&'a dyn ColorModel>,
}

impl Rules<'_> {
    pub(super) fn same(&self, a: Srgba, b: Srgba) -> bool {
        if self.exact_rgb.is_some() { a == b } else { a.same_rgb8(b) }
    }

    pub(super) fn apply(&self, color: Srgba) -> Srgba {
        let color = self.exact_rgb.map_or(color, |model| model.to_srgb(&model.from_srgb(color).values).with_alpha(color.a));
        let color = if self.web { snapped(color, WEB_LEVELS) } else { color };
        self.constrain.as_ref().map_or(color, |constrain| (constrain.0)(color))
    }
}

impl PickedColor {
    /// The colour in `model`: the kept values while they are of `model` and still give the colour's 8-bit sRGB (greys
    /// keep their hue, black its hue and saturation, a typed Lab colour outside sRGB its values), otherwise converted.
    pub(super) fn values_in(&self, model: &dyn ColorModel) -> ModelValues {
        match self.model {
            Some(values)
                if values.model == model.id()
                    && match model.channel_precision() {
                        ChannelPrecision::Float32 => model.to_srgb(&values.values).to_rgb_array() == self.color.to_rgb_array(),
                        _ => model.to_srgb(&values.values).same_rgb8(self.color),
                    } =>
            {
                values
            }
            _ => model.from_srgb(self.color),
        }
    }

    /// The colour `values` of `model` give, with this alpha, through the rules. The values stay with the colour while
    /// the rules left it alone.
    pub(super) fn with_values(&self, values: &ModelValues, model: &dyn ColorModel, rules: &Rules<'_>) -> PickedColor {
        let made = model.to_srgb(&values.values).with_alpha(self.color.a);
        if rules.exact_rgb.is_some() && ![made.r, made.g, made.b, made.a].iter().all(|v| v.is_finite()) {
            return *self;
        }
        let color = rules.apply(made);
        PickedColor { color, model: rules.same(color, made).then_some(*values) }
    }

    /// Channel `channel` of `model` set to `value`.
    pub(super) fn with_channel(&self, model: &dyn ColorModel, channel: usize, value: f64, rules: &Rules<'_>) -> PickedColor {
        let mut values = self.values_in(model);
        let value = value as f32;
        if model.channel_precision() == ChannelPrecision::Float32 && !value.is_finite() {
            return *self;
        }
        values.values[channel] = value;
        self.with_values(&values, model, rules)
    }

    /// The alpha set to `percent`.
    pub(super) fn with_alpha(&self, percent: f64, rules: &Rules<'_>) -> PickedColor {
        PickedColor { color: rules.apply(self.color.with_alpha((percent / 100.0) as f32)), model: self.model }
    }

    /// This colour set again (a click on the compare swatch or a warning chip), through the rules. Its values stay
    /// while the rules leave the colour alone.
    pub(super) fn through(self, rules: &Rules<'_>) -> PickedColor {
        let color = rules.apply(self.color);
        PickedColor { color, model: self.model.filter(|_| rules.same(color, self.color)) }
    }

    /// A colour set as such (a sample, a click on a swatch): no model values stay with it.
    pub(super) fn with_color(&self, color: Srgba, rules: &Rules<'_>) -> PickedColor {
        if rules.exact_rgb.is_some() && ![color.r, color.g, color.b, color.a].iter().all(|v| v.is_finite()) {
            return *self;
        }
        PickedColor { color: rules.apply(color), model: None }
    }

    /// The hex field's colour; this value itself while the field shows it.
    pub(super) fn with_hex(&self, rgba: Rgba, rules: &Rules<'_>) -> PickedColor {
        if rules.exact_rgb.is_none() && rgba == self.color.to_rgba() {
            return *self;
        }
        let color = Srgba::from(rgba);
        self.with_color(if rules.exact_rgb.is_some() { color.with_alpha(self.color.a) } else { color }, rules)
    }

    /// The alpha in percent.
    pub(super) fn alpha_percent(&self) -> f64 {
        f64::from(self.color.a) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_theme::FloatRgbModel;

    #[test]
    fn exact_rgb_identity_reset_and_explicit_sdr_edits_use_actual_components() {
        let model = FloatRgbModel::raw();
        let rules = Rules { web: false, constrain: None, exact_rgb: Some(&model) };
        let color = Srgba::rgb(2.0, 0.1234567, -0.25).with_alpha(0.37);
        let stale = ModelValues::new("rgb", &[3.0, 0.1234567, -0.25]);
        let picked = PickedColor { color, model: Some(stale) };
        assert!(model.to_srgb(&stale.values).same_rgb8(color));
        assert_eq!(picked.values_in(&model).values[0], 2.0, "an equal projection cannot reuse stale raw values");
        assert_eq!(picked.through(&rules).color, color, "restoring the opening color retains its raw components");
        for invalid in [f64::NAN, f64::INFINITY, f64::MAX] {
            assert_eq!(picked.with_channel(&model, 0, invalid, &rules), picked);
            assert_eq!(picked.with_color(color.with_alpha(invalid as f32), &rules), picked);
        }
        let hex = picked.with_hex(color.to_rgba(), &rules);
        assert_eq!(hex.color.to_rgb_array(), Srgba::from(color.to_rgba()).to_rgb_array());
        assert_eq!(hex.color.a, color.a);
        assert!(hex.model.is_none());
        let web_source =
            PickedColor { color: Srgba::rgb(2.0, 0.0, -0.25).with_alpha(0.37), model: Some(ModelValues::new("rgb", &[2.0, 0.0, -0.25])) };
        let web = web_source.through(&Rules { web: true, ..rules.clone() });
        assert!(web.color.same_rgb8(web_source.color), "this pair deliberately has the same SDR projection");
        assert_ne!(web.color, web_source.color);
        assert!(web.model.is_none(), "a Web clamp invalidates raw metadata even at an equal projection");
        let bounded = FloatRgbModel::bounded();
        let bounded_rules = Rules { web: false, constrain: None, exact_rgb: Some(&bounded) };
        assert_eq!(picked.through(&bounded_rules).color, Srgba::rgb(1.0, 0.1234567, 0.0).with_alpha(0.37));
    }
}

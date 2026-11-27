use crate::color::{Rgba, Srgba};
use crate::color_model::builtin_models;

fn samples() -> Vec<Srgba> {
    let mut out = Vec::new();
    for r in (0..=255).step_by(15) {
        for g in (0..=255).step_by(15) {
            for b in (0..=255).step_by(15) {
                out.push(Srgba::from(Rgba::rgb(r as u8, g as u8, b as u8)));
            }
        }
    }
    out
}

#[test]
fn every_model_round_trips_srgb_to_the_same_8_bit_colour() {
    for model in builtin_models() {
        for c in samples() {
            let back = model.to_srgb(&model.from_srgb(c).values);
            assert!(back.same_rgb8(c), "{}: {c:?} -> {back:?}", model.id());
        }
    }
}


#[test]
fn float_rgb_models_keep_raw_components_and_bound_without_eight_bit_rounding() {
    use crate::color_model::{ChannelPrecision, ColorModel, FloatRgbModel, ModelValues, RgbModel, kept_or_converted};
    let raw = FloatRgbModel::raw();
    assert_eq!(raw.channel_precision(), ChannelPrecision::Float32);
    assert_eq!(RgbModel.channel_precision(), ChannelPrecision::Decimal);
    for rgb in [[1e-8, 0.1234567, 2.5], [-0.25, f32::MIN, f32::MAX], [f32::from_bits(1), 0.0, 1.0]] {
        let color = Srgba::rgb(rgb[0], rgb[1], rgb[2]).with_alpha(0.37);
        let values = raw.from_srgb(color);
        assert_eq!(raw.to_srgb(&values.values).to_rgb_array(), rgb);
        assert!(raw.track(0, &values).iter().all(|c| c.to_rgb_array().iter().all(|v| v.is_finite())));
    }
    let bounded = FloatRgbModel::bounded();
    let color = bounded.to_srgb(&[2.5, 0.1234567, -0.25, 0.0]);
    assert_eq!(color.to_rgb_array(), [1.0, 0.1234567, 0.0]);
    let color = Srgba::rgb(2.0, 0.1234567, 0.0);
    let stale = ModelValues::new("rgb", &[3.0, 0.1234567, 0.0]);
    assert!(raw.to_srgb(&stale.values).same_rgb8(color));
    assert_eq!(kept_or_converted(&raw, color, Some(&stale)).values[0], 2.0);
    assert_eq!(builtin_models().len(), 4);
}

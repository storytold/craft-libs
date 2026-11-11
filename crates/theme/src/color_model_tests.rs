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

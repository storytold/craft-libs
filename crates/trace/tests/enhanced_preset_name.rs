//! `enhanced_preset` resolves a name the way `preset` does (trimmed, brackets stripped, any case),
//! so Shapes is off for High Fidelity Photo however its name is written.

use craft_trace::enhanced_preset;

#[test]
fn enhanced_high_fidelity_photo_ignores_name_decoration() {
    let plain = enhanced_preset("High Fidelity Photo").unwrap();
    assert!(!plain.shapes);
    for name in ["[High Fidelity Photo]", " High Fidelity Photo ", " high fidelity photo "] {
        assert_eq!(enhanced_preset(name), Some(plain.clone()), "{name:?}");
    }
}

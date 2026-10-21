use craft_blitter::{BlendMode, blit_rgba8, blit_rgba16, blit_rgba32f};
use proptest::prelude::*;

fn arb_blend_mode() -> impl Strategy<Value = BlendMode> {
    prop_oneof![
        Just(BlendMode::Normal),
        Just(BlendMode::Multiply),
        Just(BlendMode::Screen),
        Just(BlendMode::Overlay),
        Just(BlendMode::Darken),
        Just(BlendMode::Lighten),
        Just(BlendMode::ColorDodge),
        Just(BlendMode::ColorBurn),
        Just(BlendMode::HardLight),
        Just(BlendMode::SoftLight),
        Just(BlendMode::Difference),
        Just(BlendMode::Exclusion),
        Just(BlendMode::Hue),
        Just(BlendMode::Saturation),
        Just(BlendMode::Color),
        Just(BlendMode::Luminosity),
        Just(BlendMode::LinearBurn),
        Just(BlendMode::LinearDodge),
        Just(BlendMode::VividLight),
        Just(BlendMode::LinearLight),
        Just(BlendMode::PinLight),
        Just(BlendMode::HardMix),
        Just(BlendMode::Subtract),
        Just(BlendMode::Divide),
        Just(BlendMode::Reflect),
        Just(BlendMode::Glow),
        Just(BlendMode::Negation),
        Just(BlendMode::Xor),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]

    #[test]
    fn prop_never_panics_on_arbitrary_rgba8_slices(
        mut dst in prop::collection::vec(any::<u8>(), 0..64),
        src in prop::collection::vec(any::<u8>(), 0..64),
        mode in arb_blend_mode(),
        opacity in any::<f32>(),
        fill in any::<f32>(),
    ) {
        blit_rgba8(&mut dst, &src, mode, opacity, fill);
    }

    #[test]
    fn prop_never_panics_on_arbitrary_rgba16_slices(
        mut dst in prop::collection::vec(any::<u16>(), 0..64),
        src in prop::collection::vec(any::<u16>(), 0..64),
        mode in arb_blend_mode(),
        opacity in any::<f32>(),
        fill in any::<f32>(),
    ) {
        blit_rgba16(&mut dst, &src, mode, opacity, fill);
    }

    #[test]
    fn prop_never_panics_on_arbitrary_rgba32f_slices(
        mut dst in prop::collection::vec(prop::array::uniform4(any::<f32>()), 0..16),
        src in prop::collection::vec(prop::array::uniform4(any::<f32>()), 0..16),
        mode in arb_blend_mode(),
        opacity in any::<f32>(),
        fill in any::<f32>(),
        hdr in any::<bool>(),
    ) {
        blit_rgba32f(&mut dst, &src, mode, opacity, fill, hdr);
    }

    #[test]
    fn prop_zero_opacity_is_noop_rgba8(
        mut dst in prop::collection::vec(any::<u8>(), 0..32),
        src in prop::collection::vec(any::<u8>(), 0..32),
        mode in arb_blend_mode(),
        fill in any::<f32>(),
    ) {
        let original = dst.clone();
        blit_rgba8(&mut dst, &src, mode, 0.0, fill);
        prop_assert_eq!(dst, original);
    }

    #[test]
    fn prop_zero_opacity_is_noop_rgba32f(
        mut dst in prop::collection::vec(prop::array::uniform4(any::<f32>()), 0..8),
        src in prop::collection::vec(prop::array::uniform4(any::<f32>()), 0..8),
        mode in arb_blend_mode(),
        fill in any::<f32>(),
        hdr in any::<bool>(),
    ) {
        let original = dst.clone();
        blit_rgba32f(&mut dst, &src, mode, 0.0, fill, hdr);
        prop_assert_eq!(dst, original);
    }
}

use craft_blitter::{AlphaMode, BlendMode, CompositeOp, blit_rgba8_opts, blit_rgba16_opts, blit_rgba32f_opts};
use proptest::prelude::*;

fn arb_blend_mode() -> impl Strategy<Value = BlendMode> {
    prop::sample::select(&BlendMode::ALL[..])
}

fn arb_alpha_mode() -> impl Strategy<Value = AlphaMode> {
    prop_oneof![Just(AlphaMode::Straight), Just(AlphaMode::Premultiplied)]
}

fn arb_composite_op() -> impl Strategy<Value = CompositeOp> {
    prop_oneof![Just(CompositeOp::SourceOver), Just(CompositeOp::SourceAtop)]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn prop_never_panics_on_arbitrary_rgba8_slices(
        mut dst in prop::collection::vec(any::<u8>(), 0..96),
        src in prop::collection::vec(any::<u8>(), 0..96),
        mode in arb_blend_mode(),
        opacity in any::<f32>(),
        alpha in arb_alpha_mode(),
        op in arb_composite_op(),
    ) {
        blit_rgba8_opts(&mut dst, &src, mode, opacity, alpha, op);
    }

    #[test]
    fn prop_never_panics_on_arbitrary_rgba16_slices(
        mut dst in prop::collection::vec(any::<u16>(), 0..96),
        src in prop::collection::vec(any::<u16>(), 0..96),
        mode in arb_blend_mode(),
        opacity in any::<f32>(),
        alpha in arb_alpha_mode(),
        op in arb_composite_op(),
    ) {
        blit_rgba16_opts(&mut dst, &src, mode, opacity, alpha, op);
    }

    #[test]
    fn prop_never_panics_on_arbitrary_rgba32f_slices(
        mut dst in prop::collection::vec(prop::array::uniform4(any::<f32>()), 0..24),
        src in prop::collection::vec(prop::array::uniform4(any::<f32>()), 0..24),
        mode in arb_blend_mode(),
        opacity in any::<f32>(),
        hdr in any::<bool>(),
        alpha in arb_alpha_mode(),
        op in arb_composite_op(),
    ) {
        blit_rgba32f_opts(&mut dst, &src, mode, opacity, hdr, alpha, op);
    }

    #[test]
    fn prop_non_positive_opacity_is_noop(
        mut dst8 in prop::collection::vec(any::<u8>(), 0..48),
        src8 in prop::collection::vec(any::<u8>(), 0..48),
        mut dst32 in prop::collection::vec(prop::array::uniform4(any::<f32>()), 0..12),
        src32 in prop::collection::vec(prop::array::uniform4(any::<f32>()), 0..12),
        mode in arb_blend_mode(),
        alpha in arb_alpha_mode(),
        op in arb_composite_op(),
        hdr in any::<bool>(),
    ) {
        let orig8 = dst8.clone();
        blit_rgba8_opts(&mut dst8, &src8, mode, 0.0, alpha, op);
        prop_assert_eq!(dst8, orig8);

        let orig32 = dst32.clone();
        blit_rgba32f_opts(&mut dst32, &src32, mode, 0.0, hdr, alpha, op);
        for (a, b) in dst32.iter().zip(&orig32) {
            for c in 0..4 {
                prop_assert_eq!(a[c].to_bits(), b[c].to_bits());
            }
        }
    }
}

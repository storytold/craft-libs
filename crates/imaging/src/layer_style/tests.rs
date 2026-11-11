//! Tests of the Layer Style model.

use super::*;
use craft_ui::testing::trial;

fn style_with(kinds: &[(u32, EffectKind)]) -> LayerStyle {
    let effects =
        kinds.iter().map(|&(n, kind)| EffectInstance { id: EffectId(n), kind, on: true, params: EffectSpec::photoshop(kind).defaults }).collect();
    LayerStyle { effects, ..LayerStyle::default() }
}

// Per effect, the field keys in order with their kinds and ranges match the table in
// `EffectSpec::photoshop`'s doc (Outer Glow: Technique before Spread, Range after Contour).

// The ten tables' defaults equal PhotoCraft's engine `effect_defaults` (Drop Shadow Multiply,
// black, 75, 120°, global light, 5, 0, 5; Satin Multiply, 50, 19°, 11, 14, anti-aliased, inverted).

// Removing the selected of two Drop Shadows selects the other; removing the only one selects the
// Styles page; removing an unselected one keeps the page.

// `allows_several` is true for exactly Stroke, Drop Shadow, Inner Shadow, Color Overlay and
// Gradient Overlay.

// Setting a globally lit Drop Shadow's angle sets `global_light` and the angle of every globally
// lit instance, a disabled Bevel too; turning Use Global Light on adopts the shared angle.
#[trial]
fn global_light_angle_is_shared_by_effects_that_use_it() {
    let mut draft = LayerStyleDraft::new(style_with(&[(1, EffectKind::DropShadow), (2, EffectKind::BevelEmboss), (3, EffectKind::Satin)]));
    draft.toggle(EffectId(2));
    assert!(draft.style.effect(EffectId(2)).is_some_and(|e| !e.on));

    draft.set_param(EffectId(1), "angle", StyleValue::Number(45.0));
    assert_eq!(draft.style.global_light, 45.0);
    assert_eq!(draft.style.effect(EffectId(2)).and_then(|e| e.params.number("angle")), Some(45.0));
    // Satin has no Use Global Light.
    assert_eq!(draft.style.effect(EffectId(3)).and_then(|e| e.params.number("angle")), Some(19.0));

    // With Use Global Light off the angle is the instance's own.
    draft.set_param(EffectId(1), "useGlobalLight", StyleValue::Bool(false));
    draft.set_param(EffectId(1), "angle", StyleValue::Number(10.0));
    assert_eq!(draft.style.global_light, 45.0);
    assert_eq!(draft.style.effect(EffectId(2)).and_then(|e| e.params.number("angle")), Some(45.0));

    draft.set_param(EffectId(1), "useGlobalLight", StyleValue::Bool(true));
    assert_eq!(draft.style.effect(EffectId(1)).and_then(|e| e.params.number("angle")), Some(45.0));
}

// From 120° and 5 px, a drag of (4.5, −2.2) lands on whole degrees and pixels by the offset rule;
// with Use Global Light the shared angle follows; a drag back under 0.5 px keeps the angle.
#[trial]
fn shadow_drag_uses_whole_degrees_and_pixels_and_moves_the_global_angle() {
    let mut draft = LayerStyleDraft::new(style_with(&[(1, EffectKind::DropShadow)]));
    assert!(draft.drags_shadow());
    let defaults = EffectSpec::photoshop(EffectKind::DropShadow).defaults;
    assert!(draft.drag_shadow(&defaults, true, [0.0, 0.0]) || draft.shadow_drag.is_some());

    assert!(draft.drag_shadow(&defaults, false, [4.5, -2.2]));
    let params = &draft.style.effects[0].params;
    assert_eq!((params.number("angle"), params.number("distance")), (Some(163.0), Some(7.0)));
    assert_eq!(draft.style.global_light, 163.0);

    // Back to the press point: the offset is under 0.5 px, so the start angle returns.
    assert!(draft.drag_shadow(&defaults, false, [-2.5, -4.330_127]));
    let params = &draft.style.effects[0].params;
    assert_eq!((params.number("angle"), params.number("distance")), (Some(120.0), Some(0.0)));
    assert_eq!(draft.style.global_light, 120.0);

    // Other pages are not dragged.
    draft.page = LayerStylePage::Blending;
    assert!(!draft.drag_shadow(&defaults, true, [10.0, 10.0]));
}

// An instance without angle and distance starts the drag from the spec's defaults, not from 120°
// and 0.

// The blend defaults of a spec become the host's keys; unknown names stay. Bevel & Emboss has two
// Blend fields: only the one defaulting to Multiply is renamed.

// `BlendIfRange::ordered` and `moved` always give black low ≤ black high ≤ white low ≤ white
// high, pushing the other handles.
#[trial]
fn blend_if_edits_keep_the_four_values_ordered() {
    assert_eq!(BlendIfRange::ordered([10, 5, 200, 100]), BlendIfRange([10, 10, 200, 200]));
    let start = BlendIfRange([10, 30, 200, 220]);
    for handle in 0..4 {
        for to in (0..=255u8).step_by(5) {
            for split in [false, true] {
                let r = start.moved(handle, to, split).0;
                assert!(r.windows(2).all(|w| w[0] <= w[1]), "{handle} {to} {split}: {r:?}");
            }
        }
    }
    // A split move past a neighbour pushes it.
    assert_eq!(start.moved(1, 210, true), BlendIfRange([10, 210, 210, 220]));
    assert_eq!(start.moved(9, 0, false), start);
}

// A `StyleValue::Other` key no field shows survives `set_param`, `toggle` and
// `StyleParams::over` unchanged.

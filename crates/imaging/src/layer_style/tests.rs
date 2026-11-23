//! Tests of the Layer Style model.

use super::*;
use craft_ui::testing::trial;

fn style_with(kinds: &[(u32, EffectKind)]) -> LayerStyle {
    let effects =
        kinds.iter().map(|&(n, kind)| EffectInstance { id: EffectId(n), kind, on: true, params: EffectSpec::photoshop(kind).defaults }).collect();
    LayerStyle { effects, ..LayerStyle::default() }
}

fn set_offset(draft: &mut LayerStyleDraft, n: u32, offset: [f64; 2]) {
    if let Some(e) = draft.style.effect_mut(EffectId(n)) {
        e.params.set("offset", StyleValue::Point(offset));
    }
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

/// A Satin at `angle` / `distance` on its selected page, with a Drop Shadow beside it.
fn satin_draft(angle: f64, distance: f64) -> LayerStyleDraft {
    let mut draft = LayerStyleDraft::new(style_with(&[(1, EffectKind::Satin), (2, EffectKind::DropShadow)]));
    draft.page = LayerStylePage::Effect(EffectId(1));
    draft.set_param(EffectId(1), "angle", StyleValue::Number(angle));
    draft.set_param(EffectId(1), "distance", StyleValue::Number(distance));
    draft
}

fn satin_shown(draft: &LayerStyleDraft) -> (Option<f64>, Option<f64>) {
    let params = &draft.style.effect(EffectId(1)).expect("the Satin").params;
    (params.number("angle"), params.number("distance"))
}

// From Angle 90 / Distance 20, each drag gives the angle and distance measured in the reference
// application: both truncated (not rounded), negative angles toward zero.
#[trial]
fn satin_drag_truncates_the_angle_and_distance() {
    let defaults = EffectSpec::photoshop(EffectKind::Satin).defaults;
    for (total, want) in [
        ([30.0, 0.0], (146.0, 36.0)),
        ([0.0, -30.0], (-90.0, 10.0)),
        ([0.0, 30.0], (90.0, 50.0)),
        ([-30.0, 0.0], (33.0, 36.0)),
        ([20.0, -20.0], (180.0, 20.0)),
        ([5.0, 0.0], (104.0, 20.0)),
        ([5.0, -30.0], (-116.0, 11.0)),
        ([-5.0, -30.0], (-63.0, 11.0)),
        ([30.0, 10.0], (135.0, 42.0)),
    ] {
        let mut draft = satin_draft(90.0, 20.0);
        assert!(draft.drags_satin() && draft.drags_on_canvas());
        draft.drag_satin(&defaults, true, [0.0, 0.0]);
        draft.drag_satin(&defaults, false, total);
        assert_eq!(satin_shown(&draft), (Some(want.0), Some(want.1)), "drag {total:?}");
    }
}

// The zero offset gives Distance 1 and keeps the angle shown, also when one drag passes through it;
// a quarter pixel still has a direction; Distance stays within 1..=250; a non-finite drag and a
// press change nothing, fractions included; a drag back to the press restores the start values.
#[trial]
fn satin_drag_keeps_the_angle_at_zero_and_the_values_without_travel() {
    let defaults = EffectSpec::photoshop(EffectKind::Satin).defaults;
    let mut draft = satin_draft(90.0, 20.0);
    draft.drag_satin(&defaults, true, [0.0, 0.0]);
    draft.drag_satin(&defaults, false, [0.0, -20.0]);
    assert_eq!(satin_shown(&draft), (Some(90.0), Some(1.0)));

    let mut draft = satin_draft(90.0, 20.0);
    draft.drag_satin(&defaults, true, [0.0, 0.0]);
    draft.drag_satin(&defaults, false, [30.0, 0.0]);
    draft.drag_satin(&defaults, false, [0.0, -20.0]);
    assert_eq!(satin_shown(&draft), (Some(146.0), Some(1.0)), "the angle shown, not the press's");

    let mut draft = satin_draft(90.0, 20.0);
    draft.drag_satin(&defaults, true, [0.0, 0.0]);
    draft.drag_satin(&defaults, false, [0.25, -20.0]);
    assert_eq!(satin_shown(&draft), (Some(180.0), Some(1.0)));

    let mut draft = satin_draft(90.0, 20.0);
    draft.drag_satin(&defaults, true, [0.0, 0.0]);
    draft.drag_satin(&defaults, false, [0.0, 380.0]);
    assert_eq!(satin_shown(&draft).1, Some(250.0));
    assert!(!draft.drag_satin(&defaults, true, [f64::NAN, 0.0]));

    let mut draft = satin_draft(90.5, 20.75);
    assert!(!draft.drag_satin(&defaults, true, [0.0, 0.0]), "a press writes nothing");
    assert!(draft.shadow_drag.is_some());
    draft.drag_satin(&defaults, false, [30.0, 0.0]);
    assert_eq!(satin_shown(&draft), (Some(145.0), Some(36.0)));
    draft.drag_satin(&defaults, false, [0.0, 0.0]);
    assert_eq!(satin_shown(&draft), (Some(90.5), Some(20.75)));
    draft.end_drag();
    assert!(draft.shadow_drag.is_none());
}

// A press and release on a Satin whose angle and distance are not stored (they come from the spec
// defaults) leaves its parameters empty; a drag that leaves and comes back does store them.
#[trial]
fn satin_press_without_travel_keeps_a_sparse_instance_sparse() {
    let defaults = EffectSpec::photoshop(EffectKind::Satin).defaults;
    let mut draft = LayerStyleDraft::new(style_with(&[(1, EffectKind::Satin)]));
    draft.page = LayerStylePage::Effect(EffectId(1));
    draft.style.effect_mut(EffectId(1)).expect("the Satin").params = StyleParams::default();
    let before = draft.style.clone();
    assert!(!draft.drag_satin(&defaults, true, [0.0, 0.0]));
    assert!(!draft.drag_satin(&defaults, false, [0.0, 0.0]));
    draft.end_drag();
    assert_eq!(draft.style, before, "the parameter map is unchanged");
    assert_eq!(satin_shown(&draft), (None, None));

    draft.drag_satin(&defaults, true, [0.0, 0.0]);
    assert!(draft.drag_satin(&defaults, false, [30.0, 0.0]));
    assert!(draft.drag_satin(&defaults, false, [0.0, 0.0]), "back to the start values");
}

// Only the Satin page takes the Satin drag; it leaves Global Light and the shadow alone, and the
// shadow drag does not take a Satin page. Shadow, Satin and Gradient Overlay pages take canvas drags.
#[trial]
fn satin_drag_is_the_satin_pages_alone() {
    let satin = EffectSpec::photoshop(EffectKind::Satin).defaults;
    let shadow = EffectSpec::photoshop(EffectKind::DropShadow).defaults;
    let mut draft = satin_draft(90.0, 20.0);
    let light = draft.style.global_light;
    let before = draft.style.effect(EffectId(2)).cloned();
    assert!(!draft.drags_shadow());
    assert!(!draft.drag_shadow(&shadow, true, [30.0, 0.0]));
    assert_eq!(satin_shown(&draft), (Some(90.0), Some(20.0)));
    draft.drag_satin(&satin, true, [0.0, 0.0]);
    assert!(draft.drag_satin(&satin, false, [30.0, 0.0]));
    assert_eq!(draft.style.global_light, light);
    assert_eq!(draft.style.effect(EffectId(2)).cloned(), before);

    draft.end_drag();
    draft.page = LayerStylePage::Effect(EffectId(2));
    assert!(!draft.drags_satin() && draft.drags_on_canvas());
    assert!(!draft.drag_satin(&satin, true, [30.0, 0.0]));
    assert_eq!(draft.style.effect(EffectId(2)).cloned(), before);
    draft.page = LayerStylePage::Blending;
    assert!(!draft.drags_on_canvas());
    let mut gradient = LayerStyleDraft::new(style_with(&[(1, EffectKind::GradientOverlay), (2, EffectKind::ColorOverlay)]));
    gradient.page = LayerStylePage::Effect(EffectId(1));
    assert!(gradient.drags_on_canvas());
    gradient.page = LayerStylePage::Effect(EffectId(2));
    assert!(!gradient.drags_on_canvas());
}

// An instance without angle and distance starts the drag from the spec's defaults, not from 120°
// and 0.

// The blend defaults of a spec become the host's keys; unknown names stay. Bevel & Emboss has two
// Blend fields: only the one defaulting to Multiply is renamed.

// A press writes nothing; a drag moves the offset by travel over the layer size from the shown
// offset; an out-of-range or non-finite stored offset starts from the clamp or the default;
// selecting the other instance mid-press rebases on the travel so far; the result is clamped to
// ±4 and rounded to 0.001; `end_drag` clears the drag.
#[trial]
fn a_gradient_offset_drag_starts_from_the_shown_offset_and_rebases_on_another_instance() {
    let offset = |draft: &LayerStyleDraft, n: u32| draft.style.effect(EffectId(n)).and_then(|e| e.params.get("offset").cloned());
    let size = [200.0, 100.0];
    let mut draft = LayerStyleDraft::new(style_with(&[(1, EffectKind::GradientOverlay), (2, EffectKind::GradientOverlay)]));
    draft.page = LayerStylePage::Effect(EffectId(1));
    assert!(draft.drags_gradient());

    // A press writes nothing; the default offset is shown.
    assert!(!draft.drag_gradient(true, [0.0, 0.0], size));
    assert!(draft.gradient_drag.is_some());
    assert_eq!(offset(&draft, 1), Some(StyleValue::Point([0.0, 0.0])));

    // 50 px over 200 and 10 px over 100.
    assert!(draft.drag_gradient(false, [50.0, 10.0], size));
    assert_eq!(offset(&draft, 1), Some(StyleValue::Point([0.25, 0.1])));

    // Another instance, selected mid-press, starts from its own shown offset at the travel so far.
    set_offset(&mut draft, 2, [1.0, -1.0]);
    draft.page = LayerStylePage::Effect(EffectId(2));
    assert!(!draft.drag_gradient(false, [50.0, 10.0], size));
    assert_eq!(draft.gradient_drag.map(|d| d.instance()), Some(EffectId(2)));
    assert!(draft.drag_gradient(false, [70.0, 10.0], size));
    assert_eq!(offset(&draft, 2), Some(StyleValue::Point([1.1, -1.0])));
    assert_eq!(offset(&draft, 1), Some(StyleValue::Point([0.25, 0.1])));

    // Clamped to ±4 and rounded to 0.001.
    assert!(draft.drag_gradient(false, [100_000.0, -1.0 / 3.0 * 100.0 + 10.0], size));
    assert_eq!(offset(&draft, 2), Some(StyleValue::Point([4.0, -1.333])));

    draft.end_drag();
    assert!(draft.gradient_drag.is_none());

    // A stored offset beyond ±4 starts from the clamp; one that is not finite from the default.
    set_offset(&mut draft, 2, [9.0, 0.0]);
    assert!(!draft.drag_gradient(true, [0.0, 0.0], size));
    assert!(draft.drag_gradient(false, [-20.0, 0.0], size));
    assert_eq!(offset(&draft, 2), Some(StyleValue::Point([3.9, 0.0])));
    draft.end_drag();
    set_offset(&mut draft, 2, [f64::NAN, 0.0]);
    assert!(!draft.drag_gradient(true, [0.0, 0.0], size));
    assert!(matches!(offset(&draft, 2), Some(StyleValue::Point([x, _])) if x.is_nan()));
    assert!(draft.drag_gradient(false, [20.0, 0.0], size));
    assert_eq!(offset(&draft, 2), Some(StyleValue::Point([0.1, 0.0])));

    // Other pages are not dragged.
    draft.page = LayerStylePage::Blending;
    assert!(!draft.drag_gradient(true, [10.0, 10.0], size));
}

// A `StyleValue::Other` key no field shows survives `set_param`, `toggle` and
// `StyleParams::over` unchanged.

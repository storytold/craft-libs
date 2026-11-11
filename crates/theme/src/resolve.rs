//! Resolution: a theme and an appearance to concrete tokens.

use super::app_sizes::AppSizes;
use super::components::ComponentTokens;
use super::derive::{self, ColorSteps, ContrastCheck};
use super::ramp::Mode;
use super::scale::{Easing, Elevation};
use super::sizes;
use super::theme::{Appearance, Contrast, ContrastIntent, PinLayer, Pins, ThemeDef};
use super::tokens::{Edge, ElevationTokens, Motion, MotionTokens, RadiusSet, ShapeTokens, TokenRef, Tokens};
use super::values;
use crate::AppId;

/// The tokens for one theme and appearance, with the contrast pairs that fail.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// Every semantic token and the component tokens.
    pub tokens: Tokens,
    /// Pairs below their minimum: in standard contrast, pairs that a pinned colour makes fail,
    /// or pairs that no lightness move could fix.
    pub warnings: Vec<ContrastCheck>,
    /// Carried from the theme, for the picker's "Low contrast" label and the built-in check.
    pub contrast_intent: ContrastIntent,
    /// The mode whose variant was used; differs from the requested mode for single-variant
    /// themes.
    pub variant_used: Mode,
}

/// Resolves `theme` for `appearance`. Colours derive from the theme's seeds and pins for the
/// mode it has; sizes, spacing and type come from the Size preset and text size; shape comes
/// from the preset unless the theme has its own. Component tokens come from the preset, then take
/// the theme's component overrides. With `app`, that app's overrides in the theme apply over the
/// theme's own pins and components.
pub fn resolve(theme: &ThemeDef, appearance: Appearance, app: Option<&AppId>) -> Resolved {
    resolve_sized(theme, appearance, app, &AppSizes::default())
}

/// [`resolve`] with an app's size overrides: its base overrides apply after the size set and
/// before derivation.
pub fn resolve_sized(theme: &ThemeDef, appearance: Appearance, app: Option<&AppId>, sizes: &AppSizes) -> Resolved {
    resolve_core(theme, appearance, app, sizes).0.resolved
}

/// The intermediate values of [`resolve`], which [`explain`](crate::explain) reports.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Steps {
    pub color: ColorSteps,
}

/// [`resolve`], and the steps on the way.
pub(super) fn resolve_with_steps(theme: &ThemeDef, appearance: Appearance, app: Option<&AppId>, sizes: &AppSizes) -> (Resolved, Steps) {
    let (core, _) = resolve_core(theme, appearance, app, sizes);
    let Core { resolved, color } = core;
    (resolved, Steps { color })
}

/// What [`resolve_concrete`] derives: the resolution and the steps [`resolve`] needs.
struct Core {
    resolved: Resolved,
    color: ColorSteps,
}

/// The resolution of `theme`, and the theme with its references replaced by their values when it
/// had any (the theme the resolution was derived from).
fn resolve_core(theme: &ThemeDef, appearance: Appearance, app: Option<&AppId>, sizes: &AppSizes) -> (Core, Option<ThemeDef>) {
    match settle_references(theme, appearance, app, sizes) {
        Some(Settled { theme: concrete, resolution: Some(done), .. }) => (done, Some(concrete)),
        Some(Settled { theme: concrete, .. }) => (resolve_concrete(&concrete, appearance, app, sizes), Some(concrete)),
        None => (resolve_concrete(theme, appearance, app, sizes), None),
    }
}

/// The references of `theme` for `appearance` whose value still changes when the last round
/// applies them: the ones that depend on themselves through derivation, such as
/// `state.selected = "{state.selected_hover}"` with a hover state derived from the selected one.
pub(super) fn unsettled_references(theme: &ThemeDef, appearance: Appearance, app: Option<&AppId>) -> Vec<(PinLayer, TokenRef)> {
    settle_references(theme, appearance, app, &AppSizes::default()).map(|s| s.unsettled).unwrap_or_default()
}

/// What [`settle_references`] leaves.
struct Settled {
    /// The theme with every reference replaced by a colour pin holding its value.
    theme: ThemeDef,
    /// The references still changing after the last round.
    unsettled: Vec<(PinLayer, TokenRef)>,
    /// The resolution of `theme`, when the last round left it unchanged.
    resolution: Option<Core>,
}

/// Applies each reference as a colour pin holding the value its target has in the final
/// resolution, and resolves again until no reference's value changes, so a reference to a derived
/// token reads what is derived from the other references' values. The first round resolves the
/// theme with the references removed. Every round that changes a value needs the one before it to
/// have changed another, and a value depends on at most every other reference, so the rounds stop
/// after one more than the number of references; a reference still changing then is returned
/// with the pins of the last round. `None` when no reference applies.
fn settle_references(theme: &ThemeDef, appearance: Appearance, app: Option<&AppId>, sizes: &AppSizes) -> Option<Settled> {
    let mode = theme.variants.mode_for(appearance.mode);
    let high = appearance.contrast == Contrast::High;
    let has_app = app.is_some_and(|a| theme.apps.contains_key(a));
    let layers: Vec<PinLayer> = [
        Some(PinLayer::Theme),
        has_app.then_some(PinLayer::App),
        high.then_some(PinLayer::ThemeHighContrast),
        (has_app && high).then_some(PinLayer::AppHighContrast),
    ]
    .into_iter()
    .flatten()
    .collect();

    // The reference each token ends with: a later layer's pin or reference replaces an earlier one.
    let mut pending: Vec<(PinLayer, TokenRef, TokenRef)> = Vec::new();
    for &layer in &layers {
        let pins = layer_pins(theme, layer, app, mode)?;
        pending.retain(|p| !pins.colors.contains_key(&p.1) && !pins.refs.contains_key(&p.1));
        pending.extend(pins.refs.iter().map(|(&k, &t)| (layer, k, t)));
    }
    if pending.is_empty() {
        return None;
    }

    let mut work = theme.clone();
    for pins in all_pins_mut(&mut work) {
        pins.refs.clear();
    }
    let mut unsettled = Vec::new();
    let mut resolution = None;
    for _ in 0..=pending.len() + 1 {
        let core = resolve_concrete(&work, appearance, app, sizes);
        let tokens = &core.resolved.tokens;
        unsettled.clear();
        for &(layer, key, target) in &pending {
            let value = match target {
                TokenRef::Color(t) => Some(tokens.color(t)),
                TokenRef::Component(_) => None,
            };
            if let (Some(value), Some(pins)) = (value, layer_pins_mut(&mut work, layer, app, mode))
                && pins.colors.insert(key, value) != Some(value)
            {
                unsettled.push((layer, key));
            }
        }
        if unsettled.is_empty() {
            resolution = Some(core);
            break;
        }
    }
    Some(Settled { theme: work, unsettled, resolution })
}

fn layer_pins<'a>(theme: &'a ThemeDef, layer: PinLayer, app: Option<&AppId>, mode: Mode) -> Option<&'a Pins> {
    let app = app.and_then(|a| theme.apps.get(a));
    Some(match layer {
        PinLayer::Theme => theme.pins.get(mode),
        PinLayer::ThemeHighContrast => theme.high_contrast.get(mode),
        PinLayer::App => app?.pins.get(mode),
        PinLayer::AppHighContrast => app?.high_contrast.get(mode),
    })
}

fn layer_pins_mut<'a>(theme: &'a mut ThemeDef, layer: PinLayer, app: Option<&AppId>, mode: Mode) -> Option<&'a mut Pins> {
    let app = app.and_then(|a| theme.apps.get_mut(a));
    let modes = match layer {
        PinLayer::Theme => &mut theme.pins,
        PinLayer::ThemeHighContrast => &mut theme.high_contrast,
        PinLayer::App => &mut app?.pins,
        PinLayer::AppHighContrast => &mut app?.high_contrast,
    };
    Some(match mode {
        Mode::Light => &mut modes.light,
        Mode::Dark => &mut modes.dark,
    })
}

fn all_pins_mut(theme: &mut ThemeDef) -> impl Iterator<Item = &mut Pins> {
    let apps = theme.apps.values_mut().flat_map(|a| [&mut a.pins, &mut a.high_contrast]);
    [&mut theme.pins, &mut theme.high_contrast].into_iter().chain(apps).flat_map(|m| [&mut m.light, &mut m.dark])
}

fn resolve_concrete(theme: &ThemeDef, appearance: Appearance, app: Option<&AppId>, sizes: &AppSizes) -> Core {
    let mode = theme.variants.mode_for(appearance.mode);
    let (color, color_steps) = derive::colors(theme, app, mode, appearance.contrast);
    let warnings: Vec<ContrastCheck> = color_steps.warnings().collect();
    let set = sizes::builtin(appearance.size);
    let mut preset = set.base;
    sizes.apply_base(appearance.size, &mut preset);

    let seed = theme.shape;
    let radius = seed.and_then(|s| s.radius).unwrap_or(preset.radius);
    let radius = seed.and_then(|s| s.radius_offset).map_or(radius, |o| o.apply(radius));
    let radius = RadiusSet { button: seed.and_then(|s| s.button_radius).unwrap_or(radius.button), ..radius };
    let edge = seed.and_then(|s| s.edge).map_or(preset.edge, |e| e.resolve(&color));

    // Geometry per use, colour per theme: every cast shadow has `surface.shadow`'s alpha.
    let shadow_alpha = f32::from(color.surface.shadow.a()) / 255.0;
    let popup = Elevation { alpha: shadow_alpha, ..values::POPUP_SHADOW };
    let floating = Elevation { alpha: shadow_alpha, ..values::WINDOW_SHADOW };
    // A bevel is the relief of a raised control, so a bevelled knob lies flat.
    let knob = if matches!(edge, Edge::Bevel { .. }) { Elevation::level(0) } else { values::KNOB_SHADOW };

    let elevation = ElevationTokens { popup, floating, knob };

    let motion = |d: super::scale::Duration| Motion { millis: if appearance.reduced_motion { 0 } else { d.millis() }, easing: values::MOTION_EASING };
    let (hover, open, move_) = values::MOTION_DURATIONS;
    let motion_ms = |millis: u32, easing: Easing| Motion { millis: if appearance.reduced_motion { 0 } else { millis }, easing };

    let radius = RadiusSet { sm: (radius.control / 2.0).round(), ..radius };
    let shape = ShapeTokens { radius, edge, rows: seed.and_then(|s| s.rows).unwrap_or(preset.rows) };
    let typography = preset.typography.scaled(sizes.text_steps.factor(appearance.text_size), values::TYPE_ROUNDING);
    let mut component = ComponentTokens::defaults(&preset);
    for (&field, &length) in theme.components.iter().chain(app.and_then(|a| theme.apps.get(a)).map(|a| &a.components).into_iter().flatten()) {
        component.set(field, length);
    }

    let tokens = Tokens {
        color,
        shape,
        space: preset.space,
        size: preset.size,
        stroke: preset.stroke,
        typography,
        elevation,
        motion: MotionTokens {
            hover: motion(hover),
            open: motion(open),
            r#move: motion(move_),
            caret: motion_ms(values::MOTION_CARET_MS, values::MOTION_CARET_EASING),
            outline: motion_ms(values::MOTION_OUTLINE_MS, values::MOTION_OUTLINE_EASING),
            reduced: appearance.reduced_motion,
        },
        component,
    };
    let resolved = Resolved { tokens, warnings, contrast_intent: theme.contrast_intent, variant_used: mode };
    Core { resolved, color: color_steps }
}

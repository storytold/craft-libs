//! The suite-theme contract as apps see it: built-ins, resolution, derived states and advisory
//! contrast diagnostics.

use std::cell::RefCell;

use craft_theme::{
    APP_DEFAULT, AppChoice, AppDefaults, CoreRoles, Fallback, Mode, Origin, Resolution, Rgba, Role, RoleMap, SUITE_DEFAULT, Selection, SuiteTheme,
    ThemeId, Variant,
    contrast::{self, Need, Paint},
    derive, lookup_builtin, resolve,
};
use proptest::prelude::*;

fn id(s: &str) -> ThemeId {
    ThemeId::parse(s).unwrap()
}

fn role_map(roles: &CoreRoles) -> RoleMap {
    Role::ALL.into_iter().map(|r| (r.key().to_owned(), roles.get(r))).collect()
}

fn graphite(v: Variant) -> CoreRoles {
    *craft_theme::builtin(&id("craft.graphite")).unwrap().variant(v).unwrap()
}

#[test]
fn builtins_have_both_variants_and_no_diagnostics() {
    let ids: Vec<_> = craft_theme::builtins().iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["craft.artcraft", "craft.graphite", "craft.high-contrast"]);
    for t in craft_theme::builtins() {
        assert_eq!(t.origin, Origin::BuiltIn);
        assert_eq!(ThemeId::parse(t.id.as_str()).as_ref(), Ok(&t.id));
        assert!(t.id.is_builtin() && !t.id.is_app_default());
        assert!(t.light.is_some() && t.dark.is_some(), "{}", t.id);
        for (v, roles) in t.variants() {
            assert!(Role::ALL.iter().all(|&r| roles.get(r).is_opaque()), "{} {v:?}", t.id);
        }
        // Covers every derived state in the painted-pair table, hover and pressed included.
        assert_eq!(contrast::diagnose(t), [], "{}", t.id);
    }
}

#[test]
fn effective_selection() {
    let defaults = AppDefaults { theme: ThemeId::suite_default(), mode: Mode::Dark };
    let suite = Selection { theme: id("craft.graphite"), mode: Mode::Light };
    let own = Selection { theme: ThemeId::app_default(), mode: Mode::System };
    let follow = AppChoice::FollowSuite;
    // No suite selection yet: a following app shows its own defaults.
    assert_eq!(follow.effective(None, &defaults), Selection { theme: id(SUITE_DEFAULT), mode: Mode::Dark });
    assert_eq!(follow.effective(Some(&suite), &defaults), suite);
    assert_eq!(AppChoice::Own(own.clone()).effective(None, &defaults), own);
    assert_eq!(AppChoice::Own(own.clone()).effective(Some(&suite), &defaults), own);
    assert!(craft_theme::builtin(&ThemeId::suite_default()).is_some());
}

/// Fails to compile if `T: Default` (the method lookup becomes ambiguous): an app's
/// `#[serde(default)]` must not invent a choice or a suite selection.
#[test]
fn choices_and_selections_have_no_default() {
    trait AmbiguousIfDefault<A> {
        fn check() {}
    }
    impl<T: ?Sized> AmbiguousIfDefault<()> for T {}
    struct IsDefault;
    impl<T: Default> AmbiguousIfDefault<IsDefault> for T {}
    <AppChoice as AmbiguousIfDefault<_>>::check();
    <Selection as AmbiguousIfDefault<_>>::check();
    <AppDefaults as AmbiguousIfDefault<_>>::check();
}

/// One user theme with only a dark variant, alongside the built-ins.
fn lookup_with_dark_only(id: &ThemeId) -> Option<SuiteTheme> {
    if id.as_str() == "night" {
        let dark = role_map(&graphite(Variant::Dark));
        return Some(SuiteTheme::from_maps("night", "Night", 1, None, Some(&dark)).unwrap().0);
    }
    lookup_builtin(id)
}

#[test]
fn resolution_table() {
    let themes = [APP_DEFAULT, "craft.artcraft", "craft.graphite", "craft.high-contrast", "night", "gone"];
    let systems = [None, Some(Variant::Light), Some(Variant::Dark)];
    for theme in themes {
        for mode in Mode::ALL {
            for system in systems {
                for fallback in Variant::ALL {
                    let sel = Selection { theme: id(theme), mode };
                    let calls = RefCell::new(Vec::new());
                    let lookup = |i: &ThemeId| {
                        calls.borrow_mut().push(i.clone());
                        lookup_with_dark_only(i)
                    };
                    let got = resolve(&sel, system, fallback, &lookup);
                    let case = format!("{theme} {mode:?} {system:?} {fallback:?}");
                    // Pure: the same inputs give the same answer, and only the selected id is looked up.
                    assert_eq!(resolve(&sel, system, fallback, &lookup), got, "{case}");
                    assert!(calls.borrow().iter().all(|i| *i == sel.theme), "{case}");

                    let wanted = match mode {
                        Mode::Light => Variant::Light,
                        Mode::Dark => Variant::Dark,
                        Mode::System => system.unwrap_or(fallback),
                    };
                    let silent = (mode == Mode::System && system.is_none()).then_some(Fallback::SystemSilent);
                    match (theme, &got) {
                        (APP_DEFAULT, Resolution::AppDefault { variant, why }) => {
                            assert_eq!((*variant, why), (wanted, &silent), "{case}");
                        }
                        ("gone", Resolution::AppDefault { variant, why }) => {
                            assert_eq!((*variant, why), (wanted, &Some(Fallback::UnknownTheme(id("gone")))), "{case}");
                        }
                        ("night", Resolution::Theme(r)) => {
                            assert_eq!(r.variant, Variant::Dark, "{case}");
                            let note = if wanted == Variant::Dark { silent } else { Some(Fallback::OnlyVariant(Variant::Dark)) };
                            assert_eq!(r.note, note, "{case}");
                            assert_eq!(r.roles, graphite(Variant::Dark), "{case}");
                        }
                        (_, Resolution::Theme(r)) if id(theme).is_builtin() => {
                            let builtin = craft_theme::builtin(&r.theme).unwrap();
                            assert_eq!((r.theme.as_str(), r.variant, &r.note), (theme, wanted, &silent), "{case}");
                            assert_eq!(&r.roles, builtin.variant(wanted).unwrap(), "{case}");
                            assert_eq!(r.derived, derive(&r.roles, r.variant), "{case}");
                        }
                        _ => panic!("{case}: {got:?}"),
                    }
                }
            }
        }
    }
}

#[test]
fn low_contrast_user_theme_is_reported_but_kept() {
    let mut light = graphite(Variant::Light);
    light.text = light.surface;
    let (theme, missing) = SuiteTheme::from_maps("flat", "Flat", 1, Some(&role_map(&light)), None).unwrap();
    assert_eq!(missing, []);
    let diagnostics = contrast::diagnose(&theme);
    let flat = diagnostics.iter().find(|d| d.fg == Paint::Role(Role::Text) && d.bg == Paint::Role(Role::Surface)).unwrap();
    assert_eq!((flat.variant, flat.need), (Variant::Light, Need::Text));
    assert!((flat.ratio - 1.0).abs() < 1e-6);
    assert_eq!(theme.light, Some(light));

    let sel = Selection { theme: theme.id.clone(), mode: Mode::Light };
    let Resolution::Theme(r) = resolve(&sel, None, Variant::Light, &|_| Some(theme.clone())) else { panic!() };
    assert_eq!((r.roles, r.note), (light, None));
}

fn lighter(a: Rgba, b: Rgba) -> bool {
    contrast::ratio(a, Rgba::BLACK) > contrast::ratio(b, Rgba::BLACK)
}

#[test]
fn derived_states_step_away_from_text_when_toward_would_fail() {
    // ArtCraft dark: hover and pressed lighten toward text; lightening the mid-grey selection
    // would take text below 4.5:1 and text_muted below 3.5:1, so selected_hover darkens.
    let artcraft = craft_theme::builtin(&ThemeId::suite_default()).unwrap().dark.unwrap();
    let d = derive(&artcraft, Variant::Dark);
    assert_eq!((d.hover.to_hex(), d.pressed.to_hex()), ("#5f5f5f".to_owned(), "#696969".to_owned()));
    assert!(lighter(artcraft.selection, d.selected_hover));
    let light = graphite(Variant::Light);
    assert!(lighter(light.surface, derive(&light, Variant::Light).hover));

    let mut grey = light;
    (grey.background, grey.surface, grey.raised, grey.text, grey.text_muted) =
        (Rgba::from_rgb_u32(0xC4C4C4), Rgba::from_rgb_u32(0xC4C4C4), Rgba::from_rgb_u32(0xC4C4C4), Rgba::BLACK, Rgba::from_rgb_u32(0x606060));
    let d = derive(&grey, Variant::Light);
    assert!(lighter(d.hover, grey.surface) && lighter(d.pressed, grey.surface));
    for bg in [d.hover, d.pressed] {
        assert!(contrast::ratio(grey.text_muted, bg) >= 3.5);
    }
}

fn channel_range(lo: u8, hi: u8) -> impl Strategy<Value = Rgba> {
    [lo..=hi, lo..=hi, lo..=hi].prop_map(|[r, g, b]| Rgba::rgb(r, g, b))
}

fn opaque() -> impl Strategy<Value = Rgba> {
    channel_range(0, 255)
}

/// Mostly plausible themes, dark text on light backgrounds or light on dark, so the
/// preconditions below hold often; the other roles are arbitrary.
fn core_roles() -> impl Strategy<Value = CoreRoles> {
    let shaped = |(back_lo, back_hi), (ink_lo, ink_hi)| {
        (
            [channel_range(back_lo, back_hi), channel_range(back_lo, back_hi), channel_range(back_lo, back_hi), channel_range(back_lo, back_hi)],
            [channel_range(ink_lo, ink_hi), channel_range(ink_lo, ink_hi)],
            [opaque(), opaque(), opaque(), opaque(), opaque(), opaque(), opaque()],
        )
            .prop_map(|([background, surface, raised, selection], [text, text_muted], [line, accent, on_accent, focus, good, warn, bad])| {
                CoreRoles { background, surface, raised, text, text_muted, line, accent, on_accent, selection, focus, good, warn, bad }
            })
    };
    prop_oneof![shaped((170, 255), (0, 110)), shaped((0, 90), (150, 255))]
}

/// [`Rgba::lerp_gamma`] with `-t`, as the derived-state guard steps away from `text`.
fn step_away(base: Rgba, from: Rgba, t: f32) -> Rgba {
    let ch = |a: u8, b: u8| (f32::from(a) - (f32::from(b) - f32::from(a)) * t + 0.5).floor().clamp(0.0, 255.0) as u8;
    Rgba::rgb(ch(base.r(), from.r()), ch(base.g(), from.g()), ch(base.b(), from.b()))
}

/// Whether `candidate`, painted as the derived background `name`, meets every pair on it.
fn meets_pairs_on(name: &'static str, candidate: Rgba, roles: &CoreRoles) -> bool {
    contrast::painted_pairs().iter().filter(|p| p.bg == Paint::Derived(name)).all(|p| {
        let Paint::Role(fg) = p.fg else { return true };
        p.need.is_met(contrast::ratio(roles.get(fg), candidate))
    })
}

proptest! {
    #[test]
    fn derived_states_keep_pairs_their_core_roles_allow(roles in core_roles(), dark in any::<bool>()) {
        let v = if dark { Variant::Dark } else { Variant::Light };
        let map = role_map(&roles);
        let (light, dark_map) = if dark { (None, Some(&map)) } else { (Some(&map), None) };
        let theme = SuiteTheme::from_maps("random", "Random", 1, light, dark_map).unwrap().0;
        let failing: Vec<_> = contrast::diagnose(&theme).into_iter().map(|d| (d.fg, d.bg)).collect();
        let fails = |name: &'static str| failing.iter().any(|&(fg, bg)| fg == Paint::Derived(name) || bg == Paint::Derived(name));
        let text_ok = |fg| [roles.background, roles.surface, roles.raised].iter().all(|&bg| contrast::ratio(fg, bg) >= 4.5);
        if text_ok(roles.text) {
            prop_assert!(!fails("accent_text") && !fails("control_edge"));
        }
        if text_ok(roles.text_muted) {
            prop_assert!(!fails("text_faint"));
        }
        if contrast::ratio(roles.on_accent, roles.accent) >= 4.5 {
            prop_assert!(!fails("accent_hover") && !fails("accent_pressed"));
        }
        // The guard: if a step toward text or the same step away meets every pair, the state does.
        let (hover, pressed) = if dark { (0.08, 0.14) } else { (0.06, 0.12) };
        for (name, base, t) in [("hover", roles.surface, hover), ("pressed", roles.surface, pressed), ("selected_hover", roles.selection, 0.08)] {
            let toward = base.lerp_gamma(roles.text, t);
            if meets_pairs_on(name, toward, &roles) || meets_pairs_on(name, step_away(base, roles.text, t), &roles) {
                prop_assert!(!fails(name), "{}", name);
            }
        }
        // Resolution is pure and hands back the theme's colours with `derive`'s states.
        let sel = Selection { theme: theme.id.clone(), mode: if dark { Mode::Dark } else { Mode::Light } };
        let lookup = |_: &ThemeId| Some(theme.clone());
        let got = resolve(&sel, None, v, &lookup);
        prop_assert_eq!(&got, &resolve(&sel, None, v, &lookup));
        let Resolution::Theme(r) = got else { return Err(TestCaseError::fail("not a theme")) };
        prop_assert_eq!((r.variant, r.note), (v, None));
        prop_assert_eq!(r.roles, roles);
        prop_assert_eq!(r.derived, derive(&roles, v));
        prop_assert_eq!(r.derived.accent_soft, roles.surface.lerp_gamma(roles.accent, 0.14));
    }
}

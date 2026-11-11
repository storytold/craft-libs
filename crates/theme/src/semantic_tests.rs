#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use super::*;
use crate::ThemeId;
use ColorToken as T;

pub(super) fn hex(s: &str) -> Rgba {
    Rgba::from_hex(s).unwrap()
}

fn theme(id: &str, neutral: &str, accent: &str, status: [&str; 4]) -> ThemeDef {
    ThemeDef {
        id: ThemeId::parse(id).unwrap(),
        name: id.to_owned(),
        version: 1,
        seeds: Seeds {
            neutral: hex(neutral),
            accent: hex(accent),
            status: Status { good: hex(status[0]), warn: hex(status[1]), bad: hex(status[2]), info: hex(status[3]) },
        },
        mode_seeds: ModeSeeds::default(),
        shape: None,
        variants: Variants::Both,
        pins: ModePins::default(),
        high_contrast: ModePins::default(),
        components: BTreeMap::new(),
        contrast_intent: ContrastIntent::Standard,
        apps: BTreeMap::new(),
    }
}

pub(super) fn grey_blue() -> ThemeDef {
    theme("test.grey-blue", "#8b8d98", "#3e63dd", ["#30a46c", "#ffc53d", "#e5484d", "#0090ff"])
}

pub(super) fn violet() -> ThemeDef {
    theme("test.violet", "#7a6b9e", "#9b30ff", ["#00c853", "#ffab00", "#ff1744", "#2979ff"])
}

/// Text pinned far too faint in both modes.
pub(super) fn pinned_faint() -> ThemeDef {
    let mut t = grey_blue();
    t.id = ThemeId::parse("test.pinned-faint").unwrap();
    t.pins.light = Pins::from_iter([(T::TextFaint, hex("#c8c8c8"))]);
    t.pins.dark = Pins::from_iter([(T::TextFaint, hex("#404040"))]);
    t
}

pub(super) fn appearances() -> Vec<Appearance> {
    let mut out = Vec::new();
    for mode in [Mode::Light, Mode::Dark] {
        for contrast in [Contrast::Standard, Contrast::High] {
            for size in [Size::Compact, Size::Comfortable] {
                for text_size in TextSize::ALL {
                    out.push(Appearance { mode, contrast, size, text_size, reduced_motion: false });
                }
            }
        }
    }
    out
}

type Resolution = (Resolved, resolve::Steps);

/// Every built-in in every appearance of [`appearances`], indexed theme-major. The first test to
/// ask resolves all of them on every core; the others wait for it instead of repeating the work.
static BUILTIN_RESOLVED: std::sync::LazyLock<(Vec<Appearance>, Vec<Resolution>)> = std::sync::LazyLock::new(|| {
    let all = appearances();
    let jobs: Vec<_> = crate::builtins().iter().flat_map(|theme| all.iter().map(move |&a| (theme, a))).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done: std::sync::Mutex<Vec<(usize, Resolution)>> = std::sync::Mutex::default();
    std::thread::scope(|scope| {
        for _ in 0..std::thread::available_parallelism().map_or(4, usize::from) {
            scope.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&(theme, a)) = jobs.get(i) else { break };
                    let r = resolve::resolve_with_steps(theme, a, None, &AppSizes::default());
                    done.lock().unwrap().push((i, r));
                }
            });
        }
    });
    let mut done = done.into_inner().unwrap();
    done.sort_by_key(|d| d.0);
    (all, done.into_iter().map(|d| d.1).collect())
});

/// [`resolve::resolve_with_steps`], answered from [`BUILTIN_RESOLVED`] for a built-in theme (the
/// very reference `builtins()` hands out, not a modified copy) and an appearance of
/// [`appearances`].
pub(super) fn resolve_with_steps(theme: &ThemeDef, appearance: Appearance, app: Option<&AppId>) -> (Resolved, resolve::Steps) {
    let index = crate::builtins().iter().position(|b| std::ptr::eq(b, theme));
    let (all, slots) = &*BUILTIN_RESOLVED;
    match (index, app, all.iter().position(|&a| a == appearance)) {
        (Some(t), None, Some(a)) => slots[t * all.len() + a].clone(),
        _ => resolve::resolve_with_steps(theme, appearance, app, &AppSizes::default()),
    }
}

/// [`super::resolve`], shared like [`resolve_with_steps`].
pub(super) fn resolve(theme: &ThemeDef, appearance: Appearance, app: Option<&AppId>) -> Resolved {
    resolve_with_steps(theme, appearance, app).0
}

pub(super) fn app(mode: Mode, contrast: Contrast) -> Appearance {
    Appearance { mode, contrast, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false }
}

pub(super) fn over(fg: Rgba, bg: Rgba) -> Rgba {
    let a = f64::from(fg.a()) / 255.0;
    let ch = |i: usize| (f64::from(fg.0[i]) * a + f64::from(bg.0[i]) * (1.0 - a)).round() as u8;
    Rgba::rgb(ch(0), ch(1), ch(2))
}

const TEXT_BGS: [T; 7] =
    [T::SurfaceBackground, T::SurfacePanel, T::SurfaceChrome, T::SurfaceRaised, T::SurfaceSunken, T::StateHover, T::StatePressed];
const CONTROL_BGS: [T; 3] = [T::SurfacePanel, T::SurfaceRaised, T::SurfaceSunken];

/// The §3 pairs, written out independently of the pass's own table: (fg, bg, standard, high).
fn spec_pairs() -> Vec<(T, T, f64, f64)> {
    let mut p = Vec::new();
    for fg in [T::TextNormal, T::TextMuted, T::TextFaint] {
        p.extend(TEXT_BGS.map(|bg| (fg, bg, 4.5, 7.0)));
    }
    p.extend([T::StateSelected, T::StateSelectedHover].map(|bg| (T::TextOnSelected, bg, 4.5, 7.0)));
    p.extend(CONTROL_BGS.map(|bg| (T::AccentText, bg, 4.5, 7.0)));
    p.push((T::AccentOnSoft, T::AccentSoft, 4.5, 7.0));
    p.extend([T::AccentSolid, T::AccentHover, T::AccentPressed].map(|bg| (T::TextOnAccent, bg, 4.5, 7.0)));
    p.extend([T::AccentStrong, T::AccentStrongHover, T::AccentStrongPressed].map(|bg| (T::TextOnStrong, bg, 4.5, 7.0)));
    for (fg, bg, solids) in [
        (T::StatusGoodFg, T::StatusGoodBg, [T::StatusGoodSolid, T::StatusGoodSolidHover, T::StatusGoodSolidPressed]),
        (T::StatusWarnFg, T::StatusWarnBg, [T::StatusWarnSolid, T::StatusWarnSolidHover, T::StatusWarnSolidPressed]),
        (T::StatusBadFg, T::StatusBadBg, [T::StatusBadSolid, T::StatusBadSolidHover, T::StatusBadSolidPressed]),
        (T::StatusInfoFg, T::StatusInfoBg, [T::StatusInfoSolid, T::StatusInfoSolidHover, T::StatusInfoSolidPressed]),
    ] {
        p.extend([T::SurfacePanel, T::SurfaceRaised, bg].map(|b| (fg, b, 4.5, 7.0)));
        p.extend(solids.map(|solid| (T::TextOnStatus, solid, 4.5, 7.0)));
    }
    p.extend([T::SurfaceBackground, T::SurfacePanel, T::SurfaceChrome, T::SurfaceRaised, T::SurfaceSunken].map(|bg| (T::LineFocus, bg, 3.0, 4.5)));
    p.extend([T::SurfacePanel, T::SurfaceRaised, T::SurfaceSunken].map(|bg| (T::AccentSolid, bg, 3.0, 4.5)));
    p
}

fn assert_meets_spec(name: &str, r: &Resolved, contrast: Contrast) {
    let t = &r.tokens;
    let high = contrast == Contrast::High;
    for (fg, bg, std, hi) in spec_pairs() {
        let min = if high { hi } else { std };
        let ratio = contrast_ratio(t.color(fg), t.color(bg));
        assert!(ratio >= min, "{name} {contrast:?}: {} on {} is {ratio:.2}, needs {min}", fg.path(), bg.path());
    }
    let min = if high { 4.5 } else { 3.0 };
    for backdrop in [Rgba::BLACK, Rgba::WHITE] {
        let halo = over(t.color(T::OverlayHalo), backdrop);
        let ratio = contrast_ratio(t.color(T::OverlayStroke), halo);
        assert!(ratio >= min, "{name} {contrast:?}: overlay.stroke on halo over {backdrop:?} is {ratio:.2}");
    }
    let ratio = contrast_ratio(t.color(T::OverlayHandleStroke), t.color(T::OverlayHandleFill));
    assert!(ratio >= min, "{name} {contrast:?}: overlay.handle_stroke on overlay.handle_fill is {ratio:.2}");
}

#[test]
fn samples_resolve_and_derived_tokens_meet_every_minimum() {
    for def in [grey_blue(), violet(), pinned_faint()] {
        let pinned = def.id == pinned_faint().id;
        for a in appearances() {
            let r = resolve(&def, a, None);
            assert_eq!(r.variant_used, a.mode);
            if pinned && a.contrast == Contrast::Standard {
                continue;
            }
            if a.contrast == Contrast::High {
                assert!(r.warnings.is_empty(), "{} {a:?}: {:?}", def.name, r.warnings);
            }
            assert_meets_spec(&def.name, &r, a.contrast);
        }
    }
}

/// A pull for one pair never breaks another on the same background: a dark focus line and light
/// text pinned on a mid-tone panel both pass on the pulled raised surface; when no lightness
/// serves both, the earlier rule (focus) keeps its pair and the text is reported.
#[test]
fn a_later_pull_never_breaks_an_earlier_pair() {
    let focus = hex("#0a246a");
    let fails = |r: &Resolved, fg: T, bg: T| r.warnings.iter().any(|w| w.fg == fg.into() && w.bg == bg.into());
    let mut def = grey_blue();
    def.id = ThemeId::parse("test.dark-focus").unwrap();
    def.pins.dark = Pins::from_iter([(T::SurfacePanel, hex("#747474")), (T::LineFocus, focus), (T::TextNormal, Rgba::WHITE)]);
    let e = explain(&def, app(Mode::Dark, Contrast::Standard), None);
    assert_eq!(e.color(T::SurfaceRaised).unwrap().pass_move, Some(PassMove::Pulled(T::TextNormal)));
    for bg in [T::SurfaceRaised, T::SurfaceSunken] {
        assert!(!fails(&e.resolved, T::LineFocus, bg) && !fails(&e.resolved, T::TextNormal, bg), "{bg:?}: {:?}", e.resolved.warnings);
    }

    def.pins.dark = Pins::from_iter([(T::SurfacePanel, hex("#6a6a6a")), (T::LineFocus, focus), (T::TextNormal, hex("#f4f4f4"))]);
    let r = resolve(&def, app(Mode::Dark, Contrast::Standard), None);
    assert!(!fails(&r, T::LineFocus, T::SurfaceRaised), "the earlier pair keeps passing: {:?}", r.warnings);
    assert!(fails(&r, T::TextNormal, T::SurfaceRaised), "{:?}", r.warnings);
}

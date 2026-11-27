#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

fn appearance(mode: Mode, contrast: Contrast, size: Size) -> Appearance {
    Appearance { mode, contrast, size, text_size: TextSize::Small, reduced_motion: false }
}

#[test]
fn explained_resolution_is_resolve() {
    // One thread per theme: an explanation resolves again with its steps.
    std::thread::scope(|scope| {
        for theme in builtins() {
            scope.spawn(move || {
                for mode in [Mode::Light, Mode::Dark] {
                    for contrast in [Contrast::Standard, Contrast::High] {
                        for size in [Size::Compact, Size::Comfortable] {
                            let a = appearance(mode, contrast, size);
                            let e = explain(theme, a, None);
                            assert_eq!(e.resolved, resolve(theme, a, None), "{} {a:?}", theme.id);
                            assert_eq!(e.colors.iter().map(|o| o.token).collect::<Vec<_>>(), ColorToken::ALL);
                            for o in &e.colors {
                                assert_eq!(o.value, e.resolved.tokens.color(o.token), "{} {a:?} {}", theme.id, o.token.path());
                            }
                        }
                    }
                }
            });
        }
    });
}

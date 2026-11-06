//! The built-in themes. `craft.app-default` is not one of them: it stands for each app's own
//! palette, and the store and settings UI list it.

use std::sync::LazyLock;

use crate::{CoreRoles, Origin, Rgba, SuiteTheme, ThemeId};

pub(crate) mod artcraft;
pub(crate) mod graphite;
pub(crate) mod high_contrast;

const fn hex(rgb: u32) -> Rgba {
    Rgba::from_rgb_u32(rgb)
}

fn theme(id: &'static str, name: &str, light: CoreRoles, dark: CoreRoles) -> SuiteTheme {
    SuiteTheme { id: ThemeId::from_static(id), name: name.to_owned(), version: 1, light: Some(light), dark: Some(dark), origin: Origin::BuiltIn }
}

static BUILTINS: LazyLock<[SuiteTheme; 3]> = LazyLock::new(|| {
    [
        theme(artcraft::ID, artcraft::NAME, artcraft::LIGHT, artcraft::DARK),
        theme(graphite::ID, graphite::NAME, graphite::LIGHT, graphite::DARK),
        theme(high_contrast::ID, high_contrast::NAME, high_contrast::LIGHT, high_contrast::DARK),
    ]
});

/// ArtCraft, Graphite and High contrast, in the order a theme picker lists them.
pub fn builtins() -> &'static [SuiteTheme] {
    &*BUILTINS
}

pub fn builtin(id: &ThemeId) -> Option<&'static SuiteTheme> {
    builtins().iter().find(|t| t.id == *id)
}

/// A `lookup` for [`crate::resolve`] that knows only the built-ins (apps without a theme store,
/// such as web builds, and tests).
pub fn lookup_builtin(id: &ThemeId) -> Option<SuiteTheme> {
    builtin(id).cloned()
}

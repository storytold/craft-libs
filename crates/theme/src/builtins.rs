//! The built-in themes: ten `craft-design-theme` files in `crates/theme/themes/`, embedded in
//! the binary and parsed once.

use std::sync::LazyLock;

use super::ThemeDef;
use super::format;
use crate::SUITE_DEFAULT;

/// Each built-in's file name (without `.json`) and its text.
pub(super) const SOURCES: [(&str, &str); 10] = [
    ("artcraft", include_str!("../themes/artcraft.json")),
    ("pro", include_str!("../themes/pro.json")),
    ("pro-light-gray", include_str!("../themes/pro-light-gray.json")),
    ("graphite", include_str!("../themes/graphite.json")),
    ("studio", include_str!("../themes/studio.json")),
    ("solarized-dark", include_str!("../themes/solarized-dark.json")),
    ("adwaita", include_str!("../themes/adwaita.json")),
    ("breeze", include_str!("../themes/breeze.json")),
    ("classic", include_str!("../themes/classic.json")),
    ("soft", include_str!("../themes/soft.json")),
];

// An invalid embedded file is a build defect that the built-in tests catch.
#[allow(clippy::panic)]
static BUILTINS: LazyLock<Vec<ThemeDef>> = LazyLock::new(|| {
    SOURCES
        .iter()
        .map(|(name, text)| match format::parse(text.as_bytes()) {
            Ok(file) => file.theme,
            Err(errors) => {
                let list: Vec<String> = errors.0.iter().map(ToString::to_string).collect();
                panic!("built-in theme {name}.json is invalid: {}", list.join("; "))
            }
        })
        .collect()
});

/// The built-in themes, in picker order. Panics if an embedded file is invalid, which the
/// built-in tests rule out.
pub fn builtins() -> &'static [ThemeDef] {
    &BUILTINS
}

/// The built-in [`SUITE_DEFAULT`], the last fallback of selection resolution.
// Its absence is a build defect that the built-in tests catch.
#[allow(clippy::expect_used)]
pub(super) fn suite_default() -> &'static ThemeDef {
    builtins().iter().find(|t| t.id.as_str() == SUITE_DEFAULT).expect("SUITE_DEFAULT is a built-in")
}

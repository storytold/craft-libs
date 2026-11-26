//! `Looks`: the app's style replacements (4.4).

use super::Component;
use crate::style::{StyleCx, StyleReplacements};

/// Only apps write `Looks`: a replacement for one component type's style function, keyed by its `StyleKey`.
/// Variants and token groups describe themselves and are not registered. `InstallSpec::looks` freezes it.
#[derive(Default)]
pub struct Looks {
    replacements: StyleReplacements,
}

impl Looks {
    /// No replacements.
    pub const fn new() -> Self {
        Looks { replacements: StyleReplacements::new() }
    }

    /// Replaces the style function of `C` app-wide. Two replacements for one style key are an install error.
    pub fn style<C: Component>(&mut self, f: for<'a> fn(&StyleCx<'a>, C::StyleKey) -> C::Style) -> &mut Self {
        self.replacements.push::<C::StyleKey, C::Style>(f);
        self
    }
}

impl From<Looks> for StyleReplacements {
    fn from(looks: Looks) -> Self {
        looks.replacements
    }
}

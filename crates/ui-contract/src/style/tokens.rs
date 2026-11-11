//! `UiTokens`: the runtime's egui-ready view of one resolved token set.

use crate::fonts::{SEMIBOLD, StackFacts};
use craft_theme::{Contrast, GroupValues, Size, Tokens, TypeRole};
use egui::{CornerRadius, FontFamily, FontId};
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

/// One resolved token set with the theme's token-group values and a key of their content. Each theme scope
/// (mode, contrast, theme, density, text size) resolves its own; equal sets share a key, so the caches keyed by
/// it hold one entry per distinct set however often a set is rebuilt. The set remembers the contrast and density it
/// was resolved under, and the key covers both. It derefs to the craft-theme tokens, which
/// only `style` reads.
#[derive(Clone, Debug)]
pub struct UiTokens {
    tokens: Arc<Tokens>,
    groups: Arc<GroupValues>,
    contrast: Contrast,
    density: Size,
    key: u64,
}

impl UiTokens {
    /// A token set resolved under `contrast` and `density`, with no theme values for token groups.
    pub fn new(tokens: Arc<Tokens>, contrast: Contrast, density: Size) -> Self {
        let groups = Arc::new(GroupValues::new());
        UiTokens { key: content_key(&tokens, &groups, contrast, density), tokens, groups, contrast, density }
    }

    /// The contrast setting the set was resolved under.
    pub(crate) fn contrast(&self) -> Contrast {
        self.contrast
    }

    /// The density preset the set was resolved under.
    pub(crate) fn density(&self) -> Size {
        self.density
    }

    /// The source tokens.
    pub fn tokens(&self) -> &Arc<Tokens> {
        &self.tokens
    }

    /// The theme's token-group values.
    pub fn groups(&self) -> &GroupValues {
        &self.groups
    }

    /// Equal for equal sets and distinct for different ones; the token-group and style caches key on it.
    pub fn key(&self) -> u64 {
        self.key
    }

    /// The font of `role`, or the mono face for a mono run. The semibold slot draws in the `semibold` family when the font
    /// stack of `facts` has one, and in the proportional family otherwise, so text never lands in a family with no
    /// faces.
    pub(crate) fn font(&self, facts: StackFacts, role: TypeRole, mono: bool) -> FontId {
        let style = if mono { self.typography.mono } else { self.typography.get(role) };
        let family = match style.font {
            craft_theme::FontSlot::Mono => FontFamily::Monospace,
            craft_theme::FontSlot::Ui => FontFamily::Proportional,
            craft_theme::FontSlot::UiSemibold => {
                if facts.semibold {
                    FontFamily::Name(SEMIBOLD.into())
                } else {
                    FontFamily::Proportional
                }
            }
        };
        FontId::new(style.size, family)
    }

    /// The optical size (`opsz`) a run of `role` is set at, in points: its text size, when the OS interface face draws in
    /// the font stack of `facts`. `None` for mono runs and for the suite's faces, which keep their default.
    pub(crate) fn optical_size(&self, facts: StackFacts, role: TypeRole, mono: bool) -> Option<f32> {
        let style = if mono { self.typography.mono } else { self.typography.get(role) };
        (style.font != craft_theme::FontSlot::Mono && facts.system_ui).then_some(style.size)
    }

    /// `px` as a corner radius, the same at every corner. A radius above 255 clamps; egui clamps a corner to half the shape.
    #[doc(hidden)]
    pub fn corner(px: f32) -> CornerRadius {
        CornerRadius::same(px.round().clamp(0.0, 255.0) as u8)
    }
}

/// The key of a token set: the hash of its resolved values, which are all `f32` and so have no `Hash`.
fn content_key(tokens: &Tokens, groups: &GroupValues, contrast: Contrast, density: Size) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{tokens:?}{groups:?}").hash(&mut hasher);
    contrast.hash(&mut hasher);
    density.hash(&mut hasher);
    hasher.finish()
}

impl Deref for UiTokens {
    type Target = Tokens;

    fn deref(&self) -> &Tokens {
        &self.tokens
    }
}

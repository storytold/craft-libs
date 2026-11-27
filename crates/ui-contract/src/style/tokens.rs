//! `UiTokens`: the runtime's egui-ready view of one resolved token set.

use crate::fonts::{SEMIBOLD, StackFacts, TAB_TITLE};
use crate::primitives::IconSet;
use craft_theme::{AppId, AppSizes, Appearance, Contrast, GroupValues, IconPackId, Size, ThemeDef, Tokens, TypeRole, resolve_sized};
use egui::{CornerRadius, FontFamily, FontId};
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::sync::Arc;

/// Runs `log` the first time `key` is seen: lookups repeat on every repaint and every resolve.
pub(crate) fn log_once(key: String, log: impl FnOnce()) {
    static REPORTED: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);
    let mut reported = REPORTED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if reported.get_or_insert_with(Default::default).insert(key) {
        log();
    }
}

/// The set SVG icons draw from under a theme that names `pack`: the default set when it names none, and, with a log
/// line the first time, when it names one that is not installed.
fn theme_icons(pack: Option<&IconPackId>) -> IconSet {
    let Some(pack) = pack else { return IconSet::default() };
    IconSet::of_pack(pack).unwrap_or_else(|| {
        log_once(format!("pack:{pack}"), || log::warn!("icon pack {pack:?} is not installed; the default icons are drawn"));
        IconSet::default()
    })
}

/// One resolved token set with the theme's token-group values and a key of their content. Each theme scope
/// (mode, contrast, theme, density, text size) resolves its own; equal sets share a key, so the caches keyed by
/// it hold one entry per distinct set however often a set is rebuilt. The set remembers the contrast and density it
/// was resolved under and the icon set its theme draws, and the key covers them. It derefs to the craft-theme tokens, which
/// only `style` reads.
#[derive(Clone, Debug)]
pub struct UiTokens {
    tokens: Arc<Tokens>,
    groups: Arc<GroupValues>,
    contrast: Contrast,
    density: Size,
    icons: IconSet,
    key: u64,
}

/// Two sets are equal when their content keys are: equal sets share a key.
impl PartialEq for UiTokens {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for UiTokens {}

impl UiTokens {
    /// A token set resolved under `contrast` and `density`, with no theme values for token groups, drawing the default
    /// icon set.
    pub fn new(tokens: Arc<Tokens>, contrast: Contrast, density: Size) -> Self {
        Self::with_icons(tokens, contrast, density, IconSet::default())
    }

    /// The tokens `theme` resolves to under `appearance` and `sizes`, with the pins of `app` (`None`: no app's), drawing
    /// the icon pack the theme names.
    pub fn of_theme(theme: &ThemeDef, appearance: Appearance, app: Option<&AppId>, sizes: &AppSizes) -> Self {
        let tokens = Arc::new(resolve_sized(theme, appearance, app, sizes).tokens);
        Self::with_icons(tokens, appearance.contrast, appearance.size, theme_icons(theme.icon_pack.as_ref()))
    }

    fn with_icons(tokens: Arc<Tokens>, contrast: Contrast, density: Size, icons: IconSet) -> Self {
        let groups = Arc::new(GroupValues::new());
        UiTokens { key: content_key(&tokens, &groups, contrast, density, icons), tokens, groups, contrast, density, icons }
    }

    /// The icon set SVG icons are drawn from.
    pub(crate) fn icons(&self) -> IconSet {
        self.icons
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

    /// Whether the resolved source tokens used by text layout are equal.
    pub(crate) fn same_source_tokens(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.tokens, &other.tokens) || self.tokens == other.tokens
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
    /// faces. The condensed slot draws in the [`TAB_TITLE`] family when the stack has one, and as the semibold slot
    /// otherwise.
    pub(crate) fn font(&self, facts: StackFacts, role: TypeRole, mono: bool) -> FontId {
        self.font_weighted(facts, role, mono, false)
    }

    /// The font of `role` as [`Self::font`] gives it, in the semibold slot when `strong` and the run is not mono: a
    /// strong span keeps its role's size.
    pub(crate) fn font_weighted(&self, facts: StackFacts, role: TypeRole, mono: bool, strong: bool) -> FontId {
        let style = if mono { self.typography.mono } else { self.typography.get(role) };
        let slot = if strong && style.font == craft_theme::FontSlot::Ui { craft_theme::FontSlot::UiSemibold } else { style.font };
        let family = match slot {
            craft_theme::FontSlot::Mono => FontFamily::Monospace,
            craft_theme::FontSlot::Ui => FontFamily::Proportional,
            craft_theme::FontSlot::UiCondensed if facts.tab_title => FontFamily::Name(TAB_TITLE.into()),
            craft_theme::FontSlot::UiSemibold | craft_theme::FontSlot::UiCondensed => {
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
fn content_key(tokens: &Tokens, groups: &GroupValues, contrast: Contrast, density: Size, icons: IconSet) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    format!("{tokens:?}{groups:?}").hash(&mut hasher);
    contrast.hash(&mut hasher);
    density.hash(&mut hasher);
    icons.hash(&mut hasher);
    hasher.finish()
}

impl Deref for UiTokens {
    type Target = Tokens;

    fn deref(&self) -> &Tokens {
        &self.tokens
    }
}

//! The root token cache: resolved tokens keyed by what they depend on.

use super::root::{ResolvedRoot, RootEnv};
use crate::runtime::egui::pass;
use crate::runtime::pass_state;
use crate::style::UiTokens;
use craft_theme::{AppId, AppSizes, Appearance, Contrast, Mode, Rgba, SUITE_DEFAULT, Size, TextSize, ThemeDef, ThemeId, builtins};
use egui::Context;
use std::sync::Arc;

/// How many root-token inputs the cache keeps beyond those used in the current frame: the window's, a few themed
/// regions', and the ones a draft replaced last.
const KEPT_ROOT_TOKENS: usize = 8;

#[derive(Default)]
pub(crate) struct RootTokenCache {
    entries: Vec<RootTokens>,
    /// How many lookups missed the cache.
    #[cfg(feature = "testing")]
    misses: usize,
}

/// One cached resolve and the root frame that last read it.
struct RootTokens {
    inputs: TokenInputs,
    tokens: Arc<UiTokens>,
    used: u64,
}

/// How many times the context resolved root tokens it had no entry for.
#[cfg(feature = "testing")]
pub(crate) fn root_token_resolves(ctx: &Context) -> usize {
    pass_state::with::<RootTokenCache, _>(ctx, |cache| cache.misses)
}

/// The tokens `inputs` resolve to, cached by the inputs. An entry read in the current root frame is never dropped, so
/// however many themed regions a frame shows, each resolves once; past the kept count the least recently read of the
/// others goes.
pub(super) fn tokens_for(ctx: &Context, inputs: &TokenInputs) -> Arc<UiTokens> {
    let frame = pass::root_frame_nr(ctx);
    let hit = pass_state::with::<RootTokenCache, _>(ctx, |cache| {
        let entry = cache.entries.iter_mut().find(|entry| entry.inputs.same(inputs))?;
        entry.used = frame;
        // An equal definition in another allocation: the next frames find this one by pointer.
        if let (Some(kept), Some(given)) = (&mut entry.inputs.theme_def, &inputs.theme_def)
            && !Arc::ptr_eq(kept, given)
        {
            *kept = given.clone();
        }
        Some(entry.tokens.clone())
    });
    if let Some(hit) = hit {
        return hit;
    }
    let tokens = resolved(inputs);
    pass_state::with::<RootTokenCache, _>(ctx, |cache| {
        #[cfg(feature = "testing")]
        {
            cache.misses += 1;
        }
        if cache.entries.len() >= KEPT_ROOT_TOKENS
            && let Some(oldest) =
                cache.entries.iter().enumerate().filter(|(_, entry)| entry.used != frame).min_by_key(|(_, entry)| entry.used).map(|(at, _)| at)
        {
            cache.entries.remove(oldest);
        }
        cache.entries.push(RootTokens { inputs: inputs.clone(), tokens: tokens.clone(), used: frame });
    });
    tokens
}

/// The tokens `inputs` resolve to.
#[cfg(not(feature = "testing"))]
fn resolved(inputs: &TokenInputs) -> Arc<UiTokens> {
    resolve_inputs(inputs)
}

/// The tokens `inputs` resolve to, shared by the thread's contexts. A test binary builds thousands of contexts from
/// the same few inputs, and each resolution derives every colour and hashes the whole set; the inputs are compared by
/// value here, the sizes included.
#[cfg(feature = "testing")]
fn resolved(inputs: &TokenInputs) -> Arc<UiTokens> {
    use std::cell::RefCell;
    /// The newest resolutions the thread keeps.
    const KEPT: usize = 32;
    thread_local! {
        static RESOLVED: RefCell<Vec<(TokenInputs, Arc<UiTokens>)>> = const { RefCell::new(Vec::new()) };
    }
    let found = RESOLVED.with(|kept| kept.borrow().iter().find(|(key, _)| key.same_values(inputs)).map(|(_, tokens)| tokens.clone()));
    if let Some(tokens) = found {
        return tokens;
    }
    let tokens = resolve_inputs(inputs);
    RESOLVED.with(|kept| {
        let mut kept = kept.borrow_mut();
        if kept.len() == KEPT {
            kept.remove(0);
        }
        kept.push((inputs.clone(), tokens.clone()));
    });
    tokens
}

fn resolve_inputs(inputs: &TokenInputs) -> Arc<UiTokens> {
    let appearance =
        Appearance { mode: inputs.mode, contrast: inputs.contrast, size: inputs.density, text_size: inputs.text_size, reduced_motion: false };
    let themes = builtins();
    let theme: &ThemeDef = match &inputs.theme_def {
        Some(def) => def,
        None => [inputs.theme.as_str(), SUITE_DEFAULT]
            .into_iter()
            .find_map(|id| themes.iter().find(|theme| theme.id.as_str() == id))
            .unwrap_or(&themes[0]),
    };
    let accented = inputs.accent.map(|seed| theme.with_accent(seed));
    Arc::new(UiTokens::of_theme(accented.as_ref().unwrap_or(theme), appearance, inputs.app.as_ref(), &inputs.sizes))
}

/// What the resolved root tokens depend on. The sizes are compared by identity, as `set_input` does.
#[derive(Clone)]
pub(crate) struct TokenInputs {
    pub(super) sizes: Arc<AppSizes>,
    pub(super) theme: ThemeId,
    pub(super) theme_def: Option<Arc<ThemeDef>>,
    /// The user's accent over the theme.
    pub(super) accent: Option<Rgba>,
    pub(super) mode: Mode,
    pub(super) contrast: Contrast,
    pub(super) density: Size,
    pub(super) text_size: TextSize,
    /// The app whose `[apps.<id>]` theme pins apply over the chosen theme.
    pub(super) app: Option<AppId>,
}

impl TokenInputs {
    /// The inputs of `root` resolved as `resolved`, with the pins of `app` (`None`: no app's pins).
    pub(super) fn of(root: &RootEnv, resolved: ResolvedRoot, app: Option<AppId>) -> Self {
        TokenInputs {
            sizes: root.sizes.clone(),
            theme: root.theme.clone(),
            theme_def: root.theme_def.clone(),
            accent: root.accent,
            mode: resolved.mode,
            contrast: resolved.contrast,
            density: root.density,
            text_size: root.text_size,
            app,
        }
    }

    /// Whether `other` resolves to the same tokens: [`TokenInputs::same`] with the sizes compared by value.
    #[cfg(feature = "testing")]
    fn same_values(&self, other: &TokenInputs) -> bool {
        (Arc::ptr_eq(&self.sizes, &other.sizes) || self.sizes == other.sizes)
            && self.theme == other.theme
            && same_def(&self.theme_def, &other.theme_def)
            && self.accent == other.accent
            && self.mode == other.mode
            && self.contrast == other.contrast
            && self.density == other.density
            && self.text_size == other.text_size
            && self.app == other.app
    }

    fn same(&self, other: &TokenInputs) -> bool {
        Arc::ptr_eq(&self.sizes, &other.sizes)
            && self.theme == other.theme
            && same_def(&self.theme_def, &other.theme_def)
            && self.accent == other.accent
            && self.mode == other.mode
            && self.contrast == other.contrast
            && self.density == other.density
            && self.text_size == other.text_size
            && self.app == other.app
    }
}

/// Whether two theme definitions are the same theme: one allocation or equal values.
pub(super) fn same_def(a: &Option<Arc<ThemeDef>>, b: &Option<Arc<ThemeDef>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b) || a == b,
        _ => false,
    }
}

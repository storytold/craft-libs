//! The root token cache: resolved tokens keyed by what they depend on.

use super::root::{ResolvedRoot, RootEnv};
use crate::runtime::egui::memory::Service;
use crate::style::UiTokens;
use craft_theme::{AppSizes, Appearance, Contrast, Mode, SUITE_DEFAULT, Size, TextSize, ThemeDef, ThemeId, builtins, resolve_sized};
use egui::Context;
use std::sync::Arc;

/// The newest few root-token inputs are kept; an app swaps its sizes rarely.
const KEPT_ROOT_TOKENS: usize = 4;

#[derive(Default)]
struct RootTokenCache {
    entries: Vec<(TokenInputs, Arc<UiTokens>)>,
}

/// How many root-token entries the context holds.
#[cfg(feature = "testing")]
pub(crate) fn root_token_entries(ctx: &Context) -> usize {
    Service::<RootTokenCache>::run(ctx, |cache| cache.entries.len())
}

/// The tokens `inputs` resolve to, cached by the inputs.
pub(super) fn tokens_for(ctx: &Context, inputs: &TokenInputs) -> Arc<UiTokens> {
    let service = Service::<RootTokenCache>::of(ctx);
    if let Some(hit) = service.with(|cache| cache.entries.iter().find(|(key, _)| key.same(inputs)).map(|(_, tokens)| tokens.clone())) {
        return hit;
    }
    let tokens = resolved(inputs);
    service.with(|cache| {
        if cache.entries.len() == KEPT_ROOT_TOKENS {
            cache.entries.remove(0);
        }
        cache.entries.push((inputs.clone(), tokens.clone()));
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
    let theme: &ThemeDef =
        [inputs.theme.as_str(), SUITE_DEFAULT].into_iter().find_map(|id| themes.iter().find(|theme| theme.id.as_str() == id)).unwrap_or(&themes[0]);
    Arc::new(UiTokens::new(Arc::new(resolve_sized(theme, appearance, None, &inputs.sizes).tokens), inputs.contrast, inputs.density))
}

/// What the resolved root tokens depend on. The sizes are compared by identity, as `set_input` does.
#[derive(Clone)]
pub(crate) struct TokenInputs {
    pub(super) sizes: Arc<AppSizes>,
    pub(super) theme: ThemeId,
    pub(super) mode: Mode,
    pub(super) contrast: Contrast,
    pub(super) density: Size,
    pub(super) text_size: TextSize,
}

impl TokenInputs {
    pub(super) fn of(root: &RootEnv, resolved: ResolvedRoot) -> Self {
        TokenInputs {
            sizes: root.sizes.clone(),
            theme: root.theme.clone(),
            mode: resolved.mode,
            contrast: resolved.contrast,
            density: root.density,
            text_size: root.text_size,
        }
    }

    /// Whether `other` resolves to the same tokens: [`TokenInputs::same`] with the sizes compared by value.
    #[cfg(feature = "testing")]
    fn same_values(&self, other: &TokenInputs) -> bool {
        (Arc::ptr_eq(&self.sizes, &other.sizes) || self.sizes == other.sizes)
            && self.theme == other.theme
            && self.mode == other.mode
            && self.contrast == other.contrast
            && self.density == other.density
            && self.text_size == other.text_size
    }

    fn same(&self, other: &TokenInputs) -> bool {
        Arc::ptr_eq(&self.sizes, &other.sizes)
            && self.theme == other.theme
            && self.mode == other.mode
            && self.contrast == other.contrast
            && self.density == other.density
            && self.text_size == other.text_size
    }
}

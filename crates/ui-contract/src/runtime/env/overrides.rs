//! Changes to the inherited values for a stretch of code: Overrides, with_env and ThemeScope.

use super::app_values::AppValues;
use super::host_words::overlaid_words;
use super::tokens::{TokenInputs, tokens_for};
use super::{Env, Restore, Scoped};
use crate::runtime::egui::child;
use craft_behavior::MotionMode;
use craft_i18n::{Direction, WordRows, Words};
use craft_theme::{ControlSize, Mode};
use egui::{Context, Ui};
use std::sync::Arc;

/// Token-changing values for a container's body: the body and everything under it resolve their styles from the
/// inherited theme in another mode.
#[derive(Clone)]
pub struct ThemeScope {
    mode: Mode,
}

impl ThemeScope {
    /// A scope whose body resolves the inherited theme, contrast, density and text size in `mode`: a plot that is dark
    /// in every theme, a preview of the other mode.
    pub fn mode(mode: Mode) -> Self {
        ThemeScope { mode }
    }

    /// Puts the scope's tokens into `scoped`.
    pub(crate) fn apply(&self, ctx: &Context, scoped: &mut Scoped) {
        let inputs = TokenInputs { mode: self.mode, ..(*scoped.token_inputs).clone() };
        scoped.tokens = tokens_for(ctx, &inputs);
        scoped.token_inputs = Arc::new(inputs);
    }
}

impl std::fmt::Debug for ThemeScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThemeScope").field("mode", &self.mode).finish()
    }
}

/// Changes to the inherited values for a stretch of code.
#[derive(Default)]
#[non_exhaustive]
pub struct Overrides {
    /// The ambient control size.
    pub size: Option<ControlSize>,
    /// Only `false` has an effect.
    pub enabled: Option<bool>,
    /// Only `true` has an effect.
    pub read_only: Option<bool>,
    /// Only `true` has an effect.
    pub wheel_gated: Option<bool>,
    /// Can only lower the mode.
    pub motion: Option<MotionMode>,
    /// Also the child `Ui`'s direction and layout.
    pub direction: Option<Direction>,
    /// The words of the scope and everything inside it.
    pub words: Option<WordsOverride>,
    /// App values set for the scope.
    pub values: AppValues,
}

/// How a scope changes the built-in text: the only override path for words.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum WordsOverride {
    /// These words stand in for the inherited ones.
    Replace(Arc<Words>),
    /// These rows win over the inherited words; every other word reads as before.
    Overlay(WordRows),
}

impl Overrides {
    fn apply(self, ctx: &Context, scoped: &mut Scoped) {
        if let Some(size) = self.size {
            scoped.size = size;
        }
        if self.enabled == Some(false) {
            scoped.enabled = false;
        }
        if self.read_only == Some(true) {
            scoped.read_only = true;
        }
        if self.wheel_gated == Some(true) {
            scoped.wheel_gated = true;
        }
        if let Some(motion) = self.motion {
            scoped.motion.mode = scoped.motion.mode.lowered_by(motion);
        }
        if let Some(direction) = self.direction {
            scoped.direction = direction;
            scoped.text_direction = direction;
        }
        match self.words {
            Some(WordsOverride::Replace(words)) => scoped.words = words,
            Some(WordsOverride::Overlay(rows)) => scoped.words = overlaid_words(ctx, &scoped.words, rows),
            None => {}
        }
        scoped.values.extend(&self.values);
    }
}

/// Runs `body` in a child `Ui` with `set`'s changes applied to the inherited values. A scope that disables
/// disables its child `Ui`; one that changes direction builds the child with an explicit layout and never
/// inherits the parent's.
pub fn with_env<R>(ui: &mut Ui, set: impl FnOnce(&mut Overrides), body: impl FnOnce(&mut Ui) -> R) -> R {
    let mut overrides = Overrides::default();
    set(&mut overrides);
    let outer = Env::of(ui);
    let ctx = ui.ctx().clone();
    let inner = outer.derive(|scoped| overrides.apply(&ctx, scoped));
    let direction = (inner.direction != outer.direction).then(|| inner.direction.is_rtl());
    let disabled = !inner.enabled;
    child::scope(ui, disabled, direction, |child| {
        let _restore = Restore { ctx: &ctx, installed: Some(Env::install_for(child, inner)) };
        body(child)
    })
}

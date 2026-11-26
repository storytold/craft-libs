//! Changes to the inherited values for a stretch of code: Overrides, with_env and ThemeScope, and a region that is a
//! layout room of its own (in_room).

use super::app_values::AppValues;
use super::host_words::overlaid_words;
use super::tokens::{same_def, tokens_for};
use super::{Env, Restore, Scoped};
use crate::runtime::egui::child;
use craft_behavior::MotionMode;
use craft_i18n::{Direction, WordRows, Words};
use craft_theme::{Contrast, ControlSize, Mode, Size, ThemeDef};
use egui::{Context, Rect, Ui, UiBuilder};
use std::sync::Arc;

/// Token-changing values for a container: the container itself (its surface, header, rules, scroll bars, pads and
/// focus ring), its regions and body, and every layer opened from them resolve their tokens from another theme
/// definition, mode, contrast or density. Each value left unset is inherited.
///
/// The token cache keys a definition by its `Arc` first: a caller keeps one `Arc` per definition (a draft replaced on
/// each committed edit) and the scope resolves it once, however many frames show it.
#[derive(Clone, Default)]
pub struct ThemeScope {
    theme: Option<Arc<ThemeDef>>,
    mode: Option<Mode>,
    contrast: Option<Contrast>,
    density: Option<Size>,
}

impl ThemeScope {
    /// A scope that changes nothing: every value is inherited.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolves from `def`: a draft in a theme editor, a theme in a preview card.
    pub fn theme(mut self, def: Arc<ThemeDef>) -> Self {
        self.theme = Some(def);
        self
    }

    /// Resolves in `mode`: a plot that is dark in every theme, a preview of the other mode.
    pub fn mode(mut self, mode: Mode) -> Self {
        self.mode = Some(mode);
        self
    }

    /// Resolves under `contrast`.
    pub fn contrast(mut self, contrast: Contrast) -> Self {
        self.contrast = Some(contrast);
        self
    }

    /// Resolves at the density preset `density`.
    pub fn density(mut self, density: Size) -> Self {
        self.density = Some(density);
        self
    }

    /// Puts the scope's tokens into `scoped`.
    pub(crate) fn apply(&self, ctx: &Context, scoped: &mut Scoped) {
        let mut inputs = (*scoped.token_inputs).clone();
        if let Some(def) = &self.theme {
            inputs.theme = def.id.clone();
            inputs.theme_def = Some(def.clone());
        }
        if let Some(mode) = self.mode {
            inputs.mode = mode;
        }
        if let Some(contrast) = self.contrast {
            inputs.contrast = contrast;
        }
        if let Some(density) = self.density {
            inputs.density = density;
        }
        let inherited = scoped.inherited_mut();
        inherited.tokens = tokens_for(ctx, &inputs);
        inherited.token_inputs = Arc::new(inputs);
    }
}

impl PartialEq for ThemeScope {
    fn eq(&self, other: &Self) -> bool {
        same_def(&self.theme, &other.theme) && self.mode == other.mode && self.contrast == other.contrast && self.density == other.density
    }
}

impl std::fmt::Debug for ThemeScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThemeScope")
            .field("theme", &self.theme.as_ref().map(|def| &def.id))
            .field("mode", &self.mode)
            .field("contrast", &self.contrast)
            .field("density", &self.density)
            .finish()
    }
}

/// How the wheel takers of a scope take the wheel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WheelScope {
    /// A value control takes the wheel only while focused or armed; every other taker whenever the pointer is over it.
    #[default]
    Plain,
    /// A value control that senses the wheel takes it without focus: the sliders of an adjustment dialog.
    Free,
    /// Every taker takes the wheel only while focused or armed: a card in a scrolling overview. Wins over a `Free` set
    /// inside it, so a scroll of the overview never stops on a story's slider.
    Gated,
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
    /// Has no effect inside a `WheelScope::Gated` scope.
    pub wheel: Option<WheelScope>,
    /// Can only lower the mode.
    pub motion: Option<MotionMode>,
    /// Also the child `Ui`'s direction and layout.
    pub direction: Option<Direction>,
    /// The flow direction alone (layout order, arrow keys and roving, and the child `Ui`'s layout): the paragraph
    /// direction of text stays the inherited one. A strip mirrored to follow something physical (the macOS traffic
    /// lights at the right end) keeps its labels reading as the interface does. Wins over `direction`'s flow.
    pub flow: Option<Direction>,
    /// The words of the scope and everything inside it.
    pub words: Option<WordsOverride>,
    /// App values set for the scope.
    pub values: AppValues,
    /// The child `Ui`'s id salt, as `ui.push_id` gives it: the ids of what it shows derive from it.
    pub id_salt: Option<egui::Id>,
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
        if let Some(wheel) = self.wheel
            && scoped.wheel != WheelScope::Gated
        {
            scoped.wheel = wheel;
        }
        if let Some(motion) = self.motion {
            let inherited = scoped.inherited_mut();
            inherited.motion.mode = inherited.motion.mode.lowered_by(motion);
        }
        if let Some(direction) = self.direction {
            scoped.direction = direction;
            scoped.text_direction = direction;
        }
        if let Some(flow) = self.flow {
            scoped.direction = flow;
        }
        match self.words {
            Some(WordsOverride::Replace(words)) => scoped.inherited_mut().words = words,
            Some(WordsOverride::Overlay(rows)) => {
                let words = overlaid_words(ctx, &scoped.words, rows);
                scoped.inherited_mut().words = words;
            }
            None => {}
        }
        scoped.extend_values(&self.values);
    }
}

/// Runs `body` in a child `Ui` with `set`'s changes applied to the inherited values. A scope that disables
/// disables its child `Ui`; one that changes direction builds the child with an explicit layout and never
/// inherits the parent's.
///
/// The components the child `Ui` shows are children of the body around `ui` as those shown in `ui` itself are, on its
/// line and in its cells. It is the craft form of `ui.scope` and
/// `ui.push_id` (`Overrides::id_salt`) around craft content: what a body sets up for its children in flow does not
/// reach a `Ui` egui code makes below the body's own (`Env::of`). Content laid out along the other axis goes in a
/// craft `Row` or `Stack`, a body of its own.
pub fn with_env<R>(ui: &mut Ui, set: impl FnOnce(&mut Overrides), body: impl FnOnce(&mut Ui) -> R) -> R {
    let mut overrides = Overrides::default();
    set(&mut overrides);
    let id_salt = overrides.id_salt.take();
    let outer = Env::of(ui);
    let ctx = ui.ctx().clone();
    let inner = outer.derive(|scoped| overrides.apply(&ctx, scoped));
    let direction = (inner.direction != outer.direction).then(|| inner.direction.is_rtl());
    let disabled = !inner.enabled;
    child::scope(ui, id_salt, disabled, direction, |child| {
        let _restore = Restore { ctx: &ctx, installed: Some(Env::install_for(child, inner)) };
        body(child)
    })
}

/// Runs `body` in a child `Ui` held to `rect` and clipped to it, as a layout room of its own: the flow grid, content
/// column and fill sharing of the body around it do not reach in, so a child that fills takes `rect`, whatever else
/// that body holds. A split pane's body and a dock group's view show here.
pub fn in_room<R>(ui: &mut Ui, rect: Rect, body: impl FnOnce(&mut Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let room = Env::of(ui).new_room();
    ui.scope_builder(UiBuilder::new().max_rect(rect), |child| {
        child.shrink_clip_rect(rect);
        let _restore = Restore { ctx: &ctx, installed: Some(Env::install_for(child, room)) };
        body(child)
    })
    .inner
}

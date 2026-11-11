//! Style and paint helpers: token sets, a `StyleCx`, the style cache and a `Draw` over a frame.

use super::Frames;
use crate::primitives::{Draw, DrawEnv, PaintRecord};
use crate::runtime::egui::memory::Service;
use crate::runtime::env::{AppValues, Scale};
use crate::style::{ComponentStyle, EnvKey, Ink, StyleCx, StyleKey, UiTokens, resolve_style};
use craft_host::target::Os;
use craft_i18n::Direction;
use craft_theme::{Appearance, Contrast, ControlSize, Mode, Size, TextSize, Variant, Variants, builtins, resolve};
use egui::{Context, Shape};
use std::cell::RefCell;
use std::sync::Arc;

/// The suite's default built-in theme resolved for `mode` and `contrast`.
pub fn tokens(mode: Mode, contrast: Contrast) -> UiTokens {
    tokens_named("artcraft", mode, contrast)
}

/// The built-in theme `craft.<name>` resolved for `mode` and `contrast`. The built-ins are static, so each set is
/// resolved once per thread and cloned after.
pub fn tokens_named(name: &str, mode: Mode, contrast: Contrast) -> UiTokens {
    type Kept = Vec<((String, Mode, Contrast), UiTokens)>;
    thread_local! {
        static RESOLVED: RefCell<Kept> = const { RefCell::new(Vec::new()) };
    }
    let found =
        RESOLVED.with(|kept| kept.borrow().iter().find(|((n, m, c), _)| n == name && *m == mode && *c == contrast).map(|(_, tokens)| tokens.clone()));
    if let Some(tokens) = found {
        return tokens;
    }
    let tokens = resolve_named(name, mode, contrast);
    RESOLVED.with(|kept| kept.borrow_mut().push(((name.to_owned(), mode, contrast), tokens.clone())));
    tokens
}

fn resolve_named(name: &str, mode: Mode, contrast: Contrast) -> UiTokens {
    let id = format!("craft.{name}");
    let index = builtins().iter().position(|theme| theme.id.as_str() == id);
    assert!(index.is_some(), "no built-in theme {id}");
    let index = index.unwrap_or_default();
    let appearance = Appearance { mode, contrast, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false };
    let resolved = resolve(&builtins()[index], appearance, None);
    UiTokens::new(Arc::new(resolved.tokens), contrast, Size::Compact)
}

/// Every built-in theme in each mode it has, at `contrast`, as (`<theme>.<mode>`, tokens). Built once per thread and
/// contrast.
pub fn themes_at(contrast: Contrast) -> Vec<(String, UiTokens)> {
    type Kept = Vec<(Contrast, Vec<(String, UiTokens)>)>;
    thread_local! {
        static BUILT: RefCell<Kept> = const { RefCell::new(Vec::new()) };
    }
    let found = BUILT.with(|kept| kept.borrow().iter().find(|(c, _)| *c == contrast).map(|(_, themes)| themes.clone()));
    if let Some(themes) = found {
        return themes;
    }
    let themes = build_themes_at(contrast);
    BUILT.with(|kept| kept.borrow_mut().push((contrast, themes.clone())));
    themes
}

fn build_themes_at(contrast: Contrast) -> Vec<(String, UiTokens)> {
    let mut out = Vec::new();
    for theme in builtins() {
        let modes = match theme.variants {
            Variants::Both => &[Mode::Light, Mode::Dark][..],
            Variants::LightOnly => &[Mode::Light],
            Variants::DarkOnly => &[Mode::Dark],
        };
        let id = theme.id.as_str();
        let name = id.strip_prefix("craft.").unwrap_or(id);
        for &mode in modes {
            let mode_name = if mode == Mode::Light { "light" } else { "dark" };
            out.push((format!("{name}.{mode_name}"), tokens_named(name, mode, contrast)));
        }
    }
    out
}

/// The `EnvKey` of `tokens` in a left-to-right Linux window at scale 1 with no style-affecting app values.
pub fn env_key(tokens: &UiTokens) -> EnvKey {
    EnvKey::new(tokens.key(), Direction::Ltr, Scale::default(), Os::Linux, true, 0)
}

/// Runs `f` with a `StyleCx` over `tokens` in a left-to-right Linux window at scale 1.
pub fn with_cx<R>(ctx: &Context, tokens: &UiTokens, variant: Variant, size: ControlSize, f: impl FnOnce(&StyleCx<'_>) -> R) -> R {
    let values = AppValues::default();
    let line_ratio = crate::fonts::stack_facts(ctx).line_ratio;
    let cx = StyleCx::new(
        ctx,
        tokens,
        &values,
        size,
        variant,
        Direction::Ltr,
        Scale::default(),
        Os::Linux,
        true,
        env_key(tokens).with_line(line_ratio),
        line_ratio,
    );
    f(&cx)
}

/// The cached style for `key` in `cx`, built by `default` on a miss.
pub fn style_of<K: StyleKey, S: ComponentStyle>(ctx: &Context, cx: &StyleCx<'_>, key: K, default: for<'a> fn(&StyleCx<'a>, K) -> S) -> Arc<S> {
    resolve_style(cx, key, ctx, default)
}

/// The base id of the face painted for `part` of the composite `id`.
pub fn face_id(composite: &str, part: craft_behavior::PartRef) -> egui::Id {
    crate::primitives::face_id(crate::runtime::egui::memory::unique(composite), part)
}

/// What `Draw` recorded in one frame.
#[derive(Debug, Default, Clone, PartialEq)]
#[non_exhaustive]
pub struct Recorded {
    /// The full text of the first elided run.
    pub elided: Option<String>,
    /// The first baseline.
    pub baseline: Option<f32>,
}

/// Runs one frame in which `paint` draws through a `Draw` over the root `Ui`'s painter, in `direction` and on
/// `ink`. Returns the shapes painted and what `Draw` recorded.
pub fn paint_frame(
    frames: &mut Frames,
    tokens: &UiTokens,
    direction: Direction,
    ink: Ink,
    mut paint: impl FnMut(&Draw<'_>),
) -> (Vec<Shape>, Recorded) {
    let record = RefCell::new(PaintRecord::default());
    let shapes = frames.frame_shapes(|ui| {
        let draw = Draw::new(ui.painter(), &record, DrawEnv { tokens, direction, ink });
        paint(&draw);
    });
    let record = record.into_inner();
    (shapes, Recorded { elided: record.elided, baseline: record.baseline })
}

/// How many root-token entries the root env cache of `ctx` holds.
pub fn root_token_entries(ctx: &Context) -> usize {
    crate::runtime::env::root_token_entries(ctx)
}

/// A resolved style as the style cases read it: the variants its type lists and the looks that state colours
/// reach, with the whole style kept to tell two styles apart.
#[derive(Clone)]
#[non_exhaustive]
pub struct StyleLooks {
    /// The variants the style type accepts; the first is the default.
    pub variants: &'static [craft_theme::Variant],
    /// The surface, when the style has one.
    pub surface: Option<crate::style::SurfaceLook>,
    /// The focus ring, when the style has one.
    pub focus: Option<crate::style::FocusLook>,
    /// The text looks, in field order.
    pub texts: Vec<crate::style::TextLook>,
    /// The icon looks, in field order.
    pub icons: Vec<crate::style::IconLook>,
    style: std::sync::Arc<dyn std::any::Any + Send + Sync>,
    equal: fn(&dyn std::any::Any, &dyn std::any::Any) -> bool,
}

impl StyleLooks {
    /// Whether `other` is the same style, in every field.
    pub fn same_as(&self, other: &StyleLooks) -> bool {
        (self.equal)(&*self.style, &*other.style)
    }
}

pub(crate) fn style_looks<T: crate::contract::Component + 'static>(
    instance: &dyn std::any::Any,
    cx: &crate::style::StyleCx<'_>,
) -> Option<StyleLooks> {
    let component = instance.downcast_ref::<T>()?;
    Some(looks_of(&T::style(cx, component.style_key())))
}

fn looks_of<S: crate::style::ComponentStyle>(style: &S) -> StyleLooks {
    StyleLooks {
        variants: S::VARIANTS,
        surface: style.surface().cloned(),
        focus: style.focus().copied(),
        texts: style.texts().into_iter().cloned().collect(),
        icons: style.icons().into_iter().cloned().collect(),
        style: std::sync::Arc::new(style.clone()),
        equal: |a, b| matches!((a.downcast_ref::<S>(), b.downcast_ref::<S>()), (Some(a), Some(b)) if a == b),
    }
}

/// A style the pipeline resolved for a component run: what it looked like in the env it was resolved in, and how to
/// resolve the same component's style in another env.
#[non_exhaustive]
pub struct StyleSeen {
    /// The component type's `std::any::type_name`.
    pub type_name: &'static str,
    /// The variant the style was resolved under.
    pub variant: craft_theme::Variant,
    /// The control size it was resolved under.
    pub size: ControlSize,
    /// The resolved style.
    pub looks: StyleLooks,
    pub(crate) inputs: u64,
    env: crate::style::EnvKey,
    again: std::sync::Arc<dyn Fn(&crate::style::StyleCx<'_>) -> StyleLooks + Send + Sync>,
}

impl StyleSeen {
    /// The style of the same component, from the same style key, under another style context.
    pub fn resolve_in(&self, cx: &crate::style::StyleCx<'_>) -> StyleLooks {
        (self.again)(cx)
    }
}

#[derive(Default)]
struct StyleRecord {
    seen: Vec<std::sync::Arc<StyleSeen>>,
}

/// Records the style step 2 resolved for `C`, once per style key, env, variant and size, in frames a test fixture
/// drives.
pub(crate) fn record<C: crate::contract::Component>(ctx: &Context, scx: &crate::style::StyleCx<'_>, key: C::StyleKey, style: &C::Style) {
    use std::hash::{Hash, Hasher};
    if !crate::runtime::egui::fixture::driving() {
        return;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    let inputs = hasher.finish();
    let type_name = std::any::type_name::<C>();
    let known = Service::<StyleRecord>::run(ctx, |record| {
        record.seen.iter().any(|seen| {
            seen.type_name == type_name && seen.inputs == inputs && seen.env == scx.key && seen.variant == scx.variant && seen.size == scx.size
        })
    });
    if known {
        return;
    }
    let resolve: for<'a> fn(&crate::style::StyleCx<'a>, C::StyleKey) -> C::Style = C::style;
    let seen = StyleSeen {
        type_name,
        variant: scx.variant,
        size: scx.size,
        looks: looks_of(style),
        inputs,
        env: scx.key,
        again: std::sync::Arc::new(move |cx| looks_of(&resolve(cx, key))),
    };
    Service::<StyleRecord>::run(ctx, |record| record.seen.push(std::sync::Arc::new(seen)));
}

/// Every style the pipeline resolved on `ctx` for the component type `type_name`.
pub(crate) fn styles_of(ctx: &Context, type_name: &str) -> Vec<std::sync::Arc<StyleSeen>> {
    Service::<StyleRecord>::run(ctx, |record| record.seen.iter().filter(|seen| seen.type_name == type_name).cloned().collect())
}

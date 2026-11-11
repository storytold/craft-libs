//! Step 2, style: the component's style from the cache.

use super::env::EnvStep;
use crate::contract::Component;
use crate::runtime::env::Env;
use crate::style::{StyleCx, resolve_style};
use egui::Context;
use std::sync::Arc;

/// The `StyleCx` of the env the component shows in.
pub(crate) fn style_cx<'a>(ctx: &'a Context, env: &'a Env, step: &EnvStep) -> StyleCx<'a> {
    StyleCx::new(ctx, &env.tokens, &env.values, step.size, step.requested, env.direction, env.scale, env.os, env.active, env.key, env.line_ratio)
}

/// Step 2, style: `&C::Style` from the cache, keyed `(TypeId of StyleKey, key, EnvKey, variant, size)`.
pub(crate) fn style<C: Component>(ctx: &Context, scx: &StyleCx<'_>, key: C::StyleKey) -> Arc<C::Style> {
    let style = resolve_style(scx, key, ctx, C::style);
    #[cfg(feature = "testing")]
    crate::testing::style::record::<C>(ctx, scx, key, &style);
    style
}

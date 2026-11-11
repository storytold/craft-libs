//! Step 1, env: enabled, read-only and the resolved props.

use crate::contract::{Common, Component, Resolved};
use crate::runtime::commands::{self, ResolvedCommand};
use crate::runtime::env::{Env, Label};
use crate::style::{ComponentStyle, accepted_variant};
use craft_theme::{ControlSize, Variant};
use egui::{Context, Id};
use std::sync::Arc;

/// What step 1 resolves.
pub(crate) struct EnvStep {
    pub enabled: bool,
    pub read_only: bool,
    pub size: ControlSize,
    /// The variant the component accepts for the one requested.
    pub variant: Variant,
    /// The variant as requested, which the style cache logs a fallback from.
    pub requested: Variant,
    pub label: Label,
    pub command: Option<Arc<ResolvedCommand>>,
}

/// Step 1, env: enabled = instance AND scopes AND the bound command's state; read-only = instance OR scopes;
/// size; variant; label.
pub(crate) fn env<C: Component>(ctx: &Context, env: &Env, common: &Common) -> EnvStep {
    let default = <C::Style as ComponentStyle>::VARIANTS.first().copied().unwrap_or(Variant::SECONDARY);
    let requested = common.variant.unwrap_or(default);
    let variant = accepted_variant::<C::StyleKey, C::Style>(requested).variant;
    let command = common.command.as_ref().map(|command| Arc::new(commands::resolve_bound(ctx, &env.commands, command)));
    let label = match (&common.label, &command) {
        (Label::Empty, Some(command)) => Label::Shared(command.label.clone()),
        (label, _) => label.resolved(&env.words),
    };
    EnvStep {
        enabled: common.enabled.unwrap_or(true) && env.enabled && command.as_ref().is_none_or(|command| command.enabled),
        read_only: common.read_only == Some(true) || env.read_only,
        size: common.size.unwrap_or(env.size),
        variant,
        requested,
        label,
        command,
    }
}

/// `Resolved` over the frame's locals.
pub(crate) fn resolved<'p>(id: Id, step: &'p EnvStep, common: &'p Common) -> Resolved<'p> {
    Resolved {
        id,
        label: step.label.as_str(),
        label_value: &step.label,
        enabled: step.enabled,
        read_only: step.read_only,
        size: step.size,
        variant: step.variant,
        selected: common.selected,
        keeps_name: common.keeps_name,
        access_key: common.access_key,
        default_stop: common.default_stop,
        ltr_axis: common.ltr_axis,
        description: common.description.as_deref(),
        tooltip: common.tooltip.as_ref(),
        doc_key: common.doc_key,
        command: step.command.as_deref(),
    }
}

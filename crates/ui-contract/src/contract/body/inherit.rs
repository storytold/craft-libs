//! What a container's regions and body inherit from the env, and the command scopes it joins.

use super::BodyHooks;
use super::reorder::REORDER_SCOPE;
use crate::contract::{Container, ContainerProps, Cx, DragBand, Scroll, Sizing};
use crate::runtime::commands::{self, CommandSet as _, WindowCommand};
use crate::runtime::env::{AppValues, ArenaId, Scoped};
use crate::style::{ComponentStyle as _, SurfaceKind};
use craft_i18n::Direction;
use egui::{Context, Id, Vec2};

/// What a container's body env needs from the pass that built it.
pub(super) struct BodyScope {
    pub(super) room: Vec2,
    pub(super) real: bool,
    pub(super) sizing: Sizing,
    pub(super) content_column: bool,
    pub(super) scroll: Option<Scroll>,
    /// The width a side label beside the body took from the container's row.
    pub(super) label_width: f32,
}

/// The surface a container paints under its regions: its props' surface, else its style's in its current state.
pub(super) fn own_surface<C: Container>(props: &ContainerProps, cx: &Cx<'_, C>) -> Option<SurfaceKind> {
    props.surface.or_else(|| cx.style.surface().and_then(|look| look.kind_in(&cx.core.state)))
}

/// Registers the container as an instance of its command scope and of the window scope of its window.
pub(super) fn register_instances(ctx: &Context, props: &ContainerProps, id: Id, window: Option<Id>) {
    if let Some(scope) = props.command_scope {
        commands::register_instance(ctx, scope, id);
    }
    if let Some(window) = window {
        commands::register_instance(ctx, WindowCommand::SCOPE, window);
    }
}

impl BodyHooks<'_> {
    /// What the container's regions inherit from the env: its enabled state, window and reorder commands, values, axis,
    /// ink and theme.
    pub(super) fn scope_regions<C: Container>(
        &self,
        scoped: &mut Scoped,
        props: &ContainerProps,
        cx: &Cx<'_, C>,
        window: Option<Id>,
        values: &AppValues,
        real: bool,
    ) {
        let (id, ctx) = (cx.core.id, &cx.core.ctx);
        // The container's own `enabled(false)` greys everything inside it, as a scope would.
        scoped.enabled &= cx.core.resolved.enabled;
        if let Some(window) = window {
            scoped.commands = scoped.commands.pushed(WindowCommand::SCOPE, window);
        }
        scoped.values.extend(values);
        if cx.core.resolved.ltr_axis {
            scoped.direction = Direction::Ltr;
        }
        scoped.ink = cx.core.ink;
        scoped.surface = SurfaceKind::under(own_surface(props, cx), scoped.surface);
        scoped.start_room();
        // Along a width the container takes from its content, its regions measure at their content widths: a child
        // that fills takes what it needs, not the whole room.
        if !real && !self.given.0 {
            scoped.content_column = true;
        }
        if props.header.is_some_and(|header| header.band == DragBand::Reorder) {
            scoped.commands = scoped.commands.pushed(REORDER_SCOPE, id);
        }
        // A host env (a layer's) does not carry the push the container's own context made.
        if let Some(scope) = props.command_scope
            && scoped.commands.innermost(scope.name()) != Some(id)
        {
            scoped.commands = scoped.commands.pushed(scope, id);
        }
        if let Some(theme) = &props.theme {
            theme.apply(ctx, scoped);
        }
    }

    /// What the body inherits on top of the line's env: arena, transform, the room it is offered and its scroll axes.
    pub(super) fn scope_body(&self, scoped: &mut Scoped, props: &ContainerProps, id: Id, body: BodyScope) {
        let BodyScope { room, real, sizing, content_column, scroll, label_width } = body;
        if props.arena {
            scoped.arena = Some(ArenaId::new(id));
        }
        scoped.transform = props.transform;
        scoped.content_column = content_column;
        scoped.row_room = None;
        scoped.row_line = None;
        // A body with a room height of its own gives that height to the children that fill it.
        scoped.content = (!self.given.0, !self.given.1 && props.room_height.is_none());
        // A content-sized container offers its children, in the real run, the width this pass measured it at: a
        // child that fits itself into its room (a bar moving items into "More") fits the width the container
        // takes, not the room it could have taken. A fill container that is not given the width still takes at
        // least the room (`regions`), so it offers the room.
        // A side label's column is not the body's: the children are offered what is left beside it.
        scoped.offered = (!self.given.0 && room.x.is_finite()).then(|| {
            let offered =
                if real && sizing == Sizing::Content { self.wanted_width(props).map_or(room.x, |wanted| wanted.min(room.x)) } else { room.x };
            (offered - label_width).max(0.0)
        });
        match scroll {
            Some(scroll) => {
                scoped.scroll_owner = Some(id);
                scoped.unbounded = (scroll.horizontal_on(), scroll.vertical_on());
            }
            // A container that sized an axis itself (a rect it was shown at, a fixed layer) bounds the room along it.
            None => scoped.unbounded = (scoped.unbounded.0 && !self.room.x.is_finite(), scoped.unbounded.1 && !self.room.y.is_finite()),
        }
    }
}

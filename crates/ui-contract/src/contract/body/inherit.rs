//! What a container's regions and body inherit from the env, and the command scopes it joins.

use super::BodyHooks;
use super::reorder::REORDER_SCOPE;
use crate::contract::{Container, ContainerProps, Cx, DragBand, Mount, QuietHeader, Scroll};
use crate::runtime::commands::{self, CommandSet as _, WindowCommand};
use crate::runtime::env::{AppValues, ArenaId, Scoped, ScrollOwner};
use crate::style::{ComponentStyle as _, SurfaceKind};
use craft_i18n::Direction;
use egui::{Context, Id, Vec2};

/// What a container's body env needs from the pass that built it.
pub(super) struct BodyScope {
    pub(super) room: Vec2,
    pub(super) real: bool,
    pub(super) content_column: bool,
    pub(super) scroll: Option<Scroll>,
    /// The width a side label beside the body took from the container's row.
    pub(super) label_width: f32,
    /// The width the regions beside the body take on its line (`Regions::beside`).
    pub(super) beside: f32,
    /// The id the body's layout runs under (`BodyPlan::layout_id`).
    pub(super) layout: Id,
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
    /// What the container's regions inherit from the env: its enabled state, window and reorder commands, values, axis
    /// and ink. The container's token scope is in the env already (step 1).
    pub(super) fn scope_regions(
        &self,
        scoped: &mut Scoped,
        props: &ContainerProps,
        core: &crate::contract::CxCore<'_>,
        window: Option<Id>,
        values: &AppValues,
        real: bool,
    ) {
        let id = core.id;
        // A layer's content starts from its opener's scope with the layer-local fields fresh, in a measure under the
        // opener's env as in the run under the layer's.
        if matches!(props.mount, Mount::Layer(_) | Mount::Viewport { .. } | Mount::FullWindow) {
            scoped.start_layer();
            let floor = super::placement::overlay_of(props).map_or(0.0, crate::contract::overlay::anchor_floor);
            // The anchor floors the layer's outer width; its regions have less room after the layer's padding.
            let horizontal_padding = self.pads(props, &scoped.tokens).x;
            scoped.inherited_mut().values.set(crate::contract::overlay::LayerFloor((floor - 2.0 * horizontal_padding).max(0.0)));
        }
        // The container's own `enabled(false)` greys everything inside it, as a scope would.
        scoped.enabled &= core.resolved.enabled;
        if let Some(collection) = props.fallback_collection {
            scoped.fallback_collection = Some(collection);
        }
        if let Some(window) = window {
            scoped.inherited_mut().commands = scoped.commands.pushed(WindowCommand::SCOPE, window);
        }
        scoped.extend_values(values);
        if let Some(scheme) = props.labels {
            scoped.inherited_mut().values.set(scheme);
        }
        if core.resolved.ltr_axis {
            scoped.direction = Direction::Ltr;
        }
        // A container that paints a fill reads in that fill's ink, so what it holds (an app icon's accent parts, text with
        // no colour of its own) does too: OnDark on a brand rail or scrim, OnPanel on a panel nested inside one. A frame
        // paints no fill, and a container with no surface paints none either: both keep the ink they sit in.
        scoped.ink = match props.surface {
            None | Some(SurfaceKind::Frame) => core.ink,
            Some(kind) => kind.ink(),
        };
        scoped.start_room();
        // An inside label and a filling cell are their control's alone: a container shows neither to what it holds.
        scoped.inside_label = None;
        scoped.cell_fills = false;
        // Along a width the container takes from its content, its regions measure at their content widths: a child
        // that fills takes what it needs, not the whole room. A container that passes its column's width on measures
        // them across it.
        if !real && !self.wide() {
            scoped.content_column = true;
        }
        if props.header.is_some_and(|header| header.band == DragBand::Reorder) {
            scoped.inherited_mut().commands = scoped.commands.pushed(REORDER_SCOPE, id);
        }
        // A host env (a layer's) does not carry the push the container's own context made.
        if let Some(scope) = props.command_scope
            && scoped.commands.innermost(scope.name()) != Some(id)
        {
            scoped.inherited_mut().commands = scoped.commands.pushed(scope, id);
        }
        // A container given its width (a resized dialog) leaves its paragraphs the room it has.
        if let Some(wrap) = props.wrap_width.filter(|_| !self.wide()) {
            scoped.measure = Some(wrap.points(scoped.tokens.tokens()));
        }
    }

    /// What the body inherits on top of the line's env: arena, transform, the room it is offered and its scroll axes.
    pub(super) fn scope_body(&self, scoped: &mut Scoped, props: &ContainerProps, id: Id, body: BodyScope) {
        let BodyScope { room, real, content_column, scroll, label_width, beside, layout } = body;
        if props.arena {
            scoped.arena = Some(ArenaId::new(id));
        }
        // The body's flow starts after a rule when the container draws one under its header or its title heads the
        // body as one would (`QuietHeader::Heads`), or when the container draws nothing of its own (no header, surface
        // or mount) and itself started after a rule.
        let heads = scoped.values.get::<QuietHeader>() == QuietHeader::Heads;
        scoped.after_rule = props.header.is_some_and(|header| (header.rule || (header.heads && heads)) && !header.inline)
            || (props.header.is_none() && props.surface.is_none() && props.mount == Mount::Inline && self.after_rule);
        scoped.flow_body = None;
        scoped.column_row =
            self.in_column && matches!(props.layout, crate::layout::BodyLayout::Row { wrap: false, align: crate::layout::RowAlign::Start });
        // Children side by side share the layer's width, so none of them spans it: a collection in a row or a grid takes
        // its own content width, not the layer's floor. Only a floor that is set is cleared, so the bodies of rows
        // outside a layer share their scope and memo key with their parent's.
        if !matches!(props.layout, crate::layout::BodyLayout::Stack) && scoped.values.get::<crate::contract::overlay::LayerFloor>().0 != 0.0 {
            scoped.inherited_mut().values.set(crate::contract::overlay::LayerFloor(0.0));
        }
        // A bracketed row's rows report to it up to the first labelled container, whose body is its own control.
        if props.bracket.is_some() {
            scoped.bracket = Some(layout);
        } else if props.label.is_some() {
            scoped.bracket = None;
        }
        scoped.transform = props.transform;
        scoped.content_column = content_column;
        scoped.given_column = !content_column && matches!(props.layout, crate::layout::BodyLayout::Stack);
        scoped.row_room = None;
        scoped.row_line = None;
        scoped.content = (!self.wide(), !self.given.1);
        // A container not given the width offers its children, in the real run, the width this pass measured it at:
        // its parent sizes from its content, so a fill container there is shown at its measured width like a
        // content-sized one, and a child that fits itself into its room (a bar moving items into "More") fits the
        // width the container takes, not the room it could have taken. A fill container held in a content-sized
        // column shows across the width the column settles on, so it offers the room, and so does a container that
        // passes its column's width on: its measuring run offered the room too.
        // The regions beside the body on its line and a side label's column are not the body's: the children are
        // offered what is left beside them.
        scoped.offered = (!self.given.0 && room.x.is_finite()).then(|| {
            let offered =
                if real && !self.held && !self.passes { self.wanted_width(props).map_or(room.x, |wanted| wanted.min(room.x)) } else { room.x };
            (offered - beside - label_width).max(0.0)
        });
        if scroll.is_some() || props.scroll_owner {
            scoped.scroll_owner = Some(ScrollOwner { id, view: scroll.is_none() });
        }
        // A container that sized an axis itself (a rect it was shown at, a fixed layer) bounds the room along it; one
        // measured in a room unbounded along an axis it does not scroll (a row's probe for its units' leasts) leaves
        // that axis unbounded, so its children never take a width from an infinite rect.
        let (x, y) = (scoped.unbounded.0 && !self.room.x.is_finite(), scoped.unbounded.1 && !self.room.y.is_finite());
        scoped.unbounded = match scroll {
            Some(scroll) => (scroll.horizontal_on() || x, scroll.vertical_on() || y),
            None => (x, y),
        };
    }
}

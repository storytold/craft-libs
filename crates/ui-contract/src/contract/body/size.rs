//! How big a container is: its sizing, the room its mount offers and the sizes its regions took.

use super::BodyHooks;
use super::placement::overlay_of;
use super::regions::{RegionSizes, end_full_height, footer_in_end};
use crate::contract::run::ShowAt;
use crate::contract::{ContainerProps, Mount, Sizing};
use crate::runtime::egui::areas;
use crate::runtime::env::Env;
use crate::runtime::layout_memory;
use crate::runtime::memory::sub;
use egui::{Context, Id, Rect, Ui, Vec2};

/// The inputs a body's measuring run measures under. A later pass offering the same takes the body's kept
/// transcript (`layout_memory::kept_serves`); its real run checks that the content is the same.
#[derive(PartialEq)]
pub(super) struct MeasureKey {
    props: u64,
    room: Vec2,
    given: (bool, bool),
    width: Option<f32>,
    /// Whether the body shows, and how much of it an animated collapse shows.
    shows: (bool, f32),
    /// The content extent the view keeps, which decides the scroll bars' inset.
    extent: (f32, f32),
    env: EnvFacts,
}

/// What a body's measure reads of its env besides the room.
#[derive(PartialEq)]
struct EnvFacts {
    style: crate::style::EnvKey,
    words: u64,
    size: craft_theme::ControlSize,
    enabled: bool,
    text_direction: crate::runtime::env::Direction,
    content: (bool, bool),
    content_column: bool,
    unbounded: (bool, bool),
    row_room: Option<f32>,
    row_line: Option<f32>,
    offered: Option<f32>,
    grid: Option<Id>,
}

impl<'b> BodyHooks<'b> {
    /// The inputs this body's measuring run measures under, once `measured_extent` has its room.
    pub(super) fn measure_key(&self, props: &ContainerProps, env: &Env, width: Option<f32>) -> MeasureKey {
        MeasureKey {
            props: props.measure_fingerprint(),
            room: self.room,
            given: self.given,
            width,
            shows: (self.open, self.presence.t),
            extent: self.last.extent,
            env: EnvFacts {
                style: env.key,
                words: env.words.generation(),
                size: env.size,
                enabled: env.enabled,
                text_direction: env.text_direction,
                content: env.content,
                content_column: env.content_column,
                unbounded: env.unbounded,
                row_room: env.row_room,
                row_line: env.row_line,
                offered: env.offered,
                grid: env.grid,
            },
        }
    }

    /// How the container's size follows its content here: a pane or a rect the host gives is filled; a layer sizes
    /// from its content until the user sizes it; an inline container follows its props.
    pub(super) fn sizing(&self, props: &ContainerProps) -> Sizing {
        match &props.mount {
            Mount::Pane(_) | Mount::Window { .. } => Sizing::Fill,
            Mount::Layer(_) | Mount::Viewport { .. } | Mount::FullWindow if self.fixed => Sizing::Fill,
            Mount::Layer(_) | Mount::Viewport { .. } => Sizing::Content,
            _ => props.sizing,
        }
    }

    /// Whether the container fills the width of a content-sized column it sits in: the column holds it at its measured
    /// width and shows it across the width the column settles on (step 6).
    pub(super) fn held(&self, props: &ContainerProps, env: &Env) -> bool {
        env.content_column && matches!(props.mount, Mount::Inline) && self.sizing(props) != Sizing::Content
    }

    /// The axes the container is given where `env` is its parent's: a rect it is shown at gives both; an inline
    /// container that fills an axis takes the room only where its parent was given that axis.
    pub(super) fn given_axes(&self, props: &ContainerProps, env: &Env, at: ShowAt) -> (bool, bool) {
        let sizing = self.sizing(props);
        let (x, y) = (sizing != Sizing::Content, sizing == Sizing::Fill);
        match (at, &props.mount) {
            (ShowAt::Rect(_), _) => (true, true),
            (ShowAt::Flow, Mount::Inline) => (x && !env.content.0 && !self.held(props, env), y && !env.content.1),
            (ShowAt::Flow, _) => (x, y),
        }
    }

    /// What the regions took together, padding included, from the records a measuring run left in this pass, when they
    /// serve a run offered `self.room`: on each axis the room equals the recorded one, or the container is not given
    /// that axis, took less than the recorded room (the room did not bound it) and fits in the room now.
    pub(super) fn reusable(&self, props: &ContainerProps, ctx: &Context, id: Id, pad: Vec2, tolerance: f32) -> Option<Vec2> {
        let body = layout_memory::measured(ctx, id)?;
        let region =
            |key: &str, has: bool| if has { layout_memory::measured(ctx, sub(id, key)).map_or(Vec2::ZERO, |region| region.took) } else { Vec2::ZERO };
        let (header, footer, end) =
            (region("header", props.header.is_some()), region("footer", props.footer.is_some()), region("end", props.end.is_some()));
        let fits = |now: f32, was: f32, given: bool, took: f32| layout_memory::Measured::serves(now, was, given, took, tolerance);
        let full = self.regions_size(props, body.took, header, footer, end) + pad;
        if !(fits(self.room.x, body.room.x, self.given.0, full.x) && fits(self.room.y, body.room.y, self.given.1, full.y)) {
            return None;
        }
        let body = if self.open { body.took } else { Vec2::ZERO };
        Some(self.regions_size(props, body, header, footer, end) + pad)
    }

    /// What a header, footer or end region took: what its content allocated.
    pub(super) fn region_size(&self, ui: &Ui) -> Vec2 {
        ui.min_rect().size()
    }

    /// The size the regions take together, from what the body, header, footer and end region took (zero for a region
    /// the container lacks or that took the whole room). An inline header shares the line; a footer in the end region
    /// stacks with its content; a full-height end region stands beside the body and the footer.
    pub(super) fn regions_size(&self, props: &ContainerProps, body: Vec2, header: Vec2, footer: Vec2, end: Vec2) -> Vec2 {
        let in_end = footer_in_end(props);
        let end = if in_end { Vec2::new(end.x.max(footer.x), end.y + footer.y) } else { end };
        let across = if in_end { Vec2::ZERO } else { footer };
        let gap = |width: f32| if width > 0.0 { width + self.region_gap } else { 0.0 };
        let inline = props.header.is_some_and(|header| header.inline);
        // The line: the body with an inline header before it.
        let line = if inline { Vec2::new(body.x + gap(header.x), body.y.max(header.y)) } else { body };
        let (width, height) = if end_full_height(props) {
            (line.x.max(across.x) + gap(end.x), (line.y + across.y).max(end.y))
        } else {
            ((line.x + gap(end.x)).max(across.x), line.y.max(end.y) + across.y)
        };
        if inline { Vec2::new(width, height) } else { Vec2::new(width.max(header.x), height + header.y + self.header_space(header.y)) }
    }

    /// The width the regions ask for in a content-sized container: what they took together in this pass's measuring run.
    pub(super) fn wanted_width(&self, props: &ContainerProps) -> Option<f32> {
        self.measured_body.map(|body| {
            let (header, end) = (self.measured_header.unwrap_or(Vec2::ZERO), self.measured_end.unwrap_or(Vec2::ZERO));
            let wanted = self.regions_size(props, body, header, self.measured_footer, end).x.max(self.least_width);
            overlay_of(props).map_or(wanted, |overlay| wanted.max(crate::contract::overlay::opener_floor(overlay)))
        })
    }

    /// The width the header, body and footer share in a content-sized container: the widest region's, at most the
    /// room; the whole room before any pass.
    pub(super) fn shared_width(&self, room: f32, props: &ContainerProps) -> f32 {
        self.wanted_width(props).map_or(room.max(0.0), |wanted| wanted.min(room.max(0.0)))
    }

    /// The sizes the regions are placed from before any of them ran in this run: the measuring run's.
    pub(super) fn measured_sizes(&self) -> RegionSizes {
        RegionSizes { header: self.measured_header, footer: self.measured_footer, end: self.measured_end }
    }

    /// The room a content-sized body runs in: what the mount offers, less the padding.
    pub(super) fn room_size(&self, rect: Rect, inner: Rect) -> Vec2 {
        self.room - (inner.min - rect.min) * 2.0
    }

    /// The room the mount offers: a layer's room, unless the host gives it a rect of its own size, else the rect the
    /// container is shown at, else the room of the flow.
    pub(super) fn mount_room(&self, props: &ContainerProps, ui: &Ui, id: Id, at: ShowAt) -> Vec2 {
        match (overlay_of(props), self.fixed) {
            (Some(overlay), false) => {
                let env = Env::of(ui);
                let contained = crate::contract::overlay::region_for(&env, overlay).is_some();
                let overlay = &if contained { overlay.in_region() } else { overlay.clone() };
                let room = Self::layer_room(ui.ctx(), &env, overlay).size();
                // An anchored layer's body runs in the room the anchor leaves on the side it takes, as the host
                // offers it: a body taller than that scrolls instead of covering the anchor.
                let ctx = ui.ctx();
                let own = Rect::from_min_max(ui.cursor().min, ui.cursor().min);
                let rtl = Env::of(ui).direction.is_rtl();
                match crate::contract::overlay::measuring_room(
                    ctx,
                    id,
                    overlay,
                    own,
                    areas::layer_to_screen(ctx, ui.layer_id()),
                    Env::of(ui).tokens.space.pad.regular,
                    rtl,
                ) {
                    Some(side) => side.min(room),
                    None => room,
                }
            }
            _ => match at {
                ShowAt::Rect(rect) => rect.size(),
                ShowAt::Flow => {
                    let room = crate::layout::flow_room(ui);
                    let room = Vec2::new(room.x, props.room_height.unwrap_or(room.y));
                    // In a row that wraps, a body too wide for the rest of the line moves to the next one: it measures
                    // in the whole line, so its allocation carries its width and the row can wrap it.
                    let layout = ui.layout();
                    if layout.main_wrap && layout.is_horizontal() { Vec2::new(room.x.max(ui.available_size().x), room.y) } else { room }
                }
            },
        }
    }

    /// The room a layer offers its content: the safe rect (the region of a contained layer), or the share of it the overlay caps its content at.
    fn layer_room(ctx: &Context, env: &Env, overlay: &crate::contract::Overlay) -> Rect {
        let bounds = crate::contract::overlay::screen_for(ctx, env, overlay);
        match overlay.fit {
            crate::contract::overlay::OverlayFit::ContentCapped(share) => {
                Rect::from_min_size(bounds.min, Vec2::new(bounds.width() * share.x, bounds.height() * share.y))
            }
            _ => bounds,
        }
    }
}

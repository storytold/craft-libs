//! Viewport mounts: opening a mount's OS viewport and its close requests.

use super::native::route_hit_test;
use super::placement::{record_placement, saved_placement};
use super::shell::{os_of, refresh};
use super::spec::{WindowSpec, frame_of_spec};
use super::{Record, refuse_close_under_modal, shared};
use crate::runtime::egui::viewports;
use crate::runtime::frame::PerViewport;
use craft_host::target::{Os, WindowChrome, WindowKind};
use egui::{Context, Id, Pos2, Ui, Vec2};

/// What a viewport mount asks for.
pub(crate) struct ViewportOpen<'a> {
    /// The container's id.
    pub id: Id,
    /// The window title.
    pub title: &'a str,
    /// The size to open with when nothing is saved.
    pub size: Option<Vec2>,
    /// The key the placement is saved under.
    pub save: Option<&'a str>,
    /// How the window opens.
    pub spec: &'a WindowSpec,
    /// The smallest and largest inner size, from the container's overlay.
    pub limits: (Option<Vec2>, Option<Vec2>),
    /// The outer position in desktop points when nothing is saved, from the container's overlay.
    pub position: Option<Pos2>,
    /// Whether the user can resize the window, from the container's overlay.
    pub resizable: bool,
}

/// Opens the OS viewport of a mount and runs `body` once for each pass it gets: with the saved placement, the OS
/// close request turned into a request to the container, and the placement saved after the pass.
pub(crate) fn show_viewport(ctx: &Context, open: &ViewportOpen<'_>, mut body: impl FnMut(&mut Ui)) {
    let viewport = viewports::viewport_id(open.id);
    let saved = open.save.and_then(|key| saved_placement(ctx, key));
    let (size, pos) = match &saved {
        Some(placement) => (Some(Vec2::from(placement.normal.size)), placement.position().map(Pos2::from)),
        None => (open.size, None),
    };
    let spec = open.spec;
    let active = !shared(ctx, |shared| shared.away);
    let os = os_of(ctx);
    let (chrome, captions) = frame_of_spec(ctx, spec);
    let level = PerViewport::<Record>::with(ctx, viewport, |record| {
        record.kind = Some(spec.kind);
        record.chrome = Some(chrome);
        record.captions = captions.clone();
        record.resizable = Some(open.resizable);
        record.awaiting_first_paint |= spec.shown_after_first_paint && record.facts.is_none();
        if record.title.as_deref() != Some(open.title) {
            record.title = Some(open.title.to_owned());
            // A routed window keeps its handle: the title the system shows changes after this pass.
            if record.sink.is_none() {
                record.native = None;
                record.searched = None;
            }
        }
        record.level.get_or_insert(spec.level).for_egui(active, os)
    });
    let opens = viewports::Spec {
        title: open.title,
        size,
        pos: pos.or(open.position),
        chrome,
        // A tool window has no OS caption buttons; its panel's title row carries a close action.
        os_buttons: !(spec.kind == WindowKind::Tool && captions.is_none() && chrome != WindowChrome::Native),
        min_size: open.limits.0,
        max_size: open.limits.1,
        level,
        resizable: open.resizable,
        taskbar: spec.taskbar,
        hidden: PerViewport::<Record>::with(ctx, viewport, |record| record.awaiting_first_paint),
    };
    viewports::show_immediate(ctx, viewport, &opens, |ui| {
        let ctx = ui.ctx().clone();
        refresh(&ctx, viewport, viewports::current(&ctx));
        if viewports::current(&ctx).close_requested && !refuse_close_under_modal(&ctx) {
            viewports::cancel_close(&ctx);
            shared(&ctx, |shared| {
                if !shared.close_requests.contains(&open.id) {
                    shared.close_requests.push(open.id);
                }
            });
        }
        body(ui);
        if chrome == WindowChrome::Integrated && os == Os::Windows {
            route_hit_test(&ctx, viewport);
        }
        if let Some(key) = open.save {
            record_placement(&ctx, viewport, key, &viewports::current(&ctx));
        }
    });
    if PerViewport::<Record>::with(ctx, viewport, |record| std::mem::take(&mut record.awaiting_first_paint)) {
        viewports::show(ctx, viewport);
    }
}

/// Whether the OS asked the viewport of container `id` to close in this root frame; once.
pub(crate) fn take_close_request(ctx: &Context, id: Id) -> bool {
    shared(ctx, |shared| {
        let before = shared.close_requests.len();
        shared.close_requests.retain(|requested| *requested != id);
        shared.close_requests.len() != before
    })
}

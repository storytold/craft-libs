//! `Lazy`: a child of a scroll area that runs only while it is in view, and a child that runs only when something it
//! shows changed (`runtime::lazy`).

use crate::layout::{flow_region, take_in_flow};
use crate::runtime::egui::alloc;
use crate::runtime::env::Env;
use crate::runtime::focus;
use crate::runtime::layout_memory;
use crate::runtime::lazy::{self, Decision, Place};
use crate::runtime::memory::sub;
use crate::runtime::replay;
use egui::{Pos2, Rect, Ui, UiBuilder, Vec2};
use std::hash::Hash;

/// A child of a scroll area that runs only while it is in view. Out of view its content does not run: it takes the
/// extent it had in the last pass that showed it, at any room, and its stops from that pass keep their place in Tab
/// order. A child no pass showed yet takes its estimate: that long along the axes the area scrolls and nothing across
/// them, as a collection places the rows it never built by its item layout's estimate. It runs when that extent
/// meets the view, and while one of the stops of its last run holds keyboard focus or is the target of a focus
/// request. Content that was never shown has no stops to find: Tab reaches it once it was shown, and an app brings
/// it into view with `reveal`. Outside a scroll area it always runs.
///
/// It is a child in flow of its body. In a `Grid` or spread `Row` body it holds one cell: its content is the one child
/// in flow that takes the cell, and out of view it takes the cell at its remembered extent, so the cell's row counts
/// it.
///
/// The key names the child among every `Lazy` of its scroll area, nested containers included. The idiom is
/// `Lazy::new(key, estimate).show(ui, |ui| ..)` around each large child (a card, a section of a panel) in the scroll
/// body.
///
/// A settling child ([`Lazy::settles`]) also keeps what its last run painted and registered, and while it is settled
/// it adds that again in place of running, in view or not.
#[must_use]
pub struct Lazy {
    key: u64,
    estimate: f32,
    reveal: bool,
    settles: bool,
}

impl Lazy {
    /// The child keyed `key` in its scroll area. `estimate` is how long it is along the axes the area scrolls, an
    /// app's guess from what it knows without running the child; the child's real extent replaces it once it ran.
    pub fn new(key: impl Hash, estimate: f32) -> Lazy {
        use std::hash::Hasher;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        Lazy { key: hasher.finish(), estimate, reveal: false, settles: false }
    }

    /// When `reveal` holds, the scroll area glides until the child starts at the view's start (as far as the
    /// content lets it), whether or not the child ran yet: a request of the pass that sets it, as an app's jump to
    /// an item is. A wheel, drag or other move of the area ends the glide.
    pub fn reveal(mut self, reveal: bool) -> Lazy {
        self.reveal = reveal;
        self
    }

    /// When `settles` holds, the child runs only when something it shows may have changed, inside a scroll area or
    /// not. Otherwise it repaints its last run and registers again what that run registered (its stops, hit shapes,
    /// widgets and accessibility nodes), as if it ran. It runs in a frame where any of these holds:
    /// - the pointer is over it or was in its last run, a press started on it, or it holds focus or held it;
    /// - an animation in it is not at its end, something in it asked for a repaint, or a wake time it asked for came;
    /// - a bound value it read was written;
    /// - its place, room, clip or scale differs from its last run;
    /// - the frame or the one before had input other than pointer motion, the app asked for a repaint, or a given
    ///   value was edited: the framework cannot see which children show plain app data, so every child runs then;
    /// - its last run showed an overlay or painted on another layer.
    ///
    /// A bound value is known by where it lives, so a settled reader needs a binding that stays in place between
    /// frames: a value copied into a local each frame and bound there is a new place each frame, and an edit of it
    /// does not reach the reader until the next frame with input.
    pub fn settles(mut self, settles: bool) -> Lazy {
        self.settles = settles;
        self
    }

    /// Runs `add` in a child `Ui` placed in flow in `ui`, or takes the remembered extent there; `None` when it did not
    /// run.
    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        let ctx = ui.ctx().clone();
        let view = Env::of(ui).view().lazy_view();
        let id = match view {
            Some(view) => sub(view.owner, ("lazy", self.key)),
            None => sub(ui.scope_id(), ("lazy", self.key)),
        };
        let available = flow_region(ui);
        let rtl = ui.layout().prefer_right_to_left();
        let sizing = ui.is_sizing_pass();
        // The child is not part of its body's transcript: in a real run it counts there by its extent, whether it
        // runs or not, so what scrolls into reach does not change the body's content.
        let real = !sizing && !crate::runtime::measure::active(&ctx);
        let owner = view.map(|view| view.owner);
        // A run around this one that is replayed later keeps the child's entry.
        replay::journal(&ctx, || move |ctx: &egui::Context| lazy::keep(ctx, owner, id));
        if let Some(view) = view {
            let at = |size: Vec2| {
                let x = if rtl { available.max.x - size.x } else { available.min.x };
                Rect::from_min_size(Pos2::new(x, available.min.y), size)
            };
            let estimate = view.estimate(self.estimate);
            if self.reveal {
                crate::contract::queue_child_reveal(&ctx, view.owner, id);
            }
            if let Decision::Skip(size) = lazy::decide(&ctx, id, &view, at, sizing, estimate) {
                let (took, _) = take_in_flow(ui, size);
                if real {
                    layout_memory::backstop(&ctx, id, size);
                    lazy::replay(&ctx, id);
                    lazy::place(&ctx, &view, id, took);
                }
                return None;
            }
        }
        let layout = *ui.layout();
        let place = Place { room: available, clip: ui.clip_rect(), layer: ui.layer_id(), layout, pixels_per_point: ctx.pixels_per_point() };
        if self.settles
            && let Some(size) = lazy::settled(&ctx, owner, id, sizing, real.then_some(place))
        {
            let (took, _) = take_in_flow(ui, size);
            if real {
                layout_memory::backstop(&ctx, id, size);
                lazy::replay_settled(&ctx, id);
                if let Some(view) = view {
                    lazy::place(&ctx, &view, id, took);
                }
            }
            return None;
        }
        let mark = focus::mark(&ctx);
        let records = self.settles && real;
        if records {
            replay::open(&ctx);
        }
        let mut child = alloc::child(ui, UiBuilder::new().max_rect(available).layout(layout).scope_id(id));
        // Its extent in the body around it comes from its content's measure, so a change inside reaches that body.
        let out = layout_memory::boundary(&ctx, false, || add(&mut child));
        let rect = child.min_rect();
        // The child `Ui` writes its widget rect when it drops: the record holds it as written.
        drop(child);
        let record = records.then(|| replay::close(&ctx, place.layer));
        if real {
            layout_memory::backstop(&ctx, id, rect.size());
        }
        alloc::advance_past(ui, rect);
        if view.is_some() || self.settles {
            let registered = real.then(|| record.as_ref().map_or_else(|| focus::transcript(&ctx, mark), |record| record.focus.clone()));
            lazy::shown(&ctx, owner, id, rect.size(), sizing, registered);
        }
        if let Some(view) = view
            && real
        {
            lazy::place(&ctx, &view, id, rect);
        }
        if let Some(record) = record {
            lazy::ran(&ctx, id, record, place, rect);
        }
        Some(out)
    }
}

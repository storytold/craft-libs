//! `Lazy`: a child of a scroll area that runs only while it is in view (`runtime::lazy`).

use crate::layout::{flow_region, take_in_flow};
use crate::runtime::egui::alloc;
use crate::runtime::env::Env;
use crate::runtime::focus;
use crate::runtime::layout_memory;
use crate::runtime::lazy::{self, Decision};
use crate::runtime::memory::sub;
use egui::{Pos2, Rect, Ui, UiBuilder, Vec2};
use std::hash::Hash;

/// A child of a scroll area that runs only while it is in view. Out of view its content does not run: it takes the
/// extent it had in the last pass that showed it, at any room, and its stops from that pass keep their place in Tab
/// order. It runs when it was never shown, when it meets the view, while one of the stops of its last run holds
/// keyboard focus or is the target of a focus request, and when `keep` asks. Content that was never shown has no
/// stops to find: a child an app focuses or scrolls to before it was ever in view needs `keep`. Outside a scroll
/// area it always runs.
///
/// It is a child in flow of its body. In a `Grid` or spread `Row` body it holds one cell: its content is the one child
/// in flow that takes the cell, and out of view it takes the cell at its remembered extent, so the cell's row counts
/// it.
///
/// The key names the child among every `Lazy` of its scroll area, nested containers included. The idiom is
/// `Lazy::new(key).show(ui, |ui| ..)` around each large child (a card, a section of a panel) in the scroll body.
#[must_use]
pub struct Lazy {
    key: u64,
    keep: bool,
}

impl Lazy {
    /// The child keyed `key` in its scroll area.
    pub fn new(key: impl Hash) -> Lazy {
        use std::hash::Hasher;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut hasher);
        Lazy { key: hasher.finish(), keep: false }
    }

    /// Runs the child out of view too, while `keep` holds: the target of a scroll the app asks for.
    pub fn keep(mut self, keep: bool) -> Lazy {
        self.keep = keep;
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
        if let Some(view) = view {
            let at = |size: Vec2| {
                let x = if rtl { available.max.x - size.x } else { available.min.x };
                Rect::from_min_size(Pos2::new(x, available.min.y), size)
            };
            if let Decision::Skip(size) = lazy::decide(&ctx, id, &view, at, sizing, self.keep) {
                take_in_flow(ui, size);
                if real {
                    layout_memory::backstop(&ctx, id, size);
                    lazy::replay(&ctx, id);
                }
                return None;
            }
        }
        let mark = focus::mark(&ctx);
        let layout = *ui.layout();
        let mut child = alloc::child(ui, UiBuilder::new().max_rect(available).layout(layout).scope_id(id));
        // Its extent in the body around it comes from its content's measure, so a change inside reaches that body.
        let out = layout_memory::boundary(&ctx, false, || add(&mut child));
        let rect = child.min_rect();
        if real {
            layout_memory::backstop(&ctx, id, rect.size());
        }
        alloc::advance_past(ui, rect);
        if view.is_some() {
            lazy::shown(&ctx, id, rect.size(), sizing, real.then(|| focus::transcript(&ctx, mark)));
        }
        Some(out)
    }
}

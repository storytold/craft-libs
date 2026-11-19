//! `Pages`: pages that take turns in one place, which keeps the size of the largest so a switch neither moves nor
//! resizes what is around them.

use crate::layout::flow_region;
use crate::runtime::egui::alloc;
use crate::runtime::layout_memory;
use crate::runtime::lazy::{self, Place};
use crate::runtime::memory::sub;
use crate::runtime::{measure, reads, replay};
use egui::{Pos2, Rect, Ui, UiBuilder, Vec2};
use std::hash::Hash;

/// Pages that take turns in one place, such as the paint kinds of a paint chip's popover: one shows, and the place
/// is as wide and as tall as the largest of them, so switching the page moves and resizes nothing around it. A child
/// of the shown page that fills its width takes the width of the largest page.
///
/// The pages it does not show are measured only. Each runs in a measuring run (`measure::run`) inside an invisible egui
/// sizing child, where every component stops after its measure (steps 1 to 6): it senses nothing and adds no hit
/// shape, focus stop, access node, paint or memory write. A hidden page reuses the size its last measuring run found
/// while nothing it read has changed, under the settled rules of `Lazy::settles` for content without a content
/// version: it is measured again in a frame where a value it read was written, something in it still moves, the app
/// asked for a repaint, or the frame or the one before had input other than pointer motion.
///
/// The page closure runs for the shown page and for every hidden page it measures, so it builds what it shows from its
/// page argument alone and changes nothing outside a component's value (a measuring run applies no edit).
#[must_use]
pub struct Pages<'a, K> {
    salt: u64,
    pages: &'a [K],
    current: &'a K,
}

impl<'a, K: Hash + PartialEq> Pages<'a, K> {
    /// The pages `pages`, with `current` shown. `salt` names this set among the others in the same `Ui`.
    pub fn new(salt: impl Hash, pages: &'a [K], current: &'a K) -> Self {
        Pages { salt: hash(&salt), pages, current }
    }

    /// Shows the current page through `page` in a child `Ui` placed in flow in `ui`, at the size of the largest page,
    /// and returns what `page` returned for it.
    pub fn show<R>(self, ui: &mut Ui, mut page: impl FnMut(&mut Ui, &K) -> R) -> R {
        let ctx = ui.ctx().clone();
        let base = sub(ui.scope_id(), ("pages", self.salt));
        let region = flow_region(ui);
        let layout = *ui.layout();
        let hidden = self.pages.iter().filter(|key| *key != self.current).fold(Vec2::ZERO, |largest, key| {
            let id = sub(base, hash(key));
            let place = Place {
                room: Rect::from_min_size(Pos2::ZERO, Vec2::new(region.width(), f32::INFINITY)),
                clip: Rect::EVERYTHING,
                layer: ui.layer_id(),
                layout,
                pixels_per_point: ctx.pixels_per_point(),
                env: crate::runtime::env::Env::of(ui).key,
            };
            let kept = lazy::measured(&ctx, id, place).map(|(size, reads)| {
                // The page reads them again, as far as an edit later in the pass sees it.
                for key in reads {
                    reads::note(&ctx, key, id, true, &[]);
                }
                size
            });
            let size = kept.unwrap_or_else(|| {
                replay::open(&ctx);
                let size = measure::run(&ctx, || {
                    // What the hidden page measures joins no transcript around it: the run that shows the pages does
                    // not show it, so nothing there checks it.
                    layout_memory::boundary(&ctx, true, || {
                        let room = Rect::from_min_size(region.min, Vec2::new(region.width(), f32::INFINITY));
                        let mut child = alloc::child(ui, UiBuilder::new().max_rect(room).layout(layout).scope_id(id).sizing_pass().invisible());
                        page(&mut child, key);
                        child.min_rect().size()
                    })
                });
                let record = replay::close(&ctx, ui.layer_id());
                lazy::measured_run(&ctx, id, record.facts, place, size);
                size
            });
            largest.max(size)
        });
        let id = sub(base, hash(self.current));
        let mut child = alloc::child(ui, UiBuilder::new().max_rect(region).layout(layout).scope_id(id));
        child.set_min_size(hidden.min(region.size()));
        let out = page(&mut child, self.current);
        let rect = child.min_rect();
        drop(child);
        // What the place took comes from the hidden pages' measures as well as from the shown page's components.
        layout_memory::backstop(&ctx, base, rect.size());
        alloc::advance_past(ui, rect);
        out
    }
}

fn hash(value: &impl Hash) -> u64 {
    use std::hash::Hasher as _;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

//! The marquee of a collection (17.4, 8.7): a drag from empty space between rows sweeps a rectangle, and the items
//! it touches become the selection the drag started from, changed the way the held modifiers say.

use super::ItemsHooks;
use crate::contract::collection::{CollectionProps, SelectMode};
use crate::contract::{Collection, Cx};
use crate::runtime::env::Env;
use craft_behavior::{ItemLayout, MarqueeSelect, PointerButton, PressPhase, ScrollPos, SelectionChange};
use egui::{Pos2, Rect, Ui, Vec2};

/// Rows per second the view scrolls at when the pointer is a full edge band beyond the view's edge.
const EDGE_ROWS_PER_SECOND: f32 = 4.0;

/// A marquee in progress. Its corners are in the layout's logical content coordinates, so a scroll during the drag
/// moves the rectangle with the rows.
#[derive(Clone)]
pub(super) struct MarqueeDrag<K> {
    select: MarqueeSelect<K>,
    /// The selection last reported, so a pass that sweeps the same items reports nothing.
    last: Vec<K>,
    live: bool,
}

/// `at`, logical and relative to the rows' rect, in content coordinates at `offset`.
fn content_point(at: Pos2, offset: ScrollPos) -> Pos2 {
    at + Vec2::new(offset.x as f32, offset.y as f32)
}

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// Step 9: starts a marquee on a primary press on empty space of a collection with `marquee` and a multiple
    /// selection, follows the drag, selects through `MarqueeSelect` over the items it sweeps and scrolls at the edges.
    /// It ends with the release, and a cancel restores the selection the press found.
    pub(super) fn marquee_step(
        &mut self,
        comp: &C,
        cx: &mut Cx<'_, C>,
        layout: &C::ItemLayout,
        (rect, offset, rtl): (Rect, ScrollPos, bool),
        props: &CollectionProps,
    ) {
        let xf = self.xf;
        let layer = |point: Pos2| xf.map_or(point, |map| map.to_layer(point));
        match cx.core.press.phase {
            PressPhase::Down { start, .. }
                if props.marquee && start.button == PointerButton::Primary && Self::select_mode(comp) == SelectMode::Multi =>
            {
                let origin = layer(start.origin);
                // The press lands on the rows the user saw: the offset of the last pass.
                let seen = ScrollPos { x: self.offset_x_at_read, y: self.offset_at_read };
                let local = Self::logical(layout, rect, rtl, origin);
                if rect.contains(origin) && layout.hit(local, seen).is_none() {
                    let selected: Vec<C::Key> = self.selected_keys(&self.selection_at_read);
                    let mods = crate::runtime::pointer::frame(&cx.core.ctx).mods;
                    let select = MarqueeSelect::begin(content_point(local, seen), mods, selected.clone());
                    self.drags.marquee = Some(MarqueeDrag { select, last: selected, live: false });
                }
            }
            PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                if let Some(mut marquee) = self.drags.marquee.take() {
                    marquee.live = true;
                    let at = Self::logical(layout, rect, rtl, layer(pos));
                    self.edge_scroll(cx, layout, rect.height(), at.y);
                    let now = ScrollPos { x: offset.x, y: self.offset };
                    marquee.select.update(content_point(at, now));
                    self.sweep_marquee(layout, &mut marquee);
                    self.drags.marquee = Some(marquee);
                }
            }
            PressPhase::Release { .. } => self.drags.marquee = None,
            PressPhase::Cancel { .. } => {
                if let Some(marquee) = self.drags.marquee.take()
                    && marquee.live
                {
                    self.report_selection(marquee.select.cancel());
                }
            }
            _ => {}
        }
    }

    fn report_selection(&mut self, keys: Vec<C::Key>) {
        self.change(if keys.is_empty() { SelectionChange::Clear } else { SelectionChange::Set(keys) });
    }

    /// Reports the selection the marquee makes of the items in its rectangle, when it differs from the last report.
    fn sweep_marquee(&mut self, layout: &C::ItemLayout, marquee: &mut MarqueeDrag<C::Key>) {
        let swept = marquee.select.rect();
        let mut touched = Vec::new();
        layout.visible(ScrollPos { x: f64::from(swept.min.x), y: f64::from(swept.min.y) }, swept.size(), &mut touched);
        let items = touched
            .into_iter()
            .filter(|&at| self.source.enabled(at) && !self.source.is_section(at))
            .map(|at| (self.source.key(at), layout.rect(at, ScrollPos::default())));
        let keys = marquee.select.selection(items);
        if keys != marquee.last {
            marquee.last = keys.clone();
            self.report_selection(keys);
        }
    }

    /// Scrolls the rows while the pointer, `y` below the rows' top edge, is within a row of an edge of the `view`,
    /// faster the further past it: the rate of a row drag near the edge of a scroll region.
    fn edge_scroll(&mut self, cx: &Cx<'_, C>, layout: &C::ItemLayout, view: f32, y: f32) {
        if !self.scrolls || self.source.is_empty() {
            return;
        }
        let row = layout.rect(0, ScrollPos::default()).height();
        let band = row.min(view / 2.0);
        if band <= 0.0 {
            return;
        }
        let depth = if y < band { (band - y) / band } else { (y - (view - band)) / band };
        let direction = if y < band { -1.0 } else { 1.0 };
        if depth <= 0.0 {
            return;
        }
        let dt = cx.core.ctx.input(|input| input.stable_dt);
        let step = depth.min(1.0) * EDGE_ROWS_PER_SECOND * row * dt;
        self.offset = (self.offset + f64::from(direction * step)).clamp(0.0, self.max_offset(view));
        cx.core.ctx.request_repaint();
    }

    /// Paints the marquee rectangle over the rows with the overlay marquee look.
    pub(super) fn paint_marquee(&self, ui: &Ui, env: &Env, layout: &C::ItemLayout, (rect, offset, rtl): (Rect, ScrollPos, bool)) {
        let Some(marquee) = self.drags.marquee.as_ref().filter(|marquee| marquee.live) else { return };
        let swept = marquee.select.rect().translate(-Vec2::new(offset.x as f32, offset.y as f32));
        let shown = self.to_screen(layout, rect, swept, rtl);
        let corners = [shown.left_top(), shown.right_top(), shown.right_bottom(), shown.left_bottom()].map(|corner| self.on_screen(corner));
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        painter.extend(crate::primitives::marching_ants(&env.tokens, &corners, true, 0.0, painter.clip_rect(), painter.pixels_per_point()));
    }
}

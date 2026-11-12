//! The edge press of a collection that takes one (`Collection::edge_press`): with its modifiers held, the line between
//! two adjacent rows is a target of its own (Photoshop's Alt-click between two layers clips the upper one).

use super::{EDGE, ItemsHooks, ScrollPos};
use crate::contract::collection::CollectionEvent;
use crate::contract::{Collection, Cx, PartsCx};
use craft_behavior::hit::LINE_REACH;
use craft_behavior::{HitLevel, HitPriority, ItemLayout as _, Sense};
use egui::Rect;

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// Declares the band of the line under the pointer, while the edge press's modifiers are held and its test takes
    /// the two rows: a part over the rows, with the test's cursor, that takes the press from both. The band reaches
    /// as far from the line as a line's hit does (`LINE_REACH`), across the row; a toggle cell under the pointer keeps
    /// its press.
    pub(super) fn edge_parts(&mut self, comp: &C, pcx: &mut PartsCx<'_, C>) {
        self.edge = None;
        let Some(rule) = comp.edge_press() else { return };
        let frame = crate::runtime::pointer::frame(pcx.measure.ui.ctx());
        let (Some(pos), true) = (frame.pos, frame.mods == rule.mods()) else { return };
        let Some(layout) = self.layout.clone() else { return };
        let view = self.below_header(pcx.content_rect);
        let rtl = pcx.flow.is_rtl();
        let at = self.xf.map_or(pos, |map| map.to_layer(pos));
        if !view.contains(at) {
            return;
        }
        let offset = ScrollPos { x: self.offset_x, y: self.offset.clamp(0.0, self.max_offset(view.height())) };
        let local = Self::logical(&layout, view, rtl, at);
        let Some(row) = layout.hit(local, offset) else { return };
        if self.toggle_cell(comp.items(), &layout, row, offset, local).is_some() {
            return;
        }
        let rect = self.screen(&layout, view, row, offset, rtl);
        let (above, below, line) = if at.y - rect.top() <= LINE_REACH && row > 0 {
            (row - 1, row, rect.top())
        } else if rect.bottom() - at.y <= LINE_REACH && row + 1 < self.source.len() {
            (row, row + 1, rect.bottom())
        } else {
            return;
        };
        let usable = |at: usize| self.source.enabled(at) && layout.shown(at);
        if !usable(above) || !usable(below) {
            return;
        }
        let (above, below) = (self.source.key(above), self.source.key(below));
        let Some(cursor) = rule.test(&above, &below) else { return };
        let band = Rect::from_x_y_ranges(rect.x_range(), (line - LINE_REACH)..=(line + LINE_REACH));
        pcx.part(EDGE, band)
            .sense(Sense::CLICK)
            .over()
            .priority(HitPriority::new(HitLevel::Edge, 0))
            .cursor(cursor)
            .pointer_only("The edge press is a pointer shortcut; the app's commands do the same from the keyboard");
        self.edge = Some((above, below));
    }

    /// Reports a click on the band declared this pass as `CollectionEvent::EdgePressed`.
    pub(super) fn edge_step(&mut self, cx: &mut Cx<'_, C>) {
        let Some((above, below)) = self.edge.clone() else { return };
        if cx.part_activated(EDGE).is_some() {
            let mods = crate::runtime::pointer::frame(&cx.core.ctx).mods;
            self.events.push(CollectionEvent::EdgePressed { above, below, mods });
        }
    }
}

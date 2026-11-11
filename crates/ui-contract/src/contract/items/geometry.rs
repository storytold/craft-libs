//! Where rows sit: the rows' rect and scroll range, which rows a pass builds, and rows on the screen.

use super::ItemsHooks;
use crate::contract::{Collection, Cx};
use craft_behavior::{Flow, ItemLayout, ScrollPos, SelectionModel};
use egui::{Pos2, Rect};

impl<'s, 'r, C: Collection> ItemsHooks<'s, 'r, C> {
    /// The rect of the rows: the content rect below the header row.
    pub(super) fn below_header(&self, area: Rect) -> Rect {
        Rect::from_min_max(Pos2::new(area.min.x, area.min.y + self.header.height), area.max)
    }

    pub(super) fn rows_rect(&self, cx: &Cx<'_, C>) -> Rect {
        self.below_header(cx.core.content_rect)
    }

    pub(super) fn max_offset(&self, view: f32) -> f64 {
        f64::from((self.content.y - view).max(0.0))
    }

    pub(super) fn page_rows(&self, layout: &C::ItemLayout, view: f32) -> usize {
        let row = if self.source.is_empty() { 0.0 } else { layout.rect(0, ScrollPos::default()).height() };
        if row > 0.0 { ((view / row) as usize).saturating_sub(1).max(1) } else { 1 }
    }

    /// `point` of the rows' coordinates on the screen.
    pub(super) fn on_screen(&self, point: Pos2) -> Pos2 {
        self.xf.map_or(point, |map| map.to_screen(point))
    }
}

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// The rows in view and the positions the pass builds: the rows in view (out-of-view content runs nowhere,
    /// `runtime::lazy`), then the rows kept alive while scrolled out (the lead, the anchor, the renaming row and the
    /// grabbed row).
    pub(super) fn built(&self, layout: &C::ItemLayout, view: Rect, offset: ScrollPos) -> (Vec<usize>, Vec<usize>) {
        let mut shown = Vec::new();
        layout.visible(offset, view.size(), &mut shown);
        let mut order = shown.clone();
        let keep = [self.lead.key.as_ref(), self.selection.anchor(), self.renaming.key.as_ref(), self.drags.grabbed.as_ref()];
        // An item of a closed group has no rect: it is never built.
        for at in keep.into_iter().flatten().filter_map(|key| self.index_of(key)).filter(|&at| layout.shown(at)) {
            if !order.contains(&at) {
                order.push(at);
            }
        }
        (shown, order)
    }

    /// The rows since the last section row up to and including `at` (0 for a section row), continuing from `last`, the
    /// result for an earlier row: stripes start again after a section.
    pub(super) fn stripe_run(&self, last: Option<(usize, usize)>, at: usize) -> usize {
        match last {
            Some((from, run)) if from <= at => (from + 1..=at).fold(run, |run, row| if self.source.is_section(row) { 0 } else { run + 1 }),
            _ => (0..=at).rev().take_while(|&row| !self.source.is_section(row)).count(),
        }
    }

    /// The screen rect of row `at`: logical layout coordinates mirrored under right-to-left, then moved to the view.
    pub(super) fn screen(&self, layout: &C::ItemLayout, view: Rect, at: usize, offset: ScrollPos, rtl: bool) -> Rect {
        self.to_screen(layout, view, layout.rect(at, offset), rtl)
    }

    /// A rect of the layout's logical, viewport-relative coordinates on the screen.
    pub(super) fn to_screen(&self, layout: &C::ItemLayout, view: Rect, local: Rect, rtl: bool) -> Rect {
        let local = if rtl && layout.mirrors() { Flow::new(Rect::from_min_size(Pos2::ZERO, view.size()), true).to_physical(local) } else { local };
        local.translate(view.min.to_vec2())
    }

    /// The layout's grid view on the screen: sections placed in the view and mirrored under right-to-left.
    pub(super) fn grid_screen(&self, layout: &C::ItemLayout, grid: &craft_behavior::GridView, view: Rect, rtl: bool) -> craft_behavior::GridView {
        let mirrored = rtl && layout.mirrors();
        let edge = |x: f32| if mirrored { view.max.x - x } else { view.min.x + x };
        craft_behavior::GridView::new(grid.sections.iter().map(|section| {
            let edges = section.edges.iter().map(|&x| edge(x)).collect();
            craft_behavior::GridSection::new(self.to_screen(layout, view, section.rect, rtl), section.columns.clone(), edges)
        }))
    }
}

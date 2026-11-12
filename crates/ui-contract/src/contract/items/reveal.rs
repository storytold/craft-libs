//! Bringing a row into view: a requested reveal and the scroll that does it.

use super::measure::RowMeasure;
use super::{ItemsHooks, rows};
use crate::contract::Collection;
use crate::contract::collection::LeadSource;
use crate::runtime::env::{Env, EnvView};
use crate::runtime::memory::MemoryStage;
use craft_behavior::{ItemLayout, Proposal, ScrollPos};
use egui::{Id, Rect, Ui, Vec2};
use std::sync::Arc;

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// The item the collection was asked to reveal (`Collection::reveal`, a one-shot request of the pass): the lead
    /// moves to it and it scrolls into view.
    pub(super) fn requested_reveal(&mut self, comp: &C, layout: &C::ItemLayout) {
        if let Some(at) = comp.reveal().and_then(|key| self.index_of(key)).filter(|&at| !self.source.section_stop(at)) {
            self.move_lead(at, LeadSource::Keys);
            self.reveal_with(layout, at);
        }
    }

    /// Brings row `at` into view: the collection's own scroll moves before the rows are built (`place_reveal`), or,
    /// without one, the page's scroll owner is asked at the end of the item step (`page_reveal`).
    pub(super) fn reveal_with(&mut self, layout: &C::ItemLayout, at: usize) {
        if !layout.shown(at) {
            return;
        }
        if self.scrolls {
            self.revealing = Some(at);
        } else {
            self.page_reveal = Some(at);
        }
    }

    /// Scrolls the row this pass reveals into the rows' view `view`, before the rows are built. A row the pass builds
    /// for the first time takes its measured height when built and moves the rows after it, so the auto rows that
    /// share the view with the revealed row, up to it, and have no kept height are built and measured first, under the
    /// rows' env `installed`, until the scroll that shows the row places it by measured heights only. The build takes
    /// those heights (`measured_for_reveal`) and does not measure the rows again.
    pub(super) fn place_reveal(
        &mut self,
        comp: &C,
        ui: &Ui,
        (id, view_env, installed): (Id, EnvView<'_>, &Env),
        layout: &mut Arc<C::ItemLayout>,
        view: Rect,
    ) {
        let Some(at) = self.revealing.take().filter(|&at| layout.shown(at)) else { return };
        let ctx = ui.ctx().clone();
        loop {
            self.aim_reveal(layout, at, view.height());
            let mut in_view = Vec::new();
            layout.visible(ScrollPos { x: self.offset_x, y: self.offset }, view.size(), &mut in_view);
            let source = self.source;
            let unmeasured: Vec<usize> = rows::measured_heights(&ctx, id, |height| {
                in_view
                    .into_iter()
                    .chain([at])
                    .filter(|&row| row <= at && layout.is_auto(row) && !source.section_stop(row))
                    .filter(|row| !self.measured_for_reveal.contains(row) && height(rows::measured_id(id, &source.key(*row))).is_none())
                    .collect()
            });
            if unmeasured.is_empty() {
                return;
            }
            let mut memory = MemoryStage::open_peek(&ctx, id);
            for row in unmeasured {
                if self.measured_for_reveal.contains(&row) {
                    continue;
                }
                let rect = layout.rect(row, ScrollPos::default());
                let mut cells = Vec::new();
                layout.cells(row, ScrollPos::default(), &mut cells);
                let measure = RowMeasure { at: row, rect, cells, proposal: Proposal::Width(rect.width()) };
                let (size, _) =
                    crate::contract::steps::measuring(&ctx, || self.measure_row(comp, ui, (id, view_env, installed), &mut memory, measure));
                let measured = rows::measured_id(id, &self.source.key(row));
                #[cfg(feature = "testing")]
                crate::runtime::measure::count_row_measure(&ctx, measured);
                rows::record_measure(&ctx, id, measured, Vec2::new(rect.width(), size.y));
                Arc::make_mut(layout).set_height(row, size.y);
                self.measured_for_reveal.push(row);
            }
            let content = layout.content_size();
            self.content = Vec2::new(content.x as f32, content.y as f32);
        }
    }

    /// Moves the offset so that row `at` shows wholly in a view `view` high, from the nearer edge, by the layout's
    /// extents.
    fn aim_reveal(&mut self, layout: &C::ItemLayout, at: usize, view: f32) {
        let rect = layout.rect(at, ScrollPos { x: 0.0, y: self.offset });
        if rect.min.y < 0.0 {
            self.offset += f64::from(rect.min.y);
        } else if rect.max.y > view {
            self.offset += f64::from(rect.max.y - view);
        }
        self.offset = self.offset.clamp(0.0, self.max_offset(view));
    }
}

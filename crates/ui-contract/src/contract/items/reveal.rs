//! Bringing a row into view: a requested reveal and the scroll that does it.

use super::measure::{Known, RowMeasure};
use super::{ItemsHooks, key_hash, rows};
use crate::contract::Collection;
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
            let key = self.source.key(at);
            if self.lead.key.as_ref() != Some(&key) {
                self.lead.rested = None;
            }
            self.lead.key = Some(key);
            self.lead.at = at;
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
    /// those heights (they are this pass's measure) and does not measure the rows again. The rows are measured with
    /// the state `known` the pass leaves them in; returns them.
    pub(super) fn place_reveal(
        &mut self,
        comp: &C,
        ui: &Ui,
        (id, view_env, installed): (Id, EnvView<'_>, &Env),
        (layout, known): (&mut Arc<C::ItemLayout>, &Known<C::Key>),
        view: Rect,
    ) -> Vec<usize> {
        let mut measured = Vec::new();
        let Some(at) = self.revealing.take().filter(|&at| layout.shown(at)) else { return measured };
        let ctx = ui.ctx().clone();
        let (striped, tree) = (comp.items().striped, matches!(layout.nav(), craft_behavior::NavShape::Tree));
        loop {
            self.aim_reveal(layout, at, view.height());
            let mut in_view = Vec::new();
            layout.visible(ScrollPos { x: self.offset_x, y: self.offset }, view.size(), &mut in_view);
            let source = &*self.source;
            let unmeasured: Vec<usize> = rows::measured_heights(&ctx, id, |height| {
                in_view
                    .into_iter()
                    .chain([at])
                    .filter(|&row| row <= at && layout.is_auto(row) && !source.section_stop(row))
                    .filter(|row| !measured.contains(row) && height(rows::measured_id(id, &source.key(*row))).is_none())
                    .collect()
            });
            if unmeasured.is_empty() {
                return measured;
            }
            let mut memory = MemoryStage::open_peek(&ctx, id);
            for row in unmeasured {
                if measured.contains(&row) {
                    continue;
                }
                let rect = layout.rect(row, ScrollPos::default());
                let mut cells = Vec::new();
                layout.cells(row, ScrollPos::default(), &mut cells);
                let odd = striped && self.stripe_run(None, row).is_multiple_of(2) && !self.source.is_section(row);
                let (state, flags) = self.known_row(row, known, odd);
                let measure = RowMeasure { at: row, rect, cells, state, flags, tree };
                let (size, fits) = crate::contract::steps::measuring(&ctx, || {
                    self.measure_row(comp, ui, (id, view_env, installed), &mut memory, measure, |measure, _, fits| {
                        (measure(Proposal::Width(rect.width())).0, fits)
                    })
                });
                let key = self.source.key(row);
                let height_id = rows::measured_id(id, &key);
                #[cfg(feature = "testing")]
                crate::runtime::measure::count_row_measure(&ctx, height_id);
                rows::record_measure(&ctx, id, height_id, Vec2::new(rect.width(), size.y));
                Arc::make_mut(layout).set_height(row, size.y);
                let fit = fits.then_some(size.x);
                if rows::keep_fit(&ctx, id, key_hash(&key), fit) {
                    Arc::make_mut(layout).set_fit(row, fit);
                }
                measured.push(row);
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

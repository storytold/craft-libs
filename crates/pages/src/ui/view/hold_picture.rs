//! Paint-only retention for keyboard turns. The page-picture store owns the textures.

use super::PlacedPage;
use craft_ui::primitives::textures::{CachedTexture, TextureState, ThumbRequest};
use egui::{Rect, emath::GuiRounding};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct PictureGeometry {
    pub(super) epoch: u64,
    pub(super) viewport: Rect,
    pub(super) pixels_per_point: f32,
    pub(super) rotation: u16,
}

#[derive(Clone, Debug)]
pub(super) struct ReadyRow {
    geometry: PictureGeometry,
    pages: Vec<ReadyPage>,
    /// Set only by a keyboard turn. Repeats can update it while the same ready row is still being painted.
    target: Option<usize>,
}

#[derive(Clone, Debug)]
struct ReadyPage {
    page: usize,
    rect: Rect,
    band: (f32, f32),
    scale: f64,
}

impl ReadyRow {
    pub(super) fn show(
        kept: &mut Option<Self>,
        geometry: PictureGeometry,
        pages: &[PlacedPage],
        mut cached: impl FnMut(usize, &ThumbRequest) -> TextureState,
    ) -> Option<Vec<CachedTexture>> {
        let previous = kept.take();
        if pages.is_empty() || pages.len() > 2 {
            return None;
        }
        if pages.iter().all(|page| matches!(page.picture, TextureState::Ready(_))) {
            *kept = Some(Self {
                geometry,
                pages: pages.iter().map(|page| ReadyPage { page: page.page, rect: page.rect, band: page.band, scale: page.scale }).collect(),
                target: None,
            });
            return None;
        }

        let previous = previous?;
        if previous.target != pages.first().map(|page| page.page)
            || previous.geometry != geometry
            || previous.pages.len() != pages.len()
            || pages.iter().any(|page| !matches!(page.picture, TextureState::Ready(_) | TextureState::Loading { .. }))
        {
            return None;
        }

        let ppp = geometry.pixels_per_point;
        let textures = previous
            .pages
            .iter()
            .zip(pages)
            .map(|(from, to)| {
                if from.rect.round_to_pixels(ppp) != to.rect.round_to_pixels(ppp) || from.band != to.band || from.scale != to.scale {
                    return None;
                }
                let request = ThumbRequest::for_cell(from.rect.width().max(from.rect.height()), ppp);
                match cached(from.page, &request) {
                    TextureState::Ready(texture) => Some(texture),
                    _ => None,
                }
            })
            .collect::<Option<Vec<_>>>()?;
        *kept = Some(previous);
        Some(textures)
    }

    pub(super) fn pages(&self) -> impl Iterator<Item = usize> + '_ {
        self.pages.iter().map(|page| page.page)
    }

    pub(super) fn turn_to(&mut self, page: usize) {
        self.target = Some(page);
    }
}

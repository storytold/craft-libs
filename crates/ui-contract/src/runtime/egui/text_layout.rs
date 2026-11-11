//! Galleys and layout jobs.

use egui::epaint::text::FontsView;
use egui::text::LayoutJob;
use egui::{Context, Galley};
use std::sync::Arc;

/// Lays `job` out with the context's fonts. A section with no line height gets its face's row height rounded up to a
/// whole number of points: egui rounds a row to whole physical pixels, so a fractional height would give rows of
/// different logical heights at different interface scales (14.5 at scale 2, 15 at scale 1), and a wrapped text would
/// change height with the scale.
pub(crate) fn layout(ctx: &Context, job: LayoutJob) -> Arc<Galley> {
    ctx.fonts_mut(|fonts| layout_with(fonts, job))
}

/// [`layout`] over fonts the caller already holds.
pub(crate) fn layout_with(fonts: &mut FontsView<'_>, mut job: LayoutJob) -> Arc<Galley> {
    for section in &mut job.sections {
        if section.format.line_height.is_none() {
            section.format.line_height = Some(fonts.row_height(&section.format.font_id).ceil());
        }
    }
    fonts.layout_job(job)
}

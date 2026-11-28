//! Stories of the selection overlay.
//!
//! Every story draws `MockImage` under the overlay of a fake `MaskSource`: the picture's subject (the house), and for
//! the large case a soft-edged disc in a 40 000 × 30 000 document, which has no picture behind it (reading that many
//! pixels is what the story must not do).

use super::SelectionOverlay;
use super::overlay::OutlineTrace;
use crate::selection::PreviewMode;
use crate::trace::{MaskRect, MaskSource};
use crate::ui::mock_image::{MockImage, MockSubject, StoryView, story_canvas};
use craft_behavior::kurbo::Point;
use craft_ui::contract::Show as _;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The large case: a document of this size (document data, not a UI size) ...
const HUGE_SIZE: [u32; 2] = [40_000, 30_000];
/// ... with a disc of this radius and this edge width in its middle, in document pixels.
const HUGE_RADIUS: f64 = 1000.0;
const HUGE_EDGE: f64 = 24.0;
/// The view's zoom of the large case, in percent.
const HUGE_ZOOM: f64 = 12.0;
/// The subject's zoom, in percent.
const SUBJECT_ZOOM: f64 = 100.0;
/// The floating piece's offset, in document pixels.
const FLOAT_OFFSET: [i32; 2] = [220, -140];
/// Mask pixels that count as one when the large mask is reduced.
const REDUCTION: u32 = 8;

/// A soft-edged disc in the middle of a [`HUGE_SIZE`] document.
struct HugeDisc;

impl HugeDisc {
    fn centre() -> Point {
        Point::new(f64::from(HUGE_SIZE[0]) / 2.0, f64::from(HUGE_SIZE[1]) / 2.0)
    }
}

impl MaskSource for HugeDisc {
    fn size(&self) -> [u32; 2] {
        HUGE_SIZE
    }

    fn revision(&self) -> u64 {
        1
    }

    fn content_bounds(&self) -> MaskRect {
        let (c, reach) = (Self::centre(), HUGE_RADIUS + HUGE_EDGE);
        MaskRect::new((c.x - reach).floor() as i32, (c.y - reach).floor() as i32, (c.x + reach).ceil() as i32, (c.y + reach).ceil() as i32)
    }

    fn read_row(&self, y: i32, x0: i32, step: u32, out: &mut [f32]) {
        let c = Self::centre();
        for (i, value) in out.iter_mut().enumerate() {
            let x = f64::from(x0) + 0.5 + i as f64 * f64::from(step);
            let distance = (x - c.x).hypot(f64::from(y) + 0.5 - c.y) - HUGE_RADIUS;
            let t = (0.5 - distance / HUGE_EDGE).clamp(0.0, 1.0);
            *value = (t * t * (3.0 - 2.0 * t)) as f32;
        }
    }
}

/// The middle of the subject's bounds.
fn subject_centre(image: &MockImage) -> Point {
    let bounds = image.subject_bounds();
    Point::new(f64::from(bounds.x0 + bounds.x1) / 2.0, f64::from(bounds.y0 + bounds.y1) / 2.0)
}

#[story(SelectionOverlay<'static>, covers = [Default], drive = Drive::Never)]
fn ants(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let image = MockImage::default();
    let view = StoryView::At { zoom: SUBJECT_ZOOM, centre: subject_centre(&image) };
    story_canvas(ui, Some(&image), view, |ui, canvas| {
        SelectionOverlay::new("Selection", &MockSubject(image)).show_at(ui, canvas);
        StoryOutput::default()
    })
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn large_mask(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let view = StoryView::At { zoom: HUGE_ZOOM, centre: HugeDisc::centre() };
    story_canvas(ui, None, view, |ui, canvas| {
        SelectionOverlay::new("Selection", &HugeDisc).reduction(REDUCTION).show_at(ui, canvas);
        StoryOutput::default()
    })
}

#[story(OutlineTrace<'static>, covers = [Default], drive = Drive::Never)]
fn trace(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let view = StoryView::At { zoom: HUGE_ZOOM, centre: HugeDisc::centre() };
    story_canvas(ui, None, view, |ui, canvas| {
        OutlineTrace::new("Selection trace", &HugeDisc, [0, 0], REDUCTION, false).show_at(ui, canvas);
        StoryOutput::default()
    })
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn floating_piece(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let image = MockImage::default();
    story_canvas(ui, Some(&image), StoryView::Fit, |ui, canvas| {
        SelectionOverlay::new("Selection", &MockSubject(image)).offset(FLOAT_OFFSET).show_at(ui, canvas);
        StoryOutput::default()
    })
}

/// The subject under the preview of `mode`.
fn previewed(ui: &mut Ui, mode: PreviewMode) -> StoryOutput {
    let image = MockImage::default();
    story_canvas(ui, Some(&image), StoryView::Fit, |ui, canvas| {
        SelectionOverlay::new("Selection", &MockSubject(image)).preview(mode).show_at(ui, canvas);
        StoryOutput::default()
    })
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn grayscale_preview(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    previewed(ui, PreviewMode::Grayscale)
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn black_matte_preview(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    previewed(ui, PreviewMode::BlackMatte)
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn white_matte_preview(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    previewed(ui, PreviewMode::WhiteMatte)
}

#[story(SelectionOverlay<'static>, covers = [], drive = Drive::Never)]
fn quick_mask_preview(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    previewed(ui, PreviewMode::QuickMask)
}

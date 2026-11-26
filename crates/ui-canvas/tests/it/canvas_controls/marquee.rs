//! The selection outlines (20.6): a press away from every canvas control reaches the tool, a marquee maps its shape
//! through the view, the hover mask finds its edges, and the range bands tile.

use craft_ui::canvas_controls::range_bands;
use craft_ui::testing::trial;
use egui::Pos2;

const BAND_WIDTH: f32 = 4.0;

#[trial]
fn range_bands_are_the_four_sides_cut_to_the_clip() {
    let rect = egui::Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(50.0, 40.0));
    let clip = egui::Rect::from_min_max(Pos2::ZERO, Pos2::new(100.0, 100.0));
    assert_eq!(range_bands(rect, BAND_WIDTH, clip).len(), 4);
    let far = egui::Rect::from_min_max(Pos2::new(-2.0e7, 10.0), Pos2::new(2.0e7, 40.0));
    let bands = range_bands(far, BAND_WIDTH, clip);
    assert!(bands.iter().all(|band| band.width() <= clip.width() + 4.0 * BAND_WIDTH), "no side keeps its 40-million-point length");
    assert!(range_bands(egui::Rect::NOTHING, BAND_WIDTH, clip).is_empty());
}

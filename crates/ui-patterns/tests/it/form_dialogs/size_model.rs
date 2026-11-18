//! The size model of the size dialogs: linked fields and units.

use craft_behavior::PixelSize;
use craft_ui::patterns::form_dialogs::LinkedSize;
use craft_ui::runtime::env::{ColumnSize, UnitKind};
use craft_ui::testing::trial;

fn document() -> LinkedSize {
    LinkedSize::new(PixelSize::new(1200, 800))
}

fn pixels(size: &LinkedSize) -> (u32, u32) {
    (size.pixels().width, size.pixels().height)
}

/// With the lock on a side scales the other by the opening ratio, in pixels and in a unit; with it off the other side
/// stays, and a size is at least one pixel.
#[trial]
fn the_lock_scales_the_other_side_by_the_opening_ratio() {
    let mut size = document();
    size.set_width(600.0);
    assert_eq!(pixels(&size), (600, 400));
    size.set_height(200.0);
    assert_eq!(pixels(&size), (300, 200));
    assert!(size.set_unit(UnitKind::Percent));
    size.set_width(50.0);
    assert_eq!(pixels(&size), (600, 400), "a value in a unit goes through the same rule");

    size.lock(false);
    assert!(size.set_unit(UnitKind::Pixels));
    size.set_width(900.0);
    assert_eq!(pixels(&size), (900, 400));
    size.set_height(0.0);
    assert_eq!(pixels(&size).1, 1);
}

/// With Resample off the pixel count is fixed: a size in pixels cannot change, and a physical size changes the
/// resolution.
#[trial]
fn without_resampling_a_physical_size_changes_the_resolution() {
    let mut size = document();
    size.resampling(false);
    assert!(!size.is_editable());
    size.set_width(600.0);
    assert_eq!(pixels(&size), (1200, 800));

    assert!(size.set_unit(UnitKind::Inches));
    assert!(size.is_editable());
    size.set_width(8.0);
    assert_eq!(pixels(&size), (1200, 800));
    assert!((size.pixels_per_inch() - 150.0).abs() < 1e-9);
    assert!((size.height() - 800.0 / 150.0).abs() < 1e-9);
}

/// PhotoCraft #9260: a document above the resolution range opens clamped, and the clamped value counts as the opening
/// one, so the app keeps the document's own resolution until the user edits the resolution.
#[trial]
fn an_untouched_resolution_outside_the_range_is_not_reported_as_changed() {
    let mut size = document().opening_resolution(50_000.0);
    assert_eq!(size.pixels_per_inch(), 30_000.0, "the field shows the clamped resolution");
    assert!(!size.resolution_changed(), "an untouched resolution is not a change");
    size.set_width(600.0);
    assert!(!size.resolution_changed(), "a pixel edit with Resample on keeps the resolution");
    size.set_resolution(300.0);
    assert!(size.resolution_changed(), "an edit of the resolution is a change");
}

/// The 180 pt / 12 pt column size of the Units preference.
const COLUMNS: ColumnSize = ColumnSize::new(180.0, 12.0);

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-9
}

/// Columns on the width convert through the column size: `n` columns are `n·180 + (n−1)·12` points, at the document's
/// resolution, both ways. A relative change keeps 0 at 0 and its sign, a degenerate column size falls back to
/// 180 pt / 12 pt, the height keeps the shared unit, and with Resample off a Columns width changes the resolution.
#[trial]
fn columns_on_the_width_convert_through_the_column_size() {
    for ppi in [72.0, 144.0] {
        for columns in [1.0, 2.0, 3.0, 0.5] {
            let pixels = (columns * 192.0 - 12.0) * ppi / 72.0;
            let mut typed = document().opening_resolution(ppi).column_size(COLUMNS);
            assert!(typed.set_width_unit(UnitKind::Columns));
            typed.set_width(columns);
            assert_eq!(f64::from(typed.pixels().width), pixels, "{columns} col at {ppi} ppi as pixels");

            let mut read = LinkedSize::new(PixelSize::new(pixels as u32, 800)).opening_resolution(ppi).column_size(COLUMNS);
            assert!(read.set_width_unit(UnitKind::Columns));
            assert!(close(read.width(), columns), "{pixels} px at {ppi} ppi reads {columns} col, not {}", read.width());
        }
    }

    relative_columns();
    degenerate_column_sizes();
    shared_unit();
    resample_off();
}

/// A relative change keeps 0 at 0 and its sign.
fn relative_columns() {
    let mut relative = document().column_size(COLUMNS);
    relative.relative(true);
    relative.lock(false);
    assert!(relative.set_width_unit(UnitKind::Columns));
    assert_eq!(relative.width(), 0.0, "no change reads 0 columns, not the gutter");
    relative.set_width(0.0);
    assert_eq!(pixels(&relative), (1200, 800), "a change of 0 columns changes nothing");
    relative.set_width(-1.0);
    assert_eq!(pixels(&relative).0, 1020, "a change of -1 column takes one column width off");
    assert!(relative.set_width_unit(UnitKind::Pixels));
    relative.set_width(-372.0);
    assert!(relative.set_width_unit(UnitKind::Columns));
    assert!(close(relative.width(), -2.0), "a shrink of two columns reads -2, not {}", relative.width());
}

/// A degenerate or negative column size falls back to 180 pt / 12 pt, a negative gutter counts as none.
fn degenerate_column_sizes() {
    let degenerate = [ColumnSize::new(0.0, f64::NAN), ColumnSize::new(f64::NAN, f64::INFINITY), ColumnSize::new(-5.0, f64::NAN)];
    for column_size in degenerate {
        let mut size = document().column_size(column_size);
        assert!(size.set_width_unit(UnitKind::Columns));
        size.set_width(2.0);
        assert_eq!(pixels(&size).0, 372, "{column_size:?} converts as 180 pt / 12 pt");
        assert!(size.width().is_finite());
    }
    let mut no_gutter = document().column_size(ColumnSize::new(100.0, -4.0));
    assert!(no_gutter.set_width_unit(UnitKind::Columns));
    no_gutter.set_width(2.0);
    assert_eq!(pixels(&no_gutter).0, 200, "a negative gutter counts as none");
}

/// The height keeps the shared unit; another unit turns Columns off.
fn shared_unit() {
    let mut shared = document();
    assert!(!shared.set_width_unit(UnitKind::Columns), "without a column size the width offers no Columns");
    assert_eq!(shared.width_unit(), UnitKind::Pixels);
    let mut shared = shared.column_size(COLUMNS);
    assert!(shared.set_unit(UnitKind::Inches));
    assert!(shared.set_width_unit(UnitKind::Columns));
    assert_eq!((shared.width_unit(), shared.unit()), (UnitKind::Columns, UnitKind::Inches), "the height keeps the shared unit");
    assert!(close(shared.height(), 800.0 / 72.0), "the height still reads in inches");
    assert!(shared.set_width_unit(UnitKind::Pixels));
    assert_eq!((shared.width_unit(), shared.unit()), (UnitKind::Pixels, UnitKind::Pixels), "another unit turns Columns off and sets both");
}

/// With Resample off a Columns width changes the resolution.
fn resample_off() {
    let mut fixed = document().column_size(COLUMNS);
    fixed.resampling(false);
    assert!(fixed.set_width_unit(UnitKind::Columns));
    assert!(!fixed.is_editable(), "the height in pixels takes no input with Resample off");
    assert!(fixed.is_width_editable(), "a Columns width is a physical size");
    fixed.set_width(2.0);
    assert_eq!(pixels(&fixed), (1200, 800), "the pixel count stays");
    assert!(close(fixed.pixels_per_inch(), 72.0 * 1200.0 / 372.0), "the resolution follows the width");
    assert!(fixed.resolution_changed());
}

/// PhotoCraft #11722, #15451, #11721: the side typed last is reported, with the value as typed, while the size it made
/// stands, so a recorded step can keep what the user asked for. A value in pixels or percent stands through a
/// resolution change, which keeps the pixels; another side, another unit, Resample off or Reset ends it.
#[trial]
fn the_side_typed_last_stands_until_the_size_changes_otherwise() {
    let typed = |size: &LinkedSize| size.typed().map(|t| (t.width, t.value, t.unit));
    let mut size = document();
    assert_eq!(typed(&size), None, "nothing typed yet");
    size.set_width(600.0);
    assert_eq!(typed(&size), Some((true, 600.0, UnitKind::Pixels)));
    size.set_resolution(144.0);
    assert_eq!(typed(&size), Some((true, 600.0, UnitKind::Pixels)), "a resolution change keeps the pixels");
    size.set_height(200.0);
    assert_eq!(typed(&size), Some((false, 200.0, UnitKind::Pixels)), "the height typed after it");

    // A percentage is reported as typed, not worked back from the rounded pixels.
    assert!(size.set_unit(UnitKind::Percent));
    size.set_width(33.3);
    assert_eq!(pixels(&size), (400, 267));
    assert_eq!(typed(&size), Some((true, 33.3, UnitKind::Percent)));
    assert!(size.set_unit(UnitKind::Pixels));
    assert_eq!(typed(&size), None, "shown in another unit, the field no longer holds what was typed");

    size.set_width(600.0);
    size.resampling(false);
    assert_eq!(typed(&size), None, "Resample off puts back the opening size");
    size.resampling(true);
    assert_eq!(typed(&size), None);
    size.set_width(600.0);
    size.reset();
    assert_eq!(typed(&size), None, "Reset");

    // A physical size needs the resolution it was typed at: with Resample on a new resolution moves the pixels.
    assert!(size.set_unit(UnitKind::Inches));
    size.set_width(5.0);
    assert_eq!(typed(&size), Some((true, 5.0, UnitKind::Inches)));
    size.set_resolution(144.0);
    assert_eq!(typed(&size), None, "the pixels followed the resolution");
}

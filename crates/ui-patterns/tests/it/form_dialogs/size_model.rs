//! The size model of the size dialogs: linked fields and units.

use craft_behavior::PixelSize;
use craft_ui::patterns::form_dialogs::LinkedSize;
use craft_ui::runtime::env::UnitKind;
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

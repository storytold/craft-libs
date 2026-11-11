//! The pure scope helpers.

use super::*;
use craft_ui::testing::trial;

// Mean, Std Dev, the median rule (half reached), the percentile of a level and of a range, and the
// clipped percents, against counts worked out by hand (PhotoCraft's `histogram_panel/tests.rs`).
#[trial]
fn histogram_statistics_match_the_reference_counts() {
    assert!(histogram_statistics(&[0; 256], None).is_none());

    // 2 pixels at 0, 1 at 100, 1 at 255: N = 4, mean 88.75.
    let mut bins = [0u32; 256];
    bins[0] = 2;
    bins[100] = 1;
    bins[255] = 1;
    let all = histogram_statistics(&bins, None).unwrap();
    assert_eq!(all.pixels, 4);
    assert!((all.mean - 88.75).abs() < 1e-9);
    let variance = (2.0 * 88.75f64.powi(2) + 11.25f64.powi(2) + 166.25f64.powi(2)) / 4.0;
    assert!((all.standard_deviation - variance.sqrt()).abs() < 1e-9);
    assert_eq!(all.median, 0, "half the pixels are reached at level 0");
    assert_eq!((all.level, all.count, all.percentile), (None, None, None));
    assert_eq!(all.clipped, [50.0, 25.0]);

    let one = histogram_statistics(&bins, Some([100, 100])).unwrap();
    assert_eq!((one.level, one.count), (Some([100, 100]), Some(1)));
    assert_eq!(one.percentile, Some(75.0), "a level reports the percent at or below it");

    let range = histogram_statistics(&bins, Some([255, 100])).unwrap();
    assert_eq!((range.level, range.count), (Some([100, 255]), Some(2)));
    assert_eq!(range.percentile, Some(50.0), "a range reports the percent inside it");
}

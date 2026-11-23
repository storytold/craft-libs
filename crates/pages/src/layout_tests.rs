//! Page placement, fits and spreads.

use craft_ui::testing::trial;
use std::ops::Range;

use kurbo::{Point, Size};

use super::*;

/// Spacing for the tests: the numbers the assertions are written against.
const SPACING: PageSpacing = PageSpacing { gap: 14.0, margin: 28.0, side: 10.0, white_space_pad: 6.0, seam: 1.0 };

const LETTER: Size = Size::new(612.0, 792.0);

fn viewport() -> Size {
    Size::new(1000.0, 800.0)
}

#[trial]
fn spreads_follow_the_cover_rule() {
    let options = LayoutOptions { display: PageDisplay::TwoUp, cover: true, ..LayoutOptions::ACROBAT };
    let laid = place_pages(&[LETTER; 5], 0.5, viewport(), &options, &SPACING);
    assert!(laid.rects[0].x0 >= 500.0);
    assert_eq!(laid.rects[1].y0, laid.rects[2].y0);
    assert!(laid.rects[3].y0 > laid.rects[1].y0);

    let rows = |cover| -> Vec<Range<usize>> {
        let options = LayoutOptions { cover, ..options };
        (0..5).map(|p| shown_row(5, p, &options)).collect()
    };
    assert_eq!(rows(true), [0..1, 1..3, 1..3, 3..5, 3..5]);
    assert_eq!(rows(false), [0..2, 0..2, 2..4, 2..4, 4..5]);
    for cover in [true, false] {
        for (page, row) in rows(cover).iter().enumerate() {
            assert_eq!(row.start, spread_first(page, cover));
        }
    }
    assert_eq!(shown_row(0, 0, &options), 0..0);
}

#[trial]
fn hide_white_space_cuts_margins_and_keeps_coordinates() {
    let mut laid = place_pages(&[LETTER; 3], 1.0, viewport(), &LayoutOptions::ACROBAT, &SPACING);
    let before = laid.rects[1].size();
    laid.hide_white_space(&[(72.0, 720.0); 3], 1.0, 800.0, &SPACING);
    assert_eq!(laid.bands, [(0.0, 726.0), (66.0, 726.0), (66.0, 792.0)]);
    assert_eq!(laid.rects[1].size(), before);
    assert_eq!(laid.rects[1].y0 + 66.0, laid.rects[0].y0 + 726.0 + SPACING.seam);

    let centre = 400.0;
    assert_eq!(page_at(&laid, Point::new(centre, 1000.0)), Some(1));
    assert_eq!(page_at(&laid, Point::new(centre, 754.5)), None);
    assert!(laid.on_page_gap(Point::new(centre, 754.5)));
    assert!(!laid.on_page_gap(Point::new(centre, 1000.0)));
}

// Every zoom the view offers lays out as shown: 6400 % is not cut to a smaller scale.

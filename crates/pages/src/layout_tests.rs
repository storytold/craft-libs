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

#[trial]
fn a_page_fit_two_up_row_fits_the_spread_with_and_without_a_cover() {
    let sizes = [LETTER; 5];
    let room = viewport();
    for cover in [false, true] {
        let options = LayoutOptions { display: PageDisplay::TwoUp, cover, continuous: false, ..LayoutOptions::ACROBAT };
        // Page 1 is in a two-page spread either way: [0, 1] without a cover, [1, 2] with one.
        let row = shown_row(sizes.len(), 1, &options);
        assert_eq!(row.len(), 2, "cover {cover}: a spread of two");
        let fits = |scale: f64| {
            let laid = place_pages(&sizes, scale, room, &options, &SPACING);
            let spread = row.clone().map(|page| laid.rects[page]).reduce(|a, b| a.union(b));
            spread.is_some_and(|spread| {
                spread.width() <= room.width - 2.0 * SPACING.side + 1e-6 && spread.height() <= room.height - 2.0 * SPACING.margin + 1e-6
            })
        };
        let Some(scale) = fit_scale(&sizes, room, Fit::Page, &options, &SPACING, Some(1)) else { panic!("cover {cover}: a page fit has a scale") };
        assert!(fits(scale), "cover {cover}: the whole spread shows at the fitted scale {scale}");
        assert!(!fits(scale * 1.01), "cover {cover}: the fitted scale {scale} is the largest that shows it");
    }
}

#[trial]
fn columns_with_a_cover_put_page_one_alone_on_the_right_then_as_many_spreads_as_fit() {
    let book = LayoutOptions { cover: true, ..LayoutOptions::WORD };
    // Zoomed out, the five pages fit in one row: 1 on the right of its pair, then 2–3 and 4–5.
    let laid = place_pages(&[LETTER; 5], 0.25, Size::new(1400.0, 800.0), &book, &SPACING);
    let r = &laid.rects;
    assert_eq!(laid.columns % 2, 0, "whole spreads across: {}", laid.columns);
    assert!(r.iter().all(|p| p.y0 == r[0].y0), "several spreads share the row: {r:?}");
    assert!(r.windows(2).all(|w| w[1].x0 > w[0].x1), "1, 2–3, 4–5 from left to right: {r:?}");
    // The row is centred with page 1's left side in it, empty.
    let pitch = r[2].x0 - r[1].x0;
    assert!(((r[0].x0 - pitch + r[4].x1) / 2.0 - laid.size.width / 2.0).abs() < 1.0, "an empty side left of page 1: {r:?}");
}

#[trial]
fn a_right_hand_page_after_another_starts_a_spread_with_its_left_side_empty() {
    let book = LayoutOptions { cover: true, ..LayoutOptions::WORD };
    // Numbered 1, 2, 3, then a section from 1, 2: the second page 1 follows a right-hand page.
    let sides = [true, false, true, true, false];
    let laid = place_pages_on_sides(&[LETTER; 5], &sides, 0.25, Size::new(450.0, 800.0), &book, &SPACING);
    let r = &laid.rects;
    assert!(r[3].y0 > r[2].y1 && (r[3].x0 - r[0].x0).abs() < 1.0, "the second page 1 is on the right: {r:?}");
    assert!(!r.iter().any(|p| p.y0 == r[3].y0 && p.x1 < r[3].x0), "nothing faces it on the left: {r:?}");
    assert!(r[4].y0 > r[3].y1 && (r[4].x0 - r[1].x0).abs() < 1.0, "its page 2 opens the next spread on the left: {r:?}");
}

// Every zoom the view offers lays out as shown: 6400 % is not cut to a smaller scale.

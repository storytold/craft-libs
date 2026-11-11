//! Painting symmetry: the model's geometry, the menu, the handles of Transform Symmetry and the tool that carries its
//! keys.

use craft_behavior::kurbo::{Point, Size};
use craft_draw::ui::symmetry::{Symmetry, SymmetryMode};
use craft_ui::testing::trial;

/// Every mode on 100 x 100 and 101 x 53 gives segments inside the page; a horizontal axis at y -5 gives none.
#[trial]
fn segments_clip_to_the_page_and_vanish_when_the_centre_line_misses_it() {
    for size in [Size::new(100.0, 100.0), Size::new(101.0, 53.0)] {
        for mode in SymmetryMode::ALL {
            let s = Symmetry::new(mode, Point::new(size.width / 2.0, size.height / 2.0), 0.0).expect("a valid symmetry");
            let segments = s.segments(size);
            assert_eq!(segments.len(), if mode == SymmetryMode::Dual { 2 } else { 1 });
            for line in segments {
                for p in [line.p0, line.p1] {
                    assert!(p.x >= -1e-9 && p.x <= size.width + 1e-9 && p.y >= -1e-9 && p.y <= size.height + 1e-9);
                }
            }
        }
    }
    let off = Symmetry::new(SymmetryMode::Horizontal, Point::new(50.0, -5.0), 0.0).expect("a valid symmetry");
    assert!(off.segments(Size::new(100.0, 100.0)).is_empty());
}

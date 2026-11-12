//! The brush cursor: a circle or an outline carries the centre mark and the selection-mode badge the tool asks for.

use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, Cursor, CursorBadge, PaintedCursor};
use craft_ui::canvas::ToolContext;
use craft_ui::canvas_controls::brush::{BrushCursor, BrushCursorOptions, BrushTip, BrushTipOutline, PaintingCursor, TipRings};
use craft_ui::testing::trial;
use egui::{Pos2, Rect, vec2};

/// The centre mark and badge of the cursor `tip` shows with `centre` and `badge`, or `None` for a pointer that is
/// neither a circle nor an outline.
fn marks(tip: &BrushTip, centre: bool, badge: Option<CursorBadge>) -> Option<(&'static str, bool, Option<CursorBadge>)> {
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 400.0));
    let view = CanvasView::new(rect, Point::new(200.0, 200.0), 1.0);
    let cx = ToolContext::new(rect, None);
    let options = BrushCursorOptions::new(PaintingCursor::NormalBrushTip);
    match BrushCursor::new(tip).centre(centre).badge(badge).cursor(&cx, &options, &view) {
        Cursor::Painted(PaintedCursor::Circle { centre, badge, .. }) => Some(("circle", centre, badge)),
        Cursor::Painted(PaintedCursor::Outline { centre, badge, .. }) => Some(("outline", centre, badge)),
        _ => None,
    }
}

#[trial]
fn a_tip_outline_carries_the_centre_mark_and_the_badge_the_tool_asks_for() {
    let square = TipRings::new(vec![vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]]);
    let tips = [("circle", BrushTip::round(200.0, 80.0)), ("outline", BrushTip::new(200.0, BrushTipOutline::traced(square.clone(), square)))];
    for (kind, tip) in &tips {
        assert_eq!(marks(tip, true, Some(CursorBadge::Add)), Some((*kind, true, Some(CursorBadge::Add))), "{kind}: centre on, + mode");
        assert_eq!(marks(tip, false, None), Some((*kind, false, None)), "{kind}: plain");
    }
}

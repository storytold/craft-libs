//! The overlay's parts in the canvas's arena: the line, the two ends, a disc per colour stop and a diamond per midpoint.

use craft_behavior::{CursorIcon, HitLevel, HitPriority, HitShape, NavShape, PartKey, PartRef, Roving, Sense};
use craft_ui::canvas::canvas_space;
use craft_ui::contract::PartsCx;
use egui::Pos2;

use super::{GradientOverlay, screen_line};
use crate::ui::gradient::gradient_line::{at_screen, midpoint_location};
use crate::ui::gradient::{LineGradient, LinePart, StopRef, StopRow};

/// The line between the handles.
pub(super) const LINE: PartKey = PartKey::new("line");
/// The start handle.
pub(super) const START: PartKey = PartKey::new("start");
/// The end handle.
pub(super) const END: PartKey = PartKey::new("end");
/// Colour stop `i`: `STOP.at(i)`.
pub(super) const STOP: PartKey = PartKey::new("stop");
/// The diamond after colour stop `i`: `MIDPOINT.at(i)`.
pub(super) const MIDPOINT: PartKey = PartKey::new("midpoint");

/// The part ref of `part`.
pub(super) fn part_ref(part: LinePart) -> PartRef {
    match part {
        LinePart::Line => LINE.into(),
        LinePart::Start => START.into(),
        LinePart::End => END.into(),
        LinePart::Stop(i) => STOP.at(i as u64),
        LinePart::Midpoint(i) => MIDPOINT.at(i as u64),
    }
}

/// The overlay's part a part ref stands for.
pub(super) fn line_part(part: PartRef) -> Option<LinePart> {
    let index = part.index.map(|i| i as usize);
    match (part.key, index) {
        (LINE, _) => Some(LinePart::Line),
        (START, _) => Some(LinePart::Start),
        (END, _) => Some(LinePart::End),
        (STOP, Some(i)) => Some(LinePart::Stop(i)),
        (MIDPOINT, Some(i)) => Some(LinePart::Midpoint(i)),
        _ => None,
    }
}

/// The stop or midpoint a part selects.
pub(super) fn selection_of(part: LinePart) -> Option<StopRef> {
    match part {
        LinePart::Stop(i) => Some(StopRef::Color(i)),
        LinePart::Midpoint(i) => Some(StopRef::Midpoint(StopRow::Color, i)),
        LinePart::Line | LinePart::Start | LinePart::End => None,
    }
}

/// The parts of `value` in keyboard order: the line, the start, the stops and midpoints by location, the end.
pub(super) fn keyboard_order(value: &LineGradient, midpoints: bool) -> Vec<LinePart> {
    let mut order = vec![LinePart::Line, LinePart::Start];
    for i in 0..value.stops.color.len() {
        order.push(LinePart::Stop(i));
        if midpoints && midpoint_location(&value.stops, i).is_some() {
            order.push(LinePart::Midpoint(i));
        }
    }
    order.push(LinePart::End);
    order
}

impl GradientOverlay<'_> {
    /// The parts: handles at handle priority (end, start, stop, midpoint, in that order), the line under them, all in
    /// one roving group so Page keys, Home and End pick the next part.
    pub(super) fn declare_parts(&self, cx: &mut PartsCx<'_, Self>) {
        let (Some(space), Some(value)) = (canvas_space(&cx.env), cx.value()) else { return };
        let live = !cx.resolved.read_only && cx.resolved.enabled;
        let style = cx.style;
        let (start, end) = screen_line(&space.view, value);
        let sense = if live { Sense::DRAG | Sense::CLICK | Sense::TAKE_FOCUS | Sense::CONTEXT_CLICK } else { Sense::HOVER };
        let radius = |part: LinePart| match part {
            LinePart::Start | LinePart::End => style.end_radius,
            LinePart::Stop(_) => style.stop_radius,
            _ => style.diamond_radius,
        };
        let at = |part: LinePart| -> Pos2 {
            match part {
                LinePart::Start => start,
                LinePart::End => end,
                LinePart::Stop(i) => at_screen(start, end, value.stops.color[i].location, self.reverse),
                LinePart::Midpoint(i) => at_screen(start, end, midpoint_location(&value.stops, i).unwrap_or(0.0), self.reverse),
                LinePart::Line => start,
            }
        };
        for part in keyboard_order(value, self.midpoints) {
            let (shape, level, sub, cursor) = match part {
                LinePart::Line => (HitShape::segment(start, end), HitLevel::Edge, 0, CursorIcon::Move),
                LinePart::End => (HitShape::circle(at(part), radius(part)), HitLevel::Handle, 4, CursorIcon::Move),
                LinePart::Start => (HitShape::circle(at(part), radius(part)), HitLevel::Handle, 3, CursorIcon::Move),
                LinePart::Stop(_) => (HitShape::circle(at(part), radius(part)), HitLevel::Handle, 2, CursorIcon::Grab),
                LinePart::Midpoint(_) => (HitShape::circle(at(part), radius(part)), HitLevel::Handle, 1, CursorIcon::Grab),
            };
            let builder =
                cx.part(part_ref(part), shape).sense(sense).priority(HitPriority::new(level, sub)).cursor(cursor).drag_cursor(CursorIcon::Grabbing);
            if live {
                builder.focusable();
            }
        }
        if live {
            cx.roving(Roving::new(NavShape::Flat).page(Some(1)));
        }
    }
}

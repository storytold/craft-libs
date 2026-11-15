//! The pen's Curvature mode ([`PenMode::Curvature`](super::PenMode::Curvature)): the Curvature Pen while a path is
//! being drawn.

use super::PenTool;
use super::bezier::{EditSubpath, Knot};
use super::tool::{Drag, Kind, PenEnd, PenEvent, PenStyle, paint_anchors, paint_close_cue, paint_path};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::BezPath;
use craft_ui::canvas::{ToolAction, ToolContext, ToolPointer};
use craft_ui::primitives::{Dashes, Draw, StrokeWidth};
use egui::Pos2;

impl PenTool {
    /// A press in Curvature mode against the draft now:
    ///
    /// - with no draft, on the end point of an open path ([`ends`](PenTool::ends)): [`PenEvent::Resumed`], as the Pen;
    /// - on the first anchor of a draft with 3 or more anchors: closes at once (`closed` set, `Commit`, `Finished`);
    ///   the Pen closes on release instead;
    /// - on another anchor: grabs it; past the click slop it follows the pointer and the draft reflows on every
    ///   move;
    /// - elsewhere: a new smooth anchor at the press, which follows the pointer while the button is down. The first
    ///   two anchors give straight segments; from three on the handles follow
    ///   [`carried_handles`](crate::geometry::curvature::carried_handles), which keeps the first
    ///   [`kept`](PenTool::kept) knots' handles;
    /// - the second click of a double click on an anchor toggles it between smooth and corner and reflows; on empty
    ///   canvas the first click's anchor is under it, so a double click places a corner point.
    pub(super) fn press_curve(&mut self, pointer: &ToolPointer, view: &CanvasView, reach: f32) -> ToolAction {
        let mut draft = self.shown.clone();
        let n = draft.knots.len();
        let under = draft
            .knots
            .iter()
            .enumerate()
            .map(|(i, k)| (i, view.to_screen(k.anchor).distance(pointer.pos)))
            .filter(|&(_, d)| d <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i);
        if n == 0
            && let Some(&(path, knot, _)) = self.ends.iter().find(|(_, _, p)| view.to_screen(*p).distance(pointer.pos) <= reach)
        {
            self.events.push(PenEvent::Resumed(path, knot));
            return ToolAction::Done;
        }
        if under == Some(0) && n >= 3 && !draft.closed {
            draft.closed = true;
            draft.reflow_kept(self.kept);
            self.change(draft);
            self.commit();
            self.finish(PenEnd::Closed);
            return ToolAction::Done;
        }
        let index = match under {
            Some(i) => {
                if pointer.clicks == 2 {
                    let k = &mut draft.knots[i];
                    k.smooth = !k.smooth;
                    if !k.smooth {
                        k.in_ctrl = k.anchor;
                        k.out_ctrl = k.anchor;
                    }
                }
                i
            }
            None => {
                draft.knots.push(Knot { smooth: true, ..Knot::corner(view.to_doc(pointer.pos)) });
                n
            }
        };
        draft.reflow_kept(self.kept);
        let in_ctrl = draft.knots[index].in_ctrl;
        self.change(draft);
        self.drag = Some(Drag { origin: pointer.pos, kind: Kind::Move(index), last: pointer.pos, moved: false, in_ctrl });
        ToolAction::Claimed
    }

    /// The draft line, the hover preview, the square anchors (the last filled) and the close cue.
    pub(super) fn paint_curve(&self, draw: &Draw<'_>, cx: &ToolContext<'_>, look: &PenStyle, view: &CanvasView) {
        let draft = &self.shown;
        paint_path(draw, look, view, draft);
        let pointer = cx.pointer.filter(|_| self.sensing).map(|pointer| pointer.pos);
        if let Some(pointer) = pointer
            && self.drag.is_none()
            && !draft.closed
        {
            let line = next_segment(draft, self.kept, view, pointer);
            draw.dashed(&line, Dashes::new(look.dash, look.gap), StrokeWidth::Thin, look.line);
        }
        paint_anchors(draw, look, view, draft);
        if let Some(pointer) = pointer
            && self.closes_at(draft, pointer, view, super::tool::reach(cx))
        {
            paint_close_cue(draw, look, view, draft);
        }
    }
}

/// The screen line of the segment the next click would add: the curve through the anchors and the pointer, from the
/// last anchor to the pointer.
fn next_segment(draft: &EditSubpath, kept: usize, view: &CanvasView, pointer: Pos2) -> Vec<Pos2> {
    let mut next = draft.clone();
    next.knots.push(Knot { smooth: true, ..Knot::corner(view.to_doc(pointer)) });
    next.reflow_kept(kept);
    let mut line = Vec::new();
    if let [.., a, b] = next.knots.as_slice() {
        let mut segment = BezPath::new();
        segment.move_to(a.anchor);
        segment.curve_to(a.out_ctrl, b.in_ctrl, b.anchor);
        for (run, _) in craft_ui::canvas_controls::path_runs(view, &segment) {
            line.extend(run);
        }
    }
    line
}

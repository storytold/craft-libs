//! Path operations on the overlays' model: inserting and removing knots, the Curvature reflow, and
//! [`PathEdit::apply`], so a host whose path type is [`EditPath`] needs no edit code of its own,
//! and the pen draft edits itself with the same rules.

use kurbo::{ParamCurve, Point, Vec2};

use super::model::{EditPath, EditSubpath, HandleSide, Knot, KnotRef, PathEdit};
use crate::geometry::curvature::curvature_handles;

fn lerp(a: Point, b: Point, t: f64) -> Point {
    a + (b - a) * t
}

/// A smooth knot keeps its handles collinear: points the handle opposite `side` away from the
/// handle on `side`, keeping its length (a retracted handle stays retracted).
fn keep_smooth(k: &mut Knot, side: HandleSide) {
    if !k.smooth {
        return;
    }
    let (a, moved) = (k.anchor, k.handle(side));
    let opposite = match side {
        HandleSide::In => &mut k.out_ctrl,
        HandleSide::Out => &mut k.in_ctrl,
    };
    let (len, d) = (opposite.distance(a), moved.distance(a));
    if len > 1e-9 && d > 1e-9 {
        *opposite = a - (moved - a) / d * len;
    }
}

fn move_knots(path: &mut EditPath, anchors: &[KnotRef], by: Vec2) {
    let mut list = anchors.to_vec();
    list.sort_unstable();
    list.dedup();
    for r in list {
        let k = &mut path.subpaths[r.subpath].knots[r.knot];
        k.anchor += by;
        k.in_ctrl += by;
        k.out_ctrl += by;
    }
}

fn reflow_all(path: &mut EditPath, anchors: &[KnotRef]) {
    let mut subs: Vec<usize> = anchors.iter().map(|a| a.subpath).collect();
    subs.sort_unstable();
    subs.dedup();
    for s in subs {
        path.subpaths[s].reflow();
    }
}

impl EditSubpath {
    /// Add Anchor Point: inserts a knot into the segment leaving knot `segment` at its parameter
    /// `t` and returns the new knot's index (`segment + 1`). A curved segment is split at `t`
    /// (de Casteljau): its shape does not change, the two neighbours' facing handles shorten, and
    /// the new knot is smooth with the split's handles. A straight segment (both handles
    /// retracted) gets a corner knot at the cubic's point at `t` (the parameter hit tests and the
    /// preview anchor use, which is not linear along the line). `None`, and no change, when `t` is
    /// not finite or not strictly between 0 and 1, or no segment leaves `segment`.
    pub fn insert_at(&mut self, segment: usize, t: f64) -> Option<usize> {
        if !(t.is_finite() && t > 0.0 && t < 1.0) {
            return None;
        }
        let (a, b) = self.segment_ends(segment)?;
        let straight = self.is_straight(segment);
        let (ka, kb) = (self.knots[a], self.knots[b]);
        let new = if straight {
            let p = self.segment(segment)?.eval(t);
            Knot::corner(p)
        } else {
            let (p01, p12, p23) = (lerp(ka.anchor, ka.out_ctrl, t), lerp(ka.out_ctrl, kb.in_ctrl, t), lerp(kb.in_ctrl, kb.anchor, t));
            let (p012, p123) = (lerp(p01, p12, t), lerp(p12, p23, t));
            self.knots[a].out_ctrl = p01;
            self.knots[b].in_ctrl = p23;
            Knot { anchor: lerp(p012, p123, t), in_ctrl: p012, out_ctrl: p123, smooth: true }
        };
        let at = segment + 1;
        self.knots.insert(at, new);
        Some(at)
    }

    /// Delete Anchor Point: removes knot `knot` and returns whether it existed. Its neighbours are
    /// joined by one segment that keeps the previous knot's out handle and the next knot's in
    /// handle; a closed subpath stays closed. Removing the only knot leaves an empty subpath,
    /// which the caller drops (PhotoCraft removes the subpath and refuses to delete a path's only
    /// anchor; that check is the host's, on the whole path).
    pub fn remove_knot(&mut self, knot: usize) -> bool {
        if knot >= self.knots.len() {
            return false;
        }
        self.knots.remove(knot);
        true
    }

    /// The Curvature reflow: every smooth knot's handles become
    /// [`curvature_handles`](crate::geometry::curvature::curvature_handles) of the subpath's
    /// anchors, with the corner knots (`!smooth`) as corners and `closed` as it is. Corner knots
    /// keep their own handles.
    pub fn reflow(&mut self) {
        let anchors: Vec<Point> = self.knots.iter().map(|k| k.anchor).collect();
        let corners: Vec<bool> = self.knots.iter().map(|k| !k.smooth).collect();
        let fresh = curvature_handles(&anchors, &corners, self.closed);
        for (k, (in_ctrl, out_ctrl)) in self.knots.iter_mut().zip(fresh) {
            if k.smooth {
                k.in_ctrl = in_ctrl;
                k.out_ctrl = out_ctrl;
            }
        }
    }
}

impl PathEdit {
    /// Applies the edit to `path` and returns whether it changed it. Nothing changes when the
    /// edit's key is not `path.key` or a knot or segment it names does not exist. The rules are
    /// PhotoCraft's kernel's:
    ///
    /// - `MoveAnchors`: each listed knot moves once, with its handles, by `by`.
    /// - `MoveHandle`: the handle goes to `to`; `alone` makes the knot a corner; a smooth knot
    ///   turns its other handle to stay collinear, keeping that handle's length (a retracted one
    ///   stays retracted).
    /// - `BendSegment`: a straight segment moves with both its anchors; a curve's handles move so
    ///   the point at `t` (clamped to 0.05..=0.95, 0.5 when not finite) moves exactly `by`, the
    ///   nearer handle more; smooth end knots stay smooth.
    /// - `Convert`: `None` a corner with retracted handles; `Some(out)` a smooth knot with the out
    ///   handle at `out` and the in handle mirrored.
    /// - `MoveCurvature`: `MoveAnchors`, then [`EditSubpath::reflow`] on every subpath touched.
    /// - `InsertCurvature`: [`EditSubpath::insert_at`] (the new knot made smooth, on a straight
    ///   segment too, as Photoshop's Curvature Pen adds a smooth point), the new knot moved by
    ///   `by`, then the subpath reflowed.
    /// - `ToggleCurvature`: a smooth knot becomes a corner with retracted handles, a corner
    ///   becomes smooth, then the subpath reflows.
    pub fn apply(&self, path: &mut EditPath) -> bool {
        if path.key != self.path() {
            return false;
        }
        let before = path.clone();
        let knot_ok = |path: &EditPath, r: KnotRef| path.knot(r).is_some();
        match self {
            PathEdit::MoveAnchors { anchors, by, .. } | PathEdit::MoveCurvature { anchors, by, .. } => {
                if !anchors.iter().all(|&r| knot_ok(path, r)) {
                    return false;
                }
                move_knots(path, anchors, *by);
                if matches!(self, PathEdit::MoveCurvature { .. }) {
                    reflow_all(path, anchors);
                }
            }
            PathEdit::MoveHandle { knot, side, to, alone, .. } => {
                if !knot_ok(path, *knot) {
                    return false;
                }
                let k = &mut path.subpaths[knot.subpath].knots[knot.knot];
                match side {
                    HandleSide::In => k.in_ctrl = *to,
                    HandleSide::Out => k.out_ctrl = *to,
                }
                if *alone {
                    k.smooth = false;
                }
                keep_smooth(k, *side);
            }
            PathEdit::BendSegment { segment, t, by, .. } => {
                let sub = &mut path.subpaths[..];
                let Some(sp) = sub.get_mut(segment.subpath) else { return false };
                let Some((a, b)) = sp.segment_ends(segment.knot) else { return false };
                if sp.is_straight(segment.knot) {
                    for i in [a, b] {
                        let k = &mut sp.knots[i];
                        k.anchor += *by;
                        k.in_ctrl += *by;
                        k.out_ctrl += *by;
                    }
                } else {
                    // B(t) moves by 3(1-t)^2 t dc1 + 3(1-t) t^2 dc2; with dc1 = (1-t) s d and
                    // dc2 = t s d the point moves exactly d. The ends are clamped: a drag at an
                    // anchor would need an unbounded handle move.
                    let t = if t.is_finite() { t.clamp(0.05, 0.95) } else { 0.5 };
                    let u = 1.0 - t;
                    let s = 1.0 / (3.0 * t * u * (u * u + t * t));
                    sp.knots[a].out_ctrl += *by * (u * s);
                    keep_smooth(&mut sp.knots[a], HandleSide::Out);
                    sp.knots[b].in_ctrl += *by * (t * s);
                    keep_smooth(&mut sp.knots[b], HandleSide::In);
                }
            }
            PathEdit::Convert { knot, out, .. } => {
                if !knot_ok(path, *knot) {
                    return false;
                }
                let k = &mut path.subpaths[knot.subpath].knots[knot.knot];
                *k = match out {
                    Some(o) => Knot::smooth(k.anchor, *o),
                    None => Knot::corner(k.anchor),
                };
            }
            PathEdit::InsertCurvature { segment, t, by, .. } => {
                let Some(sp) = path.subpaths.get_mut(segment.subpath) else { return false };
                let Some(at) = sp.insert_at(segment.knot, *t) else { return false };
                let k = &mut sp.knots[at];
                if !k.smooth {
                    k.smooth = true;
                }
                k.anchor += *by;
                k.in_ctrl += *by;
                k.out_ctrl += *by;
                sp.reflow();
            }
            PathEdit::ToggleCurvature { knot, .. } => {
                if !knot_ok(path, *knot) {
                    return false;
                }
                let sp = &mut path.subpaths[knot.subpath];
                let k = &mut sp.knots[knot.knot];
                *k = if k.smooth { Knot::corner(k.anchor) } else { Knot { smooth: true, ..*k } };
                sp.reflow();
            }
        }
        *path != before
    }

    /// Applies the edit to the path of `paths` it names and returns whether that changed it.
    pub fn apply_all(&self, paths: &mut [EditPath]) -> bool {
        paths.iter_mut().any(|path| self.apply(path))
    }
}

//! [`BezierOverlay::hit_test`]: the part of a path under a screen point, without a pass.

use craft_behavior::CanvasView;
use egui::Pos2;
use kurbo::{Affine, CubicBez, ParamCurveNearest as _, Point};

use super::model::SelectedAnchors;
use super::{BezierHit, BezierOverlay, EditPath, HandleSide, HandlesShown, KnotRef, MAX_ANCHORS, PathSelection};

/// A shown direction handle: its path's index, its knot, side and screen end.
#[derive(Clone, Copy, Debug)]
pub(super) struct Shown {
    pub(super) path: usize,
    pub(super) knot: KnotRef,
    pub(super) side: HandleSide,
    pub(super) at: Pos2,
}

fn map_cubic(a: Affine, c: CubicBez) -> CubicBez {
    CubicBez::new(a * c.p0, a * c.p1, a * c.p2, a * c.p3)
}

/// The parameter of the point of `curve` nearest the screen point `at`, and its distance in screen points.
pub(super) fn nearest_on_screen(view: &CanvasView, curve: CubicBez, at: Pos2) -> (f64, f32) {
    let p = Point::new(f64::from(at.x), f64::from(at.y));
    let nearest = map_cubic(view.affine(), curve).nearest(p, 1e-3);
    (nearest.t, nearest.distance_sq.sqrt() as f32)
}

impl BezierOverlay<'_> {
    /// The part of `paths` under the screen point `at`, answered as the overlay's press would
    /// take it (gap row 40): the handles `handles` shows for `selection` first, then anchors,
    /// then segments; within each kind the nearest within `reach` screen points (pass
    /// the hit), of equal distances the path drawn last. Segments are measured on the
    /// screen, so a rotated or flipped view hits where it draws. Above
    /// [`MAX_ANCHORS`](super::MAX_ANCHORS) anchors only handles and segments count, as the overlay
    /// draws no anchors then.
    ///
    /// Hosts use it for a tool's hover cursor or for their own tool logic (PhotoCraft's Add and
    /// Delete Anchor targets) without showing an overlay.
    pub fn hit_test(
        paths: &[EditPath],
        selection: &PathSelection,
        handles: HandlesShown,
        view: &CanvasView,
        at: Pos2,
        reach: f32,
    ) -> Option<BezierHit> {
        // pc: crates/vector/src/edit.rs:337 `hit` (handles, anchors, segments; nearest per kind).
        let shown = shown_handles(paths, selection, &SelectedAnchors::of(selection), handles, view);
        let anchors = paths.iter().map(EditPath::anchor_count).sum::<usize>() <= MAX_ANCHORS;
        hit_in(paths, &shown, anchors, view, at, reach)
    }
}

/// The handles to show, in path order, in then out, leaving out retracted ones and those of subpaths the selection
/// does not show. `anchors` holds the anchors `sel` selects.
pub(super) fn shown_handles(
    paths: &[EditPath],
    sel: &PathSelection,
    anchors: &SelectedAnchors,
    handles: HandlesShown,
    view: &CanvasView,
) -> Vec<Shown> {
    shown_handles_of(paths, sel, |path, knot| anchors.has_anchor(path, knot), sel.selected_len(), handles, view)
}

/// [`shown_handles`] with `has_anchor` answering whether a knot of a path is a selected anchor, and `selected` as the
/// number of selected anchors that `OneAnchor` counts.
fn shown_handles_of(
    paths: &[EditPath],
    sel: &PathSelection,
    has_anchor: impl Fn(u64, KnotRef) -> bool,
    selected: usize,
    handles: HandlesShown,
    view: &CanvasView,
) -> Vec<Shown> {
    let mut shown = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let segment_ends = sel.segment.filter(|_| sel.path == Some(path.key)).and_then(|s| {
            let (a, b) = path.subpaths.get(s.subpath)?.segment_ends(s.knot)?;
            Some((KnotRef::new(s.subpath, a), KnotRef::new(s.subpath, b)))
        });
        for knot_ref in path.knot_refs().filter(|k| sel.shows(path.key, k.subpath)) {
            let Some(knot) = path.knot(knot_ref) else { continue };
            for side in [HandleSide::In, HandleSide::Out] {
                let of_selection = || {
                    has_anchor(path.key, knot_ref)
                        || next_to_selected_anchor(path, &has_anchor, knot_ref, side)
                        || segment_ends.is_some_and(|(a, b)| (side == HandleSide::Out && a == knot_ref) || (side == HandleSide::In && b == knot_ref))
                };
                let wanted = match handles {
                    HandlesShown::All => true,
                    HandlesShown::OneAnchor => selected == 1 && of_selection(),
                    HandlesShown::Selected => of_selection(),
                };
                if wanted && !knot.retracted(side) {
                    shown.push(Shown { path: index, knot: knot_ref, side, at: view.to_screen(knot.handle(side)) });
                }
            }
        }
    }
    shown
}

/// The handles to draw for the anchors `sel` lists for the path at index `at` of `paths`, which the overlay does not
/// edit: [`shown_handles`]' rule applied with that path as the selection's, the other selected anchors still counted.
/// Empty for `All`, whose handles the scene already holds. The index of each is `at`. The overlay calls it once per
/// path, so it reads the listed anchors and their count from `anchors`, the anchors `sel` selects, and copies none.
pub(super) fn other_handles(
    paths: &[EditPath],
    at: usize,
    sel: &PathSelection,
    anchors: &SelectedAnchors,
    handles: HandlesShown,
    view: &CanvasView,
) -> Vec<Shown> {
    let Some(path) = paths.get(at) else { return Vec::new() };
    if sel.path == Some(path.key) || handles == HandlesShown::All || !anchors.lists(path.key) {
        return Vec::new();
    }
    let edited = sel.path.map_or(0, |_| sel.anchors.len());
    let own = PathSelection::new(Some(path.key), Vec::new(), None);
    shown_handles_of(std::slice::from_ref(path), &own, |key, knot| anchors.listed_anchor(key, knot), anchors.others_len() + edited, handles, view)
        .into_iter()
        .map(|shown| Shown { path: at, ..shown })
        .collect()
}

/// Whether `side` of `knot` is the handle of a segment that a selected anchor ends: the previous anchor's out
/// handle or the next anchor's in handle, across a closed subpath's seam too.
fn next_to_selected_anchor(path: &EditPath, has_anchor: &impl Fn(u64, KnotRef) -> bool, knot: KnotRef, side: HandleSide) -> bool {
    let Some(sub) = path.subpaths.get(knot.subpath) else { return false };
    let count = sub.knots.len();
    match side {
        HandleSide::Out => sub.segment_ends(knot.knot).is_some_and(|(_, next)| has_anchor(path.key, KnotRef::new(knot.subpath, next))),
        HandleSide::In => {
            let previous = (knot.knot + count.saturating_sub(1)) % count.max(1);
            sub.segment_ends(previous).is_some_and(|(previous, _)| has_anchor(path.key, KnotRef::new(knot.subpath, previous)))
        }
    }
}

/// The hit among `shown` handles, then (with `anchors`) anchors, then segments.
pub(super) fn hit_in(paths: &[EditPath], shown: &[Shown], anchors: bool, view: &CanvasView, pointer: Pos2, reach: f32) -> Option<BezierHit> {
    let nearest = |best: &mut Option<(BezierHit, f32)>, hit: BezierHit, d: f32| {
        if d <= reach && best.is_none_or(|(_, bd)| d <= bd) {
            *best = Some((hit, d));
        }
    };
    let mut best = None;
    for s in shown {
        let path = paths[s.path].key;
        nearest(&mut best, BezierHit::Handle { path, knot: s.knot, side: s.side }, pointer.distance(s.at));
    }
    if best.is_none() && anchors {
        for path in paths {
            for k in path.knot_refs() {
                if let Some(knot) = path.knot(k) {
                    nearest(&mut best, BezierHit::Anchor { path: path.key, knot: k }, pointer.distance(view.to_screen(knot.anchor)));
                }
            }
        }
    }
    if best.is_none() {
        for path in paths {
            for (s, sub) in path.subpaths.iter().enumerate() {
                for i in 0..sub.segment_count() {
                    let Some(c) = sub.segment(i) else { continue };
                    let (t, d) = nearest_on_screen(view, c, pointer);
                    nearest(&mut best, BezierHit::Segment { path: path.key, segment: KnotRef::new(s, i), t }, d);
                }
            }
        }
    }
    best.map(|(hit, _)| hit)
}

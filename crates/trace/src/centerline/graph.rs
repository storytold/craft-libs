//! The skeleton as a graph: nodes (line ends, junctions) and branches (pixel runs between them),
//! with spur pruning and junction merging.
//!
//! Pruning follows the usual significance test for medial-axis branches (Blum 1967; see also
//! Shaked & Bruckstein 1998, "Pruning medial axes", CVIU 69(2)): a terminal branch that reaches
//! less than a threshold beyond the maximal disc of the junction it hangs from adds almost
//! nothing to the shape the skeleton reconstructs, so it is a boundary artefact (a spur). The
//! threshold is tied to the line width: `max(spur_min, spur_k · min(2 r, stroke_width))`.
//! Two junctions whose maximal discs overlap are one junction that thinning split in two (the
//! crossing of two thick lines thins to two forks joined by a short bridge).

use std::collections::BTreeSet;

use craft_geom::Point;
use kurbo::Vec2;

use super::Grid;

const MAX_BRANCHES: usize = 200_000;
const MAX_NODES: usize = 200_000;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Node {
    /// Position (pixel centre of its deepest pixel, or a merged junction's middle).
    pub pos: Point,
    /// Radius of the node's maximal disc (distance to the ink's edge), px.
    pub r: f64,
    /// A junction (thinning met three or more runs here) rather than a line end.
    pub junction: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Branch {
    /// Points from one node to the other (end points are the nodes' positions).
    pub pts: Vec<Point>,
    /// Distance transform at each point (px, centre to nearest paper centre).
    pub d: Vec<f64>,
    /// End nodes; both `None` for a ring.
    pub ends: [Option<usize>; 2],
    pub closed: bool,
}

impl Branch {
    pub(super) fn length(&self) -> f64 {
        self.pts.windows(2).map(|w| w[0].distance(w[1])).sum::<f64>()
            + if self.closed { self.pts.first().zip(self.pts.last()).map_or(0.0, |(a, b)| a.distance(*b)) } else { 0.0 }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Graph {
    pub nodes: Vec<Node>,
    pub branches: Vec<Branch>,
}

/// Build one paper-bordered crop. Distances use this same crop and are squared.
/// Refuse an over-budget graph in full so the caller can retain the blob as a fill.
pub(super) fn build(g: &Grid, dist: &[f32]) -> Option<Graph> {
    build_with_limits(g, dist, MAX_NODES, MAX_BRANCHES)
}

fn build_with_limits(g: &Grid, dist: &[f32], max_nodes: usize, max_branches: usize) -> Option<Graph> {
    let n = super::crop_size(g.w, g.h)?;
    if g.on.len() != n
        || dist.len() != n
        || dist.iter().any(|d| !d.is_finite() || *d < 0.0)
        || g.on.iter().enumerate().any(|(i, &on)| on && !g.is_interior(i))
    {
        return None;
    }
    let degree: Vec<u8> = (0..n).map(|i| if g.on.get(i) == Some(&true) { g.degree(i) as u8 } else { 0 }).collect();
    let (node_of, mut graph) = build_nodes(g, dist, &degree, max_nodes)?;
    let mut visited = vec![false; n];
    add_node_branches(g, dist, &node_of, &mut graph, &mut visited, max_branches)?;
    add_ring_branches(g, dist, &degree, &node_of, &mut graph, &mut visited, max_branches)?;
    Some(graph)
}

fn build_nodes(g: &Grid, dist: &[f32], degree: &[u8], max_nodes: usize) -> Option<(Vec<usize>, Graph)> {
    let dval = |i: usize| f64::from(dist.get(i).copied().unwrap_or(0.0)).sqrt();
    let mut node_of = vec![usize::MAX; g.on.len()];
    let mut graph = Graph::default();
    // Ends on their own; touching junction pixels together, deepest pixel first on ties.
    for i in g.interior() {
        let pixel_degree = *degree.get(i)?;
        if g.on.get(i) != Some(&true) || pixel_degree == 2 || *node_of.get(i)? != usize::MAX {
            continue;
        }
        // Isolated pixels cannot form a stroke; leave them to the existing fill path.
        if pixel_degree == 0 {
            continue;
        }
        if graph.nodes.len() >= max_nodes.min(MAX_NODES) {
            return None;
        }
        let id = graph.nodes.len();
        *node_of.get_mut(i)? = id;
        let mut members = vec![i];
        if pixel_degree > 2 {
            let mut cursor = 0;
            while let Some(&pixel) = members.get(cursor) {
                cursor += 1;
                for neighbour in g.neighbours(pixel) {
                    if *node_of.get(neighbour)? == usize::MAX && *degree.get(neighbour)? > 2 {
                        *node_of.get_mut(neighbour)? = id;
                        members.push(neighbour);
                    }
                }
            }
        }
        let best = members.into_iter().max_by(|&a, &b| dval(a).total_cmp(&dval(b)).then(b.cmp(&a)))?;
        graph.nodes.push(Node { pos: g.centre(best), r: (dval(best) - 0.5).max(0.5), junction: pixel_degree > 2 });
    }
    Some((node_of, graph))
}

fn add_node_branches(g: &Grid, dist: &[f32], node_of: &[usize], graph: &mut Graph, visited: &mut [bool], max_branches: usize) -> Option<()> {
    let dval = |i: usize| f64::from(dist.get(i).copied().unwrap_or(0.0)).sqrt();
    let mut seen_edge = BTreeSet::new();
    let n = visited.len();
    for pixel in g.interior() {
        let node = *node_of.get(pixel)?;
        if node == usize::MAX {
            continue;
        }
        for neighbour in g.neighbours(pixel) {
            let next_node = *node_of.get(neighbour)?;
            if next_node == node {
                continue;
            }
            if next_node != usize::MAX {
                if seen_edge.insert((pixel.min(neighbour), pixel.max(neighbour))) {
                    graph.push_branch(
                        Branch {
                            pts: vec![graph.nodes.get(node)?.pos, graph.nodes.get(next_node)?.pos],
                            d: vec![dval(pixel), dval(neighbour)],
                            ends: [Some(node), Some(next_node)],
                            closed: false,
                        },
                        max_branches,
                    )?;
                }
                continue;
            }
            if *visited.get(neighbour)? {
                continue;
            }
            let mut pixels = vec![pixel];
            let (mut previous, mut current) = (pixel, neighbour);
            let mut guard = n;
            while *node_of.get(current)? == usize::MAX && guard > 0 {
                guard -= 1;
                *visited.get_mut(current)? = true;
                pixels.push(current);
                let Some(next) = g.neighbours(current).find(|&candidate| {
                    candidate != previous && (visited.get(candidate) == Some(&false) || node_of.get(candidate) != Some(&usize::MAX))
                }) else {
                    break;
                };
                (previous, current) = (current, next);
            }
            if guard == 0 {
                return None;
            }
            let end = (*node_of.get(current)? != usize::MAX).then_some(*node_of.get(current)?);
            if end.is_some() {
                pixels.push(current);
            }
            let mut points: Vec<Point> = pixels.iter().map(|&i| g.centre(i)).collect();
            let distances = pixels.into_iter().map(dval).collect();
            *points.first_mut()? = graph.nodes.get(node)?.pos;
            if let Some(end) = end {
                *points.last_mut()? = graph.nodes.get(end)?.pos;
            }
            graph.push_branch(Branch { pts: points, d: distances, ends: [Some(node), end], closed: false }, max_branches)?;
        }
    }
    Some(())
}

fn add_ring_branches(
    g: &Grid,
    dist: &[f32],
    degree: &[u8],
    node_of: &[usize],
    graph: &mut Graph,
    visited: &mut [bool],
    max_branches: usize,
) -> Option<()> {
    let dval = |i: usize| f64::from(dist.get(i).copied().unwrap_or(0.0)).sqrt();
    let n = visited.len();
    for start in g.interior() {
        if g.on.get(start) != Some(&true) || degree.get(start) != Some(&2) || *visited.get(start)? || *node_of.get(start)? != usize::MAX {
            continue;
        }
        let mut pixels = vec![];
        let (mut previous, mut current) = (usize::MAX, start);
        let mut closed = false;
        let mut guard = n;
        while guard > 0 {
            guard -= 1;
            *visited.get_mut(current)? = true;
            pixels.push(current);
            let Some(next) = g.neighbours(current).find(|&candidate| candidate != previous && visited.get(candidate) == Some(&false)) else {
                closed = g.neighbours(current).any(|neighbour| neighbour == start) && pixels.len() > 2;
                break;
            };
            (previous, current) = (current, next);
        }
        if guard == 0 {
            return None;
        }
        graph.push_branch(
            Branch { pts: pixels.iter().map(|&i| g.centre(i)).collect(), d: pixels.into_iter().map(dval).collect(), ends: [None, None], closed },
            max_branches,
        )?;
    }
    Some(())
}

impl Graph {
    fn push_branch(&mut self, branch: Branch, limit: usize) -> Option<()> {
        if self.branches.len() >= limit.min(MAX_BRANCHES) {
            return None;
        }
        self.branches.push(branch);
        Some(())
    }
    pub(super) fn degrees(&self) -> Option<Vec<usize>> {
        let mut deg = vec![0usize; self.nodes.len()];
        for b in &self.branches {
            for e in b.ends.iter().flatten() {
                *deg.get_mut(*e)? += 1;
            }
        }
        Some(deg)
    }

    /// Removes spurs, drops tiny self-loops, merges junctions whose discs overlap and joins runs
    /// through nodes left with two of them; repeated until nothing changes (at most 32 rounds).
    pub(super) fn simplify(&mut self, stroke_width: f64) -> Option<()> {
        if !stroke_width.is_finite() || stroke_width <= 0.0 || self.nodes.len() > MAX_NODES || self.branches.len() > MAX_BRANCHES {
            return None;
        }
        let (spur_min, spur_k) = (2.0, 0.5);
        for _ in 0..32 {
            let mut changed = self.prune(spur_min, spur_k, stroke_width)?;
            changed |= self.merge_junctions()?;
            changed |= self.join_through()?;
            if !changed {
                break;
            }
        }
        // Nodes with a single branch end are line ends now.
        let deg = self.degrees()?;
        for (node, d) in self.nodes.iter_mut().zip(deg) {
            if d <= 1 {
                node.junction = false;
            }
        }
        Some(())
    }

    fn prune(&mut self, spur_min: f64, spur_k: f64, stroke_width: f64) -> Option<bool> {
        let mut deg = self.degrees()?;
        let mut order: Vec<usize> = (0..self.branches.len()).collect();
        let lens: Vec<f64> = self.branches.iter().map(Branch::length).collect();
        order.sort_by(|&a, &b| {
            lens.get(a).copied().unwrap_or(f64::INFINITY).total_cmp(&lens.get(b).copied().unwrap_or(f64::INFINITY)).then(a.cmp(&b))
        });
        let mut keep = vec![true; self.branches.len()];
        for i in order {
            let b = self.branches.get(i)?;
            match b.ends {
                [Some(a), Some(c)] if a == c => {
                    // A loop on one node: a real hole unless it lies inside the node's disc.
                    let r = self.nodes.get(a).map_or(0.0, |n| n.r);
                    if *lens.get(i)? < 2.0 * r + 2.0 {
                        *keep.get_mut(i)? = false;
                        let degree = deg.get_mut(a)?;
                        *degree = degree.saturating_sub(2);
                    }
                }
                [Some(a), Some(c)] => {
                    let (tip, fork) = if *deg.get(a)? == 1 { (a, c) } else { (c, a) };
                    if *deg.get(tip)? != 1 || *deg.get(fork)? < 3 {
                        continue;
                    }
                    let r = self.nodes.get(fork).map_or(0.0, |n| n.r);
                    let tau = spur_min.max(spur_k * (2.0 * r).min(stroke_width));
                    if *lens.get(i)? < r + tau {
                        *keep.get_mut(i)? = false;
                        *deg.get_mut(tip)? = deg.get(tip)?.saturating_sub(1);
                        *deg.get_mut(fork)? = deg.get(fork)?.saturating_sub(1);
                    }
                }
                _ => {}
            }
        }
        let before = self.branches.len();
        let mut k = keep.into_iter();
        self.branches.retain(|_| k.next().unwrap_or(true));
        Some(self.branches.len() != before)
    }

    /// Direction leaving node `node` along branch `b` (unit), from the point `reach` px along it.
    fn leaving(&self, b: &Branch, node: usize, reach: f64) -> Option<Vec2> {
        let pts: Vec<Point> = if b.ends[0] == Some(node) { b.pts.clone() } else { b.pts.iter().rev().copied().collect() };
        let first = *pts.first()?;
        let mut acc = 0.0;
        let mut far = *pts.last()?;
        for w in pts.windows(2) {
            acc += w[0].distance(w[1]);
            if acc >= reach {
                far = w[1];
                break;
            }
        }
        {
            let v = far - first;
            let len = v.hypot();
            (len > 1e-12 && len.is_finite()).then(|| v / len)
        }
    }

    /// For each node, the (branch, end) pairs that end there.
    fn ends_index(&self) -> Option<Vec<Vec<(usize, usize)>>> {
        let mut idx = vec![vec![]; self.nodes.len()];
        for (i, b) in self.branches.iter().enumerate() {
            for k in 0..2 {
                if let Some(n) = b.ends.get(k).copied().flatten() {
                    idx.get_mut(n)?.push((i, k));
                }
            }
        }
        Some(idx)
    }

    /// Two forks to merge into one junction: joined by a bridge shorter than their discs'
    /// radii plus 2 px (pixel quantisation), or the crossing of two lines that thinning split in
    /// two (degree 3 each, the bridge no longer than `8 r + 6`, and the other two runs at one fork
    /// continuing the other two at the other, each pair within 25° of straight on).
    /// All non-conflicting pairs are merged in one pass, shortest bridges first.
    fn merge_junctions(&mut self) -> Option<bool> {
        let idx = self.ends_index()?;
        let deg: Vec<usize> = idx.iter().map(Vec::len).collect();
        let mut cand: Vec<(f64, usize)> = vec![];
        for (i, b) in self.branches.iter().enumerate() {
            let [Some(a), Some(c)] = b.ends else { continue };
            if a != c && *deg.get(a)? >= 3 && *deg.get(c)? >= 3 {
                cand.push((b.length(), i));
            }
        }
        cand.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
        let mut used = vec![false; self.nodes.len()];
        let mut merges = vec![];
        for (l, i) in cand {
            let b = self.branches.get(i)?;
            let [Some(a), Some(c)] = b.ends else { continue };
            if *used.get(a)? || *used.get(c)? {
                continue;
            }
            let (ra, rc) = (self.nodes.get(a)?.r, self.nodes.get(c)?.r);
            let mut merge = l < ra + rc + 2.0;
            if !merge && *deg.get(a)? == 3 && *deg.get(c)? == 3 && l <= 8.0 * ra.max(rc) + 6.0 {
                let reach = 3.0 * ra.max(rc) + 4.0;
                let arms = |n: usize| -> Vec<Vec2> {
                    idx.get(n)
                        .into_iter()
                        .flatten()
                        .filter(|(j, _)| *j != i)
                        .filter_map(|&(j, _)| self.branches.get(j))
                        .filter(|o| o.ends[0] != o.ends[1])
                        .filter_map(|o| self.leaving(o, n, reach))
                        .collect()
                };
                let (aa, cc) = (arms(a), arms(c));
                if let ([a1, a2], [c1, c2]) = (aa.as_slice(), cc.as_slice()) {
                    let straight = |u: &Vec2, v: &Vec2| u.dot(*v) < -(25f64.to_radians().cos());
                    merge = (straight(a1, c1) && straight(a2, c2)) || (straight(a1, c2) && straight(a2, c1));
                }
            }
            if merge {
                *used.get_mut(a)? = true;
                *used.get_mut(c)? = true;
                merges.push((i, a, c));
            }
        }
        if merges.is_empty() {
            return Some(false);
        }
        let mut remove = vec![false; self.branches.len()];
        for &(i, a, c) in &merges {
            *remove.get_mut(i)? = true;
            let mid = self.branches.get(i)?.pts.get(self.branches.get(i)?.pts.len() / 2).copied().unwrap_or(self.nodes.get(a)?.pos);
            let r = self.nodes.get(a)?.r.max(self.nodes.get(c)?.r);
            self.nodes.get_mut(a)?.pos = mid;
            self.nodes.get_mut(a)?.r = r;
            for &(j, k) in idx.get(a)?.iter().chain(idx.get(c)?) {
                if j == i {
                    continue;
                }
                let Some(br) = self.branches.get_mut(j) else {
                    continue;
                };
                *br.ends.get_mut(k)? = Some(a);
                let last = br.pts.len().saturating_sub(1);
                if let Some(p) = br.pts.get_mut(if k == 0 { 0 } else { last }) {
                    *p = mid;
                }
            }
        }
        let mut k = remove.into_iter();
        self.branches.retain(|_| !k.next().unwrap_or(false));
        Some(true)
    }

    fn join_through(&mut self) -> Option<bool> {
        let mut idx = self.ends_index()?;
        let mut slots: Vec<Option<Branch>> = std::mem::take(&mut self.branches).into_iter().map(Some).collect();
        let mut changed = false;
        for node in 0..self.nodes.len() {
            let at: Vec<(usize, usize)> = idx
                .get(node)?
                .iter()
                .copied()
                .filter(|&(i, k)| slots.get(i).and_then(Option::as_ref).is_some_and(|b| b.ends.get(k) == Some(&Some(node))))
                .collect();
            let &[(i, ki), (j, kj)] = at.as_slice() else { continue };
            changed = true;
            if i == j {
                if let Some(b) = slots.get_mut(i).and_then(Option::as_mut) {
                    b.pts.pop();
                    b.d.pop();
                    b.ends = [None, None];
                    b.closed = b.pts.len() >= 3;
                }
                continue;
            }
            let (Some(mut a), Some(mut other)) = (slots.get_mut(i)?.take(), slots.get_mut(j)?.take()) else {
                continue;
            };
            // Orient: a ends at the node, other starts at it.
            if ki == 0 {
                a.pts.reverse();
                a.d.reverse();
                a.ends.reverse();
            }
            if kj == 1 {
                other.pts.reverse();
                other.d.reverse();
                other.ends.reverse();
            }
            a.pts.extend(other.pts.into_iter().skip(1));
            a.d.extend(other.d.into_iter().skip(1));
            a.ends[1] = other.ends[1];
            // The joined branch is slot i: re-point both far ends' index entries.
            for e in a.ends {
                if let Some(v) = e.and_then(|n| idx.get_mut(n)) {
                    v.retain(|&(b, _)| b != i && b != j);
                }
            }
            for (k, e) in a.ends.iter().enumerate() {
                if let Some(v) = e.and_then(|n| idx.get_mut(n)) {
                    v.push((i, k));
                }
            }
            *slots.get_mut(i)? = Some(a);
        }
        self.branches = slots.into_iter().flatten().collect();
        Some(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::centerline::{distance, thin};

    fn raw_graph(arms: &[(Point, Point)], width: f64) -> (Grid, Vec<f32>) {
        let (w, h) = (180, 180);
        let mask: Vec<_> = (0..w * h)
            .map(|i| {
                let p = Point::new((i % w) as f64 + 0.5, (i / w) as f64 + 0.5);
                arms.iter().any(|&(a, b)| {
                    let v = b - a;
                    let t = ((p - a).dot(v) / v.hypot2()).clamp(0.0, 1.0);
                    p.distance(a + v * t) <= width / 2.0
                })
            })
            .collect();
        let mut grid = Grid::from_mask(&mask, w, h).unwrap();
        let dist = distance::squared(&grid.on, grid.w, grid.h).unwrap();
        thin::thin(&mut grid);
        (grid, dist)
    }

    fn graph(arms: &[(Point, Point)], width: f64) -> Graph {
        let (g, dist) = raw_graph(arms, width);
        let mut graph = build(&g, &dist).unwrap();
        graph.simplify(13.0).unwrap();
        graph
    }

    fn arms(degrees: &[f64]) -> Vec<(Point, Point)> {
        let c = Point::new(90.27, 90.41);
        degrees
            .iter()
            .map(|a| {
                let a = a.to_radians();
                (c, c + Vec2::new(a.cos(), a.sin()) * 65.0)
            })
            .collect()
    }

    fn assert_degrees(g: &Graph, expected: &[usize]) {
        let mut degrees: Vec<_> = g.degrees().unwrap().into_iter().filter(|&d| d > 0).collect();
        degrees.sort_unstable();
        assert_eq!(degrees, expected, "{g:?}");
    }

    #[test]
    fn t_y_x_oblique_t_and_narrow_y_have_their_junction_degrees() {
        for width in [2.0, 4.0, 8.0] {
            for (name, angles) in [
                ("T", vec![0.0, 90.0, 180.0]),
                ("Y", vec![90.0, 210.0, 330.0]),
                ("X", vec![45.0, 135.0, 225.0, 315.0]),
                ("oblique T", vec![17.0, 107.0, 197.0]),
                ("narrow Y", vec![90.0, 255.0, 285.0]),
            ] {
                let g = graph(&arms(&angles), width);
                let mut want = vec![1; angles.len()];
                want.push(angles.len());
                eprintln!("{name}, width {width}");
                assert_degrees(&g, &want);
            }
        }
    }

    #[test]
    fn crossings_twenty_to_ninety_degrees_merge_into_one_four_way_junction() {
        for width in [2.0, 4.0, 8.0] {
            for angle in (20..=90).step_by(10) {
                let g = graph(&arms(&[0.0, 180.0, angle as f64, angle as f64 + 180.0]), width);
                eprintln!("crossing {angle}, width {width}");
                assert_degrees(&g, &[1, 1, 1, 1, 4]);
            }
        }
    }

    #[test]
    fn lines_at_twenty_four_angles_have_no_spurs() {
        let c = Point::new(90.27, 90.41);
        for width in [1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0] {
            for angle in 0..24 {
                let a = (angle as f64 * 15.0).to_radians();
                let v = Vec2::new(a.cos(), a.sin()) * 65.0;
                let g = graph(&[(c - v, c + v)], width);
                assert_eq!(g.branches.len(), 1, "width {width}, angle {angle}");
                assert_degrees(&g, &[1, 1]);
                assert!(g.branches[0].length() > 110.0);
                assert!(!g.branches[0].closed);
            }
        }
    }

    #[test]
    fn random_blobs_never_panic_and_build_the_same_graph_twice() {
        let mut seed = 73u64;
        let mut rnd = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            (seed >> 33) as usize
        };
        for case in 0..200 {
            let (w, h) = (3 + rnd() % 45, 3 + rnd() % 45);
            let blobs: Vec<_> = (0..1 + rnd() % 8).map(|_| (rnd() % w, rnd() % h, 1 + rnd() % 12)).collect();
            let mask: Vec<_> =
                (0..w * h).map(|i| blobs.iter().any(|&(x, y, r)| (i % w).abs_diff(x).pow(2) + (i / w).abs_diff(y).pow(2) <= r * r)).collect();
            let mut g = Grid::from_mask(&mask, w, h).unwrap();
            let dist = distance::squared(&g.on, g.w, g.h).unwrap();
            thin::thin(&mut g);
            let mut a = build(&g, &dist).unwrap();
            let mut b = build(&g, &dist).unwrap();
            assert_eq!(a, b, "raw graph, case {case}");
            a.simplify(13.0).unwrap();
            b.simplify(13.0).unwrap();
            assert_eq!(a, b, "simplified graph, case {case}");
        }
    }

    #[test]
    fn split_crossings_merge_only_when_outer_arms_continue_in_pairs() {
        let a = Point::new(20.0, 30.0);
        let b = Point::new(34.0, 30.0);
        let points = [a, b, a + Vec2::new(-20.0, -6.0), a + Vec2::new(-20.0, 6.0), b + Vec2::new(20.0, -6.0), b + Vec2::new(20.0, 6.0)];
        let mut g = Graph {
            nodes: points.iter().enumerate().map(|(i, &pos)| Node { pos, r: 1.0, junction: i < 2 }).collect(),
            branches: [(0, 1), (0, 2), (0, 3), (1, 4), (1, 5)]
                .into_iter()
                .map(|(a, b)| Branch {
                    pts: vec![points[a], points[a].lerp(points[b], 0.5), points[b]],
                    d: vec![1.5; 3],
                    ends: [Some(a), Some(b)],
                    closed: false,
                })
                .collect(),
        };
        // The bridge is 14 px: too long for the 4 px disc-overlap threshold.
        let mut bent = g.clone();
        bent.branches[4].pts = vec![b, b + Vec2::new(-20.0, 20.0)];
        assert_eq!(bent.merge_junctions(), Some(false));
        assert_eq!(g.merge_junctions(), Some(true));
        assert_degrees(&g, &[1, 1, 1, 1, 4]);
        assert_eq!(g.nodes[0].pos, Point::new(27.0, 30.0));
    }

    #[test]
    fn pruning_keeps_significant_arms_and_real_holes() {
        let centre = Point::new(20.0, 20.0);
        let mut g = Graph { nodes: vec![Node { pos: centre, r: 3.0, junction: true }], branches: vec![] };
        for (n, length) in [2.0, 8.0, 9.0].into_iter().enumerate() {
            let end = centre + Vec2::new(length, 0.0);
            g.nodes.push(Node { pos: end, r: 1.0, junction: false });
            g.branches.push(Branch { pts: vec![centre, end], d: vec![3.5, 1.5], ends: [Some(0), Some(n + 1)], closed: false });
        }
        // r + max(2, 0.5 * min(6, 13)) = 6: only the 2 px arm is a spur.
        assert_eq!(g.prune(2.0, 0.5, 13.0), Some(true));
        assert_eq!(g.branches.len(), 2);
        let mut loop_graph = Graph { nodes: vec![g.nodes[0].clone()], branches: vec![] };
        for side in [1.0, 5.0] {
            loop_graph.branches.push(Branch {
                pts: vec![centre, centre + Vec2::new(side, 0.0), centre + Vec2::new(side, side), centre],
                d: vec![3.5; 4],
                ends: [Some(0), Some(0)],
                closed: false,
            });
        }
        assert_eq!(loop_graph.prune(2.0, 0.5, 13.0), Some(true));
        assert_eq!(loop_graph.branches.len(), 1);
        assert!(loop_graph.branches[0].length() > 8.0);
        loop_graph.join_through().unwrap();
        assert!(loop_graph.branches[0].closed);
        assert_eq!(loop_graph.branches[0].ends, [None, None]);
    }

    #[test]
    fn node_and_branch_limits_refuse_the_whole_graph() {
        let (g, dist) = raw_graph(&arms(&[0.0, 90.0, 180.0]), 4.0);
        assert!(build_with_limits(&g, &dist, 1, MAX_BRANCHES).is_none());
        assert!(build_with_limits(&g, &dist, MAX_NODES, 1).is_none());
        assert!(build(&g, &[]).is_none());
        let mut invalid = dist.clone();
        invalid[0] = f32::NAN;
        assert!(build(&g, &invalid).is_none());
        invalid[0] = -1.0;
        assert!(build(&g, &invalid).is_none());
    }
}

//! Which of a pen's paths lie inside which of its closed paths, in plotter units, for drawing the
//! inner paths first.

/// The box of `p`: `[x0, y0, x1, y1]`.
pub(super) fn bounds(p: &[(i64, i64)]) -> [i64; 4] {
    p.iter().fold([i64::MAX, i64::MAX, i64::MIN, i64::MIN], |b, q| [b[0].min(q.0), b[1].min(q.1), b[2].max(q.0), b[3].max(q.1)])
}

/// For each of `paths` (`boxes` are their [`bounds`]), the closed paths it lies inside: none of
/// its segments crosses the closed path's line, each part of it between the points where it meets
/// that line lies inside the closed path (by the even-odd rule) or on the line, and at least one
/// part lies inside. A path touching the line of the one around it is inside it, a path crossing
/// it is not, and copies of one outline are not inside each other. A path is tested only against
/// closed paths larger than it (by the area of their box, then the area they enclose, then paint
/// order), so no two paths are each inside the other, and only against the closed paths in the
/// [`Grid`] cell of its box's first corner. `None` once `work` is spent: a grid cell filled and a
/// box compared cost one each, and each segment tested costs the closed path's points for its
/// crossings and again for each part of it.
pub(super) fn outer(paths: &[Vec<(i64, i64)>], boxes: &[[i64; 4]], work: &mut u64) -> Option<Vec<Vec<usize>>> {
    let size: Vec<(i128, i128)> = paths
        .iter()
        .zip(boxes)
        .map(|(p, b)| {
            let box_area = (i128::from(b[2]) - i128::from(b[0])) * (i128::from(b[3]) - i128::from(b[1]));
            (box_area, if closed(p) { twice_area(p) } else { 0 })
        })
        .collect();
    let rings: Vec<usize> = (0..paths.len()).filter(|&i| paths.get(i).is_some_and(|p| closed(p))).collect();
    let mut out = vec![vec![]; paths.len()];
    if rings.is_empty() {
        return Some(out);
    }
    let grid = Grid::new(&rings, boxes, work)?;
    for (a, (p, ab)) in paths.iter().zip(boxes).enumerate() {
        for &r in grid.at(ab[0], ab[1]) {
            spend(work, 1)?;
            let (Some(rb), Some(ring)) = (boxes.get(r), paths.get(r)) else { continue };
            let smaller = (size.get(a), a) < (size.get(r), r);
            if !smaller || rb[0] > ab[0] || rb[1] > ab[1] || rb[2] < ab[2] || rb[3] < ab[3] {
                continue;
            }
            if inside(p, ring, work)?
                && let Some(o) = out.get_mut(a)
            {
                o.push(r);
            }
        }
    }
    Some(out)
}

/// The most cells along each side of a [`Grid`].
const MAX_GRID_SIDE: usize = 512;
/// The most cells of a [`Grid`] a closed path is listed in; one whose box covers more is listed
/// once, among the wide ones. A closed path has 4 points or more, so the grid lists no more
/// entries than the pen has points.
const MAX_CELLS_PER_PATH: usize = 4;

/// A grid over the boxes of a pen's closed paths, about one cell for each, up to
/// [`MAX_GRID_SIDE`] a side: each cell lists, in paint order, the closed paths whose box covers
/// it, and the closed paths whose box covers more than [`MAX_CELLS_PER_PATH`] cells are listed
/// apart. A closed path whose box holds a point is among the wide ones or in that point's cell.
struct Grid {
    /// The first corner of the closed paths' boxes together.
    x0: i128,
    y0: i128,
    /// Their width and height, plus one.
    w: i128,
    h: i128,
    /// Cells along each side.
    side: usize,
    /// The cells row by row.
    cells: Vec<Vec<usize>>,
    /// The closed paths whose box covers more than [`MAX_CELLS_PER_PATH`] cells.
    wide: Vec<usize>,
}

impl Grid {
    /// The grid of the closed paths `rings` (indices into `boxes`). `None` once `work` is spent.
    fn new(rings: &[usize], boxes: &[[i64; 4]], work: &mut u64) -> Option<Self> {
        let all = rings
            .iter()
            .filter_map(|&r| boxes.get(r))
            .fold([i64::MAX, i64::MAX, i64::MIN, i64::MIN], |e, b| [e[0].min(b[0]), e[1].min(b[1]), e[2].max(b[2]), e[3].max(b[3])]);
        let side = ((rings.len() as f64).sqrt().ceil() as usize).clamp(1, MAX_GRID_SIDE);
        let (x0, y0) = (i128::from(all[0]), i128::from(all[1]));
        let mut grid = Self {
            x0,
            y0,
            w: (i128::from(all[2]) - x0 + 1).max(1),
            h: (i128::from(all[3]) - y0 + 1).max(1),
            side,
            cells: vec![vec![]; side * side],
            wide: vec![],
        };
        for &r in rings {
            let Some(b) = boxes.get(r) else { continue };
            let (Some((cx0, cy0)), Some((cx1, cy1))) = (grid.cell(b[0], b[1]), grid.cell(b[2], b[3])) else { continue };
            let covered = (cx1 - cx0 + 1) * (cy1 - cy0 + 1);
            if covered > MAX_CELLS_PER_PATH {
                spend(work, 1)?;
                grid.wide.push(r);
                continue;
            }
            spend(work, covered as u64)?;
            for cy in cy0..=cy1 {
                for cell in grid.cells.get_mut(cy * side + cx0..=cy * side + cx1).into_iter().flatten() {
                    cell.push(r);
                }
            }
        }
        Some(grid)
    }

    /// The column and row of the cell holding `(x, y)`, `None` outside the grid.
    fn cell(&self, x: i64, y: i64) -> Option<(usize, usize)> {
        let (dx, dy) = (i128::from(x) - self.x0, i128::from(y) - self.y0);
        if dx < 0 || dy < 0 || dx >= self.w || dy >= self.h {
            return None;
        }
        let side = self.side as i128;
        Some(((dx * side / self.w) as usize, (dy * side / self.h) as usize))
    }

    /// The wide closed paths, then those listed in the cell holding `(x, y)`.
    fn at(&self, x: i64, y: i64) -> impl Iterator<Item = &usize> {
        let cell = self.cell(x, y).and_then(|(cx, cy)| self.cells.get(cy * self.side + cx)).map_or(&[][..], Vec::as_slice);
        self.wide.iter().chain(cell)
    }
}

/// Takes `n` from `work`; `None`, with `work` at 0, when it has less.
fn spend(work: &mut u64, n: u64) -> Option<()> {
    let left = work.checked_sub(n);
    *work = left.unwrap_or(0);
    left.map(|_| ())
}

/// Does `p` start and end at one point, with three other points or more?
fn closed(p: &[(i64, i64)]) -> bool {
    p.len() > 3 && p.first() == p.last()
}

/// Twice the area the closed path `p` encloses.
fn twice_area(p: &[(i64, i64)]) -> i128 {
    let sum: i128 = p
        .windows(2)
        .map(|w| match w {
            [a, b] => i128::from(a.0) * i128::from(b.1) - i128::from(b.0) * i128::from(a.1),
            _ => 0,
        })
        .sum();
    sum.abs()
}

/// Does `p` lie inside the closed path `ring`, as [`outer`] decides? `None` once `work` is spent.
fn inside(p: &[(i64, i64)], ring: &[(i64, i64)], work: &mut u64) -> Option<bool> {
    let mut any = false;
    for w in p.windows(2) {
        let [s, e] = w else { continue };
        spend(work, ring.len() as u64)?;
        // Where the segment meets the line without crossing it: its ends and the ring's points on
        // it. Between two of these the segment is all inside or all outside.
        let mut stops = vec![*s, *e];
        for edge in ring.windows(2) {
            let [a, b] = edge else { continue };
            if crosses((*s, *e), (*a, *b)) {
                return Some(false);
            }
            if a != s && a != e && on((*s, *e), *a) {
                stops.push(*a);
            }
        }
        let along = |q: &(i64, i64)| {
            (i128::from(q.0) - i128::from(s.0)) * (i128::from(e.0) - i128::from(s.0))
                + (i128::from(q.1) - i128::from(s.1)) * (i128::from(e.1) - i128::from(s.1))
        };
        stops.sort_by_key(along);
        stops.dedup();
        for part in stops.windows(2) {
            let [m, n] = part else { continue };
            spend(work, ring.len() as u64)?;
            // The part's middle, doubled so it is whole.
            match locate((i128::from(m.0) + i128::from(n.0), i128::from(m.1) + i128::from(n.1)), ring) {
                Some(false) => return Some(false),
                Some(true) => any = true,
                None => {}
            }
        }
    }
    Some(any)
}

/// Which side of the line through `a` and `b` `c` is on: 1 left, -1 right, 0 on it.
fn side(a: (i64, i64), b: (i64, i64), c: (i64, i64)) -> i128 {
    let d = (i128::from(b.0) - i128::from(a.0)) * (i128::from(c.1) - i128::from(a.1))
        - (i128::from(b.1) - i128::from(a.1)) * (i128::from(c.0) - i128::from(a.0));
    d.signum()
}

/// Do segments `u` and `v` cross at a point inside both, each passing from one side of the other
/// to the other side?
fn crosses(u: ((i64, i64), (i64, i64)), v: ((i64, i64), (i64, i64))) -> bool {
    side(u.0, u.1, v.0) * side(u.0, u.1, v.1) < 0 && side(v.0, v.1, u.0) * side(v.0, v.1, u.1) < 0
}

/// Does `q` lie on segment `u`?
fn on(u: ((i64, i64), (i64, i64)), q: (i64, i64)) -> bool {
    let ((x0, y0), (x1, y1)) = u;
    side(u.0, u.1, q) == 0 && q.0 >= x0.min(x1) && q.0 <= x0.max(x1) && q.1 >= y0.min(y1) && q.1 <= y0.max(y1)
}

/// Where the doubled point `q` lies against the closed path `ring`: `Some(true)` inside by the
/// even-odd rule, `Some(false)` outside, `None` on its line.
fn locate(q: (i128, i128), ring: &[(i64, i64)]) -> Option<bool> {
    let mut inside = false;
    for w in ring.windows(2) {
        let [a, b] = w else { continue };
        let (a, b) = ((2 * i128::from(a.0), 2 * i128::from(a.1)), (2 * i128::from(b.0), 2 * i128::from(b.1)));
        // Which side of the segment's line `q` is on, 0 on it.
        let side = (b.0 - a.0) * (q.1 - a.1) - (b.1 - a.1) * (q.0 - a.0);
        if side == 0 && q.0 >= a.0.min(b.0) && q.0 <= a.0.max(b.0) && q.1 >= a.1.min(b.1) && q.1 <= a.1.max(b.1) {
            return None;
        }
        // A segment crossing the level of `q` to its right.
        if (a.1 > q.1) != (b.1 > q.1) && (side > 0) == (b.1 > a.1) {
            inside = !inside;
        }
    }
    Some(inside)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<(i64, i64)> {
        vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
    }

    fn nest(paths: &[Vec<(i64, i64)>]) -> Option<Vec<Vec<usize>>> {
        let boxes: Vec<[i64; 4]> = paths.iter().map(|p| bounds(p)).collect();
        let mut work = u64::MAX;
        outer(paths, &boxes, &mut work)
    }

    #[test]
    fn points_inside_outside_and_on_the_line() {
        let ring = square(0, 0, 10, 10);
        assert_eq!(locate((10, 10), &ring), Some(true));
        assert_eq!(locate((30, 10), &ring), Some(false));
        assert_eq!(locate((10, -2), &ring), Some(false));
        assert_eq!(locate((20, 10), &ring), None);
        assert_eq!(locate((0, 0), &ring), None);
    }

    #[test]
    fn a_path_inside_another_lies_inside_it_and_not_the_other_way() {
        let paths = [square(0, 0, 100, 100), square(10, 10, 20, 20), vec![(30, 30), (40, 40)]];
        assert_eq!(nest(&paths), Some(vec![vec![], vec![0], vec![0]]));
    }

    #[test]
    fn touching_counts_as_inside_and_crossing_does_not() {
        // A square with a corner on the outline's left edge, a triangle whose corners lie on the
        // outline's edges, and a square that crosses the outline.
        let paths = [square(0, 0, 100, 100), square(0, 40, 20, 60), vec![(0, 50), (50, 0), (100, 50), (0, 50)], square(90, 90, 120, 95)];
        assert_eq!(nest(&paths), Some(vec![vec![], vec![0], vec![0], vec![]]));
    }

    /// Every point of a copy lies on the other's line, and none inside it.
    #[test]
    fn copies_of_one_outline_do_not_lie_inside_each_other() {
        let paths = [square(0, 0, 10, 10), square(0, 0, 10, 10), vec![(10, 10), (10, 0), (0, 0), (0, 10), (10, 10)]];
        assert_eq!(nest(&paths), Some(vec![vec![], vec![], vec![]]));
    }

    /// A line whose ends and middle lie inside an outline but which crosses a slot cut into it.
    #[test]
    fn a_line_across_a_slot_is_not_inside() {
        let slotted = vec![(0, 0), (100, 0), (100, 100), (14, 100), (14, 40), (12, 40), (12, 100), (0, 100), (0, 0)];
        assert_eq!(nest(&[slotted, vec![(10, 50), (40, 50)]]), Some(vec![vec![], vec![]]));
    }

    /// A line that leaves an outline and comes back through two of its corners, crossing none of
    /// its sides, and one that runs along the bottom of a slot.
    #[test]
    fn a_line_through_corners_is_inside_only_where_it_stays_inside() {
        let v_slot = vec![(0, 0), (100, 0), (100, 100), (20, 100), (20, 50), (15, 40), (10, 50), (10, 100), (0, 100), (0, 0)];
        assert_eq!(nest(&[v_slot, vec![(5, 50), (40, 50)]]), Some(vec![vec![], vec![]]));
        let slot = vec![(0, 0), (100, 0), (100, 100), (20, 100), (20, 50), (10, 50), (10, 100), (0, 100), (0, 0)];
        assert_eq!(nest(&[slot, vec![(5, 50), (40, 50)]]), Some(vec![vec![], vec![0]]));
    }

    /// An outline that crosses itself encloses no area by its signed sum, yet a line in one of its
    /// lobes lies inside it.
    #[test]
    fn a_line_inside_a_lobe_of_a_crossed_outline_lies_inside_it() {
        let bow = vec![(0, 0), (100, 100), (0, 100), (100, 0), (0, 0)];
        assert_eq!(twice_area(&bow), 0);
        assert_eq!(nest(&[bow, vec![(40, 80), (60, 80)]]), Some(vec![vec![], vec![0]]));
    }

    #[test]
    fn a_ring_inside_a_ring_inside_a_ring_lies_inside_both() {
        let paths = [square(20, 20, 80, 80), square(0, 0, 100, 100), square(40, 40, 60, 60)];
        assert_eq!(nest(&paths), Some(vec![vec![1], vec![], vec![0, 1]]));
    }

    /// 10,000 parts in rows, each with a hole: each hole is tested against the few closed paths
    /// near it, not against every part.
    #[test]
    fn parts_in_rows_are_tested_only_against_the_closed_paths_near_them() {
        let mut paths = vec![];
        for row in 0..100 {
            for col in 0..100 {
                let (x, y) = (col * 30, row * 30);
                paths.push(square(x, y, x + 20, y + 20));
                paths.push(square(x + 5, y + 5, x + 15, y + 15));
            }
        }
        let boxes: Vec<[i64; 4]> = paths.iter().map(|p| bounds(p)).collect();
        let mut work = 5_000_000;
        let found = outer(&paths, &boxes, &mut work);
        let want: Vec<Vec<usize>> = (0..paths.len()).map(|i| if i % 2 == 1 { vec![i - 1] } else { vec![] }).collect();
        assert_eq!(found, Some(want));
    }

    /// 400 closed paths that each cover most of the grid are listed once each, not in every cell
    /// they cover.
    #[test]
    fn closed_paths_covering_many_cells_are_listed_once() {
        let mut paths: Vec<Vec<(i64, i64)>> = (0..400).map(|i| square(i, i, 1000 + i, 1000 + i)).collect();
        paths.extend((0..400).map(|i| {
            let (x, y) = ((i % 20) * 50, (i / 20) * 50);
            square(x, y, x + 10, y + 10)
        }));
        let boxes: Vec<[i64; 4]> = paths.iter().map(|p| bounds(p)).collect();
        let rings: Vec<usize> = (0..paths.len()).collect();
        let mut work = u64::MAX;
        let Some(grid) = Grid::new(&rings, &boxes, &mut work) else { panic!("the budget ran out") };
        assert_eq!(grid.wide, (0..400).collect::<Vec<_>>());
        assert!(grid.cells.iter().map(Vec::len).sum::<usize>() <= 400 * MAX_CELLS_PER_PATH);
    }

    #[test]
    fn finding_what_lies_inside_stops_at_its_budget() {
        let paths: Vec<Vec<(i64, i64)>> = (0..50).map(|i| square(-i, -i, 100 + i, 100 + i)).collect();
        let boxes: Vec<[i64; 4]> = paths.iter().map(|p| bounds(p)).collect();
        let mut work = 1_000;
        assert_eq!(outer(&paths, &boxes, &mut work), None);
        assert_eq!(work, 0);
        let mut work = u64::MAX;
        let found = outer(&paths, &boxes, &mut work);
        assert_eq!(found.map(|o| o.iter().map(Vec::len).sum::<usize>()), Some(50 * 49 / 2));
    }
}

use glam::DVec2;

use super::Chart;
use crate::Mesh;

#[cfg(test)]
thread_local! {
    static QUERY_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct Cone {
    center: DVec2,
    low: DVec2,
    high: DVec2,
    low_bound: f64,
    high_bound: f64,
}

struct Node {
    lo: DVec2,
    hi: DVec2,
    range: std::ops::Range<usize>,
    children: Option<(usize, usize)>,
    cone: Option<Cone>,
    axis: DVec2,
    local_lo: DVec2,
    local_hi: DVec2,
}

fn oriented_bounds(order: &[usize], triangles: &[[DVec2; 3]]) -> (DVec2, DVec2, DVec2) {
    let center = order.iter().flat_map(|i| triangles[*i]).sum::<DVec2>() / (order.len() * 3) as f64;
    let mut covariance = [0.0; 3];
    for p in order.iter().flat_map(|i| triangles[*i]) {
        let d = p - center;
        covariance[0] += d.x * d.x;
        covariance[1] += d.y * d.y;
        covariance[2] += d.x * d.y;
    }
    let angle = 0.5 * (2.0 * covariance[2]).atan2(covariance[0] - covariance[1]);
    let axis = DVec2::new(angle.cos(), angle.sin());
    let (lo, hi) = order
        .iter()
        .flat_map(|i| triangles[*i])
        .map(|p| DVec2::new(axis.dot(p), axis.perp_dot(p)))
        .fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(lo, hi), p| (lo.min(p), hi.max(p)));
    (axis, lo, hi)
}

fn cone(order: &[usize], triangles: &[[DVec2; 3]]) -> Option<Cone> {
    let center = *triangles[*order.first()?].iter().find(|p| order.iter().all(|i| triangles[*i].contains(p)))?;
    let directions = || order.iter().flat_map(|i| triangles[*i]).map(|p| p - center).filter(|p| *p != DVec2::ZERO);
    let axis = directions().filter_map(|p| p.try_normalize()).sum::<DVec2>().try_normalize()?;
    let mut low = (f64::MAX, DVec2::ZERO);
    let mut high = (f64::MIN, DVec2::ZERO);
    for p in directions() {
        let angle = axis.perp_dot(p).atan2(axis.dot(p));
        if angle < low.0 {
            low = (angle, p);
        }
        if angle > high.0 {
            high = (angle, p);
        }
    }
    if high.0 - low.0 >= std::f64::consts::PI {
        return None;
    }
    // Bounds from the vertices, as supported_cone takes them: rays whose angles round to a tie
    // can put a vertex just past the chosen ray.
    let low_bound = order.iter().flat_map(|i| triangles[*i]).map(|p| low.1.perp_dot(p - center)).fold(0.0, f64::min);
    let high_bound = order.iter().flat_map(|i| triangles[*i]).map(|p| high.1.perp_dot(p - center)).fold(0.0, f64::max);
    Some(Cone { center, low: low.1, high: high.1, low_bound, high_bound })
}

fn supported_cone(order: &[usize], triangles: &[[DVec2; 3]], center: DVec2) -> Option<Cone> {
    let radius = order.iter().flat_map(|i| triangles[*i]).map(|p| (p - center).length_squared()).fold(0.0, f64::max);
    let directions = || {
        order.iter().flat_map(|i| {
            let triangle = triangles[*i];
            let mut near = triangle[0];
            for p in &triangle[1..] {
                if (*p - center).length_squared() < (near - center).length_squared() {
                    near = *p;
                }
            }
            triangle.map(|p| p - near).into_iter().filter(move |p| p.length_squared() > radius * 0.25)
        })
    };
    let axis = directions().filter_map(|p| p.try_normalize()).sum::<DVec2>().try_normalize()?;
    let mut low = (f64::MAX, DVec2::ZERO);
    let mut high = (f64::MIN, DVec2::ZERO);
    for p in directions() {
        let angle = axis.perp_dot(p).atan2(axis.dot(p));
        if angle < low.0 {
            low = (angle, p);
        }
        if angle > high.0 {
            high = (angle, p);
        }
    }
    if high.0 - low.0 >= std::f64::consts::PI {
        return None;
    }
    let low_bound = order.iter().flat_map(|i| triangles[*i]).map(|p| low.1.perp_dot(p - center)).fold(f64::MAX, f64::min);
    let high_bound = order.iter().flat_map(|i| triangles[*i]).map(|p| high.1.perp_dot(p - center)).fold(f64::MIN, f64::max);
    Some(Cone { center, low: low.1, high: high.1, low_bound, high_bound })
}

fn bounds(triangle: &[DVec2; 3]) -> (DVec2, DVec2) {
    triangle.iter().fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
}

fn projection_center(triangles: &[[DVec2; 3]]) -> DVec2 {
    let (lo, hi) = triangles.iter().flatten().fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let radius_squared = (hi - lo).length_squared() * 1e-12;
    triangles[0]
        .map(|candidate| {
            let (count, sum) = triangles
                .iter()
                .flatten()
                .filter(|p| (**p - candidate).length_squared() <= radius_squared)
                .fold((0usize, DVec2::ZERO), |(count, sum), p| (count + 1, sum + *p));
            (count, sum / count as f64)
        })
        .into_iter()
        .max_by_key(|(count, _)| *count)
        .map_or(DVec2::ZERO, |(_, center)| center)
}

fn build(order: &mut [usize], offset: usize, triangles: &[[DVec2; 3]], nodes: &mut Vec<Node>, center: DVec2) -> usize {
    let (lo, hi) = order
        .iter()
        .map(|i| bounds(&triangles[*i]))
        .fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(lo, hi), (a, b)| (lo.min(a), hi.max(b)));
    let index = nodes.len();
    let (axis, local_lo, local_hi) = oriented_bounds(order, triangles);
    nodes.push(Node {
        lo,
        hi,
        range: offset..offset + order.len(),
        children: None,
        cone: cone(order, triangles).or_else(|| supported_cone(order, triangles, center)),
        axis,
        local_lo,
        local_hi,
    });
    if order.len() > 8 {
        let axis = usize::from(hi.y - lo.y > hi.x - lo.x);
        let centroid = |i: usize| triangles[i].iter().map(|p| p[axis]).sum::<f64>();
        let middle = order.len() / 2;
        order.select_nth_unstable_by(middle, |a, b| centroid(*a).total_cmp(&centroid(*b)));
        let (left, right) = order.split_at_mut(middle);
        let a = build(left, offset, triangles, nodes, center);
        let b = build(right, offset + middle, triangles, nodes, center);
        nodes[index].children = Some((a, b));
    }
    index
}

fn intersects(a: &[DVec2; 3], b: &[DVec2; 3]) -> bool {
    let area = |t: &[DVec2; 3]| (t[1] - t[0]).perp_dot(t[2] - t[0]);
    if area(a) == 0.0 || area(b) == 0.0 {
        return false;
    }
    for (triangle, other) in [(a, b), (b, a)] {
        let positive = area(triangle) > 0.0;
        for edge in 0..3 {
            let d = triangle[(edge + 1) % 3] - triangle[edge];
            if other.iter().all(|p| {
                let side = d.perp_dot(*p - triangle[edge]);
                if positive { side <= 0.0 } else { side >= 0.0 }
            }) {
                return false;
            }
        }
    }
    true
}

/// Node visits and triangle pair tests allowed for one chart of `faces` triangles: 64 n (log2 n + 1).
fn work_budget(faces: usize) -> usize {
    let log = (usize::BITS - faces.leading_zeros()) as usize;
    faces.saturating_mul(log + 1).saturating_mul(64)
}

/// Reports whether `face` overlaps a later face. Each node visit and each pair test spends one unit
/// of `budget`. Once the budget is spent, every remaining face is reported as overlapping, which gives
/// it a chart of its own: a valid unwrap whose total work stays within the budget.
fn query(node: usize, face: usize, triangles: &[[DVec2; 3]], order: &[usize], nodes: &[Node], budget: &mut usize) -> bool {
    if *budget == 0 {
        return true;
    }
    *budget -= 1;
    #[cfg(test)]
    QUERY_WORK.with(|work| work.set(work.get() + 1));
    let n = &nodes[node];
    let (lo, hi) = bounds(&triangles[face]);
    if n.hi.x < lo.x || n.hi.y < lo.y || hi.x < n.lo.x || hi.y < n.lo.y {
        return false;
    }
    let local = triangles[face].map(|p| DVec2::new(n.axis.dot(p), n.axis.perp_dot(p)));
    let (lo, hi) = bounds(&local);
    if n.local_hi.x < lo.x || n.local_hi.y < lo.y || hi.x < n.local_lo.x || hi.y < n.local_lo.y {
        return false;
    }
    if let Some(c) = &n.cone {
        let outside_low = triangles[face].iter().all(|p| c.low.perp_dot(*p - c.center) <= c.low_bound);
        let outside_high = triangles[face].iter().all(|p| c.high.perp_dot(*p - c.center) >= c.high_bound);
        if outside_low || outside_high {
            return false;
        }
    }
    match n.children {
        Some((left, right)) => query(left, face, triangles, order, nodes, budget) || query(right, face, triangles, order, nodes, budget),
        None => order[n.range.clone()].iter().any(|i| {
            if *i <= face {
                return false;
            }
            if *budget == 0 {
                return true;
            }
            *budget -= 1;
            intersects(&triangles[face], &triangles[*i])
        }),
    }
}

pub(super) fn overlapping_faces(mesh: &Mesh, chart: &Chart, faces: &[usize]) -> Vec<usize> {
    if faces.len() < 2 {
        return Vec::new();
    }
    let (source_faces, triangles): (Vec<usize>, Vec<[DVec2; 3]>) = faces
        .iter()
        .filter_map(|f| {
            let t = mesh.tri(*f).unwrap_or([0; 3]).map(|i| chart.local(mesh.positions[i]));
            ((t[1] - t[0]).perp_dot(t[2] - t[0]) != 0.0).then_some((*f, t))
        })
        .unzip();
    if triangles.len() < 2 {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..triangles.len()).collect();
    let mut nodes = Vec::new();
    let center = projection_center(&triangles);
    let root = build(&mut order, 0, &triangles, &mut nodes, center);
    let mut budget = work_budget(triangles.len());
    (0..triangles.len()).filter(|face| query(root, *face, &triangles, &order, &nodes, &mut budget)).map(|face| source_faces[face]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nearly_welded_fan(count: usize) -> Vec<[DVec2; 3]> {
        let point = |angle: f64, radius: f32| glam::Vec2::new(angle.cos() as f32 * radius, angle.sin() as f32 * radius).as_dvec2();
        (0..count)
            .map(|index| {
                let low = index as f64 * std::f64::consts::TAU / count as f64;
                let high = (index + 1) as f64 * std::f64::consts::TAU / count as f64;
                [point((low + high) * 0.5, 1e-8), point(low, 1.0), point(high, 1.0)]
            })
            .collect()
    }

    #[test]
    fn nearly_welded_fan_has_bounded_query_work() {
        let count = 10_000;
        let triangles = nearly_welded_fan(count);
        let positions: Vec<_> = triangles.iter().flatten().map(|p| p.as_vec2().extend(0.0)).collect();
        let ids = crate::unwrap::weld_ids(&positions);
        assert!(ids.as_chunks::<3>().0.iter().all(|face| face[0] == ids[0]));
        assert_bounded_query_work(&triangles);
    }

    #[test]
    fn exactly_welded_fan_has_bounded_query_work() {
        let mut triangles = nearly_welded_fan(10_000);
        for triangle in &mut triangles {
            triangle[0] = DVec2::ZERO;
        }
        assert_bounded_query_work(&triangles);
    }

    #[test]
    fn nearly_welded_half_fan_has_bounded_query_work() {
        let mut triangles = nearly_welded_fan(20_000);
        for (index, triangle) in triangles.iter_mut().enumerate() {
            triangle.rotate_left(index % 3);
        }
        for count in [2_500, 5_000, 10_000] {
            assert_bounded_query_work(&triangles[..count]);
        }
    }

    #[test]
    fn second_nearly_welded_fan_away_from_the_first_face_has_bounded_work() {
        for count in [16_000, 32_000] {
            let mut triangles = nearly_welded_fan(16);
            triangles.extend(nearly_welded_fan(count).into_iter().map(|triangle| triangle.map(|p| p + DVec2::new(8.0, 0.0))));
            QUERY_WORK.with(|work| work.set(0));
            let (flagged, budget_left) = overlapping(&triangles, work_budget(triangles.len()));
            let work = QUERY_WORK.with(|work| work.get());
            let total = triangles.len();
            let n_log_n = total * (total.ilog2() as usize + 2);
            assert!(work <= 64 * n_log_n, "query work {work} for {total} faces");
            // A face reported without a real overlap only gets its own chart, which is still valid.
            if budget_left > 0 {
                assert!(flagged.is_empty(), "faces {flagged:?} reported without an overlap");
            }
        }
    }

    #[test]
    fn spent_budget_reports_the_remaining_faces() {
        let triangles = nearly_welded_fan(64);
        let (flagged, budget_left) = overlapping(&triangles, 10);
        assert_eq!(budget_left, 0);
        assert!(!flagged.is_empty());
        assert_eq!(flagged.last(), Some(&63));
    }

    fn overlapping(triangles: &[[DVec2; 3]], mut budget: usize) -> (Vec<usize>, usize) {
        let mut order: Vec<_> = (0..triangles.len()).collect();
        let mut nodes = Vec::new();
        let center = projection_center(triangles);
        let root = build(&mut order, 0, triangles, &mut nodes, center);
        let flagged = (0..triangles.len()).filter(|face| query(root, *face, triangles, &order, &nodes, &mut budget)).collect();
        (flagged, budget)
    }

    fn unlimited() -> usize {
        usize::MAX
    }

    fn assert_bounded_query_work(triangles: &[[DVec2; 3]]) {
        let count = triangles.len();
        let mut order: Vec<_> = (0..count).collect();
        let mut nodes = Vec::new();
        let center = projection_center(triangles);
        let root = build(&mut order, 0, triangles, &mut nodes, center);
        QUERY_WORK.with(|work| work.set(0));
        for face in 0..count {
            assert!(!query(root, face, triangles, &order, &nodes, &mut unlimited()), "overlapping face {face}");
            let work = QUERY_WORK.with(|work| work.get());
            assert!(work <= count * 100, "query work {work} for {count} faces");
        }
        println!("query work {} for {count} faces", QUERY_WORK.with(|work| work.get()));
    }

    fn agrees_with_all_pairs(triangles: &[[DVec2; 3]]) {
        let mut order: Vec<_> = (0..triangles.len()).collect();
        let mut nodes = Vec::new();
        let center = projection_center(triangles);
        let root = build(&mut order, 0, triangles, &mut nodes, center);
        for face in 0..triangles.len() {
            let expected = (face + 1..triangles.len()).any(|other| intersects(&triangles[face], &triangles[other]));
            assert_eq!(query(root, face, triangles, &order, &nodes, &mut unlimited()), expected, "face {face}: {:?}", triangles[face]);
        }
    }

    #[test]
    fn shared_vertex_cones_bound_every_vertex() {
        // Rays from the shared vertex at nearly the same angle: their angles can round to a tie
        // while the later ray lies on the far side of the first.
        let mut checked = 0;
        for k in 1..64u32 {
            // Steps of one unit in the last place of 0.001.
            let delta = f64::from(k) * 2.0f64.powi(-62);
            let (p, q) = (DVec2::new(1.0, 0.001), DVec2::new(1.0, 0.001 - delta));
            let r = DVec2::new(0.2, 1.0);
            for rotation in 0..3 {
                let mut fan = [[DVec2::ZERO, p, r], [DVec2::ZERO, q, r]];
                for t in &mut fan {
                    t.rotate_left(rotation);
                }
                let order = [0, 1];
                let Some(c) = cone(&order, &fan) else { continue };
                for v in fan.iter().flatten() {
                    assert!(c.low.perp_dot(*v - c.center) >= c.low_bound, "k {k} rotation {rotation}: {v} below the low ray");
                    assert!(c.high.perp_dot(*v - c.center) <= c.high_bound, "k {k} rotation {rotation}: {v} above the high ray");
                }
                // A face touching the fan only along the shared vertex's low ray does not overlap it.
                let mut with_contact = fan.to_vec();
                with_contact.push([DVec2::ZERO, DVec2::new(1.0, -1.0), p]);
                agrees_with_all_pairs(&with_contact);
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn supported_planes_preserve_overlaps_and_boundary_contacts() {
        let mut fan = nearly_welded_fan(256);
        agrees_with_all_pairs(&fan);
        fan[37] = fan[201];
        fan[38] = fan[101].map(|p| p * 0.75);
        agrees_with_all_pairs(&fan);
        for scale in [2.0f64.powi(-40), 1.0, 2.0f64.powi(40)] {
            let shifted: Vec<_> = fan.iter().map(|triangle| triangle.map(|p| (p + DVec2::new(4.0, -8.0)) * scale)).collect();
            agrees_with_all_pairs(&shifted);
        }
        let mut state = 7u64;
        let mut coordinate = || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 32) % 65) as f64 - 32.0
        };
        let triangles: Vec<_> = (0..256).map(|_| std::array::from_fn(|_| DVec2::new(coordinate(), coordinate()))).collect();
        agrees_with_all_pairs(&triangles);
    }
}

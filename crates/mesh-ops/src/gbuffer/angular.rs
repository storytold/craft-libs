use std::cmp::Ordering;
use std::collections::BTreeMap;

use glam::DVec2;

#[derive(Clone)]
struct Angle {
    origin: DVec2,
    tip: DVec2,
    turn: bool,
    zero: bool,
    #[cfg(test)]
    work: std::rc::Rc<std::cell::Cell<usize>>,
}

impl Ord for Angle {
    fn cmp(&self, other: &Self) -> Ordering {
        #[cfg(test)]
        self.work.set(self.work.get().saturating_add(1));
        self.origin
            .x
            .total_cmp(&other.origin.x)
            .then_with(|| self.origin.y.total_cmp(&other.origin.y))
            .then_with(|| self.turn.cmp(&other.turn))
            .then_with(|| other.zero.cmp(&self.zero))
            .then_with(|| {
                let upper = |tip: DVec2| tip.y > self.origin.y || (tip.y == self.origin.y && tip.x >= self.origin.x);
                (!upper(self.tip)).cmp(&!upper(other.tip))
            })
            .then_with(|| {
                let side = super::edge(self.origin, self.tip, other.tip);
                if side > 0.0 {
                    Ordering::Less
                } else if side < 0.0 {
                    Ordering::Greater
                } else {
                    Ordering::Equal
                }
            })
    }
}

impl PartialOrd for Angle {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Angle {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Angle {}

pub(super) struct VertexCoverage {
    point: DVec2,
    ranges: BTreeMap<Angle, Angle>,
    #[cfg(test)]
    work: std::rc::Rc<std::cell::Cell<usize>>,
}

fn wedge(face: [DVec2; 3], point: DVec2) -> Option<(DVec2, DVec2)> {
    let at = face.iter().position(|p| *p == point)?;
    let (a, b) = (face[(at + 1) % 3], face[(at + 2) % 3]);
    let side = super::edge(point, a, b);
    if side > 0.0 {
        Some((a, b))
    } else if side < 0.0 {
        Some((b, a))
    } else {
        None
    }
}

impl VertexCoverage {
    pub(super) fn new(face: [DVec2; 3], point: DVec2) -> Option<Self> {
        let point = face.into_iter().min_by(|a, b| a.distance_squared(point).total_cmp(&b.distance_squared(point)))?;
        wedge(face, point)?;
        let mut coverage = Self {
            point,
            ranges: BTreeMap::new(),
            #[cfg(test)]
            work: std::rc::Rc::new(std::cell::Cell::new(0)),
        };
        coverage.insert(face, point)?;
        Some(coverage)
    }

    fn angle(&self, tip: DVec2, turn: bool) -> Angle {
        Angle {
            origin: self.point,
            tip,
            turn,
            zero: false,
            #[cfg(test)]
            work: self.work.clone(),
        }
    }

    pub(super) fn insert(&mut self, face: [DVec2; 3], _point: DVec2) -> Option<bool> {
        let point = self.point;
        let (a, b) = wedge(face, point)?;
        let (start, end) = (self.angle(a, false), self.angle(b, false));
        match start.cmp(&end) {
            Ordering::Less => Some(self.part(start, end)),
            Ordering::Greater => {
                let mut zero = self.angle(point, false);
                zero.zero = true;
                let turn = self.angle(point, true);
                Some(self.part(start, turn) && self.part(zero, end))
            }
            Ordering::Equal => None,
        }
    }

    fn part(&mut self, mut start: Angle, mut end: Angle) -> bool {
        if start >= end {
            return true;
        }
        if self.ranges.range(..=start.clone()).next_back().is_some_and(|(_, high)| high > &start)
            || self.ranges.range(start.clone()..end.clone()).next().is_some()
        {
            return false;
        }
        let before = self.ranges.range(..start.clone()).next_back().map(|(low, high)| (low.clone(), high.clone()));
        if let Some((low, high)) = before
            && high == start
        {
            self.ranges.remove(&low);
            start = low;
        }
        let after = self.ranges.range(start.clone()..).next().map(|(low, high)| (low.clone(), high.clone()));
        if let Some((low, high)) = after
            && low == end
        {
            self.ranges.remove(&low);
            end = high;
        }
        self.ranges.insert(start, end);
        true
    }

    #[cfg(test)]
    pub(super) fn comparisons(&self) -> usize {
        self.work.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_far_shared_vertex_keeps_both_sides_of_a_wrapping_wedge() {
        let point = DVec2::new(-2.0f64.powi(60), 0.5);
        let mut coverage = VertexCoverage::new([point, DVec2::new(0.0, -1.0), DVec2::new(0.0, 2.0)], point).unwrap();
        assert_eq!(coverage.insert([point, DVec2::new(0.0, 0.6), DVec2::new(0.0, 1.0)], point), Some(false));
    }
}

//! The traversal state of one walk over a `TESS_3D_Compressed`'s arrays, one triangle a step.

use std::collections::HashMap;

use super::bend::Planes;
use super::normals::{NormalMark, NormalReader};
use super::{Arrays, MIN_SINE, V, add, apex, cross, dot, key, scale, sub};

/// Traversal state; `log` records every edge-count increment so a component can be rewound.
/// Everything it holds is checked against [`Arrays::limit`] after each triangle.
pub(super) struct Walk<'a> {
    pub(super) pos: Vec<V>,
    pub(super) tris: Vec<[u32; 3]>,
    /// The mesh tolerance, for [`super::MIN_PLANE_HEIGHT`].
    pub(super) tol: f64,
    edges: HashMap<(u32, u32), u8>,
    log: Vec<(u32, u32)>,
    pub(super) stack: Vec<(u32, u32, u32)>,
    pub(super) next: Option<(u32, u32, u32)>,
    slot: usize,
    ri: usize,
    pi: usize,
    /// Present when the stored normals orient the walk.
    pub(super) normals: Option<NormalReader<'a>>,
    /// The last triangle whose stored normal could not orient it.
    pub(super) sliver: Option<usize>,
    /// Whether the last triangle's fold came from its stored normals.
    pub(super) signalled: bool,
    /// Whether the last triangle is at most a tolerance tall, so that decoded noise may have
    /// corrupted its stored decision.
    pub(super) weak: bool,
    /// Whether the last triangle is [`Walk::weak`] and collinear to [`MIN_SINE`]: no stored
    /// decision can orient it.
    pub(super) thin: bool,
    /// Set by the search: reverse the next apex's degenerate-frame turn.
    pub(super) turn: bool,
    /// Whether the last apex's frame was degenerate.
    pub(super) degenerate: bool,
    /// Whether any apex so far, rewound or not, would move under the other
    /// [`Arrays::ortho_turned`].
    pub(super) reads_turned: bool,
    /// The running planes of the planar faces walked so far.
    pub(super) planes: Planes,
    limit: usize,
}

/// Where a component starts, to rewind to.
#[derive(Clone, Copy)]
pub(super) struct Mark {
    pub(super) tri: usize,
    pos: usize,
    log: usize,
    slot: usize,
    ri: usize,
    pi: usize,
    normals: Option<NormalMark>,
}

impl<'a> Walk<'a> {
    /// Reserves room for `t` triangles, no more than `limit` elements' worth (two each).
    pub(super) fn new(t: usize, limit: usize, normals: Option<NormalReader<'a>>) -> Self {
        Walk {
            pos: Vec::new(),
            tris: Vec::with_capacity(t.min(limit / 2).min(1 << 20)),
            tol: 0.0,
            edges: HashMap::new(),
            log: Vec::new(),
            stack: Vec::new(),
            next: None,
            slot: 0,
            ri: 0,
            pi: 0,
            normals,
            sliver: None,
            signalled: false,
            weak: false,
            thin: false,
            turn: false,
            degenerate: false,
            reads_turned: false,
            planes: Planes::default(),
            limit,
        }
    }

    pub(super) fn mark(&self, tri: usize) -> Mark {
        Mark {
            tri,
            pos: self.pos.len(),
            log: self.log.len(),
            slot: self.slot,
            ri: self.ri,
            pi: self.pi,
            normals: self.normals.as_ref().map(NormalReader::mark),
        }
    }

    pub(super) fn rewind(&mut self, m: Mark) {
        for k in self.log.drain(m.log.min(self.log.len())..) {
            if let Some(n) = self.edges.get_mut(&k) {
                *n = n.saturating_sub(1);
            }
        }
        self.pos.truncate(m.pos);
        self.tris.truncate(m.tri);
        self.stack.clear();
        self.next = None;
        (self.slot, self.ri, self.pi) = (m.slot, m.ri, m.pi);
        if let (Some(r), Some(n)) = (self.normals.as_mut(), m.normals) {
            r.rewind(n);
        }
    }

    /// Whether every slot, reference and point is consumed.
    pub(super) fn used_up(&self, a: &Arrays<'_>) -> bool {
        self.slot == a.is_reference.len() && self.ri == a.references.len() && self.pi == a.points.len()
    }

    /// One slot: `Some(v)` a reference to vertex `v`, `None` a new point.
    fn take(&mut self, a: &Arrays<'_>) -> Result<Option<u32>, &'static str> {
        let flagged = *a.is_reference.get(self.slot).ok_or("slots run out")?;
        self.slot = self.slot.saturating_add(1);
        if !flagged {
            return Ok(None);
        }
        let v = *a.references.get(self.ri).ok_or("references run out")?;
        self.ri = self.ri.saturating_add(1);
        self.get(v)?;
        Ok(Some(v))
    }

    fn point(&mut self, a: &Arrays<'_>) -> Result<V, &'static str> {
        let Some(&[x, y, z]) = a.points.get(self.pi..self.pi.saturating_add(3)) else {
            return Err("points run out");
        };
        self.pi = self.pi.saturating_add(3);
        let tol = a.tolerance;
        Ok([x as f64 * tol, y as f64 * tol, z as f64 * tol])
    }

    pub(super) fn get(&self, v: u32) -> Result<V, &'static str> {
        self.pos.get(v as usize).copied().ok_or("a triangle refers to a vertex not yet decoded")
    }

    fn push(&mut self, p: V) -> Result<u32, &'static str> {
        let id = u32::try_from(self.pos.len()).map_err(|_| "too many vertices")?;
        self.pos.push(p);
        Ok(id)
    }

    /// The next triangle and its default fold: the seed, or the apex across the edge to
    /// continue from.
    fn triangle(&mut self, a: &Arrays<'_>) -> Result<([u32; 3], bool), &'static str> {
        self.degenerate = false;
        let invert = std::mem::take(&mut self.turn);
        let Some((p, q, w)) = self.next.take() else {
            return self.seed(a);
        };
        if let Some(r) = self.take(a)? {
            return Ok(([p, q, r], false));
        }
        let (pp, qq, ww) = (self.get(p)?, self.get(q)?, self.get(w)?);
        let (lo, hi) = if p < q { (pp, qq) } else { (qq, pp) };
        let fold = matches!(a.points.get(self.pi..self.pi.saturating_add(3)), Some(&[_, dy, 0]) if dy > 0);
        let d = self.point(a)?;
        let (v, degenerate) = apex(lo, hi, ww, d, a.ortho_turned, invert);
        if degenerate && !self.reads_turned {
            self.reads_turned = apex(lo, hi, ww, d, !a.ortho_turned, invert).0 != v;
        }
        self.degenerate = degenerate;
        Ok(([p, q, self.push(v)?], fold))
    }

    /// A component's seed triangle of three slots.
    fn seed(&mut self, a: &Arrays<'_>) -> Result<([u32; 3], bool), &'static str> {
        let mut v = [0u32; 3];
        for k in 0..3 {
            let id = match self.take(a)? {
                Some(r) => r,
                None => {
                    let d = self.point(a)?;
                    let p = match k {
                        0 => add(a.origin, d),
                        1 => add(self.get(v[0])?, d),
                        _ => add(scale(add(self.get(v[0])?, self.get(v[1])?), 0.5), d),
                    };
                    self.push(p)?
                }
            };
            if let Some(s) = v.get_mut(k) {
                *s = id;
            }
        }
        Ok((v, false))
    }

    /// Budget elements (8 bytes each) the walk's arrays hold, by length: growth can add up to
    /// as much again in spare capacity.
    pub(super) fn held(&self) -> usize {
        let pos = self.pos.len().saturating_mul(3);
        let tris = self.tris.len().saturating_mul(2);
        let edges = self.edges.len().saturating_mul(2);
        let stack = self.stack.len().saturating_mul(2);
        let normals = self.normals.as_ref().map_or(0, NormalReader::held);
        let walk = pos.saturating_add(tris).saturating_add(edges).saturating_add(self.log.len()).saturating_add(stack);
        walk.saturating_add(normals).saturating_add(self.planes.held())
    }

    fn closed(&self, e: (u32, u32, u32)) -> bool {
        self.edges.get(&key(e.0, e.1)).copied().unwrap_or(0) >= 2
    }

    /// Whether `t` folds: its stored normal decides where it can (see [`Walk::decide`]), else
    /// the default `fold` stands.
    pub(super) fn fold(&mut self, t: [u32; 3], fold: bool) -> Result<bool, &'static str> {
        let ti = self.tris.len();
        let Some(r) = self.normals.as_mut() else {
            return Ok(fold);
        };
        let o = r.read(ti, t, &self.pos).ok_or("the stored normals run out")?;
        self.decide(o, t, fold, self.thin)
    }

    /// Counts edge uses for `t`; fails on an edge shared by more than two triangles.
    fn share(&mut self, [ta, tb, tc]: [u32; 3]) -> Result<(), &'static str> {
        for (x, y) in [(ta, tb), (tb, tc), (tc, ta)] {
            let n = self.edges.entry(key(x, y)).or_insert(0);
            *n = n.saturating_add(1);
            let shared = *n;
            self.log.push(key(x, y));
            if shared > 2 {
                return Err("an edge is shared by more than two triangles");
            }
        }
        Ok(())
    }

    /// Sets [`Walk::weak`] and [`Walk::thin`] for `t`, and returns its winding.
    fn shape(&mut self, t: [V; 3], tolerance: f64) -> V {
        let [p, q, r] = t;
        let (pq, pr) = (sub(q, p), sub(r, p));
        let long = [pq, sub(r, q), sub(p, r)].iter().map(|v| dot(*v, *v).sqrt()).fold(0.0, f64::max);
        let w = cross(pq, pr);
        let sine = (dot(w, w) / (dot(pq, pq) * dot(pr, pr))).sqrt();
        self.weak = long > 0.0 && dot(w, w).sqrt() / long <= tolerance;
        self.thin = self.weak && (sine.is_nan() || sine <= MIN_SINE);
        w
    }

    /// Adds one triangle; `flip` inverts its fold. Each step spends one of [`Arrays::steps`].
    pub(super) fn step(&mut self, a: &Arrays<'_>, status: i32, flip: bool) -> Result<(), &'static str> {
        a.steps.set(a.steps.get().checked_sub(1).ok_or("the rebuild ran out of search steps")?);
        let (tri, fold) = self.triangle(a)?;
        self.share(tri)?;
        self.signalled = false;
        let corners = [self.get(tri[0])?, self.get(tri[1])?, self.get(tri[2])?];
        let w = self.shape(corners, a.tolerance);
        self.tol = a.tolerance;
        let fold = self.fold(tri, fold)? != flip;
        if let Some(n) = a.normals.as_ref() {
            let oriented = (!self.weak).then(|| if fold { scale(w, -1.0) } else { w });
            self.planes.place(n, self.tris.len(), corners, a.tolerance, oriented)?;
        }
        self.tris.push(tri);
        if self.held() > self.limit {
            return Err("the rebuild needs more than the decoded-element allowance left");
        }
        self.continue_from(tri, status, fold);
        Ok(())
    }

    /// Chooses the edge to continue across from `[ta tb tc]` and the edges to push, per
    /// `status` and whether the triangle is `folded`.
    fn continue_from(&mut self, [ta, tb, tc]: [u32; 3], status: i32, folded: bool) {
        let (mut left, mut right) = ((tc, tb, ta), (ta, tc, tb));
        if folded {
            std::mem::swap(&mut left, &mut right);
        }
        let (taken, other) = if status & 2 != 0 { (left, right) } else { (right, left) };
        if !self.signalled && status & 3 != 0 && self.closed(taken) && !self.closed(other) {
            std::mem::swap(&mut left, &mut right);
        }
        match (status & 2 != 0, status & 1 != 0) {
            (true, true) => {
                self.next = Some(left);
                self.stack.push(right);
            }
            (true, false) => self.next = Some(left),
            (false, true) => self.next = Some(right),
            (false, false) => {}
        }
        while self.next.is_none() {
            let Some(e) = self.stack.pop() else { break };
            if !self.closed(e) {
                self.next = Some(e);
            }
        }
    }
}

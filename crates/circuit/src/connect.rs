//! Joining pins, wires, junctions and labels into nets.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use crate::limits::{Error, Limits};
use crate::model::{ItemKind, ItemRef, LabelScope, Point, Project, Sheet};
use crate::natural::natural_cmp;

/// Pins, wires, junctions and labels that are connected, under one name.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Net {
    /// Unique within the project: the label name, qualified with the sheet name when a
    /// sheet label's name is used on several sheets, else `N$1`, `N$2`, …
    pub name: String,
    /// Every distinct label name on the net, supply names first, then global, then sheet.
    pub label_names: Vec<String>,
    /// Whether a supply label is on the net.
    pub has_supply: bool,
    /// The pins on the net.
    pub pins: Vec<ItemRef>,
    /// Every item on the net, pins included.
    pub members: Vec<ItemRef>,
}

/// A wire end that touches nothing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct WireEnd {
    pub wire: ItemRef,
    pub at: Point,
}

/// The nets of a project, and the loose ends found while building them.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Connectivity {
    /// Nets with at least one pin or label, in name order (named nets first).
    pub nets: Vec<Net>,
    /// Wire ends that touch no other wire, pin, junction or label.
    pub dangling_ends: Vec<WireEnd>,
    /// Labels that sit on no wire, pin or junction.
    pub floating_labels: Vec<ItemRef>,
    #[serde(skip)]
    net_of: HashMap<ItemRef, usize>,
}

impl Connectivity {
    /// Builds the nets within [`Limits::default`].
    pub fn build(project: &Project) -> Result<Self, Error> {
        Self::build_with(project, &Limits::default())
    }

    /// Builds the nets, after [`Project::validate`] with `limits`, spending at most
    /// `limits.work` comparisons.
    pub fn build_with(project: &Project, limits: &Limits) -> Result<Self, Error> {
        project.validate(limits)?;
        let layout = Layout::new(project);
        let mut dsu = Dsu::new(layout.total);
        let mut work = Work { left: limits.work, limit: limits.work };
        let mut loose = Loose::default();
        for (s, sheet) in project.sheets.iter().enumerate() {
            connect_sheet(project.tolerance, s, sheet, &layout, &mut dsu, &mut work, &mut loose)?;
        }
        join_labels(project, &layout, &mut dsu);
        let mut conn = collect(project, &layout, &mut dsu);
        conn.dangling_ends = loose.ends;
        conn.floating_labels = loose.labels;
        Ok(conn)
    }

    /// The index in [`Connectivity::nets`] of the net an item is on.
    pub fn net_of(&self, item: ItemRef) -> Option<usize> {
        self.net_of.get(&item).copied()
    }
}

/// Where each sheet's items start in the flat node numbering.
struct Layout {
    /// Per sheet: first node of its pins, wires, junctions and labels.
    starts: Vec<[usize; 4]>,
    total: usize,
}

impl Layout {
    fn new(project: &Project) -> Self {
        let mut starts = Vec::with_capacity(project.sheets.len());
        let mut at = 0usize;
        for sheet in &project.sheets {
            let counts = [sheet.pins.len(), sheet.wires.len(), sheet.junctions.len(), sheet.labels.len()];
            let mut row = [0; 4];
            for (k, n) in counts.into_iter().enumerate() {
                row[k] = at;
                at += n;
            }
            starts.push(row);
        }
        Self { starts, total: at }
    }

    fn node(&self, sheet: usize, kind: ItemKind, index: usize) -> usize {
        self.starts[sheet][kind_slot(kind)] + index
    }
}

fn kind_slot(kind: ItemKind) -> usize {
    match kind {
        ItemKind::Pin => 0,
        ItemKind::Wire => 1,
        ItemKind::Junction => 2,
        ItemKind::Label => 3,
    }
}

struct Dsu {
    parent: Vec<usize>,
    size: Vec<u32>,
}

impl Dsu {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect(), size: vec![1; n] }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] != x {
            self.parent[x] = self.parent[self.parent[x]];
            x = self.parent[x];
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (mut a, mut b) = (self.find(a), self.find(b));
        if a == b {
            return;
        }
        if self.size[a] < self.size[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b] = a;
        self.size[a] = self.size[a].saturating_add(self.size[b]);
    }
}

struct Work {
    left: u64,
    limit: u64,
}

impl Work {
    fn spend(&mut self, n: u64) -> Result<(), Error> {
        match self.left.checked_sub(n) {
            Some(left) => {
                self.left = left;
                Ok(())
            }
            None => Err(Error::TooComplex { limit: self.limit }),
        }
    }
}

#[derive(Default)]
struct Loose {
    ends: Vec<WireEnd>,
    labels: Vec<ItemRef>,
}

#[derive(Clone, Copy, PartialEq)]
enum Probe {
    Pin,
    Junction,
    Label,
    WireEnd(usize),
}

struct ProbePoint {
    node: usize,
    at: Point,
    kind: Probe,
    item: ItemRef,
    touched: bool,
}

struct Segment {
    wire: usize,
    a: Point,
    b: Point,
}

fn connect_sheet(tol: f64, s: usize, sheet: &Sheet, layout: &Layout, dsu: &mut Dsu, work: &mut Work, loose: &mut Loose) -> Result<(), Error> {
    let segments = segments_of(sheet);
    let mut probes = probes_of(s, sheet, layout);
    let grid = SegmentGrid::new(&segments, tol, work)?;
    for probe in &mut probes {
        for &id in grid.near(probe.at) {
            work.spend(1)?;
            let seg = &segments[id as usize];
            if probe.kind == Probe::WireEnd(seg.wire) {
                continue;
            }
            if distance_sq(probe.at, seg.a, seg.b) <= tol * tol {
                dsu.union(probe.node, layout.node(s, ItemKind::Wire, seg.wire));
                probe.touched = true;
            }
        }
    }
    join_points(&mut probes, tol, dsu, work)?;
    for probe in probes.into_iter().filter(|p| !p.touched) {
        match probe.kind {
            Probe::WireEnd(_) => loose.ends.push(WireEnd { wire: probe.item, at: probe.at }),
            Probe::Label => loose.labels.push(probe.item),
            Probe::Pin | Probe::Junction => {}
        }
    }
    Ok(())
}

fn segments_of(sheet: &Sheet) -> Vec<Segment> {
    let mut out = Vec::new();
    for (w, wire) in sheet.wires.iter().enumerate() {
        match wire.points.as_slice() {
            [] => {}
            [p] => out.push(Segment { wire: w, a: *p, b: *p }),
            pts => out.extend(pts.windows(2).map(|ab| Segment { wire: w, a: ab[0], b: ab[1] })),
        }
    }
    out
}

fn probes_of(s: usize, sheet: &Sheet, layout: &Layout) -> Vec<ProbePoint> {
    let mut out = Vec::new();
    let mut push = |kind: ItemKind, index: usize, at: Point, probe: Probe| {
        let item = ItemRef { sheet: s, kind, index };
        out.push(ProbePoint { node: layout.node(s, kind, index), at, kind: probe, item, touched: false });
    };
    for (i, pin) in sheet.pins.iter().enumerate() {
        push(ItemKind::Pin, i, pin.at, Probe::Pin);
    }
    for (i, at) in sheet.junctions.iter().enumerate() {
        push(ItemKind::Junction, i, *at, Probe::Junction);
    }
    for (i, label) in sheet.labels.iter().enumerate() {
        push(ItemKind::Label, i, label.at, Probe::Label);
    }
    for (w, wire) in sheet.wires.iter().enumerate() {
        if let (Some(first), Some(last)) = (wire.points.first(), wire.points.last()) {
            push(ItemKind::Wire, w, *first, Probe::WireEnd(w));
            if wire.points.len() > 1 {
                push(ItemKind::Wire, w, *last, Probe::WireEnd(w));
            }
        }
    }
    out
}

/// Joins probe points that meet each other: a pin on a pin, a label on a pin, a wire end
/// on a pin.
fn join_points(probes: &mut [ProbePoint], tol: f64, dsu: &mut Dsu, work: &mut Work) -> Result<(), Error> {
    let cell = tol * 2.0;
    let key = |p: Point| ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64);
    let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, p) in probes.iter().enumerate() {
        buckets.entry(key(p.at)).or_default().push(i);
    }
    for i in 0..probes.len() {
        let (kx, ky) = key(probes[i].at);
        for dx in -1i64..=1 {
            for dy in -1i64..=1 {
                let Some(list) = buckets.get(&(kx.saturating_add(dx), ky.saturating_add(dy))) else { continue };
                work.spend(list.len() as u64)?;
                for &j in list.iter().filter(|&&j| j > i) {
                    meet(probes, i, j, tol, dsu);
                }
            }
        }
    }
    Ok(())
}

fn meet(probes: &mut [ProbePoint], i: usize, j: usize, tol: f64, dsu: &mut Dsu) {
    let (a, b) = (&probes[i], &probes[j]);
    if a.node == b.node {
        return;
    }
    let (dx, dy) = (a.at[0] - b.at[0], a.at[1] - b.at[1]);
    if dx * dx + dy * dy > tol * tol {
        return;
    }
    dsu.union(a.node, b.node);
    let (ka, kb) = (a.kind, b.kind);
    if kb != Probe::Label {
        probes[i].touched = true;
    }
    if ka != Probe::Label {
        probes[j].touched = true;
    }
}

/// Wire segments bucketed in a uniform grid, each in every cell its box (grown by the
/// tolerance) covers, so a point needs to look in its own cell only.
struct SegmentGrid {
    cell: f64,
    origin: Point,
    cells: HashMap<(i64, i64), Vec<u32>>,
}

impl SegmentGrid {
    fn new(segments: &[Segment], tol: f64, work: &mut Work) -> Result<Self, Error> {
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for seg in segments {
            for p in [seg.a, seg.b] {
                for k in 0..2 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
        let extent = if segments.is_empty() { 0.0 } else { (hi[0] - lo[0]).max(hi[1] - lo[1]) };
        let across = (segments.len() as f64).sqrt().max(1.0);
        let cell = (extent / across).max(tol * 4.0);
        let origin = if segments.is_empty() { [0.0, 0.0] } else { lo };
        let mut grid = Self { cell, origin, cells: HashMap::new() };
        for (id, seg) in segments.iter().enumerate() {
            grid.insert(id as u32, seg, tol, work)?;
        }
        Ok(grid)
    }

    fn key(&self, p: Point) -> (i64, i64) {
        (((p[0] - self.origin[0]) / self.cell).floor() as i64, ((p[1] - self.origin[1]) / self.cell).floor() as i64)
    }

    fn insert(&mut self, id: u32, seg: &Segment, tol: f64, work: &mut Work) -> Result<(), Error> {
        let lo = [seg.a[0].min(seg.b[0]) - tol, seg.a[1].min(seg.b[1]) - tol];
        let hi = [seg.a[0].max(seg.b[0]) + tol, seg.a[1].max(seg.b[1]) + tol];
        let (x0, y0) = self.key(lo);
        let (x1, y1) = self.key(hi);
        let span = |a: i64, b: i64| (b.saturating_sub(a) as u64).saturating_add(1);
        work.spend(span(x0, x1).saturating_mul(span(y0, y1)))?;
        for x in x0..=x1 {
            for y in y0..=y1 {
                self.cells.entry((x, y)).or_default().push(id);
            }
        }
        Ok(())
    }

    fn near(&self, p: Point) -> &[u32] {
        self.cells.get(&self.key(p)).map_or(&[], Vec::as_slice)
    }
}

fn distance_sq(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let (ax, ay) = (p[0] - a[0], p[1] - a[1]);
    let (bx, by) = (p[0] - b[0], p[1] - b[1]);
    let len_sq = dx * dx + dy * dy;
    if len_sq <= 0.0 || ax * dx + ay * dy <= 0.0 {
        return ax * ax + ay * ay;
    }
    if bx * dx + by * dy >= 0.0 {
        return bx * bx + by * by;
    }
    // Beside the segment: the distance to its line, from the cross product. Projecting a foot
    // point and subtracting it from `p` would lose a small distance on a very long segment.
    let cross = dx * ay - dy * ax;
    cross * cross / len_sq
}

/// Joins labels that share a name: sheet labels on their sheet, global and supply labels
/// everywhere.
fn join_labels(project: &Project, layout: &Layout, dsu: &mut Dsu) {
    let mut first: HashMap<(Option<usize>, &str), usize> = HashMap::new();
    for (s, sheet) in project.sheets.iter().enumerate() {
        for (i, label) in sheet.labels.iter().enumerate() {
            let reach = if label.scope == LabelScope::Sheet { Some(s) } else { None };
            let node = layout.node(s, ItemKind::Label, i);
            let root = *first.entry((reach, label.name.as_str())).or_insert(node);
            dsu.union(root, node);
        }
    }
}

struct Draft {
    labels: Vec<(LabelScope, String, usize)>,
    pins: Vec<ItemRef>,
    members: Vec<ItemRef>,
}

fn collect(project: &Project, layout: &Layout, dsu: &mut Dsu) -> Connectivity {
    let mut drafts: BTreeMap<usize, Draft> = BTreeMap::new();
    let all = all_items(project);
    for &item in &all {
        if matches!(item.kind, ItemKind::Pin | ItemKind::Label) {
            let root = dsu.find(layout.node(item.sheet, item.kind, item.index));
            drafts.entry(root).or_insert_with(|| Draft { labels: Vec::new(), pins: Vec::new(), members: Vec::new() });
        }
    }
    for &item in &all {
        let root = dsu.find(layout.node(item.sheet, item.kind, item.index));
        let Some(draft) = drafts.get_mut(&root) else { continue };
        draft.members.push(item);
        match item.kind {
            ItemKind::Pin => draft.pins.push(item),
            ItemKind::Label => {
                let label = &project.sheets[item.sheet].labels[item.index];
                draft.labels.push((label.scope, label.name.clone(), item.sheet));
            }
            ItemKind::Wire | ItemKind::Junction => {}
        }
    }
    let mut nets: Vec<Net> = drafts.into_values().map(name_net).collect();
    nets.sort_by(|a, b| net_order(project, a, b));
    unique_names(project, &mut nets);
    let mut net_of = HashMap::new();
    for (n, net) in nets.iter().enumerate() {
        for &m in &net.members {
            net_of.insert(m, n);
        }
    }
    Connectivity { nets, dangling_ends: Vec::new(), floating_labels: Vec::new(), net_of }
}

fn all_items(project: &Project) -> Vec<ItemRef> {
    let mut out = Vec::new();
    for (s, sheet) in project.sheets.iter().enumerate() {
        let counts = [
            (ItemKind::Pin, sheet.pins.len()),
            (ItemKind::Wire, sheet.wires.len()),
            (ItemKind::Junction, sheet.junctions.len()),
            (ItemKind::Label, sheet.labels.len()),
        ];
        for (kind, n) in counts {
            out.extend((0..n).map(|index| ItemRef { sheet: s, kind, index }));
        }
    }
    out
}

fn scope_rank(scope: LabelScope) -> u8 {
    match scope {
        LabelScope::Power => 0,
        LabelScope::Global => 1,
        LabelScope::Sheet => 2,
    }
}

fn name_net(mut d: Draft) -> Net {
    d.labels.sort_by(|a, b| scope_rank(a.0).cmp(&scope_rank(b.0)).then_with(|| natural_cmp(&a.1, &b.1)));
    let mut seen: HashSet<&str> = HashSet::new();
    let label_names: Vec<String> = d.labels.iter().filter(|l| seen.insert(l.1.as_str())).map(|l| l.1.clone()).collect();
    let has_supply = d.labels.iter().any(|l| l.0 == LabelScope::Power);
    Net { name: label_names.first().cloned().unwrap_or_default(), label_names, has_supply, pins: d.pins, members: d.members }
}

/// The pin that names an unnamed net's place in the order: its lowest designator and number.
fn pin_key<'a>(project: &'a Project, net: &Net) -> Option<(&'a str, &'a str)> {
    net.pins
        .iter()
        .filter_map(|&p| project.pin(p))
        .map(|pin| (project.parts[pin.part].designator.as_str(), pin.number.as_str()))
        .min_by(|a, b| natural_cmp(a.0, b.0).then_with(|| natural_cmp(a.1, b.1)))
}

fn net_order(project: &Project, a: &Net, b: &Net) -> Ordering {
    match (a.label_names.is_empty(), b.label_names.is_empty()) {
        (false, true) => Ordering::Less,
        (true, false) => Ordering::Greater,
        (false, false) => natural_cmp(&a.name, &b.name).then_with(|| a.members.cmp(&b.members)),
        (true, true) => {
            let (ka, kb) = (pin_key(project, a), pin_key(project, b));
            let by_pin = match (ka, kb) {
                (Some(x), Some(y)) => natural_cmp(x.0, y.0).then_with(|| natural_cmp(x.1, y.1)),
                _ => Ordering::Equal,
            };
            by_pin.then_with(|| a.members.cmp(&b.members))
        }
    }
}

/// Numbers unnamed nets, qualifies sheet label names used on several sheets with the sheet's
/// name, and makes any name still shared unique with a `#n` suffix.
fn unique_names(project: &Project, nets: &mut [Net]) {
    let mut counter = 0usize;
    for net in nets.iter_mut().filter(|n| n.label_names.is_empty()) {
        counter += 1;
        net.name = format!("N${counter}");
    }
    let counts = name_counts(nets);
    for net in nets.iter_mut() {
        if counts.get(&net.name).copied().unwrap_or(0) > 1
            && let Some(sheet) = sheet_scope_of(project, net)
        {
            net.name = format!("{}/{}", sheet_title(project, sheet), net.name);
        }
    }
    let mut taken: HashSet<String> = HashSet::new();
    let mut repeated: Vec<usize> = Vec::new();
    for (i, net) in nets.iter().enumerate() {
        if !taken.insert(net.name.clone()) {
            repeated.push(i);
        }
    }
    let mut next: HashMap<String, usize> = HashMap::new();
    for i in repeated {
        let base = nets[i].name.clone();
        let n = next.entry(base.clone()).or_insert(2);
        while taken.contains(&format!("{base}#{n}")) {
            *n += 1;
        }
        nets[i].name = format!("{base}#{n}");
        *n += 1;
        taken.insert(nets[i].name.clone());
    }
}

fn name_counts(nets: &[Net]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for net in nets {
        *counts.entry(net.name.clone()).or_insert(0) += 1;
    }
    counts
}

/// The sheet of the labels that gave a net its name, when all of them are sheet labels. A
/// supply or global label of that name on the net wins, and its name reaches every sheet.
fn sheet_scope_of(project: &Project, net: &Net) -> Option<usize> {
    let mut naming = net
        .members
        .iter()
        .filter(|m| m.kind == ItemKind::Label)
        .filter_map(|m| project.sheets.get(m.sheet)?.labels.get(m.index).map(|label| (m.sheet, label)))
        .filter(|(_, label)| label.name == net.name)
        .peekable();
    let sheet = naming.peek()?.0;
    naming.all(|(_, label)| label.scope == LabelScope::Sheet).then_some(sheet)
}

fn sheet_title(project: &Project, sheet: usize) -> String {
    let name = &project.sheets[sheet].name;
    if name.is_empty() { format!("sheet {}", sheet + 1) } else { name.clone() }
}

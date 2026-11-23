//! Small builders for schematics, and a generator of random ones on an integer grid.

use std::collections::BTreeSet;

use craft_circuit::{Connectivity, Label, LabelScope, Part, Pin, PinKind, Point, Project, Sheet, Wire};
use proptest::prelude::*;

pub fn part(designator: &str) -> Part {
    Part { designator: designator.to_owned(), ..Part::default() }
}

pub fn pin(part: usize, number: &str, kind: PinKind, at: Point) -> Pin {
    Pin { part, number: number.to_owned(), name: String::new(), kind, at }
}

pub fn wire(points: &[Point]) -> Wire {
    Wire { points: points.to_vec() }
}

pub fn label(name: &str, scope: LabelScope, at: Point) -> Label {
    Label { name: name.to_owned(), scope, at }
}

pub fn project(parts: Vec<Part>, sheets: Vec<Sheet>) -> Project {
    Project { parts, sheets, ..Project::default() }
}

/// Which pins share a net, as sets of (designator, pin number).
pub fn pin_groups(project: &Project, conn: &Connectivity) -> BTreeSet<BTreeSet<(String, String)>> {
    conn.nets
        .iter()
        .map(|net| {
            net.pins
                .iter()
                .filter_map(|&p| project.pin(p))
                .map(|p| (project.parts[p.part].designator.clone(), p.number.clone()))
                .collect::<BTreeSet<_>>()
        })
        .filter(|g| !g.is_empty())
        .collect()
}

/// Whether two pins, given by designator and number, share a net.
pub fn joined(project: &Project, conn: &Connectivity, a: (&str, &str), b: (&str, &str)) -> bool {
    pin_groups(project, conn).iter().any(|g| g.contains(&(a.0.to_owned(), a.1.to_owned())) && g.contains(&(b.0.to_owned(), b.1.to_owned())))
}

fn grid_point() -> impl Strategy<Value = Point> {
    (0i32..12, 0i32..12).prop_map(|(x, y)| [f64::from(x), f64::from(y)])
}

fn pin_kind() -> impl Strategy<Value = PinKind> {
    prop_oneof![
        Just(PinKind::Input),
        Just(PinKind::Output),
        Just(PinKind::Passive),
        Just(PinKind::PowerIn),
        Just(PinKind::PowerOut),
        Just(PinKind::NotConnected),
    ]
}

fn scope() -> impl Strategy<Value = LabelScope> {
    prop_oneof![Just(LabelScope::Sheet), Just(LabelScope::Global), Just(LabelScope::Power)]
}

fn sheet(parts: usize) -> impl Strategy<Value = Sheet> {
    let pins = prop::collection::vec((0..parts, 1u8..=2, pin_kind(), grid_point()), 0..8)
        .prop_map(|v| v.into_iter().map(|(p, n, k, at)| pin(p, &n.to_string(), k, at)).collect::<Vec<_>>());
    let wires = prop::collection::vec(prop::collection::vec(grid_point(), 1..4).prop_map(|points| Wire { points }), 0..8);
    let junctions = prop::collection::vec(grid_point(), 0..3);
    let labels = prop::collection::vec((prop::sample::select(vec!["A", "B", "GND"]), scope(), grid_point()), 0..4)
        .prop_map(|v| v.into_iter().map(|(n, s, at)| label(n, s, at)).collect::<Vec<_>>());
    (pins, wires, junctions, labels).prop_map(|(pins, wires, junctions, labels)| Sheet { name: String::new(), pins, wires, junctions, labels })
}

/// A random schematic of one or two sheets, on integer coordinates so that sums are exact.
pub fn schematic() -> impl Strategy<Value = Project> {
    (1usize..5).prop_flat_map(|n| {
        let parts = (0..n).map(|i| part(&format!("P{}", i + 1))).collect::<Vec<_>>();
        prop::collection::vec(sheet(n), 1..3).prop_map(move |sheets| project(parts.clone(), sheets))
    })
}

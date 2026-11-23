use std::collections::HashSet;

use craft_circuit::{Connectivity, LabelScope, PinKind, Project, Sheet};
use proptest::prelude::*;

use crate::support::{joined, label, part, pin, pin_groups, project, schematic, wire};

#[test]
fn a_wire_ending_on_the_middle_of_another_joins_it() {
    let sheet = Sheet {
        pins: vec![pin(0, "1", PinKind::Passive, [0.0, 0.0]), pin(1, "1", PinKind::Passive, [5.0, 5.0])],
        wires: vec![wire(&[[0.0, 0.0], [10.0, 0.0]]), wire(&[[5.0, 5.0], [5.0, 0.0]])],
        ..Sheet::default()
    };
    let p = project(vec![part("R1"), part("R2")], vec![sheet]);
    let conn = Connectivity::build(&p).unwrap();
    assert!(joined(&p, &conn, ("R1", "1"), ("R2", "1")));
}

#[test]
fn wires_that_cross_join_only_where_a_junction_sits() {
    let mut sheet = Sheet {
        pins: vec![pin(0, "1", PinKind::Passive, [0.0, 5.0]), pin(1, "1", PinKind::Passive, [5.0, 0.0])],
        wires: vec![wire(&[[0.0, 5.0], [10.0, 5.0]]), wire(&[[5.0, 0.0], [5.0, 10.0]])],
        ..Sheet::default()
    };
    let parts = vec![part("R1"), part("R2")];
    let apart = project(parts.clone(), vec![sheet.clone()]);
    assert!(!joined(&apart, &Connectivity::build(&apart).unwrap(), ("R1", "1"), ("R2", "1")));
    sheet.junctions.push([5.0, 5.0]);
    let together = project(parts, vec![sheet]);
    assert!(joined(&together, &Connectivity::build(&together).unwrap(), ("R1", "1"), ("R2", "1")));
}

#[test]
fn sheet_labels_join_on_their_own_sheet_only_and_global_labels_join_across_sheets() {
    let sheet = |scope: LabelScope, part: usize| Sheet {
        name: format!("S{part}"),
        pins: vec![pin(part, "1", PinKind::Passive, [0.0, 0.0])],
        labels: vec![label("SIG", scope, [0.0, 0.0])],
        ..Sheet::default()
    };
    let parts = vec![part("R1"), part("R2")];
    let local = project(parts.clone(), vec![sheet(LabelScope::Sheet, 0), sheet(LabelScope::Sheet, 1)]);
    let conn = Connectivity::build(&local).unwrap();
    assert!(!joined(&local, &conn, ("R1", "1"), ("R2", "1")));
    let names: HashSet<&str> = conn.nets.iter().map(|n| n.name.as_str()).collect();
    assert_eq!(names.len(), conn.nets.len(), "each sheet's SIG net keeps a name of its own");

    let global = project(parts, vec![sheet(LabelScope::Global, 0), sheet(LabelScope::Global, 1)]);
    assert!(joined(&global, &Connectivity::build(&global).unwrap(), ("R1", "1"), ("R2", "1")));
}

/// The same schematic with its items listed in another order and moved by a whole offset.
fn reordered_and_moved(p: &Project, seed: usize, dx: f64, dy: f64) -> Project {
    let mut out = p.clone();
    for sheet in &mut out.sheets {
        let n = sheet.pins.len().max(1);
        sheet.pins.rotate_left(seed % n);
        sheet.wires.reverse();
        sheet.labels.reverse();
        let shift = |q: &mut [f64; 2]| {
            q[0] += dx;
            q[1] += dy;
        };
        sheet.pins.iter_mut().for_each(|p| shift(&mut p.at));
        sheet.junctions.iter_mut().for_each(shift);
        sheet.labels.iter_mut().for_each(|l| shift(&mut l.at));
        for w in &mut sheet.wires {
            w.points.reverse();
            w.points.iter_mut().for_each(shift);
        }
    }
    out
}

proptest! {
    #[test]
    fn which_pins_share_a_net_does_not_depend_on_drawing_order_or_position(
        p in schematic(), seed in 0usize..8, dx in -1000i32..1000, dy in -1000i32..1000,
    ) {
        let moved = reordered_and_moved(&p, seed, f64::from(dx), f64::from(dy));
        let a = pin_groups(&p, &Connectivity::build(&p).unwrap());
        let b = pin_groups(&moved, &Connectivity::build(&moved).unwrap());
        prop_assert_eq!(a, b);
    }

    #[test]
    fn net_names_are_unique(p in schematic()) {
        let conn = Connectivity::build(&p).unwrap();
        let names: HashSet<&str> = conn.nets.iter().map(|n| n.name.as_str()).collect();
        prop_assert_eq!(names.len(), conn.nets.len());
    }
}

//! Damaged and oversized input is refused with an error, never a panic or a hang.

use craft_circuit::erc::{Settings, check};
use craft_circuit::{BomOptions, Connectivity, Error, Limits, Netlist, PinKind, Project, Sheet, bom};
use proptest::prelude::*;

use crate::support::{part, pin, project, schematic, wire};

/// Everything a caller may run on a project that was read.
fn exercise(bytes: &[u8]) {
    let Ok(p) = Project::from_json(bytes) else { return };
    if let Ok(conn) = Connectivity::build(&p) {
        let _ = check(&p, &conn, &Settings::default());
        let netlist = Netlist::new(&p, &conn);
        let _ = netlist.to_csv();
    }
    let _ = bom(&p, &BomOptions::default());
}

fn damage(mut bytes: Vec<u8>, cut: usize, flips: Vec<(usize, u8)>) -> Vec<u8> {
    for (at, value) in flips {
        if !bytes.is_empty() {
            let i = at % bytes.len();
            bytes[i] = value;
        }
    }
    let keep = if bytes.is_empty() { 0 } else { cut % (bytes.len() + 1) };
    bytes.truncate(keep);
    bytes
}

proptest! {
    #[test]
    fn any_bytes_are_refused_or_read_without_panicking(bytes in prop::collection::vec(any::<u8>(), 0..512)) {
        exercise(&bytes);
    }

    #[test]
    fn a_damaged_project_is_refused_or_analysed_without_panicking(
        p in schematic(), cut in any::<usize>(), flips in prop::collection::vec((any::<usize>(), any::<u8>()), 0..6),
    ) {
        let json = p.to_json().unwrap().into_bytes();
        exercise(&damage(json, cut, flips));
    }
}

#[test]
fn a_pin_naming_a_missing_part_is_refused() {
    let json = br#"{"parts":[],"sheets":[{"pins":[{"part":3,"number":"1","at":[0,0]}]}]}"#;
    assert!(matches!(Project::from_json(json), Err(Error::NoSuchPart { part: 3, .. })));
}

#[test]
fn a_schematic_needing_too_many_comparisons_is_refused_instead_of_running_on() {
    let wires = (0..200).map(|_| wire(&[[0.0, 0.0], [10.0, 10.0]])).collect();
    let pins = (0..200).map(|i| pin(0, &i.to_string(), PinKind::Passive, [5.0, 5.0])).collect();
    let p = project(vec![part("X1")], vec![Sheet { pins, wires, ..Sheet::default() }]);
    let tight = Limits { work: 10_000, ..Limits::default() };
    assert!(matches!(Connectivity::build_with(&p, &tight), Err(Error::TooComplex { .. })));
    assert!(Connectivity::build(&p).is_ok(), "the same schematic fits the default limit");
}

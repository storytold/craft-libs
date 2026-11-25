use craft_circuit::erc::{Finding, Level, Rule, Settings, check};
use craft_circuit::{Connectivity, LabelScope, PinKind, Project, Sheet};

use crate::support::{label, part, pin, project, wire};

fn findings(p: &Project) -> Vec<Finding> {
    check(p, &Connectivity::build(p).unwrap(), &Settings::default())
}

fn has(found: &[Finding], rule: Rule) -> bool {
    found.iter().any(|f| f.rule == rule)
}

/// Pins of the given kinds, all on one wire.
fn one_net(kinds: &[PinKind]) -> Project {
    let pins = kinds.iter().enumerate().map(|(i, &k)| pin(i, "1", k, [i as f64, 0.0])).collect();
    let end = kinds.len().saturating_sub(1).max(1) as f64;
    let parts = (0..kinds.len()).map(|i| part(&format!("U{}", i + 1))).collect();
    project(parts, vec![Sheet { pins, wires: vec![wire(&[[0.0, 0.0], [end, 0.0]])], ..Sheet::default() }])
}

#[test]
fn two_outputs_on_one_net_conflict_but_an_output_driving_inputs_does_not() {
    let fight = findings(&one_net(&[PinKind::Output, PinKind::Output]));
    assert!(fight.iter().any(|f| f.rule == Rule::PinConflict && f.level == Level::Error && f.items.len() == 2));
    let drive = findings(&one_net(&[PinKind::Output, PinKind::Input, PinKind::Input]));
    assert!(!has(&drive, Rule::PinConflict) && !has(&drive, Rule::UndrivenInput));
}

#[test]
fn inputs_with_nothing_driving_them_are_reported() {
    let found = findings(&one_net(&[PinKind::Input, PinKind::Input]));
    assert!(found.iter().any(|f| f.rule == Rule::UndrivenInput && f.items.len() == 2));
}

#[test]
fn supply_pins_need_a_supply_output_or_a_supply_label() {
    let mut p = one_net(&[PinKind::PowerIn, PinKind::Passive]);
    assert!(has(&findings(&p), Rule::UnpoweredInput));
    p.sheets[0].labels.push(label("GND", LabelScope::Power, [0.5, 0.0]));
    assert!(!has(&findings(&p), Rule::UnpoweredInput));
    let fed = one_net(&[PinKind::PowerIn, PinKind::PowerOut]);
    assert!(!has(&findings(&fed), Rule::UnpoweredInput));
}

#[test]
fn a_wire_end_on_a_pin_is_connected_and_a_free_end_is_dangling() {
    let p = project(
        vec![part("R1"), part("R2")],
        vec![Sheet {
            pins: vec![pin(0, "1", PinKind::Passive, [0.0, 0.0]), pin(1, "1", PinKind::Passive, [4.0, 0.0])],
            wires: vec![wire(&[[0.0, 0.0], [4.0, 0.0]]), wire(&[[2.0, 0.0], [2.0, 3.0]])],
            ..Sheet::default()
        }],
    );
    let dangling: Vec<_> = findings(&p).into_iter().filter(|f| f.rule == Rule::DanglingWire).collect();
    assert_eq!(dangling.len(), 1, "only the stub's free end");
    assert_eq!(dangling[0].items[0].index, 1);
    assert_eq!(dangling[0].at, Some((0, [2.0, 3.0])), "the finding points at the loose end, not the joined one");
}

#[test]
fn a_pin_marked_not_connected_may_stay_alone_but_not_join_another_pin() {
    let alone = findings(&one_net(&[PinKind::NotConnected]));
    assert!(!has(&alone, Rule::UnconnectedPin) && !has(&alone, Rule::NotConnectedPinConnected));
    let joined = findings(&one_net(&[PinKind::NotConnected, PinKind::Passive]));
    assert!(has(&joined, Rule::NotConnectedPinConnected));
}

#[test]
fn duplicate_and_unnumbered_designators_are_reported_with_their_parts() {
    let mut p = one_net(&[PinKind::Passive, PinKind::Passive, PinKind::Passive]);
    p.parts[1].designator = "U1".into();
    p.parts[2].designator = "R?".into();
    let found = findings(&p);
    let dup = found.iter().find(|f| f.rule == Rule::DuplicateDesignator).expect("duplicate reported");
    assert_eq!(dup.parts, vec![0, 1]);
    let missing = found.iter().find(|f| f.rule == Rule::MissingDesignator).expect("missing reported");
    assert_eq!(missing.parts, vec![2]);
}

#[test]
fn findings_come_errors_first() {
    let mut p = one_net(&[PinKind::Output, PinKind::Output, PinKind::Input]);
    p.sheets[0].wires.push(wire(&[[9.0, 9.0], [9.0, 12.0]]));
    let found = findings(&p);
    assert!(found.iter().any(|f| f.level == Level::Warning));
    assert!(found.windows(2).all(|w| w[0].level >= w[1].level));
}

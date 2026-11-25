use std::collections::HashSet;

use craft_circuit::{BomOptions, Connectivity, LabelScope, Netlist, PinKind, Sheet, bom, natural_cmp};
use proptest::prelude::*;

use crate::support::{label, part, pin, project};

#[test]
fn designators_sort_by_their_numbers() {
    let mut names = vec!["R10", "C1", "R2", "R1", "R02"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, ["C1", "R1", "R2", "R02", "R10"]);
}

#[test]
fn a_pin_placed_on_two_sheets_is_one_node_of_its_net() {
    let unit = |name: &str| Sheet {
        name: name.into(),
        pins: vec![pin(0, "1", PinKind::Passive, [0.0, 0.0])],
        labels: vec![label("V", LabelScope::Global, [0.0, 0.0])],
        ..Sheet::default()
    };
    let p = project(vec![part("K1")], vec![unit("A"), unit("B")]);
    let netlist = Netlist::new(&p, &Connectivity::build(&p).unwrap());
    assert_eq!(netlist.nets.len(), 1);
    assert_eq!(netlist.nets[0].nodes.len(), 1);
}

fn parts_list_input() -> impl Strategy<Value = Vec<(String, String, String, bool)>> {
    let value = prop::sample::select(vec!["10k", "100n", "1k"]);
    let fp = prop::sample::select(vec!["0603", "0805"]);
    let mfr = prop::sample::select(vec!["", "X", "Y"]);
    prop::collection::vec((value, fp, mfr, any::<bool>()), 0..20)
        .prop_map(|v| v.into_iter().map(|(a, b, c, d)| (a.to_owned(), b.to_owned(), c.to_owned(), d)).collect())
}

proptest! {
    #[test]
    fn parts_list_rows_count_every_listed_part_once_and_differ_from_each_other(input in parts_list_input()) {
        let parts = input.iter().enumerate().map(|(i, (value, fp, mfr, listed))| {
            let mut p = part(&format!("R{}", i + 1));
            p.value = value.clone();
            p.footprint = fp.clone();
            if !mfr.is_empty() {
                p.fields.insert("mfr".into(), mfr.clone());
            }
            p.in_bom = *listed;
            p
        }).collect::<Vec<_>>();
        let listed = parts.iter().filter(|p| p.in_bom).count();
        let options = BomOptions { group_by: vec!["mfr".into()], include_excluded: false };
        let rows = bom(&project(parts, Vec::new()), &options);
        prop_assert_eq!(rows.iter().map(|r| r.quantity).sum::<usize>(), listed);
        prop_assert!(rows.iter().all(|r| r.quantity == r.designators.len()));
        let keys: HashSet<_> = rows.iter().map(|r| (&r.value, &r.footprint, &r.fields)).collect();
        prop_assert_eq!(keys.len(), rows.len());
    }
}

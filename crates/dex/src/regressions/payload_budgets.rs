use crate::dex_ops::{self, Payload};

fn arrays(references: usize, bytes: usize) -> Vec<u16> {
    let mut code = Vec::new();
    let offset = (references * 3).next_multiple_of(2);
    for n in 0..references {
        let relative = u32::try_from(offset - n * 3).unwrap();
        code.extend([0x0026, relative as u16, (relative >> 16) as u16]);
    }
    code.resize(offset, 0);
    let count = u32::try_from(bytes).unwrap();
    code.extend([0x0300, 1, count as u16, (count >> 16) as u16]);
    code.resize(code.len() + bytes.div_ceil(2), 0x5a5a);
    code
}

fn bytes(rows: &[dex_ops::Insn]) -> usize {
    rows.iter()
        .map(|i| match i.payload.as_ref() {
            Some(Payload::FillArrayData { data, .. }) => data.len(),
            Some(Payload::PackedSwitch { targets, .. }) => targets.len() * 4,
            Some(Payload::SparseSwitch { keys, targets }) => (keys.len() + targets.len()) * 4,
            None => 0,
        })
        .sum()
}

#[test]
fn shared_arrays_charge_the_source_and_each_attached_copy() {
    let code = arrays(4, 8);
    let (rows, complete) = dex_ops::decode_with_payload_limit(&code, 100, 24);
    assert!(!complete);
    assert_eq!(bytes(&rows), 24);
    assert!(rows[0].payload.is_some() && rows[1].payload.is_some());
    assert!(rows[2].payload.is_none() && rows[3].payload.is_none());
    assert!(rows.last().unwrap().payload.is_some());
    let (full, complete) = dex_ops::decode_with_payload_limit(&code, 100, 40);
    assert!(complete);
    assert_eq!(bytes(&full), 40);
}

#[test]
fn zero_budget_keeps_offsets_and_payload_instruction_metadata() {
    let code = arrays(2, 8);
    let (rows, complete) = dex_ops::decode_with_payload_limit(&code, 100, 0);
    assert!(!complete);
    assert_eq!(rows.iter().map(|i| i.offset).collect::<Vec<_>>(), [0, 3, 6]);
    assert_eq!(rows[2].name, "array-data-payload");
    assert_eq!(rows[2].units, 8);
    assert!(rows.iter().all(|i| i.payload.is_none()));
    assert!(dex_ops::decode_with_payload_limit(&[0x000e], 100, 0).1);
}

#[test]
fn both_switch_tables_are_accounted_and_backward_references_work() {
    // packed table: one target; sparse table: one key and one target; users follow the tables.
    let code = [0x0100, 1, 7, 0, 1, 0, 0x0200, 1, 9, 0, 2, 0, 0x002b, 0xfff4, 0xffff, 0x002c, 0xfff7, 0xffff];
    let (rows, complete) = dex_ops::decode_with_payload_limit(&code, 100, 24);
    assert!(complete);
    assert_eq!(rows[2].payload, Some(Payload::PackedSwitch { first_key: 7, targets: vec![1] }));
    assert_eq!(rows[3].payload, Some(Payload::SparseSwitch { keys: vec![9], targets: vec![2] }));
    let (rows, complete) = dex_ops::decode_with_payload_limit(&code, 100, 16);
    assert!(!complete);
    assert!(rows[2].payload.is_some());
    assert!(rows[3].payload.is_none());
    assert_eq!(bytes(&rows), 16);
}

#[test]
fn the_existing_per_array_cap_is_reported_as_partial() {
    let (rows, complete) = dex_ops::decode(&arrays(1, 65_537), 100);
    assert!(!complete);
    assert_eq!(bytes(&rows), 2 * 65_536);
    let (rows, complete) = dex_ops::decode(&arrays(1, 8), 1);
    assert!(!complete);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].payload.is_none());
}

#[test]
#[ignore = "memory regression: run separately with a 128 MiB address-space limit"]
fn repeated_array_references_stay_under_the_default_byte_budget() {
    let code = arrays(2048, 65_536);
    let (rows, complete) = dex_ops::decode(&code, 200_000);
    assert!(!complete);
    assert_eq!(rows.len(), 2049);
    assert_eq!(bytes(&rows), dex_ops::DEFAULT_MAX_PAYLOAD_BYTES);
}

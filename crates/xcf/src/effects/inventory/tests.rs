use super::*;
use crate::effects::{Error, Limit};
use crate::tests::W;
use std::cell::Cell;

fn fixture(version: u32, pointers: &[u64]) -> (Vec<u8>, usize, usize) {
    let mut w = W::new(version);
    for value in [1, 1, 0] {
        w.u32(value);
    }
    if version >= 4 {
        w.u32(150);
    }
    w.end();
    let slot = w.slot();
    w.ptr(0);
    w.ptr(0);
    if version >= 18 {
        w.ptr(0);
    }
    let owner = w.out.len();
    w.patch(slot, owner);
    for value in [1, 1, 1] {
        w.u32(value);
    }
    w.string("Owner");
    w.end();
    w.ptr(0); // no hierarchy: inventory must not request pixels
    w.ptr(0); // no mask
    let list = w.out.len();
    if version >= 20 {
        for pointer in pointers {
            w.out.extend_from_slice(&pointer.to_be_bytes());
        }
        w.ptr(0);
    }
    (w.out, owner, list)
}

#[test]
fn old_files_have_no_effect_table_and_new_files_keep_order_and_wire_identity() {
    for version in [3, 19, 20, 26] {
        let (bytes, owner, _) = fixture(version, &[u64::MAX, 8, 123]);
        let xcf = crate::Xcf::parse(&bytes).unwrap();
        assert_eq!(xcf.image().layers[0].effects, if version >= 20 { 3 } else { 0 });
        let mut budget = Budget::default();
        let inventory = xcf.layer_effects(0, &mut budget, &|| true).unwrap();
        assert_eq!(inventory.owner, Owner { kind: OwnerKind::Layer, offset: owner as u64 });
        assert_eq!(inventory.offsets, if version >= 20 { vec![u64::MAX, 8, 123] } else { vec![] });
        assert!(xcf.layer_pixels(0, &mut 0, &|| true).unwrap().is_none());
        assert!(matches!(xcf.layer_effects(1, &mut budget, &|| true), Err(Error::Source(crate::Error::Invalid(_)))));
    }
}

#[test]
fn source_owner_is_carried_into_the_separately_decoded_record() {
    let (mut bytes, owner, list) = fixture(26, &[1]);
    let record = bytes.len();
    bytes[list..list + 8].copy_from_slice(&(record as u64).to_be_bytes());
    let mut w = W { out: bytes, wide: true };
    for value in ["Exposure", "", "gegl:exposure", "1:0"] {
        w.string(value);
    }
    w.end();
    w.ptr(0);
    let xcf = crate::Xcf::parse(&w.out).unwrap();
    let mut budget = Budget::default();
    let inventory = xcf.layer_effects(0, &mut budget, &|| true).unwrap();
    let effect = Reader::new(&w.out, 26).read(inventory.owner, inventory.offsets[0], &mut budget, &|| true).unwrap();
    assert_eq!(effect.owner.offset, owner as u64);
    assert_eq!(effect.record.operation, "gegl:exposure");
    assert_eq!(effect.record.record_offset, record);
}

#[test]
fn repeated_inventory_spends_work_and_failed_retention_refunds_only_storage() {
    let (bytes, _, _) = fixture(26, &[8, 123]);
    let xcf = crate::Xcf::parse(&bytes).unwrap();
    let mut measured = Budget::default();
    let before = measured.remaining_work();
    xcf.layer_effects(0, &mut measured, &|| true).unwrap();
    let cost = before - measured.remaining_work();
    let mut exact = Budget::new(4096, cost);
    xcf.layer_effects(0, &mut exact, &|| true).unwrap();
    assert_eq!(exact.remaining_work(), 0);
    assert!(matches!(xcf.layer_effects(0, &mut exact, &|| true), Err(Error::Limit(Limit::Work))));
    let mut no_storage = Budget::new(0, 4096);
    assert!(matches!(xcf.layer_effects(0, &mut no_storage, &|| true), Err(Error::Limit(Limit::Storage))));
    assert_eq!(no_storage.remaining_storage(), 0);
    assert!(no_storage.remaining_work() < 4096);
    // Repeated wire entries retain both applications and spend work per entry.
    let (duplicate, _, _) = fixture(26, &[8, 8]);
    let xcf = crate::Xcf::parse(&duplicate).unwrap();
    let mut budget = Budget::new(4096, 4096);
    assert_eq!(xcf.layer_effects(0, &mut budget, &|| true).unwrap().offsets, [8, 8]);
    assert!(budget.remaining_storage() < 4096);
    assert!(budget.remaining_work() < 4096);
}

#[test]
fn cancellation_is_checked_before_lookup_and_during_pointer_traversal() {
    let (bytes, _, _) = fixture(26, &[8, 123, 456]);
    let xcf = crate::Xcf::parse(&bytes).unwrap();
    let mut budget = Budget::new(4096, 4096);
    assert!(matches!(xcf.layer_effects(usize::MAX, &mut budget, &|| false), Err(Error::Source(crate::Error::Cancelled))));
    assert_eq!((budget.remaining_storage(), budget.remaining_work()), (4096, 4096));
    let calls = Cell::new(0);
    let keep_going = || {
        calls.set(calls.get() + 1);
        calls.get() < 5
    };
    assert!(matches!(xcf.layer_effects(0, &mut budget, &keep_going), Err(Error::Source(crate::Error::Cancelled))));
    assert!(budget.remaining_work() < 4096);
    assert!(budget.remaining_storage() < 4096, "fatal cancellation retains provisional charges");
}

#[test]
fn pointer_retention_rolls_back_after_later_work_or_framing_failure() {
    let (bytes, _, _) = fixture(26, &[8, 123]);
    let xcf = crate::Xcf::parse(&bytes).unwrap();
    // The lookup and first wide pointer are priced before the second attempt.
    let mut budget = Budget::new(4096, 2 * std::mem::size_of::<usize>() + 24);
    assert!(matches!(xcf.layer_effects(0, &mut budget, &|| true), Err(Error::Limit(Limit::Work))));
    assert_eq!(budget.remaining_storage(), 4096);
    assert_eq!(budget.remaining_work(), 0);
    let bytes = 8u64.to_be_bytes(); // first pointer retained, missing terminator
    let mut budget = Budget::new(4096, 4096);
    assert!(matches!(Reader::new(&bytes, 26).pointers(0, &mut budget, &|| true), Err(Error::Source(_))));
    assert_eq!(budget.remaining_storage(), 4096);
    assert!(budget.remaining_work() < 4096);
}

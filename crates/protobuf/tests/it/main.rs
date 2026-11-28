//! Messages written by the tests, field by field, as the public encoding description lays them
//! out, and arbitrary bytes, which never panic.

use craft_protobuf::{Message, Value, read_varint};
use proptest::prelude::*;

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn tag(out: &mut Vec<u8>, field: u32, wire: u8) {
    varint(out, u64::from(field) << 3 | u64::from(wire));
}

#[test]
fn a_repeated_field_reads_in_order_and_its_last_value_wins() {
    let mut m = Vec::new();
    // Field 1 twice, as varints; field 2 packed, then once more unpacked; field 3 two messages.
    for v in [7, 9] {
        tag(&mut m, 1, 0);
        varint(&mut m, v);
    }
    let mut packed = Vec::new();
    for v in [1, 300, 2] {
        varint(&mut packed, v);
    }
    tag(&mut m, 2, 2);
    varint(&mut m, packed.len() as u64);
    m.extend_from_slice(&packed);
    tag(&mut m, 2, 0);
    varint(&mut m, 4);
    for inner in [b"\x08\x01", b"\x08\x02"] {
        tag(&mut m, 3, 2);
        varint(&mut m, inner.len() as u64);
        m.extend_from_slice(inner);
    }
    let message = Message::new(&m);
    assert_eq!(message.varint(1), Some(9));
    assert_eq!(message.all(1).collect::<Vec<_>>(), [Value::Varint(7), Value::Varint(9)]);
    assert_eq!(message.varints(2), [1, 300, 2, 4]);
    // Read from a message made on the spot: the fields borrow the bytes, not the message.
    let inner: Vec<u64> = Message::new(&m).messages(3).filter_map(|m| m.varint(1)).collect();
    assert_eq!(inner, [1, 2]);
}

#[test]
fn floats_are_read_from_either_fixed_width() {
    let mut m = Vec::new();
    tag(&mut m, 1, 5);
    m.extend_from_slice(&1.5f32.to_le_bytes());
    tag(&mut m, 2, 1);
    m.extend_from_slice(&(-2.25f64).to_le_bytes());
    let message = Message::new(&m);
    assert_eq!((message.float(1), message.float(2)), (Some(1.5), Some(-2.25)));
    assert_eq!(message.text(1), None, "a number isn't text");
}

/// Walk every field, and every length-delimited value as a message, a few levels down.
fn walk(m: Message<'_>, depth: u32) {
    for f in m.fields() {
        let _ = (m.varint(f.number), m.float(f.number), m.text(f.number), m.varints(f.number));
        if let Value::Bytes(b) = f.value
            && depth < 4
        {
            walk(Message::new(b), depth + 1);
        }
    }
}

proptest! {
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..1024)) {
        walk(Message::new(&bytes), 0);
        let mut pos = 0;
        while pos < bytes.len() && read_varint(&bytes, &mut pos).is_some() {}
    }
}

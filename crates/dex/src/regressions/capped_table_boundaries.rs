use crate::dex::{self, ClassDef, Code};

const MEMBER_CAP: usize = 65_536;
const HANDLER_CAP: usize = 256;

fn uleb(mut value: u32, bytes: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        bytes.push(byte | if value == 0 { 0 } else { 0x80 });
        if value == 0 {
            break;
        }
    }
}

fn sleb(mut value: i32, bytes: &mut Vec<u8>) {
    loop {
        let byte = value as u8 & 0x7f;
        value >>= 7;
        let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
        bytes.push(byte | if done { 0 } else { 0x80 });
        if done {
            break;
        }
    }
}

fn header() -> Vec<u8> {
    let mut bytes = vec![0; 0x70];
    bytes[..8].copy_from_slice(b"dex\n035\0");
    bytes[36..40].copy_from_slice(&0x70u32.to_le_bytes());
    bytes[40..44].copy_from_slice(&0x1234_5678u32.to_le_bytes());
    bytes
}

fn class_data(count: usize) -> dex::ClassData {
    let mut bytes = header();
    for n in [count as u32, 0, 1, 0] {
        uleb(n, &mut bytes);
    }
    bytes.extend_from_slice(&[0, 1].repeat(count));
    bytes.extend_from_slice(&[3, 9, 7]);
    let len = bytes.len() as u32;
    bytes[32..36].copy_from_slice(&len.to_le_bytes());
    let class = ClassDef {
        class_idx: 0,
        access: 0,
        superclass_idx: 0,
        interfaces_off: 0,
        source_file_idx: 0,
        annotations_off: 0,
        class_data_off: 0x70,
        static_values_off: 0,
    };
    dex::parse(&bytes).unwrap().class_data(&class)
}

fn code(size: i32, payload: &[u8]) -> Code {
    let mut bytes = header();
    bytes.extend_from_slice(&[1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 8, 0, 0, 0]);
    bytes.extend_from_slice(&[0; 16]);
    bytes.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
    bytes.push(1);
    sleb(size, &mut bytes);
    bytes.extend_from_slice(payload);
    let len = bytes.len() as u32;
    bytes[32..36].copy_from_slice(&len.to_le_bytes());
    dex::parse(&bytes).unwrap().code(0x70).unwrap()
}

#[test]
fn capped_class_members_do_not_become_the_next_group() {
    let data = class_data(MEMBER_CAP + 1);
    assert_eq!(data.static_fields.len(), MEMBER_CAP);
    assert!(data.direct_methods.is_empty());
    assert!(!data.complete);
}

#[test]
fn an_exact_member_limit_keeps_the_following_group() {
    let data = class_data(MEMBER_CAP);
    assert_eq!(data.static_fields.len(), MEMBER_CAP);
    assert!(data.complete);
    assert_eq!(data.direct_methods, [dex::EncodedMethod { method_idx: 3, access: 9, code_off: 7 }]);
}

#[test]
fn an_unread_typed_handler_is_not_a_catch_all() {
    let mut payload = [0, 1].repeat(HANDLER_CAP + 1);
    payload.push(7);
    let parsed = code(-((HANDLER_CAP + 1) as i32), &payload);
    assert_eq!(parsed.tries[0].handlers.len(), HANDLER_CAP);
    assert!(parsed.tries[0].handlers.iter().all(|handler| handler.exception.is_some() && handler.address == 1));
}

#[test]
fn an_incomplete_typed_handler_cannot_supply_a_catch_all_address() {
    let parsed = code(-2, &[0, 1, 0]);
    assert_eq!(parsed.tries[0].handlers.len(), 1);
    assert!(parsed.tries[0].handlers[0].exception.is_some());
    assert_eq!(parsed.tries[0].handlers[0].address, 1);
}

#[test]
fn complete_typed_lists_keep_their_real_catch_all() {
    for count in [0, 1, HANDLER_CAP] {
        let mut payload = [0, 1].repeat(count);
        payload.push(7);
        let parsed = code(-(count as i32), &payload);
        assert_eq!(parsed.tries[0].handlers.len(), count + 1);
        assert_eq!(parsed.tries[0].handlers.last(), Some(&dex::Handler { exception: None, address: 7 }));
    }
}

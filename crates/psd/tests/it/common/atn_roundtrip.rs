//! Shared round-trip invariant for the `.atn` parser contract suite.
use craft_psd::atn::{self, AtnFile};
use craft_psd::descriptor::{Descriptor, Value};

fn scalar(value: &mut f64, bits: &mut Vec<u64>) {
    bits.push(value.to_bits());
    // AST equality must remain meaningful when a valid descriptor contains NaN.
    if value.is_nan() {
        *value = 0.0;
    }
}

fn descriptor(desc: &mut Descriptor, bits: &mut Vec<u64>) {
    for (_, item) in &mut desc.items {
        value(item, bits);
    }
}

fn value(item: &mut Value, bits: &mut Vec<u64>) {
    match item {
        Value::Double(v) | Value::UnitFloat { value: v, .. } => scalar(v, bits),
        Value::UnitFloats { values, .. } => {
            for v in values {
                scalar(v, bits);
            }
        }
        Value::Descriptor(d) | Value::GlobalObject(d) => descriptor(d, bits),
        Value::ObjectArray(array) => descriptor(&mut array.body, bits),
        Value::List(items) => {
            for item in items {
                value(item, bits);
            }
        }
        Value::Reference(_)
        | Value::Text(_)
        | Value::Enumerated { .. }
        | Value::Integer(_)
        | Value::LargeInteger(_)
        | Value::Boolean(_)
        | Value::Class(_)
        | Value::GlobalClass(_)
        | Value::Alias(_)
        | Value::Path(_)
        | Value::RawData(_) => {}
    }
}

fn floats(file: &mut AtnFile) -> Vec<u64> {
    let mut bits = Vec::new();
    for action in &mut file.actions {
        for item in &mut action.items {
            if let Some(d) = &mut item.descriptor {
                descriptor(d, &mut bits);
            }
        }
    }
    bits
}

pub fn check(data: &[u8]) {
    if let Ok(mut original) = atn::parse(data) {
        let encoded = atn::write(&original).expect("a parsed set is writable");
        let mut again = atn::parse(&encoded).expect("a written set parses");
        assert_eq!(atn::write(&again).expect("a reparsed set is writable"), encoded);
        assert_eq!(floats(&mut original), floats(&mut again), "every floating-point bit pattern survives");
        // Trailing-input diagnostics are not serialized into an action set.
        original.warnings.clear();
        again.warnings.clear();
        assert_eq!(again, original, "set metadata, action metadata and every descriptor value survive");
    }
}

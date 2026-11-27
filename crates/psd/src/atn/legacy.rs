//! Version 12 descriptors use bare codes and omit Unicode class names.

use crate::descriptor::{Class, Descriptor, Id, MAX_DEPTH, ObjectArray, ReferenceItem, UnicodeString, Value, vec_for_count};
use crate::error::{PsdError, Result};
use crate::io::Reader;

/// Read the unnamed, classless descriptor at the root of a legacy action step.
pub(super) fn read(r: &mut Reader<'_>) -> Result<Descriptor> {
    descriptor(r, Id::Code(*b"null"), 0)
}

fn class(r: &mut Reader<'_>) -> Result<Class> {
    Ok(Class { name: UnicodeString::default(), class_id: Id::Code(r.array()?) })
}

/// Read items after their enclosing class, with the same depth bound as modern descriptors.
fn descriptor(r: &mut Reader<'_>, class_id: Id, depth: usize) -> Result<Descriptor> {
    if depth > MAX_DEPTH {
        return Err(PsdError::LimitExceeded("descriptor nesting too deep"));
    }
    let n = r.u32()?;
    // A bare key, type and Boolean need nine bytes, without a modern ID length.
    r.check_count(u64::from(n), 9)?;
    let mut items = vec_for_count(n);
    for _ in 0..n {
        items.push((Id::Code(r.array()?), value(r, depth + 1)?));
    }
    Ok(Descriptor { name: UnicodeString::default(), class_id, items })
}

/// Decode a legacy value without trying the modern dialect on malformed input.
fn value(r: &mut Reader<'_>, depth: usize) -> Result<Value> {
    if depth > MAX_DEPTH {
        return Err(PsdError::LimitExceeded("descriptor nesting too deep"));
    }
    Ok(match &r.array::<4>()? {
        b"obj " => {
            let n = r.u32()?;
            r.check_count(u64::from(n), 8)?;
            let mut items = vec_for_count(n);
            for _ in 0..n {
                items.push(reference(r)?);
            }
            Value::Reference(items)
        }
        ty @ (b"Objc" | b"GlbO") => {
            let class_id = Id::Code(r.array()?);
            let d = descriptor(r, class_id, depth + 1)?;
            if ty == b"Objc" { Value::Descriptor(d) } else { Value::GlobalObject(d) }
        }
        b"VlLs" => {
            let n = r.u32()?;
            r.check_count(u64::from(n), 4)?;
            let mut items = vec_for_count(n);
            for _ in 0..n {
                items.push(value(r, depth + 1)?);
            }
            Value::List(items)
        }
        b"doub" => Value::Double(r.f64()?),
        b"UntF" => Value::UnitFloat { unit: r.array()?, value: r.f64()? },
        b"UnFl" => {
            let unit = r.array()?;
            let n = r.u32()?;
            r.check_count(u64::from(n), 8)?;
            let mut values = vec_for_count(n);
            for _ in 0..n {
                values.push(r.f64()?);
            }
            Value::UnitFloats { unit, values }
        }
        b"TEXT" => Value::Text(UnicodeString::read(r)?),
        b"enum" => Value::Enumerated { type_id: Id::Code(r.array()?), value: Id::Code(r.array()?) },
        b"long" => Value::Integer(r.i32()?),
        b"comp" => Value::LargeInteger(r.i64()?),
        b"bool" => Value::Boolean(r.u8()? != 0),
        b"type" => Value::Class(class(r)?),
        b"GlbC" => Value::GlobalClass(class(r)?),
        ty @ (b"alis" | b"Pth " | b"tdta") => {
            let n = r.u32()?;
            let bytes = r.bytes_u64(u64::from(n))?.to_vec();
            match ty {
                b"alis" => Value::Alias(bytes),
                b"Pth " => Value::Path(bytes),
                _ => Value::RawData(bytes),
            }
        }
        b"ObAr" => {
            let class_id = Id::Code(r.array()?);
            let prefix = r.u32()?;
            Value::ObjectArray(ObjectArray { prefix, body: descriptor(r, class_id, depth + 1)? })
        }
        other => return Err(PsdError::Unsupported(format!("legacy descriptor OSType {:?}", String::from_utf8_lossy(other)))),
    })
}

/// Every legacy reference form carries a bare class, including index and identifier forms.
fn reference(r: &mut Reader<'_>) -> Result<ReferenceItem> {
    let ty = r.array::<4>()?;
    let class = class(r)?;
    Ok(match &ty {
        b"prop" => ReferenceItem::Property { class, key: Id::Code(r.array()?) },
        b"Clss" => ReferenceItem::Class(class),
        b"Enmr" => ReferenceItem::Enumerated { class, type_id: Id::Code(r.array()?), value: Id::Code(r.array()?) },
        b"rele" => ReferenceItem::Offset { class, offset: r.i32()? },
        b"Idnt" => ReferenceItem::Identifier { class, id: r.u32()? },
        b"indx" => ReferenceItem::Index { class, index: r.u32()? },
        b"name" => ReferenceItem::Name { class, name: UnicodeString::read(r)? },
        other => return Err(PsdError::Unsupported(format!("legacy reference item type {:?}", String::from_utf8_lossy(other)))),
    })
}

//! File framing and ordered typed values, never operation mapping or evaluation.
use super::config::{parse_config, reserve};
use super::{Argument, ArgumentValue, ColorLiteral, Error, Limit, Result, SourceProperty, StoredEffect, invalid};
use crate::{Cursor, MAX_PAYLOAD, MAX_STRING};

const MAX_EFFECTS: usize = 1024;
const MAX_PROPERTIES: usize = 4096;

/// Shared, nonrefundable byte-work allowance. Rejection exhausts it.
#[derive(Debug)]
pub(super) struct DecodeWork {
    remaining: usize,
}
impl DecodeWork {
    pub(super) fn new(remaining: usize) -> Self {
        Self { remaining }
    }
    pub(super) fn remaining(&self) -> usize {
        self.remaining
    }
    pub(super) fn charge(&mut self, bytes: usize) -> Result<()> {
        match self.remaining.checked_sub(bytes) {
            Some(left) => {
                self.remaining = left;
                Ok(())
            }
            None => {
                self.remaining = 0;
                Err(Error::Limit(Limit::Work))
            }
        }
    }
}

fn check(keep_going: &dyn Fn() -> bool) -> Result<()> {
    if keep_going() { Ok(()) } else { Err(crate::Error::Cancelled.into()) }
}

fn pointer(cursor: &mut Cursor<'_>) -> Result<u64> {
    Ok(if cursor.wide { (u64::from(cursor.u32()?) << 32) | u64::from(cursor.u32()?) } else { u64::from(cursor.u32()?) })
}

fn charge(budget: &mut usize, bytes: usize) -> Result<()> {
    *budget = budget.checked_sub(bytes).ok_or(Error::Limit(Limit::Storage))?;
    Ok(())
}

fn push_error(effect: &mut StoredEffect<'_>, message: String, budget: &mut usize) -> Result<()> {
    charge(budget, message.capacity())?;
    reserve(&mut effect.errors, 1, budget).map_err(Error::from)?;
    effect.errors.push(message);
    Ok(())
}

pub(super) fn read_pointers_with_work(
    cursor: &mut Cursor<'_>,
    budget: &mut usize,
    keep_going: &dyn Fn() -> bool,
    work: &mut DecodeWork,
) -> Result<Vec<u64>> {
    let mut pointers = Vec::new();
    loop {
        check(keep_going)?;
        work.charge(if cursor.wide { 24 } else { 20 })?;
        let offset = pointer(cursor)?;
        if offset == 0 {
            return Ok(pointers);
        }
        if pointers.len() >= MAX_EFFECTS {
            return Err(invalid("more than 1,024 effects on one drawable"));
        }
        reserve(&mut pointers, 1, budget).map_err(Error::from)?;
        pointers.push(offset);
    }
}

/// Decode with the import's nonrefundable work allowance, shared across all owners.
pub(super) fn decode_with_work<'a>(
    base: &Cursor<'a>,
    offset: usize,
    version: u32,
    budget: &mut usize,
    work: &mut DecodeWork,
    ctl: &dyn Fn() -> bool,
) -> Result<StoredEffect<'a>> {
    check(ctl)?;
    if version < 20 {
        return Err(invalid("stored effect envelopes are not defined before XCF 20"));
    }
    charge(budget, 2 * std::mem::size_of::<StoredEffect<'a>>())?;
    let mut cursor = base.at(offset);
    work.charge(64)?;
    let name = string(&mut cursor, work)?;
    let icon = optional_string(&mut cursor, work)?;
    let operation = string(&mut cursor, work)?;
    let stored_version = if version >= 22 { optional_string(&mut cursor, work)? } else { None };
    charge(
        budget,
        name.len().saturating_add(icon.map_or(0, str::len)).saturating_add(operation.len()).saturating_add(stored_version.map_or(0, str::len)),
    )?;
    let mut effect = StoredEffect {
        source_span: super::SourceSpan { bytes: base.b, version, start: offset, end: offset, mask: 0 },
        name,
        icon,
        operation,
        version: stored_version,
        arguments: Vec::new(),
        visible: true,
        opacity: 1.0,
        mode: 0,
        blend_space: 0,
        composite_space: 0,
        composite_mode: 0,
        clip: false,
        region: 0,
        mask_offset: 0,
        record_offset: offset,
        raw: &[],
        unknown_properties: Vec::new(),
        errors: Vec::new(),
    };
    let mut count = 0usize;
    loop {
        check(ctl)?;
        work.charge(8)?;
        let property_offset = cursor.pos;
        let kind = cursor.u32()?;
        let length = cursor.u32()? as usize;
        if kind == 0 {
            if length != 0 {
                return Err(invalid("stored effect terminator has a nonzero length"));
            }
            break;
        }
        count = count.checked_add(1).ok_or_else(|| invalid("effect property count overflow"))?;
        if count > MAX_PROPERTIES {
            return Err(invalid("more than 4,096 properties on one effect"));
        }
        if length > MAX_PAYLOAD {
            return Err(invalid("stored effect property exceeds 64 MiB"));
        }
        work.charge(length)?;
        let bytes = cursor.take(length)?;
        match kind {
            45 => {
                let mut argument_cursor = Cursor { b: bytes, pos: 0, wide: base.wide };
                let argument_name = match string(&mut argument_cursor, work) {
                    Ok(name) => name,
                    Err(error @ (Error::Limit(_) | Error::Source(crate::Error::Cancelled))) => return Err(error),
                    Err(_) => {
                        // The bounded outer payload already locates the next property.
                        // Do not let a malformed optional name hide the mask or siblings.
                        push_error(&mut effect, "effect argument has a malformed name".into(), budget)?;
                        continue;
                    }
                };
                let value_kind = match argument_cursor.u32() {
                    Ok(kind) => kind,
                    Err(_) => {
                        // Outer property framing is intact, so a missing type
                        // cannot hide later arguments or the auxiliary mask.
                        push_error(&mut effect, "effect argument is missing its type word".into(), budget)?;
                        continue;
                    }
                };
                reserve(&mut effect.arguments, 1, budget).map_err(Error::from)?;
                charge(budget, argument_name.len())?;
                let value_bytes = bytes.get(argument_cursor.pos..).ok_or_else(|| invalid("effect argument framing"))?;
                let before_value = *budget;
                let value = match argument(&mut argument_cursor, value_kind, budget, work, ctl) {
                    Ok(value) => value,
                    Err(error @ (Error::Limit(_) | Error::Source(crate::Error::Cancelled))) => return Err(error),
                    Err(error) => {
                        // Parsed configuration discarded for malformed syntax or trailing
                        // payload bytes retains only its diagnostic, not its temporary tree.
                        *budget = before_value;
                        let message = error.to_string();
                        charge(budget, message.capacity())?;
                        ArgumentValue::Invalid { kind: value_kind, bytes: value_bytes, message }
                    }
                };
                effect.arguments.push(Argument { name: argument_name, value });
            }
            7 | 8 | 33 | 35 | 36 | 37 | 44 | 46 => {
                let value = match bytes {
                    [a, b, c, d] => u32::from_be_bytes([*a, *b, *c, *d]),
                    _ => {
                        let message = format!("effect property {kind} is not four bytes");
                        push_error(&mut effect, message, budget)?;
                        continue;
                    }
                };
                match kind {
                    7 => effect.mode = value as i32,
                    8 if value <= 1 => effect.visible = value != 0,
                    33 => {
                        let opacity = f32::from_bits(value);
                        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
                            push_error(&mut effect, "effect opacity is not finite normalized coverage".into(), budget)?;
                        } else {
                            effect.opacity = opacity;
                        }
                    }
                    35 => effect.composite_mode = value as i32,
                    36 => effect.composite_space = value as i32,
                    37 => effect.blend_space = value as i32,
                    44 => effect.region = value,
                    46 if value <= 1 => effect.clip = value != 0,
                    _ => {
                        push_error(&mut effect, format!("effect property {kind} has invalid boolean {value}"), budget)?;
                    }
                }
            }
            _ => {
                reserve(&mut effect.unknown_properties, 1, budget).map_err(Error::from)?;
                effect.unknown_properties.push(SourceProperty { offset: property_offset, kind, bytes });
            }
        }
    }
    let mask = pointer(&mut cursor)?;
    effect.mask_offset = mask;
    if mask != 0 && !usize::try_from(mask).ok().is_some_and(|offset| offset < base.b.len()) {
        push_error(&mut effect, "effect mask pointer is outside the file".into(), budget)?;
    }
    effect.raw = base.b.get(offset..cursor.pos).ok_or_else(|| invalid("stored effect record range"))?;
    effect.source_span.end = cursor.pos;
    effect.source_span.mask = mask;
    Ok(effect)
}

/// Preserve null versus explicitly empty source strings without relaxing
/// termination, UTF-8, work or size validation for non-null values.
fn optional_string<'a>(cursor: &mut Cursor<'a>, work: &mut DecodeWork) -> Result<Option<&'a str>> {
    let null = cursor.b.get(cursor.pos..).is_some_and(|bytes| bytes.starts_with(&[0; 4]));
    let value = string(cursor, work)?;
    if null { Ok(None) } else { Ok(Some(value)) }
}

/// Reads a bounded NUL-terminated UTF-8 effect string and rejects embedded NULs or truncated framing.
fn string<'a>(cursor: &mut Cursor<'a>, work: &mut DecodeWork) -> Result<&'a str> {
    work.charge(4)?;
    let count = cursor.u32()? as usize;
    if count == 0 {
        return Ok("");
    }
    if count > MAX_STRING {
        return Err(invalid("effect string exceeds 1 MiB"));
    }
    work.charge(count)?;
    let bytes = cursor.take(count)?;
    let bytes = bytes.strip_suffix(&[0]).ok_or_else(|| invalid("effect string is missing its NUL terminator"))?;
    if bytes.contains(&0) {
        return Err(invalid("effect string contains an embedded NUL"));
    }
    std::str::from_utf8(bytes).map_err(|_| invalid("effect string is not valid UTF-8"))
}

/// Decodes one exactly consumed typed payload, retaining unknown bytes and rejecting malformed finite/scalar/color framing.
fn argument<'a>(cursor: &mut Cursor<'a>, kind: u32, budget: &mut usize, work: &mut DecodeWork, ctl: &dyn Fn() -> bool) -> Result<ArgumentValue<'a>> {
    let value = match kind {
        0 => {
            let bytes = cursor.take(cursor.b.len().saturating_sub(cursor.pos))?;
            ArgumentValue::Unknown { kind, bytes }
        }
        1 => ArgumentValue::Int(cursor.u32()? as i32),
        2 => match cursor.u32()? {
            0 => ArgumentValue::Bool(false),
            1 => ArgumentValue::Bool(true),
            _ => return Err(invalid("effect argument boolean is not 0 or 1")),
        },
        3 => {
            let value = f32::from_bits(cursor.u32()?);
            if !value.is_finite() {
                return Err(invalid("effect argument float is not finite"));
            }
            ArgumentValue::Float(value)
        }
        4 => {
            let null = cursor.b.get(cursor.pos..) == Some(&[0; 4]);
            let text = string(cursor, work)?;
            if null { ArgumentValue::NullString } else { ArgumentValue::String(text) }
        }
        5 => ArgumentValue::Enum(cursor.u32()? as i32),
        6 => {
            let raw = string(cursor, work)?;
            if cursor.pos != cursor.b.len() {
                return Err(invalid("effect argument has unconsumed trailing bytes"));
            }
            // Configuration arguments require an actual serialized value, not
            // a null/empty string that merely parses as an empty syntax tree.
            if raw.is_empty() {
                return Err(invalid("effect configuration argument is null or empty"));
            }
            work.charge(raw.len())?;
            ArgumentValue::Config(parse_config(raw, budget, ctl).map_err(Error::from)?)
        }
        7 => ArgumentValue::UInt(cursor.u32()?),
        // Before 3.2 a null color has no bytes; later files use a zero word.
        8 if cursor.b.get(cursor.pos..).is_some_and(|bytes| bytes.is_empty() || bytes == [0; 4]) => {
            cursor.pos = cursor.b.len();
            ArgumentValue::Color(None)
        }
        8 => {
            let encoding = string(cursor, work)?;
            if encoding.is_empty() {
                return Err(invalid("effect color has an empty encoding"));
            } else {
                let count = cursor.u32()? as usize;
                if count > 256 {
                    return Err(invalid("effect color pixel exceeds 256 bytes"));
                }
                let pixels = cursor.take(count)?;
                let count = cursor.u32()? as usize;
                if count > MAX_PAYLOAD {
                    return Err(invalid("effect color ICC exceeds 64 MiB"));
                }
                let profile = cursor.take(count)?;
                ArgumentValue::Color(Some(ColorLiteral { encoding, pixels, profile }))
            }
        }
        _ => {
            let bytes = cursor.take(cursor.b.len().saturating_sub(cursor.pos))?;
            ArgumentValue::Unknown { kind, bytes }
        }
    };
    if cursor.pos != cursor.b.len() {
        return Err(invalid("effect argument has unconsumed trailing bytes"));
    }
    Ok(value)
}

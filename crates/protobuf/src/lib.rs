//! The Protocol Buffers wire format, read without a schema: a message is a run of fields, each a
//! tag (field number and wire type) and a value. Written from the public encoding description
//! (protobuf.dev, "Encoding"). The field numbers say nothing about what a field means: the
//! format that uses the messages gives them meaning.
//!
//! Reading is lenient, for bytes from untrusted files: a message stops at the first field that
//! can't be read, so damaged bytes give fewer fields, never an error or a panic. Nothing is
//! copied: values borrow the bytes they are read from. Groups (wire types 3 and 4), which the
//! format deprecates, end a message like damage.

/// The longest varint the format allows: ten bytes carry 64 bits.
const MAX_VARINT_BYTES: usize = 10;

/// Read a varint at `*pos`, moving `*pos` past it. `None` when the bytes end first or the value
/// takes more than 64 bits.
pub fn read_varint(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for i in 0..MAX_VARINT_BYTES {
        let b = *bytes.get(*pos)?;
        *pos += 1;
        let low = u64::from(b & 0x7F);
        // The tenth byte may only hold the 64th bit.
        if i == MAX_VARINT_BYTES - 1 && low > 1 {
            return None;
        }
        value |= low << (7 * i);
        if b & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

/// A field's value, by wire type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value<'a> {
    /// Wire type 0.
    Varint(u64),
    /// Wire type 1: eight little-endian bytes.
    Fixed64(u64),
    /// Wire type 2: bytes, text, an embedded message or packed values.
    Bytes(&'a [u8]),
    /// Wire type 5: four little-endian bytes.
    Fixed32(u32),
}

/// One field of a message.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field<'a> {
    pub number: u32,
    pub value: Value<'a>,
}

/// A message's bytes, read field by field on demand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Message<'a> {
    bytes: &'a [u8],
}

impl<'a> Message<'a> {
    pub const fn new(bytes: &'a [u8]) -> Self {
        Message { bytes }
    }

    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The fields in the order they are stored, up to the first that can't be read.
    pub fn fields(&self) -> Fields<'a> {
        Fields { bytes: self.bytes, pos: 0 }
    }

    /// The values of field `number`, in order.
    pub fn all(&self, number: u32) -> impl Iterator<Item = Value<'a>> + use<'a> {
        self.fields().filter(move |f| f.number == number).map(|f| f.value)
    }

    /// The last value of field `number`: for a field that appears more than once, the format's
    /// rule is that the last one wins.
    pub fn last(&self, number: u32) -> Option<Value<'a>> {
        self.all(number).last()
    }

    pub fn has(&self, number: u32) -> bool {
        self.all(number).next().is_some()
    }

    pub fn varint(&self, number: u32) -> Option<u64> {
        match self.last(number)? {
            Value::Varint(v) => Some(v),
            _ => None,
        }
    }

    /// A 32-bit float (wire type 5), or a 64-bit one (wire type 1) narrowed to 32 bits.
    pub fn float(&self, number: u32) -> Option<f32> {
        match self.last(number)? {
            Value::Fixed32(v) => Some(f32::from_bits(v)),
            Value::Fixed64(v) => Some(f64::from_bits(v) as f32),
            _ => None,
        }
    }

    /// A length-delimited field's bytes.
    pub fn data(&self, number: u32) -> Option<&'a [u8]> {
        match self.last(number)? {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// A length-delimited field as UTF-8 text; `None` when it isn't valid UTF-8.
    pub fn text(&self, number: u32) -> Option<&'a str> {
        std::str::from_utf8(self.data(number)?).ok()
    }

    /// A length-delimited field as an embedded message.
    pub fn message(&self, number: u32) -> Option<Message<'a>> {
        self.data(number).map(Message::new)
    }

    /// Every occurrence of a repeated message field, in order.
    pub fn messages(&self, number: u32) -> impl Iterator<Item = Message<'a>> + use<'a> {
        self.all(number).filter_map(|v| match v {
            Value::Bytes(b) => Some(Message::new(b)),
            _ => None,
        })
    }

    /// The varints of a packed repeated field (all occurrences, joined), or of unpacked varint
    /// occurrences.
    pub fn varints(&self, number: u32) -> Vec<u64> {
        let mut out = Vec::new();
        for v in self.all(number) {
            match v {
                Value::Varint(v) => out.push(v),
                Value::Bytes(b) => {
                    let mut pos = 0;
                    while pos < b.len() {
                        let Some(v) = read_varint(b, &mut pos) else { break };
                        out.push(v);
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// The fields of a [`Message`].
#[derive(Clone, Debug)]
pub struct Fields<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Fields<'a> {
    fn read(&mut self) -> Option<Field<'a>> {
        let tag = read_varint(self.bytes, &mut self.pos)?;
        let number = u32::try_from(tag >> 3).ok().filter(|n| *n != 0)?;
        let value = match tag & 7 {
            0 => Value::Varint(read_varint(self.bytes, &mut self.pos)?),
            1 => Value::Fixed64(u64::from_le_bytes(self.take(8)?.try_into().ok()?)),
            2 => {
                let len = usize::try_from(read_varint(self.bytes, &mut self.pos)?).ok()?;
                Value::Bytes(self.take(len)?)
            }
            5 => Value::Fixed32(u32::from_le_bytes(self.take(4)?.try_into().ok()?)),
            // Groups (3 and 4) are deprecated and never seen; 6 and 7 aren't wire types.
            _ => return None,
        };
        Some(Field { number, value })
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let out = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(out)
    }
}

impl<'a> Iterator for Fields<'a> {
    type Item = Field<'a>;

    fn next(&mut self) -> Option<Field<'a>> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let field = self.read();
        if field.is_none() {
            // Damaged: stop here for good.
            self.pos = self.bytes.len();
        }
        field
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_longer_than_64_bits_are_refused() {
        let max = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
        assert_eq!(read_varint(&max, &mut 0), Some(u64::MAX));
        let over = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x02];
        assert_eq!(read_varint(&over, &mut 0), None);
        let endless = [0x80; 11];
        assert_eq!(read_varint(&endless, &mut 0), None);
    }

    #[test]
    fn a_damaged_field_ends_the_message_and_keeps_the_fields_before_it() {
        // Field 1 = 5, then field 2 claims 9 bytes but has 1.
        let m = Message::new(&[0x08, 0x05, 0x12, 0x09, 0x41]);
        let fields: Vec<_> = m.fields().collect();
        assert_eq!(fields, vec![Field { number: 1, value: Value::Varint(5) }]);
        assert_eq!(m.data(2), None);
    }
}

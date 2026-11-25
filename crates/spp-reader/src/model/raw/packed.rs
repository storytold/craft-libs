//! [`PackedList`]: a list of small flat objects (only scalar fields) kept as their payload bytes instead of one
//! [`super::RawObject`] each. Brush strokes hold millions of such points; packed they cost a few bytes more
//! than the file itself. Field layouts are stored once and shared by the rows that use them.

use crate::decode::document::{Payload, ValueTag};

/// The class and the `(field name, tag)` sequence of a row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PackedLayout {
    pub class: String,
    pub fields: Vec<(String, ValueTag)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PackedList {
    layouts: Vec<PackedLayout>,
    /// Per row: layout index and start of its payloads in `data`.
    rows: Vec<(u32, u32)>,
    data: Vec<u8>,
    /// False if the walk met something a row cannot hold (a nested object or list, a bare value); the rows
    /// before it are kept, the rest of the list was skipped.
    pub complete: bool,
}

impl PackedList {
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn rows(&self) -> impl Iterator<Item = PackedRow<'_>> + '_ {
        self.rows.iter().enumerate().filter_map(|(i, &(layout, start))| {
            let end = self.rows.get(i + 1).map_or(self.data.len(), |&(_, s)| s as usize);
            Some(PackedRow { layout: self.layouts.get(layout as usize)?, data: self.data.get(start as usize..end)? })
        })
    }
}

/// One row: typed access to its fields by name.
#[derive(Debug, Clone, Copy)]
pub struct PackedRow<'a> {
    layout: &'a PackedLayout,
    data: &'a [u8],
}

impl<'a> PackedRow<'a> {
    pub fn class(&self) -> &'a str {
        &self.layout.class
    }

    /// The tag and payload of field `name` (strings without their length).
    pub fn field(&self, name: &str) -> Option<(ValueTag, &'a [u8])> {
        let mut at = 0usize;
        for (n, tag) in &self.layout.fields {
            let (start, len) = match tag.payload() {
                Payload::Fixed(len) => (at, len),
                Payload::String => {
                    let prefix = self.data.get(at..at + 4)?;
                    (at + 4, u32::from_le_bytes(prefix.try_into().ok()?) as usize)
                }
                Payload::List | Payload::Object => return None,
            };
            let payload = self.data.get(start..start + len)?;
            if n == name {
                return Some((*tag, payload));
            }
            at = start + len;
        }
        None
    }

    pub fn f32(&self, name: &str) -> Option<f32> {
        match self.field(name)? {
            (ValueTag::Float, p) => Some(f32::from_le_bytes(p.try_into().ok()?)),
            _ => None,
        }
    }

    /// An integer, enum or bool field as an unsigned number (little-endian, any fixed width up to 8 bytes).
    pub fn uint(&self, name: &str) -> Option<u64> {
        let (tag, p) = self.field(name)?;
        if !matches!(tag, ValueTag::Int | ValueTag::Enum | ValueTag::Bool | ValueTag::UInt | ValueTag::Uid | ValueTag::UidRef) || p.len() > 8 {
            return None;
        }
        let mut b = [0u8; 8];
        b.get_mut(..p.len())?.copy_from_slice(p);
        Some(u64::from_le_bytes(b))
    }

    /// The first `N` components of a vector or matrix field.
    pub fn floats<const N: usize>(&self, name: &str) -> Option<[f32; N]> {
        let (tag, p) = self.field(name)?;
        if !matches!(tag, ValueTag::Vec2 | ValueTag::Vec3 | ValueTag::Vec4 | ValueTag::Mat3 | ValueTag::Mat4) {
            return None;
        }
        let mut out = [0.0; N];
        for (i, v) in out.iter_mut().enumerate() {
            *v = f32::from_le_bytes(p.get(i * 4..i * 4 + 4)?.try_into().ok()?);
        }
        Some(out)
    }
}

/// Collects rows while the walk streams a list.
#[derive(Debug, Default)]
pub struct PackedBuilder {
    list: PackedList,
    /// The row being read: its class and fields so far.
    row: Option<PackedLayout>,
    row_start: usize,
    pending: Option<String>,
    /// Open containers being skipped (inside a row that cannot be packed, or after the list became incomplete).
    skipping: usize,
}

impl PackedBuilder {
    pub fn new() -> PackedBuilder {
        PackedBuilder { list: PackedList { complete: true, ..PackedList::default() }, ..PackedBuilder::default() }
    }

    /// Stops packing: the rest of the list is skipped.
    fn give_up(&mut self) {
        self.list.complete = false;
        if self.row.take().is_some() {
            self.list.data.truncate(self.row_start);
        }
    }

    pub fn object(&mut self, class: &str) {
        if self.skipping > 0 || self.row.is_some() || !self.list.complete {
            if self.row.is_some() {
                self.give_up();
            }
            self.skipping += 1;
            return;
        }
        self.row_start = self.list.data.len();
        self.row = Some(PackedLayout { class: class.to_string(), fields: Vec::new() });
    }

    pub fn end_object(&mut self) {
        if self.skipping > 0 {
            self.skipping -= 1;
            return;
        }
        let Some(row) = self.row.take() else { return };
        let index = match self.list.layouts.iter().rposition(|l| *l == row) {
            Some(i) => i,
            None => {
                self.list.layouts.push(row);
                self.list.layouts.len() - 1
            }
        };
        match (u32::try_from(index), u32::try_from(self.row_start)) {
            (Ok(layout), Ok(start)) => self.list.rows.push((layout, start)),
            _ => self.give_up(),
        }
    }

    pub fn field(&mut self, name: &[u8]) {
        if self.skipping == 0 {
            self.pending = Some(String::from_utf8_lossy(name).into_owned());
        }
    }

    pub fn scalar(&mut self, tag: ValueTag, payload: &[u8]) {
        if self.skipping > 0 || !self.list.complete {
            return;
        }
        let Some(row) = self.row.as_mut() else {
            // A bare value in the list: not a list of objects.
            self.give_up();
            return;
        };
        row.fields.push((self.pending.take().unwrap_or_default(), tag));
        if let Payload::String = tag.payload() {
            self.list.data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        }
        self.list.data.extend_from_slice(payload);
    }

    /// A nested list (inside a row or in place of one): rows cannot hold it.
    pub fn list(&mut self) {
        if self.skipping == 0 {
            self.give_up();
        }
        self.skipping += 1;
    }

    /// Returns true when this ends the packed list itself.
    pub fn end_list(&mut self) -> bool {
        if self.skipping > 0 {
            self.skipping -= 1;
            return false;
        }
        true
    }

    pub fn finish(self) -> PackedList {
        self.list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(b: &mut PackedBuilder, class: &str, fields: &[(&str, ValueTag, &[u8])]) {
        b.object(class);
        for (name, tag, payload) in fields {
            b.field(name.as_bytes());
            b.scalar(*tag, payload);
        }
        b.end_object();
    }

    #[test]
    fn rows_share_layouts_and_read_back() {
        let mut b = PackedBuilder::new();
        let pos = [1.5f32.to_le_bytes(), (-2.0f32).to_le_bytes()].concat();
        for t in [0.0f32, 0.25] {
            row(&mut b, "Key", &[("position", ValueTag::Vec2, &pos), ("time", ValueTag::Float, &t.to_le_bytes())]);
        }
        row(&mut b, "Key", &[("label", ValueTag::String, b"ab"), ("time", ValueTag::Float, &1f32.to_le_bytes())]);
        assert!(b.end_list());
        let list = b.finish();
        assert!(list.complete);
        assert_eq!((list.len(), list.layouts.len()), (3, 2));
        let rows: Vec<_> = list.rows().collect();
        assert_eq!(rows[1].floats::<2>("position"), Some([1.5, -2.0]));
        assert_eq!(rows[1].f32("time"), Some(0.25));
        assert_eq!(rows[2].f32("time"), Some(1.0));
        assert_eq!(rows[2].field("label"), Some((ValueTag::String, &b"ab"[..])));
        assert_eq!(rows[0].f32("position"), None, "wrong tag");
        assert_eq!(rows[0].floats::<3>("position"), None, "too few components");
        assert_eq!(rows[0].class(), "Key");
    }

    #[test]
    fn nested_containers_stop_packing() {
        let mut b = PackedBuilder::new();
        row(&mut b, "Key", &[("time", ValueTag::Float, &0f32.to_le_bytes())]);
        b.object("Key");
        b.field(b"time");
        b.scalar(ValueTag::Float, &1f32.to_le_bytes());
        b.field(b"inner");
        b.object("Inner");
        b.field(b"x");
        b.scalar(ValueTag::Float, &1f32.to_le_bytes());
        b.end_object();
        b.end_object();
        row(&mut b, "Key", &[("time", ValueTag::Float, &2f32.to_le_bytes())]);
        b.list();
        assert!(!b.end_list());
        assert!(b.end_list());
        let list = b.finish();
        assert!(!list.complete);
        assert_eq!(list.rows().filter_map(|r| r.f32("time")).collect::<Vec<_>>(), [0.0]);
    }
}

//! Test-only QDataStream encoder, so tests build synthetic records instead of shipping bytes from real files.

use super::variant::type_id;

#[derive(Default)]
pub(crate) struct QdsWriter {
    out: Vec<u8>,
}

impl QdsWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn finish(self) -> Vec<u8> {
        self.out
    }

    /// Plain big-endian u32 (a count or length, not a QVariant).
    pub fn raw_u32(&mut self, v: u32) -> &mut Self {
        self.out.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn raw_bytes(&mut self, b: &[u8]) -> &mut Self {
        self.out.extend_from_slice(b);
        self
    }

    /// Plain `QString` (no QVariant header).
    pub fn raw_string(&mut self, s: &str) -> &mut Self {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.raw_u32(units.len() as u32 * 2);
        for u in units {
            self.out.extend_from_slice(&u.to_be_bytes());
        }
        self
    }

    fn header(&mut self, id: u32) -> &mut Self {
        self.raw_u32(id);
        self.out.push(0);
        self
    }

    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.header(type_id::BOOL).raw_bytes(&[v as u8])
    }

    pub fn int(&mut self, v: i32) -> &mut Self {
        self.header(type_id::INT).raw_bytes(&v.to_be_bytes())
    }

    pub fn uint(&mut self, v: u32) -> &mut Self {
        self.header(type_id::UINT).raw_bytes(&v.to_be_bytes())
    }

    pub fn long_long(&mut self, v: i64) -> &mut Self {
        self.header(type_id::LONG_LONG).raw_bytes(&v.to_be_bytes())
    }

    pub fn ulong_long(&mut self, v: u64) -> &mut Self {
        self.header(type_id::ULONG_LONG).raw_bytes(&v.to_be_bytes())
    }

    pub fn double(&mut self, v: f64) -> &mut Self {
        self.header(type_id::DOUBLE).raw_bytes(&v.to_be_bytes())
    }

    pub fn float(&mut self, v: f32) -> &mut Self {
        self.header(type_id::FLOAT).raw_bytes(&f64::from(v).to_be_bytes())
    }

    pub fn vector3d(&mut self, v: [f32; 3]) -> &mut Self {
        self.header(type_id::VECTOR3D);
        for c in v {
            self.raw_bytes(&f64::from(c).to_be_bytes());
        }
        self
    }

    pub fn string(&mut self, s: &str) -> &mut Self {
        self.header(type_id::STRING).raw_string(s)
    }

    pub fn null_string(&mut self) -> &mut Self {
        self.header(type_id::STRING).raw_u32(u32::MAX)
    }

    pub fn string_list(&mut self, items: &[&str]) -> &mut Self {
        self.header(type_id::STRING_LIST).raw_u32(items.len() as u32);
        for s in items {
            self.raw_string(s);
        }
        self
    }

    pub fn byte_array(&mut self, b: &[u8]) -> &mut Self {
        self.header(type_id::BYTE_ARRAY).raw_u32(b.len() as u32).raw_bytes(b)
    }

    pub fn url(&mut self, s: &str) -> &mut Self {
        self.header(type_id::URL).raw_u32(s.len() as u32).raw_bytes(s.as_bytes())
    }

    /// A QVariantMap variant; values are already-encoded QVariants.
    pub fn map(&mut self, entries: &[(&str, Vec<u8>)]) -> &mut Self {
        self.header(type_id::MAP);
        self.raw_map(entries)
    }

    /// A bare QVariantMap (no QVariant header), as at the start of the reference app's records.
    pub fn raw_map(&mut self, entries: &[(&str, Vec<u8>)]) -> &mut Self {
        self.raw_u32(entries.len() as u32);
        for (k, v) in entries {
            self.raw_string(k).raw_bytes(v);
        }
        self
    }

    /// One encoded QVariantList variant.
    pub fn encode_list(items: &[Vec<u8>]) -> Vec<u8> {
        let mut w = QdsWriter::new();
        w.header(type_id::LIST).raw_u32(items.len() as u32);
        for item in items {
            w.raw_bytes(item);
        }
        w.finish()
    }

    /// One encoded variant, built by `f`.
    pub fn encode(f: impl FnOnce(&mut QdsWriter) -> &mut QdsWriter) -> Vec<u8> {
        let mut w = QdsWriter::new();
        f(&mut w);
        w.finish()
    }
}

//! Decoded QVariant values.

/// Qt 5 `QMetaType` ids this reader understands.
pub(super) mod type_id {
    pub const BOOL: u32 = 1;
    pub const INT: u32 = 2;
    pub const UINT: u32 = 3;
    pub const LONG_LONG: u32 = 4;
    pub const ULONG_LONG: u32 = 5;
    pub const DOUBLE: u32 = 6;
    pub const MAP: u32 = 8;
    pub const LIST: u32 = 9;
    pub const STRING: u32 = 10;
    pub const STRING_LIST: u32 = 11;
    pub const BYTE_ARRAY: u32 = 12;
    pub const URL: u32 = 17;
    pub const HASH: u32 = 28;
    pub const FLOAT: u32 = 38;
    pub const VECTOR3D: u32 = 83;
}

#[derive(Debug, Clone, PartialEq)]
pub enum QVariant {
    Bool(bool),
    Int(i32),
    UInt(u32),
    LongLong(i64),
    ULongLong(u64),
    Double(f64),
    /// `None` is Qt's null string.
    String(Option<String>),
    StringList(Vec<String>),
    ByteArray(Vec<u8>),
    List(Vec<QVariant>),
    /// A `QVariantMap`, or a `QVariantHash` (same wire layout).
    Map(QVariantMap),
    /// A `QUrl` in its encoded form, e.g. `resource://<shelf>/...` (seen in `/editor/exporter.ini`).
    Url(String),
    /// Stored as a double, like every float Qt writes at its default (double) precision.
    Float(f32),
    /// A `QVector3D`: three floats, each stored as a double (Observed in the mesh's `.alg_meta` record).
    Vector3D([f32; 3]),
}

impl QVariant {
    /// Any integer variant as i64.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            QVariant::Int(v) => Some(v.into()),
            QVariant::UInt(v) => Some(v.into()),
            QVariant::LongLong(v) => Some(v),
            QVariant::ULongLong(v) => i64::try_from(v).ok(),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            QVariant::String(Some(s)) | QVariant::Url(s) => Some(s),
            _ => None,
        }
    }
}

/// Key/value pairs in file order (the reference app's maps are not sorted).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QVariantMap {
    entries: Vec<(String, QVariant)>,
}

impl QVariantMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a pair, keeping file order. Duplicate keys are kept; [`get`](Self::get) returns the first.
    pub fn push(&mut self, key: impl Into<String>, value: QVariant) {
        self.entries.push((key.into(), value));
    }

    pub fn get(&self, key: &str) -> Option<&QVariant> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &QVariant)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<K: Into<String>> FromIterator<(K, QVariant)> for QVariantMap {
    fn from_iter<I: IntoIterator<Item = (K, QVariant)>>(iter: I) -> Self {
        QVariantMap { entries: iter.into_iter().map(|(k, v)| (k.into(), v)).collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_variants_widen_to_i64() {
        assert_eq!(QVariant::Int(-3).as_i64(), Some(-3));
        assert_eq!(QVariant::UInt(u32::MAX).as_i64(), Some(u32::MAX as i64));
        assert_eq!(QVariant::ULongLong(u64::MAX).as_i64(), None);
        assert_eq!(QVariant::Double(1.0).as_i64(), None);
    }

    #[test]
    fn map_keeps_order_and_finds_first_key() {
        let map: QVariantMap = [("b", QVariant::Int(1)), ("a", QVariant::Int(2)), ("b", QVariant::Int(3))].into_iter().collect();
        assert_eq!(map.iter().map(|(k, _)| k).collect::<Vec<_>>(), ["b", "a", "b"]);
        assert_eq!(map.get("b"), Some(&QVariant::Int(1)));
        assert_eq!(map.get("c"), None);
        assert_eq!(QVariant::String(None).as_str(), None);
    }
}

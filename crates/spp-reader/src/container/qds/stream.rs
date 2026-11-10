//! Sequential reader over QDataStream-encoded bytes.

use super::QdsError;
use super::variant::{QVariant, QVariantMap, type_id};
use crate::container::bytes::ByteReader;

/// Marks a null `QString` / `QByteArray` instead of a length.
const NULL_LENGTH: u32 = 0xFFFF_FFFF;
/// Deepest nesting of maps and lists accepted; the reference app's records go two or three levels deep.
const MAX_DEPTH: usize = 64;

/// Sequential reader over QDataStream-encoded bytes.
pub struct QDataStream<'a> {
    reader: ByteReader<'a>,
    depth: usize,
}

impl<'a> QDataStream<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        QDataStream { reader: ByteReader::new(data), depth: 0 }
    }

    pub fn position(&self) -> usize {
        self.reader.pos()
    }

    pub fn is_at_end(&self) -> bool {
        self.reader.is_at_end()
    }

    pub fn read_u32(&mut self) -> Result<u32, QdsError> {
        Ok(self.reader.u32_be()?)
    }

    /// A `QString`: u32 byte length (0xFFFFFFFF = null) followed by UTF-16BE.
    pub fn read_string(&mut self) -> Result<Option<String>, QdsError> {
        let at = self.reader.pos();
        let len = self.reader.u32_be()?;
        if len == NULL_LENGTH {
            return Ok(None);
        }
        if len % 2 != 0 {
            return Err(QdsError::InvalidUtf16 { at });
        }
        let bytes = self.reader.bytes(len as usize)?;
        // `len` is even, so `as_chunks` leaves no remainder.
        let units = bytes.as_chunks::<2>().0.iter().map(|&pair| u16::from_be_bytes(pair));
        char::decode_utf16(units).collect::<Result<String, _>>().map(Some).map_err(|_| QdsError::InvalidUtf16 { at })
    }

    /// A `QByteArray`: u32 length (0xFFFFFFFF = null, returned as empty) followed by the bytes.
    pub fn read_byte_array(&mut self) -> Result<&'a [u8], QdsError> {
        let len = self.reader.u32_be()?;
        if len == NULL_LENGTH {
            return Ok(&[]);
        }
        Ok(self.reader.bytes(len as usize)?)
    }

    /// A `QStringList`: u32 count, then QStrings (null entries become empty strings).
    pub fn read_string_list(&mut self) -> Result<Vec<String>, QdsError> {
        let count = self.reader.u32_be()?;
        // No pre-allocation: `count` is untrusted, and each item consumes input, so the loop is bounded.
        let mut out = Vec::new();
        for _ in 0..count {
            out.push(self.read_string()?.unwrap_or_default());
        }
        Ok(out)
    }

    /// A `QVariant`: u32 type id, u8 is-null flag, then the value in that type's encoding.
    pub fn read_variant(&mut self) -> Result<QVariant, QdsError> {
        let at = self.reader.pos();
        let id = self.reader.u32_be()?;
        let _is_null = self.reader.u8()?;
        let r = &mut self.reader;
        Ok(match id {
            type_id::BOOL => QVariant::Bool(r.u8()? != 0),
            type_id::INT => QVariant::Int(r.i32_be()?),
            type_id::UINT => QVariant::UInt(r.u32_be()?),
            type_id::LONG_LONG => QVariant::LongLong(r.i64_be()?),
            type_id::ULONG_LONG => QVariant::ULongLong(r.u64_be()?),
            type_id::DOUBLE => QVariant::Double(r.f64_be()?),
            // Qt writes `float` at the stream's precision, double by default (Observed: 8 bytes in the reference app's records).
            type_id::FLOAT => QVariant::Float(r.f64_be()? as f32),
            type_id::VECTOR3D => QVariant::Vector3D([r.f64_be()? as f32, r.f64_be()? as f32, r.f64_be()? as f32]),
            type_id::STRING => QVariant::String(self.read_string()?),
            type_id::STRING_LIST => QVariant::StringList(self.read_string_list()?),
            type_id::BYTE_ARRAY => QVariant::ByteArray(self.read_byte_array()?.to_vec()),
            type_id::URL => QVariant::Url(String::from_utf8_lossy(self.read_byte_array()?).into_owned()),
            type_id::MAP | type_id::HASH => QVariant::Map(self.read_variant_map()?),
            type_id::LIST => QVariant::List(self.read_variant_list()?),
            type_id => return Err(QdsError::UnsupportedType { type_id, at }),
        })
    }

    /// A `QVariantMap`/`QVariantHash`: u32 count, then (QString key, QVariant value) pairs.
    pub fn read_variant_map(&mut self) -> Result<QVariantMap, QdsError> {
        self.nested(|s| {
            let count = s.reader.u32_be()?;
            let mut map = QVariantMap::new();
            for _ in 0..count {
                let key = s.read_string()?.unwrap_or_default();
                let value = s.read_variant()?;
                map.push(key, value);
            }
            Ok(map)
        })
    }

    /// A `QVariantList`: u32 count, then QVariants.
    pub fn read_variant_list(&mut self) -> Result<Vec<QVariant>, QdsError> {
        self.nested(|s| {
            let count = s.reader.u32_be()?;
            let mut list = Vec::new();
            for _ in 0..count {
                list.push(s.read_variant()?);
            }
            Ok(list)
        })
    }

    /// Runs `read` one container level deeper, refusing to go past [`MAX_DEPTH`].
    fn nested<T>(&mut self, read: impl FnOnce(&mut Self) -> Result<T, QdsError>) -> Result<T, QdsError> {
        if self.depth >= MAX_DEPTH {
            return Err(QdsError::TooDeep { at: self.reader.pos() });
        }
        self.depth += 1;
        let result = read(self);
        self.depth -= 1;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_encoder::QdsWriter;
    use super::*;

    fn read(bytes: &[u8]) -> Result<QVariant, QdsError> {
        QDataStream::new(bytes).read_variant()
    }

    #[test]
    fn reads_every_supported_scalar() {
        let mut w = QdsWriter::new();
        w.bool(true).int(-5).uint(7).long_long(-9).ulong_long(11).double(2.5).float(0.5).vector3d([1.0, -2.0, 0.25]);
        let bytes = w.finish();
        let mut s = QDataStream::new(&bytes);
        let expected = [
            QVariant::Bool(true),
            QVariant::Int(-5),
            QVariant::UInt(7),
            QVariant::LongLong(-9),
            QVariant::ULongLong(11),
            QVariant::Double(2.5),
            QVariant::Float(0.5),
            QVariant::Vector3D([1.0, -2.0, 0.25]),
        ];
        for value in expected {
            assert_eq!(s.read_variant(), Ok(value));
        }
        assert!(s.is_at_end());
    }

    #[test]
    fn reads_strings_including_null_and_non_bmp() {
        let mut w = QdsWriter::new();
        w.string("Vase 🏺").null_string().string_list(&["a", "b"]).byte_array(b"{}").url("resource://x");
        let bytes = w.finish();
        let mut s = QDataStream::new(&bytes);
        assert_eq!(s.read_variant(), Ok(QVariant::String(Some("Vase 🏺".into()))));
        assert_eq!(s.read_variant(), Ok(QVariant::String(None)));
        assert_eq!(s.read_variant(), Ok(QVariant::StringList(vec!["a".into(), "b".into()])));
        assert_eq!(s.read_variant(), Ok(QVariant::ByteArray(b"{}".to_vec())));
        assert_eq!(s.read_variant().unwrap().as_str(), Some("resource://x"));
    }

    #[test]
    fn reads_nested_maps_and_lists_in_file_order() {
        let mut inner = QdsWriter::new();
        inner.int(1);
        let mut w = QdsWriter::new();
        w.map(&[("z", inner.finish()), ("a", QdsWriter::encode_list(&[]))]);
        let map = match read(&w.finish()) {
            Ok(QVariant::Map(m)) => m,
            other => panic!("expected map, got {other:?}"),
        };
        assert_eq!(map.iter().map(|(k, _)| k).collect::<Vec<_>>(), ["z", "a"]);
        assert_eq!(map.get("z"), Some(&QVariant::Int(1)));
        assert_eq!(map.get("a"), Some(&QVariant::List(vec![])));
    }

    #[test]
    fn unknown_type_reports_id_and_offset() {
        let bytes = [0, 0, 0, 0, 0, 0, 0, 99, 0];
        let mut s = QDataStream::new(&bytes);
        s.read_u32().unwrap();
        assert_eq!(s.read_variant(), Err(QdsError::UnsupportedType { type_id: 99, at: 4 }));
    }

    #[test]
    fn corrupt_input_is_an_error_not_a_panic() {
        // Odd byte length, unpaired surrogate, huge counts, truncation.
        assert!(matches!(read(&[0, 0, 0, 10, 0, 0, 0, 0, 3, 0, 0]), Err(QdsError::InvalidUtf16 { .. })));
        assert!(matches!(read(&[0, 0, 0, 10, 0, 0, 0, 0, 2, 0xD8, 0]), Err(QdsError::InvalidUtf16 { .. })));
        assert!(matches!(read(&[0, 0, 0, 9, 0, 0xFF, 0xFF, 0xFF, 0xF0]), Err(QdsError::Truncated(_))));
        assert!(matches!(read(&[0, 0, 0, 10, 0, 0x7F, 0xFF, 0xFF, 0xFE]), Err(QdsError::Truncated(_))));
        assert!(matches!(read(&[0, 0, 0, 2]), Err(QdsError::Truncated(_))));
    }

    #[test]
    fn deep_nesting_is_refused() {
        let mut bytes = Vec::new();
        for _ in 0..10_000 {
            bytes.extend_from_slice(&[0, 0, 0, 9, 0, 0, 0, 0, 1]);
        }
        assert!(matches!(read(&bytes), Err(QdsError::TooDeep { .. })));
    }
}

//! The versioned JSON envelope used by AniCraft's native project file.
//!
//! The app owns its document model and the path-table scope used while serializing it. This
//! crate owns the format envelope: field order, duplicate-field handling, and the point at which
//! the caller decodes the document relative to the `paths` field.
//!
//! [`read`] passes the actual nested Serde deserializer to the caller's [`PayloadDecoder`]. That
//! keeps payload parsing in the app's model crate while preserving the original stream order,
//! global JSON error locations, and path-table guard lifetime.

use serde::{
    Serialize,
    de::{Deserialize, DeserializeSeed, MapAccess, Visitor},
};
use std::{fmt, marker::PhantomData};

/// The native project envelope's identifying format string.
pub const FORMAT_NAME: &str = "anicraft";
/// Current envelope version (v1 inline paths; v2 shared path table; v3 named scene timelines).
pub const FORMAT_VERSION: u32 = 3;
/// The largest native project file AniCraft accepts.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
/// Files below this size are pretty-printed for diffs and manual inspection.
pub const PRETTY_LIMIT: usize = 1024 * 1024;

#[derive(Serialize)]
struct EnvelopeOut<'a, P, T> {
    format: &'a str,
    version: u32,
    generator: &'a str,
    paths: &'a [P],
    document: &'a T,
}

/// Serializes the envelope in its stable wire order and uses AniCraft's compact/pretty threshold.
pub fn write<P: Serialize, T: Serialize>(generator: &str, paths: &[P], document: &T) -> Result<String, serde_json::Error> {
    let envelope = EnvelopeOut { format: FORMAT_NAME, version: FORMAT_VERSION, generator, paths, document };
    let compact = serde_json::to_string(&envelope)?;
    if compact.len() < PRETTY_LIMIT { serde_json::to_string_pretty(&envelope) } else { Ok(compact) }
}

/// Decodes one native-document payload under the path table that preceded it in the envelope.
pub trait PayloadDecoder<P> {
    type Document;

    fn decode<'de, D>(&mut self, deserializer: D, paths: Option<Vec<P>>) -> Result<Self::Document, D::Error>
    where
        D: serde::Deserializer<'de>;
}

/// The parsed top-level format, version and document payload.
#[derive(Debug, PartialEq)]
pub struct Envelope<T> {
    pub format: String,
    pub version: u32,
    /// Missing is distinct from JSON `null`, which the payload decoder decides whether to accept.
    pub document: Option<T>,
}

struct DocumentSeed<'a, P, D> {
    decoder: &'a mut D,
    paths: Option<Vec<P>>,
}

impl<'de, P, D> DeserializeSeed<'de> for DocumentSeed<'_, P, D>
where
    D: PayloadDecoder<P>,
{
    type Value = D::Document;

    fn deserialize<De>(self, deserializer: De) -> Result<Self::Value, De::Error>
    where
        De: serde::Deserializer<'de>,
    {
        self.decoder.decode(deserializer, self.paths)
    }
}

struct EnvelopeSeed<'a, P, D> {
    decoder: &'a mut D,
    marker: PhantomData<fn() -> P>,
}

impl<'de, P, D> DeserializeSeed<'de> for EnvelopeSeed<'_, P, D>
where
    P: Deserialize<'de> + Clone,
    D: PayloadDecoder<P>,
{
    type Value = Envelope<D::Document>;

    fn deserialize<De>(self, deserializer: De) -> Result<Self::Value, De::Error>
    where
        De: serde::Deserializer<'de>,
    {
        struct EnvelopeVisitor<'a, P, D> {
            decoder: &'a mut D,
            marker: PhantomData<fn() -> P>,
        }

        impl<'de, P, D> Visitor<'de> for EnvelopeVisitor<'_, P, D>
        where
            P: Deserialize<'de> + Clone,
            D: PayloadDecoder<P>,
        {
            type Value = Envelope<D::Document>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an .anicraft envelope object")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let (mut format, mut version, mut paths, mut document) = (None, None, None, None);
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "format" => format = Some(map.next_value::<String>()?),
                        "version" => version = Some(map.next_value::<u32>()?),
                        "paths" => paths = Some(map.next_value::<Vec<P>>()?),
                        "document" => {
                            document = Some(map.next_value_seed(DocumentSeed { decoder: self.decoder, paths: paths.clone() })?);
                        }
                        _ => {
                            map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(Envelope {
                    format: format.ok_or_else(|| serde::de::Error::missing_field("format"))?,
                    version: version.ok_or_else(|| serde::de::Error::missing_field("version"))?,
                    document,
                })
            }
        }

        deserializer.deserialize_map(EnvelopeVisitor { decoder: self.decoder, marker: PhantomData })
    }
}

/// Reads the envelope while delegating its typed document payload to `decoder`.
///
/// Unknown top-level fields are ignored. Repeated recognized keys use the last successful value,
/// and each `document` is decoded immediately with the latest preceding `paths` field. A
/// malformed earlier `document` therefore errors before a later duplicate is considered, just as
/// it does with a streaming Serde visitor.
pub fn read<'de, P, D>(json: &'de str, decoder: &mut D) -> Result<Envelope<D::Document>, serde_json::Error>
where
    P: Deserialize<'de> + Clone,
    D: PayloadDecoder<P>,
{
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let envelope = EnvelopeSeed { decoder, marker: PhantomData }.deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(envelope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::Value;

    #[derive(Debug, Deserialize, Serialize, PartialEq)]
    struct Document {
        value: u32,
    }

    #[derive(Default)]
    struct CaptureDocument {
        calls: usize,
    }

    impl PayloadDecoder<String> for CaptureDocument {
        type Document = (Document, Option<Vec<String>>);

        fn decode<'de, D>(&mut self, deserializer: D, paths: Option<Vec<String>>) -> Result<Self::Document, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            self.calls += 1;
            Ok((Document::deserialize(deserializer)?, paths))
        }
    }

    struct ValueDocument;

    impl PayloadDecoder<String> for ValueDocument {
        type Document = (Value, Option<Vec<String>>);

        fn decode<'de, D>(&mut self, deserializer: D, paths: Option<Vec<String>>) -> Result<Self::Document, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            Ok((Value::deserialize(deserializer)?, paths))
        }
    }

    #[test]
    fn writer_keeps_the_versioned_field_order_and_pretty_threshold() -> Result<(), Box<dyn std::error::Error>> {
        let paths = ["path-a", "path-b"];
        let document = Document { value: 3 };
        let json = write("AniCraft 0.1", &paths, &document)?;
        let keys = ["\"format\"", "\"version\"", "\"generator\"", "\"paths\"", "\"document\""];
        let offsets: Vec<usize> = keys.iter().map(|key| json.find(key).unwrap_or(usize::MAX)).collect();
        assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(json.contains("\n  \"format\""), "small envelopes stay human-readable");
        assert!(json.contains("\"format\": \"anicraft\""));
        assert!(json.contains("\"version\": 3"));
        Ok(())
    }

    #[test]
    fn repeated_fields_and_paths_before_document_keep_streaming_semantics() -> Result<(), Box<dyn std::error::Error>> {
        let json = r#"{"format":"old","format":"anicraft","version":1,"paths":["first"],"paths":["chosen"],"document":{"value":7},"ignored":true}"#;
        let mut decoder = CaptureDocument::default();
        let envelope = read::<String, _>(json, &mut decoder)?;
        assert_eq!(envelope.format, "anicraft");
        assert_eq!(envelope.version, 1);
        assert_eq!(envelope.document, Some((Document { value: 7 }, Some(vec!["chosen".to_string()]))));
        assert_eq!(decoder.calls, 1);
        Ok(())
    }

    #[test]
    fn paths_after_document_do_not_retroactively_change_its_scope() -> Result<(), Box<dyn std::error::Error>> {
        let json = r#"{"format":"anicraft","version":1,"document":{"value":1},"paths":["later"]}"#;
        let mut decoder = ValueDocument;
        let envelope = read::<String, _>(json, &mut decoder)?;
        assert_eq!(envelope.document, Some((serde_json::json!({"value":1}), None)));
        Ok(())
    }

    #[test]
    fn malformed_earlier_duplicate_document_stops_before_later_document() {
        let json = r#"{"format":"anicraft","version":1,"document":null,"document":{"value":2}}"#;
        let mut decoder = CaptureDocument::default();
        let result = read::<String, _>(json, &mut decoder);
        assert!(result.is_err());
        assert_eq!(decoder.calls, 1);
    }

    #[test]
    fn malformed_top_level_keeps_the_native_envelope_diagnostic() {
        let mut decoder = CaptureDocument::default();
        let error = read::<String, _>("[]", &mut decoder).unwrap_err();
        assert!(error.to_string().contains("expected an .anicraft envelope object"));
        assert_eq!(decoder.calls, 0);
    }

    #[test]
    fn duplicate_documents_use_the_last_successful_value() -> Result<(), Box<dyn std::error::Error>> {
        let json = r#"{"format":"anicraft","version":1,"document":{"value":1},"document":{"value":2}}"#;
        let mut decoder = CaptureDocument::default();
        let envelope = read::<String, _>(json, &mut decoder)?;
        assert_eq!(envelope.document, Some((Document { value: 2 }, None)));
        assert_eq!(decoder.calls, 2);
        Ok(())
    }
}

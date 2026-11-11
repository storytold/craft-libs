//! Versioned and lenient serde helpers.

use super::*;

use std::fmt;

use serde::Deserialize;
use serde::de::{self, DeserializeOwned, Deserializer, MapAccess, SeqAccess, Visitor};

/// The top-level member that holds a document's version number: VectorCraft's `version`,
/// PhotoCraft's `format_version` or EffectCraft's `schema`.
fn version_member(kind: Kind) -> &'static str {
    match kind {
        Kind::PhotoCraft => "format_version",
        Kind::VectorCraft => "version",
        Kind::EffectCraft => "schema",
    }
}

/// The version number in the member a [`Visitor`] is given, read from the top-level JSON object;
/// the other members are skipped, and a repeated version member is refused.
struct VersionIn(&'static str);

impl<'de> Visitor<'de> for VersionIn {
    type Value = Option<u32>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Option<u32>, A::Error> {
        let mut version = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == self.0 {
                if version.is_some() {
                    return Err(de::Error::duplicate_field(self.0));
                }
                version = Some(map.next_value()?);
            } else {
                map.next_value::<de::IgnoredAny>()?;
            }
        }
        Ok(version)
    }
}

/// Parse `T` from `json`; when that fails, report [`Error::TooNew`] instead if the file's version
/// (its top-level member that [`version_member`] names) is newer than `kind` supports, since a
/// newer format may change what `T` expects.
pub(crate) fn parse_versioned<T: DeserializeOwned>(json: &[u8], budget: &Budget, kind: Kind) -> Result<T, Error> {
    fn version<'de, R: serde_json::de::Read<'de>>(mut de: serde_json::Deserializer<R>, kind: Kind) -> Result<Option<u32>, serde_json::Error> {
        de.disable_recursion_limit();
        let version = (&mut de).deserialize_map(VersionIn(version_member(kind)))?;
        de.end()?;
        Ok(version)
    }
    check_depth(json, budget.limits.max_depth, budget.control)?;
    counting(budget.limits.max_items, || {
        parse_checked(json, budget).map_err(|e| {
            if let Err(cancelled) = budget.check_cancelled() {
                return cancelled;
            }
            let version = match budget.control {
                Some(control) => version(serde_json::Deserializer::from_reader(cancel::Reader::new(json, Some(control))), kind),
                None => version(serde_json::Deserializer::from_slice(json), kind),
            };
            match version {
                Ok(Some(version)) if version > kind.supported_version() => too_new(kind, version),
                _ => malformed(e),
            }
        })
    })
}

/// A member read as `None` when its value has the wrong shape, as the format crates read a
/// file's optional members. The value is read as it is parsed: the elements of an array or
/// object of the wrong shape are skipped one at a time with [`de::IgnoredAny`], and none of the
/// value is kept.
pub(crate) struct Lenient<T>(pub(crate) Option<T>);

/// A type [`Lenient`] reads.
pub(crate) trait Shaped: Sized {
    /// The value, or `None` when it has the wrong shape.
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Self>, D::Error>;
}

impl<'de, T: Shaped> Deserialize<'de> for Lenient<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        T::read(d).map(Lenient)
    }
}

impl Shaped for String {
    fn read<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        Ok(Scalar::deserialize(d)?.string())
    }
}

/// [`Lenient`] as a `deserialize_with` function.
pub(crate) fn lenient<'de, D: Deserializer<'de>, T: Shaped>(d: D) -> Result<Option<T>, D::Error> {
    Lenient::deserialize(d).map(|l| l.0)
}

/// One JSON value: a string, a non-negative integer or a boolean is kept, and an array or object
/// is skipped one element at a time with [`de::IgnoredAny`].
pub(crate) enum Scalar {
    Str(String),
    Unsigned(u64),
    Bool(bool),
    Null,
    Other,
}

impl Scalar {
    pub(crate) fn string(self) -> Option<String> {
        match self {
            Scalar::Str(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn unsigned(self) -> Option<u64> {
        match self {
            Scalar::Unsigned(n) => Some(n),
            _ => None,
        }
    }

    pub(crate) fn bool(self) -> Option<bool> {
        match self {
            Scalar::Bool(b) => Some(b),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for Scalar {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct ScalarVisitor;
        impl<'de> Visitor<'de> for ScalarVisitor {
            type Value = Scalar;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a JSON value")
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Scalar, E> {
                Ok(Scalar::Bool(v))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Scalar, E> {
                Ok(u64::try_from(v).map_or(Scalar::Other, Scalar::Unsigned))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Scalar, E> {
                Ok(Scalar::Unsigned(v))
            }

            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Scalar, E> {
                Ok(Scalar::Other)
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Scalar, E> {
                Ok(Scalar::Str(v.to_owned()))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Scalar, E> {
                Ok(Scalar::Str(v))
            }

            fn visit_unit<E: de::Error>(self) -> Result<Scalar, E> {
                Ok(Scalar::Null)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Scalar, A::Error> {
                while seq.next_element::<de::IgnoredAny>()?.is_some() {}
                Ok(Scalar::Other)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Scalar, A::Error> {
                while map.next_entry::<de::IgnoredAny, de::IgnoredAny>()?.is_some() {}
                Ok(Scalar::Other)
            }
        }
        d.deserialize_any(ScalarVisitor)
    }
}

pub(crate) fn too_new(kind: Kind, version: u32) -> Error {
    Error::TooNew { kind, version, supported: kind.supported_version() }
}

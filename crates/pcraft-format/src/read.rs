use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{Error, Limits, Manifest, Step, migrate, parse_manifest};

/// The members of the envelope other than `document`.
#[derive(Deserialize)]
struct Envelope {
    format_version: u32,
    generator: String,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    composite: Option<String>,
}

/// Reads manifest JSON into a [`Manifest`] whose document is `D`.
///
/// 1. [`parse_manifest`] checks `json` against `limits` and parses it.
/// 2. [`migrate`] refuses a `format_version` newer than `target` with [`Error::TooNew`], which
///    holds the manifest's `generator`, and upgrades an older one with `steps`.
/// 3. The document is read as `D`, then the other envelope members.
///
/// A missing `thumbnail` or `composite` reads as `None`, and members the envelope does not name
/// are skipped. A missing or mistyped member is [`Error::Json`].
pub fn read_manifest<D: DeserializeOwned, E: From<Error>>(json: &[u8], limits: &Limits, steps: &[Step<E>], target: u32) -> Result<Manifest<D>, E> {
    read_manifest_with(json, limits, steps, target, |document| serde_json::from_value(document).map_err(|e| E::from(Error::Json(e))))
}

/// [`read_manifest`], with the document read by `document` from its JSON value instead of by
/// `D`'s `Deserialize`, for example to decode a deep tree a level at a time.
pub fn read_manifest_with<D, E: From<Error>>(
    json: &[u8],
    limits: &Limits,
    steps: &[Step<E>],
    target: u32,
    document: impl FnOnce(Value) -> Result<D, E>,
) -> Result<Manifest<D>, E> {
    let mut value = parse_manifest(json, limits)?;
    migrate(&mut value, steps, target)?;
    // The document is read before the other members, and a missing document is reported after
    // their errors, in the order serde reports a struct's errors.
    let doc = value.as_object_mut().and_then(|object| object.remove("document")).map(document).transpose()?;
    let envelope = Envelope::deserialize(value).map_err(Error::Json)?;
    let doc = doc.ok_or_else(|| Error::Json(serde::de::Error::missing_field("document")))?;
    Ok(Manifest {
        format_version: envelope.format_version,
        generator: envelope.generator,
        document: doc,
        thumbnail: envelope.thumbnail,
        composite: envelope.composite,
    })
}
